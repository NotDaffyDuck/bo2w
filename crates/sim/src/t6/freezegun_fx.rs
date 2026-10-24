//! Black Ops 1's Winter's Howl, the client half
//! (clientscripts/mp/zombies/_zm_weap_freezegun.csc on the Declassified
//! maps): the frost on a frozen zombie's elbows, knees and lower spine, and
//! its end, `level._gib_overload_func` = freezegun_gib_override, which
//! turns the server's `gib("freeze")` into the frozen body crumbling and
//! `gib("up")` into it shattering. The server scripts do the damage, the
//! freeze death and the shatter triggers.

use bevy_ecs::prelude::World;
use gsc_t6::{ObjRef, Value, Vm};

use super::Zm;
use crate::EventAudience;

const FROST_LIMB: &str = "maps/zombie/fx_zombie_freeze_md";
const FROST_TORSO: &str = "maps/zombie/fx_zombie_freeze_torso";
const SHATTER: &str = "weapon/freeze_gun/fx_freezegun_shatter";
const CRUMPLE: &str = "weapon/freeze_gun/fx_freezegun_crumple";
/// The upgraded gun's shatter and crumple are both its impact burst.
const UPGRADED: &str = "weapon/freeze_gun/fx_exp_freezegun_impact";
const SHATTER_GIB: &str = "weapon/bullet/fx_flesh_gib_fatal_01";
const SHATTER_TRAIL: &str = "weapon/freeze_gun/fx_trail_freezegun_blood_streak";

const EXTREMITY: &str = "freezegun_extremity_damage_fx";
const TORSO: &str = "freezegun_torso_damage_fx";
/// freezegun_play_all_extremity_damage_fx: the frost on each limb's joint.
const LIMBS: [&str; 4] = ["J_Elbow_RI", "J_Elbow_LE", "J_Knee_RI", "J_Knee_LE"];

/// How the frozen body ends: the gib pieces' blood, trail and throw.
pub(super) struct Freeze {
    pub(super) shatter: bool,
}

impl Freeze {
    /// freezegun_get_gibfx: blood where the limb was on a shatter only.
    pub(super) fn gib_fx(&self) -> Option<&'static str> {
        self.shatter.then_some(SHATTER_GIB)
    }

    /// freezegun_get_gibtrailfx: the blood streak behind a shattered limb.
    pub(super) fn trail(&self) -> Option<&'static str> {
        self.shatter.then_some(SHATTER_TRAIL)
    }
}

/// Each tag's frost off, then on again when `on`. The torso's goes at once
/// (freezegun_end_all_torso_damage_fx: `deletefx(lc, fx, 1)`, its ice shell
/// with it); the limbs' only stop (their deletefx takes no 1).
fn frost(world: &mut World, n: u32, tags: &[&'static str], fx: &'static str, on: bool) {
    for &tag in tags {
        if fx == FROST_TORSO {
            super::natives_fx::kill_bolted(world, n, fx, tag);
        } else {
            super::natives_fx::bolt_effect(world, n, fx, tag, true);
        }
        if on {
            super::clientfields::bolt_queued(world, n, fx, tag, false);
        }
    }
}

/// An actor's freeze field changed. True when it was one of these.
pub(super) fn entity_field(world: &mut World, n: u32, name: &str, value: i64) -> bool {
    match name {
        EXTREMITY => frost(world, n, &LIMBS, FROST_LIMB, value != 0),
        TORSO => frost(world, n, &["J_SpineLower"], FROST_TORSO, value != 0),
        _ => return false,
    }
    true
}

/// freezegun_gib_override: a `gib("freeze")` or `gib("up")` on a zombie
/// the freeze reached (its frost fields set) takes the frost off and its
/// eyes out, and plays the crumble or the shatter with its sound; the
/// pieces then fall (crumple) or burst from the spine (shatter).
pub(super) fn gib_override(vm: &mut Vm<World>, world: &mut World, n: u32, obj: ObjRef, kind: &str) -> Option<Freeze> {
    if kind != "freeze" && kind != "up" {
        return None;
    }
    let frozen = {
        let fx = &world.resource::<Zm>().client_fx;
        fx.value(n, EXTREMITY) != 0 || fx.value(n, TORSO) != 0
    };
    if !frozen {
        return None;
    }
    let shatter = kind == "up";
    let field = vm.intern("damageweapon");
    let upgraded = match vm.raw_field(obj, field) {
        Value::Undefined => false,
        v => vm.to_text(&v) == "freezegun_upgraded_zm",
    };
    frost(world, n, &LIMBS, FROST_LIMB, false);
    frost(world, n, &["J_SpineLower"], FROST_TORSO, false);
    super::clientfields::eyes_out(world, n);
    let origin = world.resource::<Zm>().ents.get(&n).map_or([0.0; 3], |e| e.origin);
    let fx = match (upgraded, shatter) {
        (true, _) => UPGRADED,
        (false, true) => SHATTER,
        (false, false) => CRUMPLE,
    };
    super::natives_fx::effect(world, fx, origin, [0.0, 0.0, 1.0]);
    let alias = if shatter {
        "wpn_freezegun_shatter_zombie"
    } else {
        "wpn_freezegun_collapse_zombie"
    };
    super::natives_fx::sound(world, EventAudience::All, alias, origin);
    Some(Freeze { shatter })
}
