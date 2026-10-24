//! Test-aid logging for bullets. Each switch is an environment variable read once.

use std::sync::OnceLock;

use super::collider::{BulletTraceQuery, ColliderId, HITLOC_SHIELD, TraceOutcome};
use super::geometry::{pose_bones_bounds, pose_box, ray_aabb_box, RayBox};
use super::poses::PlayerCollisionPose;
use super::tracer::Strike;
use super::CONTENTS_BODY;

fn env_switch(cell: &'static OnceLock<bool>, name: &str) -> bool {
    *cell.get_or_init(|| std::env::var_os(name).is_some())
}

/// `BO2MP_HITBOXLOG`: each ray that starts inside a hit box.
pub(super) fn hitbox_log_on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    env_switch(&ON, "BO2MP_HITBOXLOG")
}

/// `BO2MP_SHIELDLOG`: what each bullet did at a player who carries a riot shield.
pub(super) fn shield_log_on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    env_switch(&ON, "BO2MP_SHIELDLOG")
}

/// `BO2MP_DAMAGE_LOG`: why a penetrating bullet stopped early.
pub(super) fn pen_stop_log(reason: &str, at: [f32; 3]) {
    if crate::combat::damage_log() {
        diag::info!(Sim, "bo2mp damage: bullet stopped ({reason}) at ({:.1} {:.1} {:.1})", at[0], at[1], at[2]);
    }
}

fn hitloc_name(hitloc: u8) -> &'static str {
    if hitloc == HITLOC_SHIELD {
        "riotshield"
    } else {
        crate::t6::T6_HITLOC_NAMES.get(usize::from(hitloc)).copied().unwrap_or("none")
    }
}

fn describe_outcome(outcome: &TraceOutcome, holder: &PlayerCollisionPose) -> String {
    match outcome {
        TraceOutcome::Hit { collider: ColliderId::Player { client, hitloc, .. }, fraction, .. }
            if *client == holder.client =>
        {
            if *hitloc == HITLOC_SHIELD {
                format!("hit SHIELD at {fraction:.3}")
            } else {
                format!("hit BODY {} at {fraction:.3}", hitloc_name(*hitloc))
            }
        }
        TraceOutcome::Hit { fraction, .. } => format!("blocked by something else at {fraction:.3}"),
        TraceOutcome::StartSolid { .. } => "started inside something".to_string(),
        _ => "MISSED him".to_string(),
    }
}

/// For each bullet that reaches a player carrying a riot shield: whether it hit the shield or a part of his body
/// (and which), or went by him.
pub(super) fn log_shield_holders(
    players: &[PlayerCollisionPose],
    query: &BulletTraceQuery,
    reach: [f32; 3],
    strikes: &[Strike],
    outcome: &TraceOutcome,
) {
    for pose in players {
        let shield_boxes = pose.bones.iter().filter(|b| b.part_classification == HITLOC_SHIELD).count();
        let carries_shield = pose.shield.is_some() || shield_boxes > 0;
        let bullet_can_see_him = query.ignore != Some(pose.client)
            && query.ignore_hit != Some(pose.client)
            && query.mask & CONTENTS_BODY != 0;
        if !carries_shield || !bullet_can_see_him {
            continue;
        }
        let (mins, maxs) = pose_bones_bounds(pose).unwrap_or_else(|| pose_box(pose));
        if matches!(ray_aabb_box(query.start, reach, mins, maxs), RayBox::Miss) {
            continue;
        }
        let mut crossed: Vec<(f32, &'static str)> = strikes
            .iter()
            .filter_map(|strike| match strike.collider {
                ColliderId::Player { client, life, hitloc } if client == pose.client && life == pose.life_sequence => {
                    Some((strike.fraction, hitloc_name(hitloc)))
                }
                _ => None,
            })
            .collect();
        crossed.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        let crossed: Vec<String> = crossed.iter().map(|(fraction, name)| format!("{name}@{fraction:.3}")).collect();
        diag::info!(
            Sim,
            "bo2mp shield: holder {}: bullet {}; his boxes on its line (nearest first) [{}]; shield boxes {shield_boxes}; cone fallback {}",
            pose.client.0,
            describe_outcome(outcome, pose),
            crossed.join(", "),
            if shield_boxes == 0 && pose.shield.is_some() { "in use" } else { "off" }
        );
    }
}
