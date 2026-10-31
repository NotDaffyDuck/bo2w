//! The match's script state and how it runs. A match installs one program
//! (the BO2 start-up stubs), queues its entry functions, and from then on the
//! engine runs them at the start of the next authority tick. Nothing here
//! waits: a script function runs to its end in one go, so the state that
//! survives between ticks is only the heap, the players and the engine-side
//! things the stubs set up (map props, tracked missiles).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use bevy_ecs::prelude::{Res, Resource, World};

use super::level::{LevelData, Prop};
use super::natives::{Ctx, NativeRegistry};
use super::players::PlayerSlot;
use super::presence::Shown;
use super::weapons::Missile;
use crate::gsc_host::lang::{Fault, Heap, ScriptWorld, LEVEL, Location, Program, Value, play_call};

/// Run before the map's own `script_struct`s are appended to `level.struct`.
const STRUCT_INIT: &str = "codescripts/struct::initstructs";

/// An entry function waiting for the next authority tick.
#[derive(Clone, Debug)]
struct Queued {
    function: usize,
    receiver: Value,
    args: Vec<Value>,
}

#[derive(Resource, Clone, Debug, Default)]
pub(crate) struct Runtime {
    pub(crate) program: Option<Arc<Program>>,
    commands: Vec<fn(&mut Ctx<'_>, &Value, &[Value]) -> Result<Value, String>>,
    pub fault: Option<Fault>,
    pub(crate) fault_reported: bool,
    errors: BTreeMap<(String, String), u64>,
    pub(crate) latest_step_seen: Option<crate::StepNo>,
    pub(crate) started: bool,
    next_serial: u64,
    queue: Vec<Queued>,
    pub(crate) heap: Heap,
    pub(crate) dvars: BTreeMap<String, String>,
    signals: Vec<Arc<str>>,
    pub(crate) exit_level: bool,
    pub(crate) players: BTreeMap<u32, PlayerSlot>,
    pub(crate) personal_classes: BTreeMap<(u32, u32), crate::LoadoutClassSpec>,
    pub(crate) disconnects: BTreeSet<u32>,
    pub(crate) props: Vec<Prop>,
    /// Everything that takes an entity number: map entities, players, missiles.
    pub(crate) entity_count: usize,
    pub(crate) retired_presence: Vec<crate::ScriptedModel>,
    pub(crate) next_spawned_presence: u32,
    pub(crate) missiles: BTreeMap<crate::ThrownId, Missile>,
    pub(crate) missiles_seen_ms: i32,
    /// What the engine was last told about each script model.
    pub(crate) shown: BTreeMap<crate::ScriptedModel, Shown>,
}

impl Runtime {
    pub(crate) fn program_fingerprint(&self) -> Option<[u8; 32]> {
        self.program.as_ref().map(|program| program.fingerprint())
    }

    pub(crate) fn receiver_for(&self, object: u64) -> Option<u32> {
        self.players
            .iter()
            .find(|(_, slot)| slot.object == object)
            .map(|(&client, _)| client)
    }
}

/// The text of a dvar the scripts or the engine set; `name` is lower case.
pub(crate) fn dvar(world: &World, name: &str) -> Option<String> {
    world.resource::<Runtime>().dvars.get(name).cloned()
}

/// The client a player object stands for.
pub(crate) fn receiver_for(world: &World, object: u64) -> Option<u32> {
    world.resource::<Runtime>().receiver_for(object)
}

/// The session state a client's scripts last declared.
pub(crate) fn session_state(world: &World, client: u32) -> Option<Arc<str>> {
    world
        .resource::<Runtime>()
        .players
        .get(&client)
        .map(|slot| slot.sessionstate.clone())
}

fn spot(module: &str, function: &str) -> Location {
    Location::engine(module, function)
}

/// What the script heap and the engine commands see while a function runs.
struct WorldHost<'w> {
    world: &'w mut World,
    heap: Heap,
    commands: Vec<fn(&mut Ctx<'_>, &Value, &[Value]) -> Result<Value, String>>,
    last_fault: Option<Fault>,
}

impl ScriptWorld for WorldHost<'_> {
    fn heap(&mut self) -> &mut Heap {
        &mut self.heap
    }

    fn native(&mut self, slot_at: usize, receiver: &Value, args: &[Value]) -> Result<Value, String> {
        let command = self.commands[slot_at];
        command(
            &mut super::natives::Ctx { world: &mut *self.world, heap: &mut self.heap },
            receiver,
            args,
        )
    }

    fn load_field(&mut self, object: u64, name: &str) -> Option<Value> {
        super::players::load_field(self.world, object, name)
    }

    fn store_field(&mut self, object: u64, name: &str, value: &Value) -> Result<bool, String> {
        super::players::store_field(self.world, object, name, value)
    }

    fn raised(&mut self, receiver: &Value, event: &str) {
        if matches!(receiver, Value::Object(LEVEL)) {
            self.world.resource_mut::<Runtime>().signals.push(event.into());
        }
    }

    fn report(&mut self, fault: Fault) {
        report(self.world, &fault);
        self.last_fault = Some(fault);
    }
}

/// Run one function to its end. Script mistakes are reported and the function
/// carries on; a run that had to be cut off stops the match.
fn call(world: &mut World, program: &Program, function: usize, receiver: Value, args: Vec<Value>) {
    let (heap, commands) = {
        let mut runtime = world.resource_mut::<Runtime>();
        (std::mem::take(&mut runtime.heap), runtime.commands.clone())
    };
    let mut host = WorldHost { world, heap, commands, last_fault: None };
    let done_now = play_call(program, &mut host, function, receiver, args).is_some();
    let WorldHost { heap, last_fault, .. } = host;
    let mut runtime = world.resource_mut::<Runtime>();
    runtime.heap = heap;
    if !done_now {
        runtime.fault = last_fault.or_else(|| {
            Some(Fault::at(&spot("<runtime>", "run"), "script run cut off"))
        });
    }
}

fn report(world: &mut World, fault: &Fault) {
    let site = format!(
        "{}:{}:{}",
        fault.location.module, fault.location.line, fault.location.column
    );
    let mut runtime = world.resource_mut::<Runtime>();
    let hits = runtime
        .errors
        .entry((site.clone(), fault.message.clone()))
        .or_default();
    *hits += 1;
    if *hits == 1 {
        diag::warn!(Sim, "gsc: script runtime error: {fault}");
        diag::script_boundary(
            "runtime_error",
            &format!(
                " at={site} function={} fault=\"{}\"",
                fault.location.function,
                fault.message.replace('"', "'")
            ),
        );
    }
}

/// The program and function a named entry point refers to.
fn entry(world: &World, name: &str) -> Result<(Arc<Program>, usize), Fault> {
    let here = spot("<runtime>", name);
    let runtime = world.resource::<Runtime>();
    if let Some(fault) = &runtime.fault {
        return Err(fault.clone());
    }
    let program = runtime
        .program
        .clone()
        .ok_or_else(|| Fault::at(&here, "no loaded GSC program"))?;
    let function = program
        .find(name)
        .ok_or_else(|| Fault::at(&here, "unknown script entry point"))?;
    Ok((program, function))
}

/// Queue `name` to run at the start of the next authority tick.
pub(crate) fn start(
    world: &mut World,
    name: &str,
    receiver: Value,
    args: Vec<Value>,
) -> Result<u64, Fault> {
    let (_, function) = entry(world, name)?;
    let mut runtime = world.resource_mut::<Runtime>();
    let serial = runtime.next_serial;
    runtime.next_serial = serial
        .checked_add(1)
        .ok_or_else(|| Fault::at(&spot("<runtime>", name), "thread identifier exhausted"))?;
    runtime.started = true;
    runtime.queue.push(Queued { function, receiver, args });
    Ok(serial)
}

/// Run `name` right now, to its end.
pub(super) fn run_now(
    world: &mut World,
    name: &str,
    receiver: Value,
    args: Vec<Value>,
) -> Result<(), Fault> {
    let (program, function) = entry(world, name)?;
    world.resource_mut::<Runtime>().started = true;
    call(world, &program, function, receiver, args);
    match world.resource::<Runtime>().fault.clone() {
        Some(fault) => Err(fault),
        None => Ok(()),
    }
}

pub(crate) fn seat_program(
    world: &mut World,
    program: Program,
    natives: NativeRegistry,
    level: LevelData,
) -> Result<(), Fault> {
    let here = spot("<runtime>", "install");
    if world.resource::<Runtime>().program.is_some() {
        return Err(Fault::at(
            &here,
            "program already installed; reset the match before replacing scripts",
        ));
    }
    let mut unbound: Vec<&str> = program
        .natives()
        .iter()
        .filter(|builtin| natives.get(builtin.namespace, builtin.name).is_none())
        .map(|builtin| builtin.name)
        .collect();
    unbound.sort_unstable();
    if !unbound.is_empty() {
        return Err(Fault::at(
            &here,
            format!("{} unbound natives: {}", unbound.len(), unbound.join(" ")),
        ));
    }
    let program = Arc::new(program);
    {
        let mut runtime = world.resource_mut::<Runtime>();
        runtime.commands = program
            .natives()
            .iter()
            .filter_map(|builtin| natives.get(builtin.namespace, builtin.name))
            .collect();
        runtime.program = Some(program.clone());
    }
    world.insert_resource(natives);
    if let Some(function) = program.find(STRUCT_INIT) {
        call(world, &program, function, Value::level(), Vec::new());
        if let Some(fault) = world.resource::<Runtime>().fault.clone() {
            return Err(fault);
        }
        world.resource_mut::<Runtime>().started = false;
    }
    let props = super::level::spawn_map(&level).map_err(|message| Fault::at(&here, message))?;
    let mut runtime = world.resource_mut::<Runtime>();
    runtime.entity_count = props.entities;
    runtime.props = props.props;
    Ok(())
}

pub(crate) fn take_signals(world: &mut World) -> Vec<Arc<str>> {
    std::mem::take(&mut world.resource_mut::<Runtime>().signals)
}

pub(crate) fn healthy(runtime: Res<Runtime>) -> bool {
    runtime.fault.is_none()
}

fn scheduler_fault() -> Fault {
    Fault::at(
        &spot("<scheduler>", ""),
        "authority script ticks must increase; restore the runtime before replaying",
    )
}

/// Refuse a tick the scripts cannot take: after a fault, out of order, or
/// before anything has been started.
pub(crate) fn preflight(
    world: &World,
    tick: crate::StepNo,
    reason: crate::AdvanceCause,
) -> Result<(), Fault> {
    let runtime = world.resource::<Runtime>();
    if let Some(fault) = &runtime.fault {
        return Err(fault.clone());
    }
    if !reason.moves_server_world_forward() {
        return Ok(());
    }
    if runtime.latest_step_seen.is_some_and(|last| last.0 >= tick.0) {
        return Err(scheduler_fault());
    }
    if !runtime.started || runtime.program.is_none() {
        return Err(Fault::at(
            &spot("<runtime>", "step"),
            "no loaded and started GSC program",
        ));
    }
    Ok(())
}

/// The start of an authority tick: run what was queued since the last one.
pub(crate) fn advance_scheduler(world: &mut World) {
    let request = world.resource::<crate::frame_step::FrameTicket>();
    if !request.reason.moves_server_world_forward() {
        return;
    }
    let tick = request.tick;
    let runtime = world.resource::<Runtime>();
    let Some(program) = runtime.program.clone() else {
        return;
    };
    if runtime.fault.is_some() {
        return;
    }
    if runtime.latest_step_seen.is_some_and(|last| last.0 >= tick.0) {
        world.resource_mut::<Runtime>().fault = Some(scheduler_fault());
        return;
    }
    let queued = {
        let mut runtime = world.resource_mut::<Runtime>();
        runtime.latest_step_seen = Some(tick);
        std::mem::take(&mut runtime.queue)
    };
    for Queued { function, receiver, args } in queued {
        call(world, &program, function, receiver, args);
        if world.resource::<Runtime>().fault.is_some() {
            break;
        }
    }
}

pub(crate) fn set_dvar(world: &mut World, name: &str, setting: &str) {
    world
        .resource_mut::<Runtime>()
        .dvars
        .insert(name.to_ascii_lowercase(), setting.to_owned());
}

pub(crate) fn copy_state(source: &World, target: &mut World) {
    target.insert_resource(source.resource::<Runtime>().clone());
    target.insert_resource(source.resource::<super::controls::Mechanics>().clone());
    target.insert_resource(source.resource::<NativeRegistry>().clone());
}

pub(crate) fn reset(world: &mut World) {
    world.insert_resource(Runtime::default());
    world.insert_resource(super::controls::Mechanics::default());
}
