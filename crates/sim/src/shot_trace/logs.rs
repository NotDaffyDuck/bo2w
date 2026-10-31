//! Test-aid logging for bullets. Each switch is an environment variable read once.

use std::sync::OnceLock;

use super::collider::{ShotRayRequest, HitTargetId, HITLOC_SHIELD, RayResult};
use super::ray_boxes::{pose_bones_bounds, pose_box, ray_aabb_box, RayBox};
use super::poses::PlayerHitboxPose;
use super::ray_walk::Strike;
use super::SOLID_PLAYER_BODY;

fn env_switch(cell: &'static OnceLock<bool>, called: &str) -> bool {
    *cell.get_or_init(|| std::env::var_os(called).is_some())
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
    if crate::gunfire::damage_log() {
        diag::info!(Sim, "bo2mp damage: bullet stopped ({reason}) at ({:.1} {:.1} {:.1})", at[0], at[1], at[2]);
    }
}

fn hitloc_name(hitloc: u8) -> &'static str {
    if hitloc == HITLOC_SHIELD {
        "riotshield"
    } else {
        crate::zombie_rt::T6_HITLOC_NAMES.get(usize::from(hitloc)).copied().unwrap_or("none")
    }
}

fn describe_outcome(result_kind: &RayResult, holder: &PlayerHitboxPose) -> String {
    match result_kind {
        RayResult::Hit { struck: HitTargetId::Player { client, hitloc, .. }, fraction, .. }
            if *client == holder.client =>
        {
            if *hitloc == HITLOC_SHIELD {
                format!("hit SHIELD at {fraction:.3}")
            } else {
                format!("hit BODY {} at {fraction:.3}", hitloc_name(*hitloc))
            }
        }
        RayResult::Hit { fraction, .. } => format!("blocked by something else at {fraction:.3}"),
        RayResult::BeganInside { .. } => "started inside something".to_string(),
        _ => "MISSED him".to_string(),
    }
}

/// For each bullet that reaches a player carrying a riot shield: whether it hit the shield or a part of his body
/// (and which), or went by him.
pub(super) fn log_shield_holders(
    players: &[PlayerHitboxPose],
    query: &ShotRayRequest,
    reach: [f32; 3],
    strikes: &[Strike],
    outcome: &RayResult,
) {
    for holder in players {
        let shield_boxes = holder.joints.iter().filter(|rhs| rhs.body_part_category == HITLOC_SHIELD).count();
        let carries_shield = holder.shield.is_some() || shield_boxes > 0;
        let bullet_can_see_him = query.ignore != Some(holder.client)
            && query.skip_client_hit != Some(holder.client)
            && query.bits & SOLID_PLAYER_BODY != 0;
        if !carries_shield || !bullet_can_see_him {
            continue;
        }
        let (mins, maxs) = pose_bones_bounds(holder).unwrap_or_else(|| pose_box(holder));
        if matches!(ray_aabb_box(query.from, reach, mins, maxs), RayBox::Miss) {
            continue;
        }
        let mut crossed: Vec<(f32, &'static str)> = strikes
            .iter()
            .filter_map(|strike| match strike.collider {
                HitTargetId::Player { client, life, hitloc } if client == holder.client && life == holder.life_sequence => {
                    Some((strike.share, hitloc_name(hitloc)))
                }
                _ => None,
            })
            .collect();
        crossed.sort_by(|lhs, rhs| lhs.0.partial_cmp(&rhs.0).unwrap_or(std::cmp::Ordering::Equal));
        let crossed: Vec<String> = crossed.iter().map(|(fraction, name)| format!("{name}@{fraction:.3}")).collect();
        diag::info!(
            Sim,
            "bo2mp shield: holder {}: bullet {}; his boxes on its line (nearest first) [{}]; shield boxes {shield_boxes}; cone fallback {}",
            holder.client.0,
            describe_outcome(outcome, holder),
            crossed.join(", "),
            if shield_boxes == 0 && holder.shield.is_some() { "in use" } else { "off" }
        );
    }
}
