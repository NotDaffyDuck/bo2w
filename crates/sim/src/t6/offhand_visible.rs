//! bo2mp gun brain, window 5 (gun/offhand): BO2's `setoffhandvisible`. The scripts turn it on
//! when he pulls back a piece of equipment and off when it is thrown, planted or put away
//! (maps/mp/gametypes/_weapons: `watchgrenadeusage` / `watchoffhandend`).
//!
//! BO2's rule: the builtin only sets a flag on his client state. The others see the offhand
//! item in his hand (its first world model, on `tag_inhand`, next to his gun) while that flag
//! is set and his offhand is a piece of equipment (claymore, C4, Betty, tac insert ...):
//! grenades and tacticals never show this way, and nothing shows when the item has no world
//! model. Nothing changes in his own first-person view.
//!
//! The scripts keep the flag on while their `isusingoffhandequipment` holds: `isusingoffhand`,
//! then `getcurrentoffhand` and `isweaponequipment` on its name. BO2's answers to those two,
//! bound here on the T6 state: the current offhand weapon's name ("none" without one), and
//! "its offhand slot is equipment". The older answers (MW2's offhand class, the inventory type)
//! stay for the legacy brain and maps without BO2 facts.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use bevy_ecs::prelude::{Resource, World};
use gsc_t6::{Native, Value, Vm};
use weapon_t6::facts::offhand_slot;

use super::playeranim::edit_body;
use super::{Zm, client, frame, int, text, weapon, weapon_text};
use crate::t6_gun;
use crate::world::ClientId;

/// The tag BO2 hangs the offhand item on.
const INHAND_TAG: &str = "tag_inhand";

#[derive(Resource, Default)]
pub(crate) struct OffhandVisible {
    /// Each player's flag as the scripts last set it.
    on: BTreeMap<u32, bool>,
    /// The model each player's body was last sent with in his hand.
    shown: BTreeMap<u32, Option<String>>,
}

/// The `getcurrentoffhand` / `isweaponequipment` bound before this file's.
static PREV_OFFHAND: OnceLock<Option<Native<World>>> = OnceLock::new();
static PREV_EQUIPMENT: OnceLock<Option<Native<World>>> = OnceLock::new();

fn prev(
    f: &OnceLock<Option<Native<World>>>,
    vm: &mut Vm<World>,
    world: &mut World,
    s: &Value,
    a: &[Value],
) -> Result<Value, String> {
    match f.get().copied().flatten() {
        Some(f) => f(vm, world, s, a),
        None => Ok(Value::Undefined),
    }
}

pub(super) fn bind(vm: &mut Vm<World>) {
    let _ = PREV_OFFHAND.set(vm.bound("getcurrentoffhand", true));
    let _ = PREV_EQUIPMENT.set(vm.bound("isweaponequipment", false));
    vm.bind("getcurrentoffhand", true, |vm, world, s, a| {
        let id = client(vm, world, s)?;
        match t6_gun::script_read(&frame(world), id, |ps, _| ps.off_hand_weapon) {
            Some(0) => Ok(vm.string("none")),
            Some(w) => {
                let n = weapon_text(world, w);
                Ok(vm.string(&n))
            }
            None => prev(&PREV_OFFHAND, vm, world, s, a),
        }
    });
    vm.bind("isweaponequipment", false, |vm, world, s, a| {
        if frame(world).gun_facts().is_empty() {
            return prev(&PREV_EQUIPMENT, vm, world, s, a);
        }
        let w = weapon(world, &text(vm, a, 0)).unwrap_or(0);
        let slot = frame(world).gun_facts().get(w).core.offhand_slot;
        Ok(Value::Int(i32::from(w != 0 && slot == offhand_slot::EQUIPMENT)))
    });
    vm.bind("setoffhandvisible", true, |vm, world, s, a| {
        let id = client(vm, world, s)?;
        let on = int(a, 0)? != 0;
        if world.get_resource::<OffhandVisible>().is_none() {
            world.insert_resource(OffhandVisible::default());
        }
        let was = world.resource_mut::<OffhandVisible>().on.insert(id.0, on);
        if was != Some(on) && std::env::var_os("BO2MP_BODYLOG").is_some() {
            let why = what_he_holds(world, id.0);
            diag::info!(Sim, "bo2mp offhand visible {}: {} ({why})", id.0, u8::from(on));
        }
        Ok(Value::Undefined)
    });
}

/// The item each player's body shows in his hand, sent when it changes.
pub(super) fn advance(world: &mut World) {
    let Some(flags) = world.get_resource::<OffhandVisible>().map(|v| v.on.clone()) else {
        return;
    };
    let clients: Vec<u32> = world.resource::<Zm>().players.keys().copied().collect();
    for c in clients {
        let want = flags.get(&c).copied().unwrap_or(false).then(|| in_hand(world, c)).flatten();
        let shown = world.resource::<OffhandVisible>().shown.get(&c).cloned().flatten();
        if want == shown {
            continue;
        }
        if std::env::var_os("BO2MP_BODYLOG").is_some() {
            diag::info!(Sim, "bo2mp offhand in hand {c}: {want:?}");
        }
        edit_body(world, c, |l| l.inhand = want.clone().map(|m| (m, INHAND_TAG)));
        world.resource_mut::<OffhandVisible>().shown.insert(c, want);
    }
}

/// His offhand item's world model, when it is a piece of equipment that has one.
fn in_hand(world: &mut World, c: u32) -> Option<String> {
    let f = frame(world);
    let offhand = u32::try_from(f.player(ClientId(c))?.off_hand_index).ok().filter(|&w| w != 0)?;
    if f.gun_facts().get(offhand).core.offhand_slot != offhand_slot::EQUIPMENT {
        return None;
    }
    let model = f.weapon_world_model(offhand).map(|(m, _)| m.to_owned())?;
    (!model.is_empty()).then_some(model)
}

/// The log's view of his offhand: weapon index, BO2 offhand slot, first world model.
fn what_he_holds(world: &mut World, c: u32) -> String {
    let f = frame(world);
    let Some(ps) = f.player(ClientId(c)) else {
        return "no player".to_owned();
    };
    let w = u32::try_from(ps.off_hand_index).unwrap_or(0);
    let slot = f.gun_facts().get(w).core.offhand_slot;
    let model = f.weapon_world_model(w).map(|(m, _)| m.to_owned());
    format!("offhand {w}, slot {slot}, model {model:?}")
}
