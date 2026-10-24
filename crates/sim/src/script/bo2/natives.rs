//! The engine commands BO2's start-up scripts call.
//!
//! One table names every command; the catalog the linker checks scripts
//! against and the registry the runtime binds them to are both built from it,
//! so they cannot drift apart.

use bevy_ecs::prelude::{Resource, World};

use crate::frame::FrameWorld;
use crate::script::lang::{Builtin, Heap, Namespace, Value};
use crate::{ClientId, script_player};

/// What a command may touch while a script function runs: the world and the
/// script heap (the runtime keeps the heap outside the world for the run).
pub(crate) struct Ctx<'a> {
    pub(crate) world: &'a mut World,
    pub(crate) heap: &'a mut Heap,
}

/// A command: what it was sent to (`self`, or `level` for a plain call) and
/// its arguments.
pub(crate) type Native = fn(&mut Ctx<'_>, &Value, &[Value]) -> Result<Value, String>;

const COMMANDS: [(Namespace, &str, Native); 7] = [
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
    commands: Vec<(Namespace, &'static str, Native)>,
}

impl Default for NativeRegistry {
    fn default() -> Self {
        Self { commands: COMMANDS.to_vec() }
    }
}

impl NativeRegistry {
    /// `name` is lower case, as the catalog spells it.
    pub(crate) fn get(&self, space: Namespace, name: &str) -> Option<Native> {
        self.commands
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
fn target(world: &World, receiver: &Value) -> Result<ClientId, String> {
    match receiver {
        Value::Object(object) => super::runtime::client_of(world, *object).map(ClientId),
        _ => None,
    }
    .ok_or_else(|| "receiver is not a player".to_owned())
}

/// The weapon argument `at` names.
fn weapon_param(world: &mut World, args: &[Value], at: usize) -> Result<u32, String> {
    let name = text_param(args, at)?;
    script_player::weapon_named(&FrameWorld::from_world(world), &name)
}

/// `player spawn(origin, angles)`.
fn spawn_player(ctx: &mut Ctx<'_>, receiver: &Value, args: &[Value]) -> Result<Value, String> {
    let client = target(ctx.world, receiver)?;
    let origin = vector_param(args, 0)?;
    let angles = vector_param(args, 1)?;
    let state = super::runtime::session_state(ctx.world, client.0).ok_or("player has disconnected")?;
    let tick = ctx.world.resource::<crate::step::StepRequest>().tick;
    let mut frame = FrameWorld::from_world(ctx.world);
    script_player::spawn(&mut frame, tick, client, origin, angles, &state);
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
    script_player::give_weapon(&mut FrameWorld::from_world(ctx.world), client, weapon, akimbo)?;
    Ok(Value::Undefined)
}

/// `player setspawnweapon(name)`.
fn set_spawn_weapon(ctx: &mut Ctx<'_>, receiver: &Value, args: &[Value]) -> Result<Value, String> {
    let client = target(ctx.world, receiver)?;
    let weapon = weapon_param(ctx.world, args, 0)?;
    script_player::set_spawn_weapon(&mut FrameWorld::from_world(ctx.world), client, weapon)?;
    Ok(Value::Undefined)
}

/// `player setweaponammoclip(name, count)`.
fn set_ammo_clip(ctx: &mut Ctx<'_>, receiver: &Value, args: &[Value]) -> Result<Value, String> {
    let client = target(ctx.world, receiver)?;
    let weapon = weapon_param(ctx.world, args, 0)?;
    let rounds = int_param(args, 1)?;
    script_player::set_ammo_clip(&mut FrameWorld::from_world(ctx.world), client, weapon, rounds);
    Ok(Value::Undefined)
}

/// Offhand classes by number. Slot 0 means "no offhand".
const OFFHAND_CLASSES: [&str; 6] = ["none", "frag", "smoke", "flash", "throwingknife", "other"];

/// `player setoffhandprimaryclass(name)`: a class name, or a weapon whose
/// equipment row carries the class. Answers the class name it chose.
fn set_offhand_class(ctx: &mut Ctx<'_>, receiver: &Value, args: &[Value]) -> Result<Value, String> {
    let client = target(ctx.world, receiver)?;
    let name = text_param(args, 0)?;
    let mut frame = FrameWorld::from_world(ctx.world);
    let class = match OFFHAND_CLASSES[1..].iter().position(|known| *known == name) {
        Some(at) => at as i32 + 1,
        None => frame
            .weapon_index_by_script_name(&name)
            .and_then(|weapon| frame.offhand_loadout_row(weapon))
            .map_or(0, |row| row.offhand_class),
    };
    if let Some(player) = frame.player_mut(client) {
        player.offhand_primary = class;
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
