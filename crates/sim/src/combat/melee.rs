//! Melee swings: a knife slash or a bash, traced ahead of the eye and applied to whatever it touches.

use super::shot_log;
use crate::bullet_collision::{
    AuthorityModelOwner, BulletTraceQuery, BulletTraceSegment, ColliderId, HITLOC_SHIELD, MASK_BULLET_WORLD, bullet_trace_segments_filtered,
    glass_piece_from_hit,
};
use crate::frame::FrameWorld;
use crate::identities::{DamageSource, LifeSequence, PelletId};
use crate::match_state::{ClientLifecycle, EventAudience};
use crate::world::{ClientId, Tick};
use entity_t6::EntityEventKind;
use weapon_t6::melee_trace::{
    MELEE_TRACE_OFFSETS, PLAYER_MELEE_HEIGHT_DEFAULT, PLAYER_MELEE_WIDTH_DEFAULT, melee_trace_count, melee_trace_end,
};
use weapon_t6::penetration::BulletPenFacts;

/// The parm bit of a melee hit event that says a riot shield (carried or planted) took the swing. BO2's client plays
/// the knife-on-shield sound for a knife and nothing for a shield bash.
pub const MELEE_PARM_SHIELD: i32 = 2;

/// Surface flag of geometry a melee trace passes through.
const SURFACE_NO_MELEE: u32 = 0x10;

/// BO2 starts a player's swing this far behind the eye, so a target pressed against the attacker is still ahead of
/// the trace.
const SWING_START_BEHIND_EYE: f32 = 10.0;

/// One melee swing's numbers: base damage, hit reach, and whether it is a knife (the hit sound).
pub(crate) struct MeleeSwing {
    pub damage: i32,
    pub range: f32,
    pub knife: bool,
}

/// What a swing's traces found.
struct SwingScan {
    start: [f32; 3],
    /// Where the first line, as far as it reaches, crosses the damage triggers.
    trigger_end: Option<[f32; 3]>,
    best: Option<BulletTraceSegment>,
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn length_squared(v: [f32; 3]) -> f32 {
    v[0] * v[0] + v[1] * v[1] + v[2] * v[2]
}

/// Traces the swing box ahead of the eye (BO2's Melee_Trace) and picks the thing hit. The first offset that touches a
/// player is the hit, even when another offset met a wall nearer; otherwise the nearest thing touched wins, and a
/// later offset wins a tie.
fn scan_swing(
    world: &FrameWorld,
    tick: Tick,
    attacker: ClientId,
    origin: [f32; 3],
    angles: [f32; 3],
    range: f32,
) -> SwingScan {
    let (forward, right, up) = game_math::angle_vectors(angles);
    let (width, height) = (PLAYER_MELEE_WIDTH_DEFAULT, PLAYER_MELEE_HEIGHT_DEFAULT);
    let query = world.lagcomp_query_for(attacker, tick);
    let glass_pairs = world.world_objects().glass_damage_pairs();
    let pane_stands = |piece: u16| {
        let damage = glass_pairs
            .iter()
            .find(|(id, _)| *id == u32::from(piece))
            .map_or(0, |(_, damage)| *damage);
        crate::world_objects::glass_piece_is_solid(damage)
    };
    let start = [
        origin[0] - forward[0] * SWING_START_BEHIND_EYE,
        origin[1] - forward[1] * SWING_START_BEHIND_EYE,
        origin[2] - forward[2] * SWING_START_BEHIND_EYE,
    ];

    let mut scan = SwingScan { start, trigger_end: None, best: None };
    let mut nearest = 1.0f32;
    let lines = melee_trace_count(width, height);
    for (index, offset) in MELEE_TRACE_OFFSETS.iter().take(lines).enumerate() {
        let end = melee_trace_end(origin, forward, right, up, range, width, height, *offset);
        let (segments, _) = bullet_trace_segments_filtered(
            world.clip_brushes(),
            world.clip_bsp(),
            world.clip_cmodels(),
            world.clip_mesh(),
            &query.players.poses,
            &query.entities.rows,
            &BulletTraceQuery {
                start,
                end,
                mask: MASK_BULLET_WORLD,
                ignore: Some(attacker),
                ignore_hit: None,
                ignore_model: None,
            },
            BulletPenFacts::default(),
            world.penetration_table(),
            &pane_stands,
            None,
        );
        let touched = segments.iter().find(|segment| segment.collider.is_some());
        if index == 0 {
            scan.trigger_end = Some(touched.map_or(end, |segment| segment.end));
        }
        let Some(segment) = touched else {
            continue;
        };
        if segment.surface_flags & SURFACE_NO_MELEE != 0 {
            continue;
        }
        let line = length_squared(sub(end, start));
        if line <= 0.0 {
            continue;
        }
        let fraction = length_squared(sub(segment.end, start)).sqrt() / line.sqrt();
        if fraction >= 1.0 {
            continue;
        }
        if matches!(segment.collider, Some(ColliderId::Player { .. })) {
            scan.best = Some(*segment);
            break;
        }
        if fraction > nearest {
            continue;
        }
        nearest = fraction;
        scan.best = Some(*segment);
    }
    scan
}

/// The server's melee hit: traces the swing box ahead of the eye and damages what it finds.
#[allow(clippy::too_many_arguments)]
pub(crate) fn fire_melee_hit(
    world: &mut FrameWorld,
    tick: Tick,
    attacker: ClientId,
    attacker_life: LifeSequence,
    weapon: u32,
    origin: [f32; 3],
    angles: [f32; 3],
    swing: MeleeSwing,
) {
    if swing.damage <= 0 {
        return;
    }
    let forward = game_math::angle_vectors(angles).0;
    let scan = scan_swing(world, tick, attacker, origin, angles, swing.range);

    if let Some(to) = scan.trigger_end
        && world.publishes_snapshot()
    {
        crate::t6::damage_triggers::bullet_path(world.ecs(), attacker, scan.start, to, swing.damage, "MOD_MELEE");
    }
    let Some(segment) = scan.best else {
        if shot_log() {
            diag::info!(Sim, "melee hit: attacker {} touched nothing", attacker.0);
        }
        return;
    };

    let amount = swing.damage + (world.combat_rng_mut().next_u32() % 5) as i32;
    // A riot shield takes a swing it stops (the scripts hear "riotshield" and hurt no one). A riot-shield bash into a
    // shield still reaches the player, with no hit location.
    let on_shield = matches!(segment.collider, Some(ColliderId::Player { hitloc: HITLOC_SHIELD, .. }));
    let other = struck_entity_number(world, segment.collider);

    // Weapon_Melee_internal's sound and blood. On a BO2 map its rules decide (crate::t6::melee); otherwise a player or
    // a model's bones struck is a hit that bleeds, and a shield or anything else a miss.
    let bo2 = crate::t6::melee::runs(world.ecs());
    let verdict = if bo2 {
        bo2_verdict(world, attacker, segment.collider, on_shield)
    } else {
        let bleeds = !on_shield
            && matches!(
                segment.collider,
                Some(ColliderId::Player { .. } | ColliderId::EntityDObjBone { .. })
            );
        crate::t6::melee::Verdict { hit: bleeds, shield: false, blood: bleeds }
    };
    let victim_alive = match segment.collider {
        Some(ColliderId::Player { client, .. }) => world
            .client_meta(client)
            .is_some_and(|meta| meta.lifecycle == ClientLifecycle::Alive),
        _ => true,
    };
    let glass = match segment.collider {
        Some(ColliderId::World { .. }) => glass_piece_from_hit(segment.hit_type, segment.hit_id),
        _ => None,
    };
    if shot_log() {
        let word = if verdict.hit { "hit" } else { "miss" };
        diag::info!(Sim, "melee hit: attacker {} hit {other} ({word}) for {amount}", attacker.0);
    }

    let describe = |event_parm| crate::EntityEventPayload {
        number: attacker.0 as i32,
        attacker_entity_num: attacker.0 as i32,
        other_entity_num: other,
        event_parm,
        weapon,
        origin: segment.end,
        direction: forward,
        surf_type: segment.surf_type,
        surface_flags: segment.surface_flags,
        ..Default::default()
    };
    // BO2 sends the blood first, then the hit or miss; a pane of glass gets neither (it breaks instead).
    if verdict.blood && victim_alive {
        world.push_entity_event(tick, EventAudience::All, EntityEventKind::MELEE_BLOOD, describe(0));
    }
    if !(bo2 && glass.is_some()) {
        // The parm: bit 0 = a knife, not a shield bash (the hit sound); bit 1 = a riot shield took it.
        let parm = i32::from(swing.knife) | if verdict.shield { MELEE_PARM_SHIELD } else { 0 };
        let kind = if verdict.hit { EntityEventKind::MELEE_HIT } else { EntityEventKind::MELEE_MISS };
        world.push_entity_event(tick, EventAudience::All, kind, describe(parm));
    }

    match segment.collider {
        Some(ColliderId::Player { client, life, hitloc }) if victim_alive => {
            let attempt = crate::DamageAttempt {
                source: DamageSource::Melee,
                pellet: PelletId(0),
                attacker,
                attacker_life,
                target: client,
                target_life: life,
                weapon,
                amount,
                killcam_entity_start_time: 0,
                inflictor_origin: Some(origin),
                hitloc: if on_shield && !swing.knife { 0 } else { hitloc },
                blast: false,
                penetrated: false,
                impact: false,
            };
            let _ = crate::damage::apply_damage_attempt(world, tick, &attempt);
        }
        // A script entity (a zombie, a planted riot shield, a car) takes the knife as it takes a bullet.
        Some(ColliderId::EntityDObjBone { owner, bone, .. }) => {
            hurt_entity(world, owner, Some(usize::from(bone)), &segment, attacker, weapon, amount, forward);
        }
        Some(ColliderId::EntityLinkedBrush { owner, .. }) => {
            hurt_entity(world, owner, None, &segment, attacker, weapon, amount, forward);
        }
        Some(ColliderId::World { .. }) => {
            if let Some(piece) = glass {
                break_pane(world, tick, piece, segment.end, forward);
            }
        }
        _ => {}
    }
}

/// The entity number the swing struck: a player's slot, a script entity's slot, or the world.
fn struck_entity_number(world: &FrameWorld, collider: Option<ColliderId>) -> i32 {
    let world_slot = i32::from(trace_t6::ENTITYNUM_WORLD);
    match collider {
        Some(ColliderId::Player { client, .. }) => client.0 as i32,
        Some(ColliderId::EntityDObjBone { owner, .. } | ColliderId::EntityLinkedBrush { owner, .. }) => owner
            .script_model()
            .and_then(|model| world.gentity_number(model))
            .unwrap_or(world_slot),
        _ => world_slot,
    }
}

/// BO2's rules for what a swing at `collider` sounds like and whether it bleeds.
fn bo2_verdict(
    world: &mut FrameWorld,
    attacker: ClientId,
    collider: Option<ColliderId>,
    on_shield: bool,
) -> crate::t6::melee::Verdict {
    let struck = match collider {
        Some(ColliderId::Player { client, .. }) => {
            Some(crate::t6::melee::Struck::Player { client: client.0, shield: on_shield })
        }
        Some(ColliderId::EntityDObjBone { owner, .. } | ColliderId::EntityLinkedBrush { owner, .. }) => {
            owner.script_model().map(crate::t6::melee::Struck::Model)
        }
        _ => None,
    };
    struck
        .and_then(|struck| crate::t6::melee::verdict(world.ecs(), attacker.0, struck))
        .unwrap_or_default()
}

#[allow(clippy::too_many_arguments)]
fn hurt_entity(
    world: &mut FrameWorld,
    owner: AuthorityModelOwner,
    bone: Option<usize>,
    segment: &BulletTraceSegment,
    attacker: ClientId,
    weapon: u32,
    amount: i32,
    dir: [f32; 3],
) {
    let Some(target) = owner.script_model() else {
        return;
    };
    crate::script::damage_entity(
        world.ecs(),
        &crate::script::EntityHit {
            target,
            amount,
            attacker: Some(attacker),
            means: "MOD_MELEE",
            weapon,
            point: segment.end,
            dir,
            bone,
            flags: 0,
        },
    );
}

/// Black Ops II: a melee swing breaks a glass pane at once.
fn break_pane(world: &mut FrameWorld, tick: Tick, piece: u32, at: [f32; 3], dir: [f32; 3]) {
    let at_time_ms = i32::try_from(tick.0.saturating_mul(crate::MATCH_TICK_MS)).unwrap_or(i32::MAX);
    let mut holdrand = *world.stuck_holdrand_mut();
    if world.world_objects().is_t6_glass(piece) {
        world.world_objects_mut().apply_t6_glass_damage(
            piece,
            crate::world_objects::T6_GLASS_MELEE_DAMAGE,
            crate::world_objects::T6_MOD_MELEE,
            at_time_ms,
            at,
            dir,
            &mut || crate::item::random_unit(&mut holdrand),
        );
    }
    *world.stuck_holdrand_mut() = holdrand;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shield_parm_is_its_own_bit() {
        assert_eq!(MELEE_PARM_SHIELD & 1, 0);
        assert_eq!(i32::from(true) | MELEE_PARM_SHIELD, 3);
    }

    #[test]
    fn vector_helpers_work_on_all_three_axes() {
        assert_eq!(sub([5.0, 7.0, 9.0], [1.0, 2.0, 3.0]), [4.0, 5.0, 6.0]);
        assert_eq!(length_squared([1.0, 2.0, 2.0]), 9.0);
    }
}
