//! The Zombie Shield as BO2's engine gives it to the zombies scripts
//! (`_zm_weap_riotshield`, `_zm_riotshield`; TranZit, Mob of the Dead and
//! Origins build one). Fire with the shield in hand: the weapon code notifies
//! `raise_riotshield`, the script asks where it would stand
//! (`canplaceriotshield`), and when the plant lands the weapon code notifies
//! `deploy_riotshield` (1 = planted, 0 = called off). Both come from the T6
//! gun brain's shield states (gun_script). The script then makes the standing
//! shield's model and calls `deployriotshield(player, shield)`, which hands
//! back the trigger he uses to pick it up again.
//!
//! The spot is BO2's `BG_CanPlaceRiotshield_Box`, and the weapon code's hold
//! test (`riotshield_blocked`) uses the same check. Dvars from the BO2 exe.

use bevy_ecs::prelude::World;
use gsc_t6::{Key, ObjKind, Value, Vm};
use math_iw4::{add_lean_to_position, angle_vectors, axis_to_angles};
use playerstate_iw4::PlayerState;
use weapon_t6::facts::{FactsTable, weap_type};
use weapon_t6::player::{T6WeaponPs, wf};

use super::{Ent, Zm, arg, client, entnum, frame, int, origin_of};
use crate::t6_gun;
use crate::world::SimState;

/// `riotshield_placement_foffset`.
const FOFFSET: f32 = 20.0;
/// `riotshield_placement_maxs` (the box is +-this on every axis).
const MAXS: f32 = 50.0;
/// `riotshield_placement_pitch_max`, `_roll_max`, `_zdiff_max`.
const PITCH_MAX: f32 = 20.0;
const ROLL_MAX: f32 = 25.0;
const ZDIFF_MAX: f32 = 30.0;
/// `player_standingViewHeight`: the probe starts from standing eye height.
const STANDING_VIEW_HEIGHT: f32 = 60.0;
/// The trace mask BO2 uses for both traces.
const MASK: u32 = 0x0281_8091;
/// No lean while this eFlag is set.
const EF_NO_LEAN: u32 = 0x4000;
/// The pick-up trigger round the planted shield.
const PICKUP_RADIUS: f32 = 64.0;
const PICKUP_HEIGHT: f32 = 72.0;
/// Half a standing body: an actor's or player's middle (`getcentroid`).
const CENTROID_UP: f32 = 36.0;

fn normalize_180(a: f32) -> f32 {
    let a = a % 360.0;
    if a > 180.0 {
        a - 360.0
    } else if a < -180.0 {
        a + 360.0
    } else {
        a
    }
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// BG_CanPlaceRiotshield_Box: whether the shield can stand in front of him,
/// and where (on the ground) and how it is turned.
pub(crate) fn place(sim: &SimState, ps: &PlayerState) -> (bool, [f32; 3], [f32; 3]) {
    let (why, out, angles) = test(sim, ps);
    (why.is_none(), out, angles)
}

/// The same test, with the step that failed (None = it can stand there).
fn test(sim: &SimState, ps: &PlayerState) -> (Option<&'static str>, [f32; 3], [f32; 3]) {
    let yaw = ps.viewangles[1];
    let mut eye = [ps.origin[0], ps.origin[1], ps.origin[2] + ps.view_height_current];
    if ps.e_flags & EF_NO_LEAN == 0 {
        eye = add_lean_to_position(eye, yaw, ps.leanf, 16.0, 20.0);
    }
    eye[2] = eye[2] - ps.view_height_current + STANDING_VIEW_HEIGHT;
    let (f, _, _) = angle_vectors([20.0, yaw, 0.0]);
    let start = [eye[0] + f[0] * FOFFSET, eye[1] + f[1] * FOFFSET, eye[2] + f[2] * FOFFSET];
    let end = [start[0], start[1], start[2] - (eye[2] - ps.origin[2])];
    let no = |why| (Some(why), [0.0; 3], [0.0; 3]);
    if sim.trace_world(start, end, [0.0; 3], [0.0; 3], MASK).fraction >= 1.0 {
        return no("no ground");
    }
    let tr = sim.trace_world(start, end, [-MAXS; 3], [MAXS; 3], MASK);
    if tr.startsolid != 0 || tr.allsolid != 0 {
        return no("box starts inside something");
    }
    if tr.fraction < 0.0001 {
        return no("box cannot move");
    }
    if tr.normal[2] < 0.7 {
        return no("too steep");
    }
    let mut out = [
        start[0] + (end[0] - start[0]) * tr.fraction,
        start[1] + (end[1] - start[1]) * tr.fraction,
        start[2] + (end[2] - start[2]) * tr.fraction,
    ];
    out[2] -= MAXS;
    // (The trace is the world's own clip, so what it hit is the world.)
    let n = tr.normal;
    let c1 = cross(n, f);
    let c2 = cross(c1, n);
    let mut angles = axis_to_angles([c2, c1, n]);
    angles[0] = normalize_180(angles[0]);
    angles[2] = normalize_180(angles[2]);
    let why = if angles[0].abs() > PITCH_MAX {
        Some("pitch")
    } else if angles[2].abs() > ROLL_MAX {
        Some("roll")
    } else if (ps.origin[2] - out[2]).abs() > ZDIFF_MAX {
        Some("height")
    } else {
        None
    };
    (why, out, angles)
}

fn is_shield(facts: &FactsTable, w: u32) -> bool {
    w != 0 && facts.get(w).core.weap_type == weap_type::RIOTSHIELD
}

/// The weapon code's hold test: the spot fails while a raised shield is in
/// his hands (PM_Weapon_RiotshieldStart asks the same check each frame).
pub(crate) fn blocked(sim: &SimState, ps: &PlayerState, gun: &T6WeaponPs, facts: &FactsTable) -> bool {
    use weapon_t6::state;
    (state::RIOTSHIELD_PREPARE..=state::RIOTSHIELD_HOLD).contains(&gun.weaponstate)
        && is_shield(facts, gun.weapon)
        && !place(sim, ps).0
}

pub(super) fn bind(vm: &mut Vm<World>) {
    // player canplaceriotshield(event): ["result"], and ["origin"] / ["angles"]
    // only when it can stand there.
    vm.bind("canplaceriotshield", true, |vm, world, s, _| {
        let id = client(vm, world, s)?;
        let f = frame(world);
        let ps = f.player(id).copied();
        let (why, origin, angles) = match &ps {
            Some(ps) => test(&f, ps),
            None => (Some("no player"), [0.0; 3], [0.0; 3]),
        };
        let ok = why.is_none();
        let (o, yaw) = ps.map_or(([0.0; 3], 0.0), |ps| (ps.origin, ps.viewangles[1]));
        diag::info!(
            Sim,
            "bo2zm t6 riotshield: player {} spot from ({:.0} {:.0} {:.0}) yaw {:.0}: {}",
            id.0,
            o[0],
            o[1],
            o[2],
            yaw,
            why.unwrap_or("ok")
        );
        let mut arr = gsc_t6::Array::new();
        let k = |vm: &mut Vm<World>, s: &str| Key::Str(vm.intern(s));
        arr.set(k(vm, "result"), Value::bool(ok));
        if ok {
            arr.set(k(vm, "origin"), Value::Vec3(origin));
            arr.set(k(vm, "angles"), Value::Vec3(angles));
        }
        Ok(Value::array(arr))
    });
    // deployriotshield(player, shield): the trigger that picks it up again.
    vm.bind("deployriotshield", false, |vm, world, _, a| {
        let at = origin_of(vm, world, arg(a, 1)).unwrap_or([0.0; 3]);
        let angles = entnum(vm, arg(a, 1))
            .and_then(|n| world.resource::<Zm>().ents.get(&n).map(|e| e.angles))
            .unwrap_or([0.0; 3]);
        let n = world.resource_mut::<Zm>().alloc_entnum();
        let obj = vm.alloc_object(ObjKind::Entity(n));
        world.resource_mut::<Zm>().ents.insert(
            n,
            Ent {
                obj: Some(obj),
                classname: "trigger_radius_use".to_owned(),
                origin: at,
                angles,
                radius: PICKUP_RADIUS,
                height: PICKUP_HEIGHT,
                ..Default::default()
            },
        );
        if std::env::var_os("IW4L_T6_PLACE").is_some() {
            diag::info!(
                Sim,
                "bo2zm t6 riotshield planted at ({:.0} {:.0} {:.0}), pick-up trigger ent{n}",
                at[0],
                at[1],
                at[2]
            );
        }
        Ok(Value::Object(obj))
    });
    // entity getcentroid(): the middle of a body, else its origin.
    vm.bind("getcentroid", true, |vm, world, s, _| {
        let n = entnum(vm, s);
        let o = origin_of(vm, world, s).unwrap_or([0.0; 3]);
        let zm = world.resource::<Zm>();
        let body = n.is_some_and(|n| zm.players.contains_key(&n) || zm.actors.contains_key(&n));
        Ok(Value::Vec3([o[0], o[1], o[2] + if body { CENTROID_UP } else { 0.0 }]))
    });
    // setheldweaponmodel(n): the held weapon's model (the shield's red
    // "can't place" one is 1).
    vm.bind("setheldweaponmodel", true, |vm, world, s, a| {
        let id = client(vm, world, s)?;
        let m = int(a, 0).unwrap_or(0).clamp(0, 15) as u8;
        t6_gun::script_edit(&mut frame(world), id, |ps, _, _| {
            let w = ps.weapon;
            if let Some(h) = ps.held_mut(w) {
                h.model = m;
            }
        });
        Ok(Value::Undefined)
    });
    // setplacementhint(n): nonzero clears the "can't place" flag, 0 sets it.
    vm.bind("setplacementhint", true, |vm, world, s, a| {
        let id = client(vm, world, s)?;
        let on = int(a, 0).unwrap_or(0) != 0;
        t6_gun::script_edit(&mut frame(world), id, |ps, _, _| {
            if on {
                ps.weap_flags &= !wf::RIOTSHIELD_CANT_PLACE;
            } else {
                ps.weap_flags |= wf::RIOTSHIELD_CANT_PLACE;
            }
        });
        Ok(Value::Undefined)
    });
    // setriotshieldfailhint(): BO2 sends EV_CANNOTPLANT for the "can't plant
    // here" line on his screen. That line is not drawn here yet.
    vm.bind("setriotshieldfailhint", true, |_, _, _, _| Ok(Value::Undefined));
    // setscriptmoverflag(n): drawing only, nothing the game keeps.
    vm.bind("setscriptmoverflag", true, |_, _, _, _| Ok(Value::Undefined));
}
