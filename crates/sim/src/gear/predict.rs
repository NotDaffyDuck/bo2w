//! Looking ahead: where a missile will be a few seconds from now, bouncing off what it meets, for the script host's
//! trajectory previews. Nothing here changes the world.

use entity_t6::flight::path_kind::AT_REST;
use trace_t6::surface_kind_of_flags;

use super::bounce::bounce_velocity;
use super::scene::{missile_bodies, required_facts, script_models_without};
use super::flying::FlyingProjectile;
use super::stick::sticks_to_surface;
use super::vec3::gap;
use crate::step_view::StepWorldView;
use crate::{ShotRayRequest, HitTargetId, SHOT_WORLD_FILTER, SIM_STEP_MILLIS, RayResult, trace_shot_against_models};

/// The spot `missile` reaches within `duration_ms` of `now`, or where it first meets something that stops it (a
/// sticky surface, a player it sticks to, an armed impact fuse). None when the path starts inside solid.
pub(crate) fn predict_projectile(
    world: &StepWorldView,
    mut missile: FlyingProjectile,
    now: i32,
    span_ms: i32,
) -> Option<[f32; 3]> {
    let facts = required_facts(world, missile.gun);
    let models = script_models_without(world, missile.id);
    let content = world.content();
    let (solids, bsp, cmodels, mesh) =
        (content.collision_solids(), content.collision_tree(), content.collision_submodel_set(), content.collision_triangles());
    let players = missile_bodies(world);

    let finish_stamp = now
        .saturating_add(span_ms)
        .min(missile.cleanup_at_ms)
        .min(missile.detonate_at_ms.unwrap_or(i32::MAX));
    let mut time = now;
    while time < finish_stamp {
        if missile.at.tr_type == AT_REST {
            return Some(missile.origin);
        }
        let next = time.saturating_add(SIM_STEP_MILLIS as i32).min(finish_stamp);
        crate::homing::steer(world, &mut missile, next, facts);
        let target = missile.spot_at(next);
        let mask = SHOT_WORLD_FILTER;
        let query = ShotRayRequest {
            from: missile.origin,
            to: target,
            bits: mask,
            ignore: Some(missile.owner),
            skip_client_hit: Default::default(),
            ignore_model: None,
        };
        let swept = trace_shot_against_models(
            &solids,
            &bsp,
            &cmodels,
            &mesh,
            &players,
            &models,
            &query,
            &|piece| world.world_objects().pane_blocks_shots(u32::from(piece)),
        );
        match swept {
            RayResult::Miss { .. } => {
                missile.travel_distance += gap(target, missile.origin);
                missile.origin = target;
            }
            RayResult::Hit { end: till, normal, struck: collider, fraction } => {
                missile.travel_distance += gap(till, missile.origin);
                let hit_player = matches!(collider, HitTargetId::Player { .. });
                let sticks = if hit_player { facts.adheres_to_bodies } else { sticks_to_surface(facts, normal) };
                let stops_here =
                    sticks || (facts.shell_detonate_on_hit && missile.is_forecast_live(facts.shell_arming_range));
                if stops_here {
                    return Some(till);
                }
                let surface = match collider {
                    HitTargetId::World { surface_flags: face_bits, .. }
                    | HitTargetId::PosedBone { surface_flags: face_bits, .. }
                    | HitTargetId::AttachedBrush { surface_flags: face_bits, .. } => surface_kind_of_flags(face_bits),
                    HitTargetId::Player { .. } => 0,
                };
                let hit_time = time + ((next - time) as f32 * fraction) as i32;
                missile.motion = missile.speed_vector_at(hit_time);
                bounce_velocity(&mut missile, normal, &facts, surface);
                missile.origin = till;
                missile.at.base_point = till;
                missile.at.began_at = next;
                missile.at.rate = missile.motion;
            }
            RayResult::BeganInside { .. } | RayResult::Invalid { .. } => return None,
        }
        time = next;
    }
    Some(missile.origin)
}
