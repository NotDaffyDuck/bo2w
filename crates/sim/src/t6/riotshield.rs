//! bo2mp: the riot shield plant as BO2's engine gives it to `_riotshield.gsc`.
//! Fire with the shield in hand raises it (`raise_riotshield`); on release the
//! weapon code plants it or lowers it again (`deploy_riotshield`, 1 = planted,
//! 0 = called off). The script asks where it stands (`canplaceriotshield`),
//! spawns the standing shield and calls `deployriotshield(player, shield)`,
//! which takes the shield out of his hands and hands back the engine item he
//! holds use on to take it back (its `trigger` deletes the standing one).
//!
//! The spot is BO2's `BG_CanPlaceRiotshield_Box` (bg.c), and the weapon code's
//! hold test (`riotshield_blocked`) uses the same check. Dvars from the BO2 exe.

use bevy_ecs::prelude::World;
use entity_t6::trajectory::tr_type::STATIONARY as TR_STATIONARY;
use gsc_t6::{Key, ObjKind, Value, Vm};
use game_math::{add_lean_to_position, angle_vectors, axis_to_angles};
use playerstate_t6::game::PlayerState;
use weapon_t6::facts::{FactsTable, weap_type};
use weapon_t6::player::{T6WeaponPs, wf};

use super::{Ent, Zm, arg, client, entnum, frame, int, tick, weapon_text};
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
/// The dropped item is drawn by the standing shield, not by itself.
const EF_NODRAW: u32 = 0x20;
/// Drop_Weapon's item height over his feet.
const ITEM_UP: f32 = 20.0;

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
    if tr.startsolid || tr.allsolid {
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

/// `bg_shieldHitEncodeWidthWorld` / `HeightWorld`: the stretch of the shield
/// (across its tag's left axis, up its up axis) a hit's place is told in,
/// and how many parts each is cut into.
const HIT_WIDTH: [f32; 2] = [-14.0, 14.0];
const HIT_HEIGHT: [f32; 2] = [-25.0, 20.0];
const PARTS_ACROSS: f32 = 16.0;
const PARTS_UP: f32 = 8.0;

/// Which part of `[lo, hi]` cut `count` ways a place falls in (past either
/// end: the end part).
fn hit_part(v: f32, [lo, hi]: [f32; 2], count: f32) -> u8 {
    let size = (hi - lo) / count;
    let top = hi - size * 0.5;
    let mut at = if v - top < 0.0 { v } else { top };
    if lo - v >= 0.0 {
        at = lo;
    }
    ((at - lo) / size) as u8
}

/// BG_ShieldHitEncode: a hit's place on the shield (in its tag's space) in
/// one byte: the part across in the top four bits, the part up in the next
/// three, and whether the shot came from the front in the lowest.
pub(crate) fn hit_encode(local: [f32; 3], front: bool) -> u8 {
    let across = hit_part(local[1], HIT_WIDTH, PARTS_ACROSS);
    let up = hit_part(local[2], HIT_HEIGHT, PARTS_UP);
    (((across << 3) | up) << 1) | u8::from(front)
}

/// EncodeShieldHitPos: the hit's place on the shield, by the tag it hangs on
/// (`tag`, world space) and where the shot started. No tag: 0.
pub(crate) fn encode_hit_pos(
    tag: Option<&xmodel_runtime::CollisionBone>,
    hit: [f32; 3],
    start: [f32; 3],
) -> u8 {
    let Some(tag) = tag else {
        return 0;
    };
    let local = |p: [f32; 3]| {
        let d = [p[0] - tag.center[0], p[1] - tag.center[1], p[2] - tag.center[2]];
        tag.axes.map(|a| a[0] * d[0] + a[1] * d[1] + a[2] * d[2])
    };
    hit_encode(local(hit), local(start)[0] > 0.0)
}

/// BroadcastShieldHitEvent and BroadcastShieldHitEventToVictim: a bullet
/// striking the shield `victim` bears. Everyone else gets the shield hit
/// where it struck; he alone gets his own, with the hit's place on his
/// shield in its parm (`tag` is where the shield hung when the shot struck).
pub(crate) fn broadcast_hit(
    sim: &mut SimState,
    tick: crate::world::Tick,
    victim: crate::world::ClientId,
    tag: Option<&xmodel_runtime::CollisionBone>,
    payload: crate::EntityEventPayload,
) {
    use crate::bullet_collision::SURF_TYPE_RIOTSHIELD;
    use crate::match_state::EventAudience;
    use entity_t6::EntityEventKind;
    sim.push_entity_event(
        tick,
        EventAudience::AllExcept(victim),
        EntityEventKind::BULLET_HIT_SHIELD,
        crate::EntityEventPayload { event_parm: 0, ..payload },
    );
    sim.push_entity_event(
        tick,
        EventAudience::Client(victim),
        EntityEventKind::BULLET_HIT_CLIENT_SHIELD,
        crate::EntityEventPayload {
            surf_type: SURF_TYPE_RIOTSHIELD as u8,
            event_parm: i32::from(encode_hit_pos(tag, payload.origin, payload.origin2)),
            ..payload
        },
    );
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
            "bo2mp riotshield: player {} spot from ({:.0} {:.0} {:.0}) yaw {:.0}: {}",
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
    // deployriotshield(player, shield): G_DeployRiotshield. The round the
    // plant spent goes back in (Add_Ammo), the shield leaves his hands as an
    // unseen item where he stands, the standing shield takes damage, and the
    // item comes back as the script's pick-up trigger.
    vm.bind("deployriotshield", false, |vm, world, _, a| {
        let owner = arg(a, 0).clone();
        let id = client(vm, world, &owner)?;
        let shield_ent = entnum(vm, arg(a, 1));
        let angles = shield_ent
            .and_then(|n| world.resource::<Zm>().ents.get(&n).map(|e| e.angles))
            .unwrap_or([0.0; 3]);
        let Some((w, clip)) = t6_gun::script_edit(&mut frame(world), id, |ps, facts, _| {
            let w = if is_shield(facts, ps.weapon) {
                ps.weapon
            } else {
                ps.held.iter().map(|h| h.weapon).find(|&w| is_shield(facts, w))?
            };
            let clip = (ps.ammo_in_clip(facts, w) + 1).min(facts.get(w).core.clip_size.max(0));
            ps.set_ammo_in_clip(facts, w, clip);
            Some((w, clip))
        })
        .flatten() else {
            return Ok(Value::Undefined);
        };
        let Some(origin) = frame(world).player(id).map(|ps| ps.origin) else {
            return Ok(Value::Undefined);
        };
        let t = tick(world);
        let Some(number) = crate::item::drop_weapon(&mut frame(world), t, id, w) else {
            return Ok(Value::Undefined);
        };
        let at = [origin[0], origin[1], origin[2] + ITEM_UP];
        if let Some(it) = frame(world).dropped_item_mut_by_number(number) {
            it.origin = at;
            it.falling = false;
            it.clip_r = clip;
            it.state.lerp.pos.tr_type = TR_STATIONARY;
            it.state.lerp.pos.tr_base = at;
            it.state.lerp.pos.tr_delta = [0.0; 3];
            it.state.lerp.apos.tr_base = angles;
            it.state.lerp.e_flags |= EF_NODRAW as i32;
        }
        // The T6 side of the take (and the empty hand BO2 leaves).
        if let Some(take) = vm.bound("takeweapon", true) {
            let name = vm.string(&weapon_text(world, w));
            let _ = take(vm, world, &owner, &[name]);
        }
        let mut zm = world.resource_mut::<Zm>();
        if let Some(e) = shield_ent.and_then(|n| zm.ents.get_mut(&n)) {
            e.can_damage = true;
        }
        let n = zm.alloc_entnum();
        let obj = vm.alloc_object(ObjKind::Entity(n));
        zm.ents.insert(
            n,
            Ent {
                obj: Some(obj),
                classname: "weapon_item".to_owned(),
                origin: at,
                angles,
                item: Some(number),
                ..Default::default()
            },
        );
        diag::info!(
            Sim,
            "bo2mp riotshield: player {} planted at ({:.0} {:.0} {:.0}), item {number}",
            id.0,
            at[0],
            at[1],
            at[2]
        );
        Ok(Value::Object(obj))
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(b: u8) -> (u8, u8, u8) {
        ((b & 0xF0) >> 4, (b & 0x0E) >> 1, b & 1)
    }

    #[test]
    fn the_middle_of_the_shield_is_part_8_across_and_4_up() {
        assert_eq!(hit_encode([0.0; 3], true), 137);
        assert_eq!(decode(137), (8, 4, 1));
        assert_eq!(decode(hit_encode([0.0; 3], false)), (8, 4, 0));
    }

    #[test]
    fn hits_past_the_edges_fall_in_the_end_parts() {
        assert_eq!(decode(hit_encode([0.0, 100.0, 100.0], false)), (15, 7, 0));
        assert_eq!(decode(hit_encode([0.0, -100.0, -100.0], false)), (0, 0, 0));
        assert_eq!(decode(hit_encode([0.0, 14.0, 20.0], true)), (15, 7, 1));
        assert_eq!(decode(hit_encode([0.0, -14.0, -25.0], true)), (0, 0, 1));
    }

    /// A shield tag at (100, 0, 50) facing +y.
    fn tag() -> xmodel_runtime::CollisionBone {
        xmodel_runtime::CollisionBone {
            bone: 0,
            part_classification: 0,
            center: [100.0, 0.0, 50.0],
            axes: [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            half_size: [0.0; 3],
        }
    }

    #[test]
    fn the_place_is_told_in_the_tags_own_space() {
        // 7 to the tag's left (world -x) and 10 up.
        let hit = [93.0, 0.0, 60.0];
        let front = encode_hit_pos(Some(&tag()), hit, [100.0, 100.0, 50.0]);
        assert_eq!(decode(front), (12, 6, 1));
        let behind = encode_hit_pos(Some(&tag()), hit, [100.0, -100.0, 50.0]);
        assert_eq!(decode(behind), (12, 6, 0));
    }

    #[test]
    fn no_tag_tells_nothing() {
        assert_eq!(encode_hit_pos(None, [1.0, 2.0, 3.0], [4.0, 5.0, 6.0]), 0);
    }
}
