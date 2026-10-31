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
use entity_t6::flight::path_kind;
use gsc_t6::{Key, ObjKind, ScriptValue, Vm};
use game_math::{lean_offset, euler_basis, rows_to_euler};
use playerstate_t6::game::{AvatarState, movement_state_bits};
use weapon_t6::facts::{FactsTable, gun_family_code};
use weapon_t6::player::{T6WeaponPs, wf};

use super::{Ent, Zm, arg, client, entnum, frame, int, tick, weapon_text};
use crate::gun_brain;
use crate::sim_parts::MatchSimulation;

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

fn cross(lhs: [f32; 3], rhs: [f32; 3]) -> [f32; 3] {
    [
        lhs[1] * rhs[2] - lhs[2] * rhs[1],
        lhs[2] * rhs[0] - lhs[0] * rhs[2],
        lhs[0] * rhs[1] - lhs[1] * rhs[0],
    ]
}

/// BG_CanPlaceRiotshield_Box: whether the shield can stand in front of him,
/// and where (on the ground) and how it is turned.
pub(crate) fn place(sim: &MatchSimulation, ps: &AvatarState) -> (bool, [f32; 3], [f32; 3]) {
    let (why, out, angles) = test(sim, ps);
    (why.is_none(), out, angles)
}

/// The eye moved out by the lean, unless the player may not lean (a `movement_state_bits` bit, not an entity flag).
fn leaned_eye(pstate: &AvatarState, eye_at: [f32; 3]) -> [f32; 3] {
    if pstate.movement_state_bits & movement_state_bits::NO_LEAN == 0 {
        lean_offset(eye_at, pstate.look_angles[1], pstate.tilt_sideways, 16.0, 20.0)
    } else {
        eye_at
    }
}

/// The same test, with the step that failed (None = it can stand there).
fn test(sim: &MatchSimulation, pstate: &AvatarState) -> (Option<&'static str>, [f32; 3], [f32; 3]) {
    let yaw = pstate.look_angles[1];
    let eye_at = [pstate.origin[0], pstate.origin[1], pstate.origin[2] + pstate.eye_height_now];
    let mut eye_at = leaned_eye(pstate, eye_at);
    eye_at[2] = eye_at[2] - pstate.eye_height_now + STANDING_VIEW_HEIGHT;
    let (f, _, _) = euler_basis([20.0, yaw, 0.0]);
    let start = [eye_at[0] + f[0] * FOFFSET, eye_at[1] + f[1] * FOFFSET, eye_at[2] + f[2] * FOFFSET];
    let end = [start[0], start[1], start[2] - (eye_at[2] - pstate.origin[2])];
    let no = |why| (Some(why), [0.0; 3], [0.0; 3]);
    if sim.trace_world(start, end, [0.0; 3], [0.0; 3], MASK).fraction >= 1.0 {
        return no("no ground");
    }
    let tr = sim.trace_world(start, end, [-MAXS; 3], [MAXS; 3], MASK);
    if tr.starts_inside || tr.fully_embedded {
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
    let mut angles = rows_to_euler([c2, c1, n]);
    angles[0] = normalize_180(angles[0]);
    angles[2] = normalize_180(angles[2]);
    let why = if angles[0].abs() > PITCH_MAX {
        Some("pitch")
    } else if angles[2].abs() > ROLL_MAX {
        Some("roll")
    } else if (pstate.origin[2] - out[2]).abs() > ZDIFF_MAX {
        Some("height")
    } else {
        None
    };
    (why, out, angles)
}

fn is_shield(facts: &FactsTable, w: u32) -> bool {
    w != 0 && facts.get(w).core.gun_family_code == gun_family_code::RIOTSHIELD
}

/// The weapon code's hold test: the spot fails while a raised shield is in
/// his hands (PM_Weapon_RiotshieldStart asks the same check each frame).
pub(crate) fn blocked(sim: &MatchSimulation, ps: &AvatarState, gun: &T6WeaponPs, facts: &FactsTable) -> bool {
    use weapon_t6::state;
    (state::RIOTSHIELD_PREPARE..=state::RIOTSHIELD_HOLD).contains(&gun.gun_phase)
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
    tag: Option<&xmodel_runtime::HitShapeBone>,
    hit_at: [f32; 3], from_pt: [f32; 3],
) -> u8 {
    let Some(tag) = tag else {
        return 0;
    };
    let local = |p: [f32; 3]| {
        let d = [p[0] - tag.center[0], p[1] - tag.center[1], p[2] - tag.center[2]];
        tag.axes.map(|a| a[0] * d[0] + a[1] * d[1] + a[2] * d[2])
    };
    hit_encode(local(hit_at), local(from_pt)[0] > 0.0)
}

/// BroadcastShieldHitEvent and BroadcastShieldHitEventToVictim: a bullet
/// striking the shield `victim` bears. Everyone else gets the shield hit
/// where it struck; he alone gets his own, with the hit's place on his
/// shield in its parm (`tag` is where the shield hung when the shot struck).
pub(crate) fn broadcast_hit(
    sim: &mut MatchSimulation,
    tick: crate::sim_parts::Tick,
    struck: crate::sim_parts::ClientId,
    tag_name: Option<&xmodel_runtime::HitShapeBone>,
    fx_event: crate::EntityEventPayload,
) {
    use crate::shot_trace::SURF_TYPE_RIOTSHIELD;
    use crate::player_ledger::EventRecipients;
    use entity_t6::EventCode;
    sim.post_entity_event(
        tick,
        EventRecipients::AllExcept(struck),
        EventCode::ROUND_HIT_SHIELD,
        crate::EntityEventPayload { event_parm: 0, ..fx_event },
    );
    let to_victim = EventRecipients::Client(struck);
    sim.post_entity_event(
        tick,
        to_victim,
        EventCode::BULLET_HIT_CLIENT_SHIELD,
        crate::EntityEventPayload {
            surf_type: SURF_TYPE_RIOTSHIELD as u8,
            event_parm: i32::from(encode_hit_pos(tag_name, fx_event.origin, fx_event.origin2)),
            ..fx_event
        },
    );
}

pub(super) fn bind(vm: &mut Vm<World>) {
    // player canplaceriotshield(event): ["result"], and ["origin"] / ["angles"]
    // only when it can stand there.
    vm.bind("canplaceriotshield", true, |vm, world, s, _| {
        let id = client(vm, world, s)?;
        let f = frame(world);
        let pstate = f.avatar(id).copied();
        let (why, origin, angles) = match &pstate {
            Some(pstate) => test(&f, pstate),
            None => (Some("no player"), [0.0; 3], [0.0; 3]),
        };
        let ok = why.is_none();
        let (o, yaw) = pstate.map_or(([0.0; 3], 0.0), |pstate| (pstate.origin, pstate.look_angles[1]));
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
        arr.set(k(vm, "result"), ScriptValue::bool(ok));
        if ok {
            arr.set(k(vm, "origin"), ScriptValue::Vec3(origin));
            arr.set(k(vm, "angles"), ScriptValue::Vec3(angles));
        }
        Ok(ScriptValue::array(arr))
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
            .and_then(|n| world.resource::<Zm>().ents.get(&n).map(|e| e.rot))
            .unwrap_or_default();
        let Some((wd, clip)) = gun_brain::script_edit(&mut frame(world), id, |pstate, known, _| {
            let wd = if is_shield(known, pstate.weapon) {
                pstate.weapon
            } else {
                pstate.held.iter().map(|h| h.weapon).find(|&wd| is_shield(known, wd))?
            };
            let clip = (pstate.ammo_in_clip(known, wd) + 1).min(known.get(wd).core.magazine_capacity.max(0));
            pstate.set_ammo_in_clip(known, wd, clip);
            Some((wd, clip))
        })
        .flatten() else {
            return Ok(ScriptValue::Undefined);
        };
        let Some(origin) = frame(world).avatar(id).map(|pstate| pstate.origin) else {
            return Ok(ScriptValue::Undefined);
        };
        let t = tick(world);
        let Some(number) = crate::floor_guns::drop_weapon(&mut frame(world), t, id, wd) else {
            return Ok(ScriptValue::Undefined);
        };
        let at = [origin[0], origin[1], origin[2] + ITEM_UP];
        if let Some(it) = frame(world).loot_at_slot_mut(number) {
            it.origin = at;
            it.dropping = false;
            it.right_mag = clip;
            it.state.lerp.pos.tr_type = path_kind::AT_REST;
            it.state.lerp.pos.base_point = at;
            it.state.lerp.pos.rate = [0.0; 3];
            it.state.lerp.apos.base_point = angles;
            it.state.lerp.e_flags |= EF_NODRAW as i32;
        }
        // The T6 side of the take (and the empty hand BO2 leaves).
        if let Some(take) = vm.bound("takeweapon", true) {
            let name = vm.string(&weapon_text(world, wd));
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
                class_tag: "weapon_item".to_owned(),
                origin: at,
                rot: angles,
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
        Ok(ScriptValue::Object(obj))
    });
    // setheldweaponmodel(n): the held weapon's model (the shield's red
    // "can't place" one is 1).
    vm.bind("setheldweaponmodel", true, |vm, world, s, a| {
        let id = client(vm, world, s)?;
        let m = int(a, 0).unwrap_or(0).clamp(0, 15) as u8;
        gun_brain::script_edit(&mut frame(world), id, |pstate, _, _| {
            let wd = pstate.weapon;
            if let Some(h) = pstate.held_mut(wd) {
                h.model = m;
            }
        });
        Ok(ScriptValue::Undefined)
    });
    // setplacementhint(n): nonzero clears the "can't place" flag, 0 sets it.
    vm.bind("setplacementhint", true, |vm, world, s, a| {
        let id = client(vm, world, s)?;
        let on = int(a, 0).unwrap_or(0) != 0;
        gun_brain::script_edit(&mut frame(world), id, |pstate, _, _| {
            if on {
                pstate.weapon_state_bits &= !wf::RIOTSHIELD_CANT_PLACE;
            } else {
                pstate.weapon_state_bits |= wf::RIOTSHIELD_CANT_PLACE;
            }
        });
        Ok(ScriptValue::Undefined)
    });
    // setriotshieldfailhint(): BO2 sends EV_CANNOTPLANT for the "can't plant
    // here" line on his screen. That line is not drawn here yet.
    vm.bind("setriotshieldfailhint", true, |_, _, _, _| Ok(ScriptValue::Undefined));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(b: u8) -> (u8, u8, u8) {
        ((b & 0xF0) >> 4, (b & 0x0E) >> 1, b & 1)
    }

    #[test]
    fn a_player_who_may_not_lean_is_not_leaned() {
        let mut ps = AvatarState::ZERO;
        ps.tilt_sideways = 1.0;
        let eye = [0.0; 3];
        assert_ne!(leaned_eye(&ps, eye), eye);
        // The old numbering took 0x4000 of the entity flags (riding a vehicle) for the ban.
        ps.e_flags = 0x4000;
        assert_ne!(leaned_eye(&ps, eye), eye);
        ps.movement_state_bits = movement_state_bits::NO_LEAN;
        assert_eq!(leaned_eye(&ps, eye), eye);
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
    fn tag() -> xmodel_runtime::HitShapeBone {
        xmodel_runtime::HitShapeBone {
            bone: 0, body_part_category: 0,
            center: [100.0, 0.0, 50.0],
            axes: [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            half_ext: [0.0; 3],
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
