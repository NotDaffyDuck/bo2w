//! bo2mp: the helicopters' client script (clientscripts/mp/_helicopter.csc
//! `startfx`): the Stealth Chopper's, the Escort Drone's and the VTOL
//! Warship's light, green to its side and red to the rest, and the VTOL's
//! two nacelle exhausts, from its first showing until it is gone.
//!
//! Not ported: the engine exhaust every helicopter takes from its vehicle
//! def (`exhaustfxname`, engine data we do not read), the VTOL's nacelle
//! tilt (`heli_gunner_vtol_state`: two held animations at once, one prop
//! animation per entity here), the rotor-wash dust and the missile alarms.

use bevy_ecs::prelude::World;
use gsc_t6::{ObjRef, ScriptValue};

use super::{Lamps, Lit, Pending, Cue, Zm, rc_sides, relight};
use crate::EventRecipients;
use crate::sim_parts::ClientId;

const LIGHT_TAG: &str = "tag_origin";
const VTOL: &str = "heli_player_gunner_mp";
const EXHAUST: [(&str, &str); 2] = [
    ("vehicle/exhaust/fx_exhaust_vtol_mp", "tag_engine_left"),
    ("vehicle/exhaust/fx_exhaust_vtol_rt_mp", "tag_engine_right"),
];
/// Each snapshot: who is on which side can change (a respawn, a team swap).
const TICK_MS: i64 = 50;

/// Its light for each side, by vehicle type.
fn lights(vehicle: &str) -> Option<Lamps> {
    Some(match vehicle {
        "heli_ai_mp" => Lamps { friend: "light/fx_vlight_mp_attack_heli_grn", foe: "light/fx_vlight_mp_attack_heli_red" },
        VTOL => Lamps { friend: "light/fx_vlight_mp_vtol_grn", foe: "light/fx_vlight_mp_vtol_red" },
        "heli_guard_mp" => Lamps { friend: "light/fx_vlight_mp_escort_eye_grn", foe: "light/fx_vlight_mp_escort_eye_red" },
        _ => return None,
    })
}

/// One helicopter as its client script keeps it.
#[derive(Clone, Debug)]
pub(super) struct Heli {
    obj: ObjRef,
    lamps: Lamps,
    vtol: bool,
    /// Who sees which light now.
    lit: Vec<Lit>,
    /// Who sees its exhausts now (the VTOL's).
    exhaust: Vec<u32>,
}

/// `startfx_loop`: a helicopter with a light comes into play.
pub(super) fn spawned(world: &mut World, n: u32, kind: &str) {
    let kind = kind.to_ascii_lowercase();
    let Some(lamps) = lights(&kind) else {
        return;
    };
    let Some(obj) = world.resource::<Zm>().ents.get(&n).and_then(|e| e.obj) else {
        return;
    };
    let mut zm = world.resource_mut::<Zm>();
    let now = zm.clock_millis;
    zm.client_fx.pending.retain(|p| p.ent != n || !matches!(p.step, Cue::Heli(_)));
    zm.client_fx.pending.push(Pending {
        due: now,
        until: i64::MAX,
        ent: n,
        step: Cue::Heli(Box::new(Heli {
            obj,
            lamps,
            vtol: kind == VTOL,
            lit: Vec::new(),
            exhaust: Vec::new(),
        })),
    });
}

/// Each snapshot: its light to who should see it, and the VTOL's exhausts to
/// everyone. None once it is gone (what it showed went with it).
pub(super) fn tick(world: &mut World, n: u32, mut h: Box<Heli>, now: i64) -> Option<(i64, Box<Heli>)> {
    let alive = world.resource::<Zm>().ents.get(&n).is_some_and(|e| e.obj == Some(h.obj));
    if !alive {
        return None;
    }
    if super::super::natives_fx::shown(world, n).is_none() {
        return Some((now + TICK_MS, h));
    }
    let sides = super::super::with_vm(world, |vm, world| rc_sides(vm, world, &ScriptValue::Object(h.obj)));
    let want = h.lamps.shown_to(sides);
    let players: Vec<u32> = world.resource::<Zm>().players.keys().copied().collect();
    let bolt = |world: &mut World, c: u32, fx: &'static str, tag: &'static str, stop: bool| {
        super::super::natives_fx::bolt_effect_to(world, EventRecipients::Client(ClientId(c)), n, fx, tag, stop);
    };
    relight(world, n, LIGHT_TAG, &h.lit, &want);
    if h.vtol {
        for &c in &players {
            if !h.exhaust.contains(&c) {
                for (fx, tag) in EXHAUST {
                    bolt(world, c, fx, tag, false);
                }
            }
        }
        h.exhaust = players;
    }
    h.lit = want;
    Some((now + TICK_MS, h))
}
