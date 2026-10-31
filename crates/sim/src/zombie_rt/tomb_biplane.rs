//! Origins (zm_tomb) fire staff biplane test aid.
//!
//! BO2MP_T6_BIPLANE=<log|shoot>[@<seconds>] (from 20 s unless given). Every
//! second while the biplane (`spawnvehicle` model `veh_t6_dlc_zm_biplane`)
//! flies: where it is, how far from the player, its health and its
//! `biplane_down` flag. `shoot`: the player looks at it and fires every
//! 0.3 s (clip refilled) while it is within 7000 units (bullets reach 8192;
//! its loop never comes nearer than about 4000 to the spawn), until it is
//! down (one hit does it, `level.callbackvehicledamage`).
//! It flies only once `activate_zone_village_0` is set and a round has gone
//! by (`staff_biplane_drop_pieces`), so run it with BO2MP_T6_OPENALL=buy;
//! `shoot` ends the round he bought the village in (round_number + 1, once)
//! so the plane takes off without a round to play.

use std::sync::{Mutex, OnceLock};

use bevy_ecs::prelude::World;
use gsc_t6::{ObjRef, ScriptValue};
use playerstate_t6::game::buttons;

use super::{Closest, frame, with_vm};
use crate::sim_parts::ClientId;

fn want() -> Option<&'static (String, i64)> {
    static WANT: OnceLock<Option<(String, i64)>> = OnceLock::new();
    WANT.get_or_init(|| {
        let v = std::env::var("BO2MP_T6_BIPLANE").ok()?;
        Some(match v.split_once('@') {
            Some((m, s)) => (
                m.to_owned(),
                s.parse::<f32>().map_or(20_000, |s| (s * 1000.0) as i64),
            ),
            None => (v, 20_000),
        })
    })
    .as_ref()
}

struct State {
    seen: bool,
    gone: bool,
    round_ended: bool,
    village_at: Option<i64>,
    moved_at: Option<i64>,
}

static STATE: Mutex<State> = Mutex::new(State {
    seen: false,
    gone: false,
    round_ended: false,
    village_at: None,
    moved_at: None,
});

/// The shot from `eye` at the plane goes clear to within 300 of it.
fn clear_to(game: &mut World, eye_at: [f32; 3], pln: [f32; 3]) -> bool {
    let t = frame(game).trace_static_world(eye_at, pln, [0.0; 3], [0.0; 3], crate::shot_trace::MASK_SHOT);
    let hit: [f32; 3] = std::array::from_fn(|i| eye_at[i] + (pln[i] - eye_at[i]) * t.fraction);
    !t.starts_inside && gsc_t6::math::length(gsc_t6::math::sub(hit, pln)) < 300.0
}

/// A floor within 3200 of `from`, open overhead, whose shot at the plane
/// goes clear: the one nearest the plane.
fn open_spot(game: &mut World, from: [f32; 3], pln: [f32; 3]) -> Option<[f32; 3]> {
    let mut pick = Closest::new();
    for dist in [400.0, 800.0, 1200.0, 1800.0, 2400.0, 3200.0] {
        for k in 0..12 {
            let yaw = (30.0 * k as f32).to_radians();
            for up in [80.0, 400.0] {
                let c = [from[0] + dist * yaw.cos(), from[1] + dist * yaw.sin(), from[2] + up];
                let Some(f) = super::tomb_staffs::floor_under(game, c) else {
                    continue;
                };
                let eye = [f[0], f[1], f[2] + 60.0];
                let sky = frame(game).trace_static_world(
                    eye,
                    [eye[0], eye[1], eye[2] + 600.0],
                    [0.0; 3],
                    [0.0; 3],
                    crate::shot_trace::MASK_SHOT,
                );
                if sky.starts_inside || sky.fraction < 1.0 || !clear_to(game, eye, pln) {
                    continue;
                }
                let d = gsc_t6::math::length(gsc_t6::math::sub(pln, eye));
                pick.offer(d, f);
            }
        }
    }
    pick.into_item()
}

pub(crate) fn biplane_test(game: &mut World, now: i64) {
    let Some((mode, at)) = want() else {
        return;
    };
    if now < *at {
        return;
    }
    let peer = ClientId(0);
    let tick = i64::from(crate::SIM_STEP_MILLIS);
    let Some(pstate) = frame(game).avatar(peer).copied() else {
        return;
    };
    let plane: Option<ObjRef> = game
        .resource::<super::Zm>()
        .ents
        .values()
        // The flying one (`spawnvehicle`), not the map's parked template.
        .find(|e| e.mesh == "veh_t6_dlc_zm_biplane" && !e.map)
        .and_then(|e| e.obj);
    let mut st = STATE.lock().unwrap();
    if plane.is_none() && !st.seen && !st.round_ended && mode == "shoot" {
        // Two seconds after the village opens (the quest thread has noted
        // its round by then).
        let open = with_vm(game, |vm, _| super::tomb_test::level_flag(vm, "activate_zone_village_0"))
            .unwrap_or(false);
        if open && st.village_at.is_none() {
            st.village_at = Some(now);
        }
        if st.village_at.is_some_and(|t| now >= t + 2000) {
            st.round_ended = true;
            let n = with_vm(game, |vm, game| {
                let f = vm.intern("round_number");
                let level = vm.level;
                let n = vm.get_field(game, level, f).as_int().unwrap_or(1) + 1;
                vm.set_field(game, level, f, ScriptValue::Int(n));
                n
            })
            .unwrap_or(0);
            diag::info!(Sim, "bo2zm t6 biplane test {}s: the village is open; round now {n}", now / 1000);
        }
    }
    let Some(p) = plane else {
        if st.seen && !st.gone {
            st.gone = true;
            diag::info!(Sim, "bo2zm t6 biplane test {}s: the biplane is gone", now / 1000);
        }
        return;
    };
    let (spot, health, down) = with_vm(game, |vm, game| {
        let o = super::origin_of(vm, game, &ScriptValue::Object(p));
        let h = vm.intern("health");
        let h = vm.get_field(game, p, h);
        let d = super::tomb_test::ent_flag(vm, game, p, "biplane_down");
        (o, vm.to_text(&h), d)
    })
    .unwrap_or((None, String::new(), String::new()));
    let Some(spot) = spot else {
        return;
    };
    if !st.seen {
        st.seen = true;
        diag::info!(Sim, "bo2zm t6 biplane test {}s: the biplane flies", now / 1000);
    }
    let eye_at = [pstate.origin[0], pstate.origin[1], pstate.origin[2] + pstate.eye_height_now];
    // Under a roof (plane3: the spawn's dugout) every shot stops in it: while
    // his shot at it is blocked, every 3 s stand him on open ground with a
    // clear shot, the nearest such to the plane.
    if mode == "shoot"
        && down != "1"
        && now % 1000 < tick
        && st.moved_at.is_none_or(|t| now - t >= 3000)
        && !clear_to(game, eye_at, spot)
    {
        st.moved_at = Some(now);
        if let Some(f) = open_spot(game, pstate.origin, spot) {
            super::teleport_player(game, peer, f);
            diag::info!(
                Sim,
                "bo2zm t6 biplane test {}s: no clear shot at it; player to ({:.0} {:.0} {:.0})",
                now / 1000,
                f[0],
                f[1],
                f[2]
            );
        }
    }
    let dist = gsc_t6::math::length(gsc_t6::math::sub(spot, eye_at));
    if mode == "shoot" && down != "1" && dist < 7000.0 {
        super::set_player_view(game, peer, super::autoplay::look(eye_at, spot));
        if now % 300 < tick {
            {
                let mut f = frame(game);
                if let Some(w) = f.avatar(peer).map(|p| p.weapon) {
                    crate::client_script::set_ammo_clip(&mut f, peer, w, 8);
                }
            }
            let mut req = game.resource_mut::<crate::frame_step::FrameTicket>();
            for (cid, command) in &mut req.input.cmds {
                if *cid == peer {
                    command.buttons |= buttons::ATTACK;
                }
            }
        }
    }
    if now % 1000 < tick {
        diag::info!(
            Sim,
            "bo2zm t6 biplane test {}s: biplane at ({:.0} {:.0} {:.0}) {dist:.0} from the player, health {health}, biplane_down {down}",
            now / 1000,
            spot[0],
            spot[1],
            spot[2]
        );
    }
}
