//! bo2mp: the console's `bot add N` and `bot hold on|off`, multiplayer only.
//!
//! `bot add` asks BO2's own bot script for each bot: `spawn_bot(team)` in
//! maps/mp/bots/_bot, the call its lobby fill makes, so the bot connects
//! through `addtestclient`, picks a class and plays with BO2's bot brain like
//! any other. The team follows the mod menu's "Add Enemy/Friendly Bot": an
//! enemy goes to the side opposite the host, a friend to the host's side,
//! enemies first so `bot add 1` gives a target and `bot add 2` one of each.
//!
//! `bot hold` has no BO2 counterpart that lasts across a respawn, so it is a
//! flag the bot driver reads: while it is on, every bot's command keeps its
//! view and loses its movement and buttons.

use bevy_ecs::prelude::{Resource, World};
use gsc_t6::{Key, Value, Vm};

use super::{Zm, with_vm};

#[derive(Resource, Default)]
struct ConsoleBots {
    /// Bots asked for that the script has not been asked to spawn yet.
    waiting: u32,
    /// `bot hold on`.
    frozen: bool,
}

pub(crate) fn ask(world: &mut World, count: u32) {
    let mut asks = world.get_resource_or_insert_with(ConsoleBots::default);
    asks.waiting = asks.waiting.saturating_add(count);
}

pub(crate) fn hold(world: &mut World, on: bool) {
    world.get_resource_or_insert_with(ConsoleBots::default).frozen = on;
}

/// Is `bot hold` on?
pub(super) fn frozen(world: &World) -> bool {
    world.get_resource::<ConsoleBots>().is_some_and(|c| c.frozen)
}

/// The side client `n` plays on: its `team`, or before it has one the
/// `pers["team"]` that `spawn_bot` gave it. Empty for none (or spectator).
fn side_of(vm: &mut Vm<World>, world: &World, n: u32) -> String {
    let Some(obj) = world.resource::<Zm>().players.get(&n).map(|p| p.obj) else {
        return String::new();
    };
    let team_field = vm.intern("team");
    let mut side = vm.raw_field(obj, team_field);
    if matches!(side, Value::Undefined) {
        let pers_field = vm.intern("pers");
        if let Value::Array(pers) = vm.raw_field(obj, pers_field) {
            let key = Key::Str(team_field);
            side = pers.get(&key).unwrap_or(Value::Undefined);
        }
    }
    let side = vm.to_text(&side);
    if side == "spectator" { String::new() } else { side }
}

/// Hand every waiting bot to BO2's `spawn_bot`.
pub(super) fn spawn_waiting(world: &mut World) {
    let waiting = match world.get_resource_mut::<ConsoleBots>() {
        Some(mut asks) if asks.waiting > 0 => std::mem::take(&mut asks.waiting),
        _ => return,
    };
    if !world.resource::<Zm>().mp {
        diag::warn!(Sim, "bo2mp bot: console add {waiting} dropped, bot add is multiplayer only");
        return;
    }
    let done = with_vm(world, |vm, world| {
        // The host is client 0 (BO2's gethostplayerforbots); with no side
        // yet it counts as allies, as BO2's bot fill does.
        let host = match side_of(vm, world, 0) {
            side if side.is_empty() => "allies".to_owned(),
            side => side,
        };
        let other = if host == "allies" { "axis" } else { "allies" };
        let teams_field = vm.intern("teambased");
        let teams = gsc_t6::truthy(&vm.raw_field(vm.level, teams_field));
        // Bots already on each side, so a second `bot add` keeps them even.
        let (mut friends, mut enemies) = (0u32, 0u32);
        let ids: Vec<u32> = world.resource::<Zm>().bots.keys().copied().collect();
        for n in ids {
            match side_of(vm, world, n) {
                side if side.is_empty() => {}
                side if teams && side == host => friends += 1,
                _ => enemies += 1,
            }
        }
        // After the menu's "Kick All Bots" the script refuses new bots; a
        // console add still gets one, as the menu's own add does.
        let gate = vm.intern("menu_nobots");
        let kept = vm.raw_field(vm.level, gate);
        vm.set_raw_field(vm.level, gate, Value::Int(0));
        let mut spawned = 0;
        for _ in 0..waiting {
            let team = if !teams || enemies <= friends {
                enemies += 1;
                other
            } else {
                friends += 1;
                host.as_str()
            };
            let arg = vm.string(team);
            if vm.spawn_named(world, "maps/mp/bots/_bot", "spawn_bot", Value::Undefined, vec![arg]).is_none() {
                break;
            }
            spawned += 1;
            diag::info!(Sim, "bo2mp bot: console add on {team}");
        }
        vm.set_raw_field(vm.level, gate, kept);
        spawned
    });
    if done != Some(waiting) {
        diag::warn!(Sim, "bo2mp bot: console add {waiting}, the bot script took {}", done.unwrap_or(0));
    }
}
