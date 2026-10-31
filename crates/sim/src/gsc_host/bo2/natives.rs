//! The engine commands BO2's start-up scripts call.
//!
//! One table names every command; the catalog the linker checks scripts
//! against and the registry the runtime binds them to are both built from it,
//! so they cannot drift apart.

use bevy_ecs::prelude::{Resource, World};

use crate::step_view::StepWorldView;
use crate::gsc_host::lang::{Builtin, Heap, Namespace, Value};
use crate::{SeatNo, client_script};

/// What a command may touch while a script function runs: the world and the
/// script heap (the runtime keeps the heap outside the world for the run).
pub(crate) struct Ctx<'a> {
    pub(crate) world: &'a mut World,
    pub(crate) heap: &'a mut Heap,
}

const COMMANDS: [(Namespace, &str, fn(&mut Ctx<'_>, &Value, &[Value]) -> Result<Value, String>); 7] = [
    (Namespace::Function, "getdvar", get_dvar),
    (Namespace::Function, "spawnstruct", spawn_struct),
    (Namespace::Method, "spawn", spawn_player),
    (Namespace::Method, "giveweapon", give_weapon),
    (Namespace::Method, "setspawnweapon", set_spawn_weapon),
    (Namespace::Method, "setweaponammoclip", set_ammo_clip),
    (Namespace::Method, "setoffhandprimaryclass", set_offhand_class),
];

/// The commands the BO2 catalog lists.
pub(super) fn builtins() -> Vec<Builtin> {
    COMMANDS.iter().map(|&(space, name, _)| Builtin::new(space, name)).collect()
}

/// The commands a program is bound against when it is installed.
#[derive(Resource, Clone)]
pub struct NativeRegistry {
    bound: Vec<(Namespace, &'static str, fn(&mut Ctx<'_>, &Value, &[Value]) -> Result<Value, String>)>,
}

impl Default for NativeRegistry {
    fn default() -> Self {
        Self { bound: COMMANDS.to_vec() }
    }
}

impl NativeRegistry {
    /// `name` is lower case, as the catalog spells it.
    pub(crate) fn get(&self, space: Namespace, name: &str) -> Option<fn(&mut Ctx<'_>, &Value, &[Value]) -> Result<Value, String>> {
        self.bound
            .iter()
            .find(|(s, n, _)| *s == space && *n == name)
            .map(|&(_, _, command)| command)
    }
}

fn param(args: &[Value], at: usize) -> Result<&Value, String> {
    args.get(at).ok_or_else(|| format!("parameter {} does not exist", at + 1))
}

fn int_param(args: &[Value], at: usize) -> Result<i32, String> {
    match param(args, at)? {
        Value::Int(n) => Ok(*n),
        Value::Float(x) => Ok(*x as i32),
        other => Err(format!("parameter {} is {}, not an int", at + 1, other.kind())),
    }
}

fn vector_param(args: &[Value], at: usize) -> Result<[f32; 3], String> {
    match param(args, at)? {
        Value::Vector(v) => Ok(*v),
        other => Err(format!("parameter {} is {}, not a vector", at + 1, other.kind())),
    }
}

fn text_param(args: &[Value], at: usize) -> Result<String, String> {
    param(args, at)?
        .to_text()
        .ok_or_else(|| format!("parameter {} cannot be cast to string", at + 1))
}

/// `getdvar(name [, fallback])`: the dvar's text, else the fallback, else "".
fn get_dvar(ctx: &mut Ctx<'_>, _: &Value, args: &[Value]) -> Result<Value, String> {
    let fallback = match args.len() {
        1 => String::new(),
        2 => text_param(args, 1)?,
        _ => return Err("wrong number of parameters".into()),
    };
    let name = text_param(args, 0)?.to_ascii_lowercase();
    let text = super::runtime::dvar(ctx.world, &name).unwrap_or(fallback);
    Ok(Value::string(&text))
}

/// `spawnstruct()`: a fresh object with no fields.
fn spawn_struct(ctx: &mut Ctx<'_>, _: &Value, args: &[Value]) -> Result<Value, String> {
    if !args.is_empty() {
        return Err("spawnstruct expects no arguments".into());
    }
    ctx.heap.new_object().map(Value::Object)
}

/// The client a player command was sent to.
fn target(world: &World, receiver: &Value) -> Result<SeatNo, String> {
    match receiver {
        Value::Object(object) => super::runtime::receiver_for(world, *object).map(SeatNo),
        _ => None,
    }
    .ok_or_else(|| "receiver is not a player".to_owned())
}

/// The weapon argument `at` names.
fn weapon_param(world: &mut World, args: &[Value], spot_at: usize) -> Result<u32, String> {
    let name = text_param(args, spot_at)?;
    client_script::weapon_named(&StepWorldView::from_world(world), &name)
}

/// `player spawn(origin, angles)`.
fn spawn_player(ctx: &mut Ctx<'_>, receiver: &Value, args: &[Value]) -> Result<Value, String> {
    let client = target(ctx.world, receiver)?;
    let origin = vector_param(args, 0)?;
    let angles = vector_param(args, 1)?;
    let state = super::runtime::session_state(ctx.world, client.0).ok_or("player has disconnected")?;
    let tick = ctx.world.resource::<crate::frame_step::FrameTicket>().tick;
    let mut frame = StepWorldView::from_world(ctx.world);
    client_script::spawn(&mut frame, tick, client, origin, angles, &state);
    Ok(Value::Undefined)
}

/// `player giveweapon(name [, model, akimbo])`.
fn give_weapon(ctx: &mut Ctx<'_>, receiver: &Value, args: &[Value]) -> Result<Value, String> {
    let client = target(ctx.world, receiver)?;
    let weapon = weapon_param(ctx.world, args, 0)?;
    let akimbo = match args.get(2) {
        None | Some(Value::Undefined) => false,
        Some(_) => int_param(args, 2)? != 0,
    };
    client_script::give_weapon(&mut StepWorldView::from_world(ctx.world), client, weapon, akimbo)?;
    Ok(Value::Undefined)
}

/// `player setspawnweapon(name)`.
fn set_spawn_weapon(ctx: &mut Ctx<'_>, receiver: &Value, args: &[Value]) -> Result<Value, String> {
    let client = target(ctx.world, receiver)?;
    let weapon = weapon_param(ctx.world, args, 0)?;
    client_script::set_spawn_weapon(&mut StepWorldView::from_world(ctx.world), client, weapon)?;
    Ok(Value::Undefined)
}

/// `player setweaponammoclip(name, count)`.
fn set_ammo_clip(ctx: &mut Ctx<'_>, receiver: &Value, args: &[Value]) -> Result<Value, String> {
    let client = target(ctx.world, receiver)?;
    let weapon = weapon_param(ctx.world, args, 0)?;
    let rounds = int_param(args, 1)?;
    client_script::set_ammo_clip(&mut StepWorldView::from_world(ctx.world), client, weapon, rounds);
    Ok(Value::Undefined)
}

/// Offhand classes by number. Slot 0 means "no offhand".
const OFFHAND_CLASSES: [&str; 6] = ["none", "frag", "smoke", "flash", "throwingknife", "other"];

/// `player setoffhandprimaryclass(name)`: a class name, or a weapon whose
/// equipment row carries the class. Answers the class name it chose.
fn set_offhand_class(ctx: &mut Ctx<'_>, receiver: &Value, args: &[Value]) -> Result<Value, String> {
    let client = target(ctx.world, receiver)?;
    let name = text_param(args, 0)?;
    let mut frame = StepWorldView::from_world(ctx.world);
    let class = match OFFHAND_CLASSES[1..].iter().position(|known| *known == name) {
        Some(at) => at as i32 + 1,
        None => frame
            .gun_for_script_label(&name)
            .and_then(|weapon| frame.side_slot_row(weapon))
            .map_or(0, |row| row.secondary_hand_category),
    };
    if let Some(player) = frame.occupant_mut(client) {
        player.secondary_gear_main = class;
    }
    let label = usize::try_from(class).ok().and_then(|at| OFFHAND_CLASSES.get(at));
    Ok(Value::string(label.copied().unwrap_or("none")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_command_is_bound() {
        let registry = NativeRegistry::default();
        for builtin in builtins() {
            assert!(registry.get(builtin.namespace, builtin.name).is_some(), "{}", builtin.name);
        }
        assert!(registry.get(Namespace::Function, "giveweapon").is_none());
    }

    #[test]
    fn parameters_say_what_is_wrong() {
        let args = [Value::Float(2.9), Value::string("x")];
        assert_eq!(int_param(&args, 0), Ok(2));
        assert_eq!(int_param(&args, 1).unwrap_err(), "parameter 2 is a string, not an int");
        assert_eq!(int_param(&args, 2).unwrap_err(), "parameter 3 does not exist");
        assert_eq!(vector_param(&args, 0).unwrap_err(), "parameter 1 is a float, not a vector");
        assert_eq!(text_param(&[Value::Object(5)], 0).unwrap_err(), "parameter 1 cannot be cast to string");
    }
}
