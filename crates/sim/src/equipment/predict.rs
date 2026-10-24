//! Looking ahead: where a missile will be a few seconds from now, bouncing off what it meets, for the script host's
//! trajectory previews. Nothing here changes the world.

use entity_t6::trajectory::tr_type::STATIONARY;
use trace_t6::surface_type_from_flags;

use super::bounce::bounce_velocity;
use super::scene::{missile_bodies, required_facts, script_models_without};
use super::state::ProjectileState;
use super::stick::sticks_to_surface;
use super::vec3::gap;
use crate::frame::FrameWorld;
use crate::{BulletTraceQuery, ColliderId, MASK_BULLET_WORLD, MATCH_TICK_MS, TraceOutcome, bullet_trace_with_entity_models};

/// The spot `missile` reaches within `duration_ms` of `now`, or where it first meets something that stops it (a
/// sticky surface, a player it sticks to, an armed impact fuse). None when the path starts inside solid.
pub(crate) fn predict_projectile(
    world: &FrameWorld,
    mut missile: ProjectileState,
    now: i32,
    duration_ms: i32,
) -> Option<[f32; 3]> {
    let facts = required_facts(world, missile.weapon);
    let models = script_models_without(world, missile.id);
    let content = world.content();
    let (brushes, bsp, cmodels, mesh) =
        (content.clip_brushes(), content.clip_bsp(), content.clip_cmodels(), content.clip_mesh());
    let players = missile_bodies(world);

    let end_time = now
        .saturating_add(duration_ms)
        .min(missile.cleanup_at_ms)
        .min(missile.detonate_at_ms.unwrap_or(i32::MAX));
    let mut time = now;
    while time < end_time {
        if missile.pos.tr_type == STATIONARY {
            return Some(missile.origin);
        }
        let next = time.saturating_add(MATCH_TICK_MS as i32).min(end_time);
        crate::missile_guidance::steer(world, &mut missile, next, facts);
        let target = missile.origin_at(next);
        let query = BulletTraceQuery {
            start: missile.origin,
            end: target,
            mask: MASK_BULLET_WORLD,
            ignore: Some(missile.owner),
            ignore_hit: None,
            ignore_model: None,
        };
        let outcome = bullet_trace_with_entity_models(
            &brushes,
            &bsp,
            &cmodels,
            &mesh,
            &players,
            &models,
            &query,
            &|piece| world.world_objects().glass_is_solid(u32::from(piece)),
        );
        match outcome {
            TraceOutcome::Miss { .. } => {
                missile.travel_distance += gap(target, missile.origin);
                missile.origin = target;
            }
            TraceOutcome::Hit { end, normal, collider, fraction } => {
                missile.travel_distance += gap(end, missile.origin);
                let hit_player = matches!(collider, ColliderId::Player { .. });
                let sticks = if hit_player { facts.stick_to_players } else { sticks_to_surface(facts, normal) };
                let stops_here =
                    sticks || (facts.proj_impact_explode && missile.is_armed(facts.projectile_activate_dist));
                if stops_here {
                    return Some(end);
                }
                let surface = match collider {
                    ColliderId::World { surface_flags, .. }
                    | ColliderId::EntityDObjBone { surface_flags, .. }
                    | ColliderId::EntityLinkedBrush { surface_flags, .. } => surface_type_from_flags(surface_flags),
                    ColliderId::Player { .. } => 0,
                };
                let hit_time = time + ((next - time) as f32 * fraction) as i32;
                missile.velocity = missile.velocity_at(hit_time);
                bounce_velocity(&mut missile, normal, &facts, surface);
                missile.origin = end;
                missile.pos.tr_base = end;
                missile.pos.tr_time = next;
                missile.pos.tr_delta = missile.velocity;
            }
            TraceOutcome::StartSolid { .. } | TraceOutcome::Invalid { .. } => return None,
        }
        time = next;
    }
    Some(missile.origin)
}
