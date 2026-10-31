//! A player's grenade hand as BO2's engine shows it to the scripts:
//! `isthrowinggrenade` (a grenade or other offhand is out), and
//! `isswitchingweapons`. TranZit's Denizen grab calls `forcegrenadethrow`
//! on a player holding a cooked grenade: the grenade leaves his hand as if
//! he let go of the button, and his hands go to the Denizen.

use super::pairs::KeepByValue;
use bevy_ecs::prelude::World;
use gsc_t6::{ScriptValue, Vm};

use super::{Zm, client, frame};
use crate::sim_parts::ClientId;

const OFFHAND: u32 = playerstate_t6::game::buttons::SAMPLE_FRAG_ASSET | playerstate_t6::game::buttons::OBSCURANT;

fn state(world: &mut World, id: ClientId) -> i32 {
    frame(world).avatar(id).map_or(0, |ps| ps.weaponstate_primary)
}

/// Taking the grenade out, cooking or holding it: the throw waits on him.
fn held_back(state: i32) -> bool {
    matches!(
        state,
        weapon_t6::state::OFFHAND_INIT | weapon_t6::state::OFFHAND_PREPARE | weapon_t6::state::OFFHAND_HOLD
    )
}

pub(super) fn bind(vm: &mut Vm<World>) {
    vm.bind("isthrowinggrenade", true, |vm, world, s, _| {
        let id = client(vm, world, s)?;
        Ok(ScriptValue::bool(matches!(
            state(world, id),
            weapon_t6::state::OFFHAND_INIT
                | weapon_t6::state::OFFHAND_PREPARE
                | weapon_t6::state::OFFHAND_HOLD
                | weapon_t6::state::OFFHAND_START
                | weapon_t6::state::OFFHAND
                | weapon_t6::state::OFFHAND_END
        )))
    });
    vm.bind("isswitchingweapons", true, |vm, world, s, _| {
        let id = client(vm, world, s)?;
        Ok(ScriptValue::bool((weapon_t6::state::RAISING..=weapon_t6::state::DROPPING_ALTSWITCH).contains(
            &state(world, id),
        )))
    });
    // player forcegrenadethrow(): a grenade he is holding goes now.
    vm.bind("forcegrenadethrow", true, |vm, world, s, _| {
        let id = client(vm, world, s)?;
        if held_back(state(world, id)) {
            if let Some(p) = world.resource_mut::<Zm>().players.get_mut(&id.0) {
                p.force_throw = true;
            }
        }
        Ok(ScriptValue::Undefined)
    });
}

/// Each tick, before the weapon step reads his buttons: a forced throw
/// lets go of the grenade button for him, until the grenade is gone and he
/// has let go himself (so the button still down is not a fresh throw).
pub(super) fn tick(world: &mut World) {
    let forced: Vec<u32> = world
        .resource::<Zm>()
        .players
        .iter()
        .where_value(|pt| pt.force_throw)
        .map(|(c, _)| *c)
        .collect();
    for c in forced {
        let id = ClientId(c);
        let mut down = false;
        {
            let mut req = world.resource_mut::<crate::frame_step::FrameTicket>();
            for (who, cmd) in &mut req.input.cmds {
                if *who == id {
                    down |= cmd.buttons & OFFHAND != 0;
                    cmd.buttons &= !OFFHAND;
                }
            }
        }
        if !down && !held_back(state(world, id)) {
            if let Some(pt) = world.resource_mut::<Zm>().players.get_mut(&c) {
                pt.force_throw = false;
            }
        }
    }
}
