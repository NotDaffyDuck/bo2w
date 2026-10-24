//! bo2mp: BO2 Target Finder (ATTACHMENT_RANGEFINDER), the part that needs the presented players:
//! whether the sight screen draws, whether the view trace has spotted an enemy (the screen turns
//! red), and which enemies get a diamond. Facts: Streaks notes facts/target-finder.md (sections
//! 2-4). The renderer places the diamonds through the viewmodel's tag_hud
//! (hud_t6::rangefinder::diamond_rect) and draws the screen into the UI3D texture.
use bevy::prelude::*;
use hud_t6::rangefinder as rf;
use net::{CEntity, CEntityRuntime, FrameClock, LocalPresentClient, PresentedSnapshot};
use playerstate_t6::game::{eflags, get_viewmodel_weapon_index};

use crate::overhead_names::{
    OverheadPosedHead, OverheadPosedPlayerFrame, OverheadPosedPlayerFramePublished,
    aimed_distance, head_visible, live_other_player, name_trace,
};

/// One frame of the Target Finder's sight screen. `draws_screen` off: nothing to draw.
#[derive(Resource, Debug, Default, Clone, PartialEq)]
pub struct RangefinderSpots {
    pub draws_screen: bool,
    /// The colour change: the targeting element list, red.
    pub targeting: bool,
    /// Enemies that get a diamond: origin and stance.
    pub spots: Vec<([f32; 3], rf::Stance)>,
}

pub(crate) fn register(app: &mut App) {
    app.init_resource::<RangefinderSpots>().add_systems(
        Update,
        update_rangefinder_spots
            .after(OverheadPosedPlayerFramePublished)
            .in_set(frame::LifeFrontPublished),
    );
}

// Our wire's stance bits (playerstate_t6::game::eflags); the heights are BO2's (facts section 3).
fn stance(e_flags: u32) -> rf::Stance {
    if e_flags & eflags::PRONE != 0 {
        rf::Stance::Prone
    } else if e_flags & eflags::DUCK != 0 {
        rf::Stance::Crouch
    } else {
        rf::Stance::Stand
    }
}

fn has_perk(perks: u64, perk: u32) -> bool {
    perks & (1u64 << perk) != 0
}

#[allow(clippy::too_many_arguments)]
fn update_rangefinder_spots(
    presented: Res<PresentedSnapshot>,
    local: Res<LocalPresentClient>,
    posed: Res<OverheadPosedPlayerFrame>,
    players: Query<(&CEntity, &CEntityRuntime)>,
    cg_clock: Res<FrameClock>,
    weapons: Option<Res<assets::PreparedWeapons>>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    prediction: Option<Res<net::ClientPredictionState>>,
    mut out: ResMut<RangefinderSpots>,
    mut latch: Local<rf::SpotLatch>,
) {
    let mut next = RangefinderSpots::default();
    let spotted = 'frame: {
        let Some(snapshot) = presented.snapshot() else { break 'frame false };
        let Some(ps) = presented.alive_player(local.0) else { break 'frame false };
        let has = weapons
            .as_ref()
            .and_then(|w| w.0.facts_of(get_viewmodel_weapon_index(ps)))
            .is_some_and(|facts| facts.rangefinder);
        // EMP-jammed: the same playerstate bit the weapon bar's emp_jammed reads.
        let emp_jammed = ps.other_flags & 0x400 != 0;
        next.draws_screen = rf::draws_screen(has, ps.f_weapon_pos_frac, emp_jammed);
        if !next.draws_screen {
            break 'frame false;
        }
        let Some(local_meta) = snapshot.meta.for_client(local.0) else { break 'frame false };
        let Some((_, transform)) = cameras.iter().find(|(c, _)| c.is_active) else {
            break 'frame false;
        };
        let Some(world) = prediction
            .as_ref()
            .filter(|p| p.0.is_armed() && p.0.world().has_world_clip())
            .map(|p| p.0.world())
        else {
            break 'frame false;
        };
        let eye = transform.translation();
        let forward = *transform.forward();
        let ffa = local_meta.client_state_team == 0;
        // The view trace: the world's distance along the aim, then the nearest player box before it.
        let wall_dist = name_trace(world, eye, eye + forward * rf::TRACE_RANGE).0 * rf::TRACE_RANGE;
        let mut aimed: Option<(f32, rf::TraceTarget)> = None;
        for (identity, runtime) in &players {
            let Some((_, meta)) = live_other_player(snapshot, local.0, identity, runtime) else {
                continue;
            };
            let enemy = ffa || meta.client_state_team != local_meta.client_state_team;
            let origin = Vec3::from_array(runtime.origin);
            let stance = stance(runtime.next_state.lerp.e_flags as u32);
            let head = match posed.get(identity.number()) {
                Some(OverheadPosedHead::ExactWorld(head)) => Vec3::from_array(head),
                _ => origin + Vec3::Z * rf::stance_height(stance),
            };
            if let Some(d) = aimed_distance(eye, forward, origin, head)
                && d <= wall_dist
                && aimed.is_none_or(|(best, _)| d < best)
            {
                // Perk 42 PERK_NONAME blocks spotting (facts section 4).
                let has_noname = has_perk(meta.t6_perks, rf::PERK_NONAME);
                aimed = Some((d, rf::TraceTarget { is_client: true, enemy, has_noname }));
            }
            // Perk 35 PERK_IMMUNERANGEFINDER: no diamond (facts section 3).
            if enemy
                && !has_perk(meta.t6_perks, rf::PERK_IMMUNE_RANGEFINDER)
                && head_visible(world, eye, head)
            {
                next.spots.push((runtime.origin, stance));
            }
        }
        rf::active(has, ps.f_weapon_pos_frac)
            && rf::spotted(
                aimed.map(|a| a.1),
                false,
                aimed.map_or(0.0, |a| a.0),
                rf::MIN_ENEMY_SPOTTING_DIST,
            )
    };
    next.targeting = latch.update(spotted, cg_clock.time(), rf::COLOR_CHANGE_DELAY_MS);
    if *out != next {
        *out = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stance_reads_our_wire_bits() {
        assert_eq!(stance(0), rf::Stance::Stand);
        assert_eq!(stance(eflags::DUCK), rf::Stance::Crouch);
        assert_eq!(stance(eflags::PRONE | eflags::DUCK), rf::Stance::Prone);
    }

    #[test]
    fn perk_bits_are_perk_numbers() {
        assert!(has_perk(1 << 35, rf::PERK_IMMUNE_RANGEFINDER));
        assert!(!has_perk(1 << 35, rf::PERK_NONAME));
        assert!(has_perk(1 << 42, rf::PERK_NONAME));
    }
}
