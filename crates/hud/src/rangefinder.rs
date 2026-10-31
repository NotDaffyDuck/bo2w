//! bo2mp: BO2 Target Finder (ATTACHMENT_RANGEFINDER), the part that needs the presented players:
//! whether the sight screen draws, whether the view trace has spotted an enemy (the screen turns
//! red), and which enemies get a diamond. Facts: Streaks notes facts/target-finder.md (sections
//! 2-4). The renderer places the diamonds through the viewmodel's tag_hud
//! (hud_t6::rangefinder::diamond_rect) and draws the screen into the UI3D texture.
use bevy::prelude::*;
use hud_t6::rangefinder::{COLOR_CHANGE_DELAY_MS, MIN_ENEMY_SPOTTING_DIST, PERK_IMMUNE_RANGEFINDER, PERK_NONAME, SpotLatch, Stance, TRACE_RANGE, TraceTarget, active, draws_screen, spotted, stance_height};
use net::{ClientEntityRecord, ClientEntityPoseState, FrameClock, HostViewClient, ShownWorldFrame};
use playerstate_t6::game::{entity_flag_bits, get_viewmodel_weapon_index};

use crate::name_tags::{
    NameplateSkullPose, NameplateBodyPose, NameplateBodyPoseShared,
    crosshair_hit_distance, clear_line_to, markable_player, tag_trace,
};
use bevy::app::Update;

/// One frame of the Target Finder's sight screen. `draws_screen` off: nothing to draw.
#[derive(Resource, Debug, Default, Clone, PartialEq)]
pub struct RangefinderSpots {
    pub draws_screen: bool,
    /// The colour change: the targeting element list, red.
    pub targeting: bool,
    /// Enemies that get a diamond: origin and stance.
    pub spots: Vec<([f32; 3], Stance)>,
}

pub(crate) fn install_rangefinder(app: &mut App) {
    let after_poses = update_rangefinder_spots.after(NameplateBodyPoseShared);
    app.init_resource::<RangefinderSpots>()
        .add_systems(Update, after_poses.in_set(frame::LifeStatePosted));
}

// Our wire's stance bits (playerstate_t6::game::eflags); the heights are BO2's (facts section 3).
fn stance(e_flags: u32) -> Stance {
    if e_flags & entity_flag_bits::LYING_FLAT != 0 {
        Stance::Prone
    } else if e_flags & entity_flag_bits::CROUCH_BIT != 0 {
        Stance::Crouch
    } else {
        Stance::Stand
    }
}

fn has_perk(perks: u64, perk: u32) -> bool {
    perks & (1u64 << perk) != 0
}

/// The head to test a player's box with: the posed one when the render side
/// has it, else a stance-height guess above the origin.
fn marked_head(pose: Option<NameplateSkullPose>, feet: Vec3, stance: Stance) -> Vec3 {
    match pose {
        Some(NameplateSkullPose::TrueSceneSpace(at)) => Vec3::from_array(at),
        _ => feet + Vec3::Z * stance_height(stance),
    }
}

/// Keeps the nearest player box the view ray enters before the wall.
fn nearer_aim(
    best: &mut Option<(f32, TraceTarget)>,
    hit: Option<f32>,
    wall_dist: f32,
    target: TraceTarget,
) {
    if let Some(d) = hit
        && d <= wall_dist
        && best.is_none_or(|(held, _)| d < held)
    {
        *best = Some((d, target));
    }
}

/// One frame of the Target Finder survey. Fills `next` and says whether an
/// enemy is spotted; `None` when something it needs is missing, in which case
/// nothing is spotted.
#[allow(clippy::too_many_arguments)]
fn survey(
    next: &mut RangefinderSpots,
    presented: &ShownWorldFrame,
    me: sim::ClientId,
    heads: &NameplateBodyPose,
    others: &Query<(&ClientEntityRecord, &ClientEntityPoseState)>,
    armory: Option<&assets::StagedWeapons>,
    eye_at: Option<&GlobalTransform>,
    prediction: Option<&net::ClientGuessState>,
) -> Option<bool> /* None: nothing to survey */ {
    let snapshot = presented.snapshot()?;
    let ps = presented.living_participant(me)?;
    let has_finder = armory
        .and_then(|w| w.0.summary_for(get_viewmodel_weapon_index(ps)))
        .is_some_and(|facts| facts.rangefinder);
    // EMP-jammed: the same playerstate bit the weapon bar's emp_jammed reads.
    let emp_jammed = ps.secondary_state_bits & 0x400 != 0;
    next.draws_screen = draws_screen(has_finder, ps.weapon_raise_fraction, emp_jammed);
    if !next.draws_screen {
        return None;
    }
    let my_meta = snapshot.meta.for_client(me)?;
    let transform = eye_at?;
    let world = prediction
        .filter(|p| p.0.world().has_static_collision() && p.0.is_forecast_live())
        .map(|p| p.0.world())?;
    let eye = transform.translation();
    let forward = *transform.forward();
    let free_for_all = my_meta.client_state_team == 0;
    // The view trace: the world's distance along the aim, then the nearest player box before it.
    let wall_dist = tag_trace(world, eye, eye + forward * TRACE_RANGE).0 * TRACE_RANGE;
    let mut aimed: Option<(f32, TraceTarget)> = None;
    for (ident, live) in others {
        let Some(meta) = markable_player(snapshot, me, ident, live).map(|found| found.1) else {
            continue;
        };
        let enemy = free_for_all || meta.client_state_team != my_meta.client_state_team;
        let origin = Vec3::from_array(live.origin);
        let stance = stance(live.upcoming_condition.lerp.e_flags as u32);
        let head = marked_head(heads.get(ident.number()), origin, stance);
        // Perk 42 PERK_NONAME blocks spotting (facts section 4).
        let has_noname = has_perk(meta.t6_perks, PERK_NONAME);
        let target = TraceTarget { is_client: true, enemy, has_noname };
        nearer_aim(&mut aimed, crosshair_hit_distance(eye, forward, origin, head), wall_dist, target);
        // Perk 35 PERK_IMMUNERANGEFINDER: no diamond (facts section 3).
        if enemy
            && !has_perk(meta.t6_perks, PERK_IMMUNE_RANGEFINDER)
            && clear_line_to(world, eye, head)
        {
            next.spots.push((live.origin, stance));
        }
    }
    Some(
        active(has_finder, ps.weapon_raise_fraction)
            && spotted(
                aimed.map(|a| a.1),
                false,
                aimed.map_or(0.0, |a| a.0),
                MIN_ENEMY_SPOTTING_DIST,
            ),
    )
}

#[allow(clippy::too_many_arguments)]
fn update_rangefinder_spots(
    shown_frame: Res<ShownWorldFrame>,
    local: Res<HostViewClient>,
    heads: Res<NameplateBodyPose>,
    others: Query<(&ClientEntityRecord, &ClientEntityPoseState)>,
    game_clock: Res<FrameClock>,
    armory: Option<Res<assets::StagedWeapons>>,
    view_cameras: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    prediction: Option<Res<net::ClientGuessState>>,
    mut out: ResMut<RangefinderSpots>,
    mut latch: Local<SpotLatch>,
) {
    let mut next = RangefinderSpots::default();
    let eye_at = view_cameras
        .iter()
        .find(|(camera, _)| camera.is_active)
        .map(|(_, transform)| transform);
    let spotted = survey(
        &mut next,
        &shown_frame,
        local.0,
        &heads,
        &others,
        armory.as_deref(),
        eye_at,
        prediction.as_deref(),
    )
    .unwrap_or(false);
    next.targeting = latch.feed(spotted, game_clock.time(), COLOR_CHANGE_DELAY_MS);
    if *out != next {
        *out = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(enemy: bool) -> TraceTarget {
        TraceTarget { is_client: true, enemy, has_noname: false }
    }

    #[test]
    fn stance_reads_our_wire_bits() {
        assert_eq!(stance(0), Stance::Stand);
        assert_eq!(stance(entity_flag_bits::CROUCH_BIT), Stance::Crouch);
        assert_eq!(stance(entity_flag_bits::LYING_FLAT | entity_flag_bits::CROUCH_BIT), Stance::Prone);
    }

    #[test]
    fn perk_bits_are_perk_numbers() {
        assert!(has_perk(1 << 35, PERK_IMMUNE_RANGEFINDER));
        assert!(!has_perk(1 << 35, PERK_NONAME));
        assert!(has_perk(1 << 42, PERK_NONAME));
    }

    #[test]
    fn posed_head_wins_over_the_stance_guess() {
        let origin = Vec3::new(10.0, 20.0, 30.0);
        let posed = Some(NameplateSkullPose::TrueSceneSpace([1.0, 2.0, 3.0]));
        assert_eq!(marked_head(posed, origin, Stance::Stand), Vec3::new(1.0, 2.0, 3.0));
        let guess = marked_head(None, origin, Stance::Prone);
        assert_eq!(guess, origin + Vec3::Z * stance_height(Stance::Prone));
        let unposed = Some(NameplateSkullPose::MissingBodyOrSkull);
        assert_eq!(marked_head(unposed, origin, Stance::Crouch), marked_head(None, origin, Stance::Crouch));
    }

    #[test]
    fn aim_keeps_the_nearest_box_before_the_wall() {
        let mut kept = None;
        nearer_aim(&mut kept, Some(500.0), 400.0, target(true));
        assert!(kept.is_none(), "behind the wall");
        nearer_aim(&mut kept, None, 400.0, target(true));
        assert!(kept.is_none(), "no hit");
        nearer_aim(&mut kept, Some(300.0), 400.0, target(true));
        nearer_aim(&mut kept, Some(350.0), 400.0, target(false));
        let (distance, held) = kept.unwrap();
        assert_eq!(distance, 300.0);
        assert!(held.enemy);
        nearer_aim(&mut kept, Some(100.0), 400.0, target(false));
        let (distance, held) = kept.unwrap();
        assert_eq!(distance, 100.0);
        assert!(!held.enemy);
    }
}
