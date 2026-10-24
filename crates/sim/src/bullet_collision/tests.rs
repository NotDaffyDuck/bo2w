use weapon_t6::penetration::{BulletPenFacts, PenetrationDepthTable};

use super::geometry::{RayBox, ray_aabb_box, ray_obb};
use super::*;
use crate::identities::{LifeSequence, ScriptModelId};
use crate::world::{ClientId, SimBrush, Tick};

const NOT_GLASS: &dyn Fn(u16) -> bool = &|_| false;

fn query(start: [f32; 3], end: [f32; 3]) -> BulletTraceQuery {
    BulletTraceQuery { start, end, mask: MASK_SHOT, ignore: None, ignore_hit: None, ignore_model: None }
}

fn box_brush(min: [f32; 3], max: [f32; 3], surface: u32) -> SimBrush {
    SimBrush {
        planes: vec![
            [-1.0, 0.0, 0.0, -min[0]],
            [1.0, 0.0, 0.0, max[0]],
            [0.0, -1.0, 0.0, -min[1]],
            [0.0, 1.0, 0.0, max[1]],
            [0.0, 0.0, -1.0, -min[2]],
            [0.0, 0.0, 1.0, max[2]],
        ],
        contents: CONTENTS_SOLID,
        plane_surface_flags: vec![surface; 6],
        glass_encoded: 0,
        verts: Vec::new(),
        hit_surface_flags: surface,
    }
}

fn standing_pose(client: u32, shield: Option<ShieldCover>) -> PlayerCollisionPose {
    PlayerCollisionPose {
        client: ClientId(client),
        origin: [0.0; 3],
        mins: [-15.0, -15.0, 0.0],
        maxs: [15.0, 15.0, 72.0],
        life_sequence: LifeSequence(1),
        hit_volume: HitVolumeKind::Standing,
        bones: Vec::new(),
        shield,
        shield_tag: None,
    }
}

// ---- a riot shield ------------------------------------------------------------------------------------------

/// A bullet flying through the player's chest from `from_x` to the opposite side; returns the part it hit.
fn part_hit_from(players: &[PlayerCollisionPose], from_x: f32) -> Option<u8> {
    match bullet_trace(&[], players, &query([from_x, 0.0, 40.0], [-from_x, 0.0, 40.0]), NOT_GLASS) {
        TraceOutcome::Hit { collider: ColliderId::Player { hitloc, .. }, .. } => Some(hitloc),
        _ => None,
    }
}

fn is_body_hit(part: Option<u8>) -> bool {
    part.is_some_and(|p| p != HITLOC_SHIELD)
}

/// A shield facing +x that covers 60 degrees to either side.
const FACING_PLUS_X: ShieldCover = ShieldCover { facing: [1.0, 0.0], cos_limit: 0.5 };

#[test]
fn shield_takes_a_bullet_from_the_side_it_faces() {
    assert_eq!(part_hit_from(&[standing_pose(3, Some(FACING_PLUS_X))], 200.0), Some(HITLOC_SHIELD));
}

#[test]
fn shield_leaves_the_body_open_from_behind() {
    assert!(is_body_hit(part_hit_from(&[standing_pose(3, Some(FACING_PLUS_X))], -200.0)));
}

#[test]
fn no_shield_no_shield_hit() {
    assert!(is_body_hit(part_hit_from(&[standing_pose(3, None)], 200.0)));
}

#[test]
fn shield_turned_away_does_not_cover() {
    let turned = ShieldCover { facing: [0.0, 1.0], cos_limit: 0.5 };
    assert!(is_body_hit(part_hit_from(&[standing_pose(3, Some(turned))], 200.0)));
}

#[test]
fn shield_cover_angle_is_a_cosine_of_the_direction_toward_the_shooter() {
    assert!(FACING_PLUS_X.stops([-1.0, 0.0, 0.0]));
    assert!(!FACING_PLUS_X.stops([1.0, 0.0, 0.0]));
    // Shooting straight down has no flat direction, so nothing can be said to face it.
    assert!(!FACING_PLUS_X.stops([0.0, 0.0, -1.0]));
    // 60 degrees off the facing is just inside, 70 just outside.
    let off = |deg: f32| [-deg.to_radians().cos(), -deg.to_radians().sin(), 0.0];
    assert!(FACING_PLUS_X.stops(off(59.0)));
    assert!(!FACING_PLUS_X.stops(off(70.0)));
}

// ---- sky ----------------------------------------------------------------------------------------------------

/// The furthest x a bullet fired down +x reaches, through a brush at x 100..150 made of `first_surface`, then a wall
/// at x 300. The penetration table gives nothing any depth.
fn farthest_reach(first_surface: u32) -> f32 {
    let brushes = [
        box_brush([100.0, -50.0, -50.0], [150.0, 50.0, 50.0], first_surface),
        box_brush([300.0, -50.0, -50.0], [320.0, 50.0, 50.0], 2 << 20),
    ];
    let pen = BulletPenFacts { penetrate_type: 1, penetrate_multiplier: 1.0, rifle_bullet: false, impact_explode: false };
    let (legs, _) = bullet_trace_segments(
        &brushes,
        &[],
        &query([0.0; 3], [1000.0, 0.0, 0.0]),
        pen,
        &PenetrationDepthTable::empty(),
        NOT_GLASS,
    );
    legs.iter().map(|leg| leg.end[0]).fold(0.0, f32::max)
}

#[test]
fn sky_is_always_penetrable() {
    let x = farthest_reach(SURF_SKY);
    assert!((x - 300.0).abs() < 1.0, "reached {x}");
}

#[test]
fn ordinary_surface_with_no_depth_stops_the_bullet() {
    let x = farthest_reach(2 << 20);
    assert!((x - 100.0).abs() < 1.0, "reached {x}");
}

// ---- single traces ------------------------------------------------------------------------------------------

#[test]
fn nearest_of_two_brushes_is_hit() {
    let brushes = [
        box_brush([300.0, -50.0, -50.0], [320.0, 50.0, 50.0], 0),
        box_brush([100.0, -50.0, -50.0], [150.0, 50.0, 50.0], 0),
    ];
    match bullet_trace(&brushes, &[], &query([0.0; 3], [1000.0, 0.0, 0.0]), NOT_GLASS) {
        TraceOutcome::Hit { fraction, end, normal, collider } => {
            assert!((fraction - 0.1).abs() < 0.001, "fraction {fraction}");
            assert!((end[0] - 100.0).abs() < 0.5, "end {end:?}");
            assert_eq!(normal, [-1.0, 0.0, 0.0]);
            assert!(collider.is_world());
        }
        other => panic!("expected a hit, got {other:?}"),
    }
}

#[test]
fn a_line_that_touches_nothing_misses_to_its_end() {
    let outcome = bullet_trace(&[], &[], &query([0.0; 3], [10.0, 20.0, 30.0]), NOT_GLASS);
    assert_eq!(outcome, TraceOutcome::Miss { end: [10.0, 20.0, 30.0] });
}

#[test]
fn a_line_that_is_not_a_number_is_refused() {
    let outcome = bullet_trace(&[], &[], &query([f32::NAN, 0.0, 0.0], [1.0, 0.0, 0.0]), NOT_GLASS);
    assert_eq!(outcome, TraceOutcome::Invalid { reason: TraceInvalidReason::NonFiniteInput });
}

#[test]
fn the_shooter_is_never_hit_and_a_player_already_hit_is_skipped() {
    let bystander = [standing_pose(7, None)];
    let mut q = query([200.0, 0.0, 40.0], [-200.0, 0.0, 40.0]);
    assert!(matches!(bullet_trace(&[], &bystander, &q, NOT_GLASS), TraceOutcome::Hit { .. }));
    q.ignore = Some(ClientId(7));
    assert!(matches!(bullet_trace(&[], &bystander, &q, NOT_GLASS), TraceOutcome::Miss { .. }));
    q.ignore = None;
    q.ignore_hit = Some(ClientId(7));
    assert!(matches!(bullet_trace(&[], &bystander, &q, NOT_GLASS), TraceOutcome::Miss { .. }));
}

#[test]
fn a_mask_without_the_body_bit_goes_through_players() {
    let bystander = [standing_pose(7, None)];
    let mut q = query([200.0, 0.0, 40.0], [-200.0, 0.0, 40.0]);
    q.mask = MASK_BULLET_WORLD & !CONTENTS_BODY;
    assert!(matches!(bullet_trace(&[], &bystander, &q, NOT_GLASS), TraceOutcome::Miss { .. }));
}

#[test]
fn a_player_in_front_of_a_wall_is_hit_first() {
    let wall = [box_brush([-300.0, -50.0, -50.0], [-280.0, 50.0, 100.0], 0)];
    let people = [standing_pose(2, None)];
    match bullet_trace(&wall, &people, &query([200.0, 0.0, 40.0], [-400.0, 0.0, 40.0]), NOT_GLASS) {
        TraceOutcome::Hit { collider, .. } => assert!(collider.is_player()),
        other => panic!("expected a player hit, got {other:?}"),
    }
}

// ---- boxes --------------------------------------------------------------------------------------------------

#[test]
fn aabb_ray_reports_the_face_it_enters_through() {
    let lo = [-1.0; 3];
    let hi = [1.0; 3];
    match ray_aabb_box([-5.0, 0.0, 0.0], [5.0, 0.0, 0.0], lo, hi) {
        RayBox::Hit(hit) => {
            assert!((hit.fraction - 0.4).abs() < 1e-6);
            assert_eq!(hit.normal, [-1.0, 0.0, 0.0]);
            assert!((hit.endpos[0] + 1.0).abs() < 1e-5 && hit.endpos[1] == 0.0);
        }
        _ => panic!("expected a hit"),
    }
    match ray_aabb_box([0.0, 5.0, 0.0], [0.0, -5.0, 0.0], lo, hi) {
        RayBox::Hit(hit) => assert_eq!(hit.normal, [0.0, 1.0, 0.0]),
        _ => panic!("expected a hit"),
    }
}

#[test]
fn aabb_ray_misses_inside_and_short() {
    let lo = [-1.0; 3];
    let hi = [1.0; 3];
    assert!(matches!(ray_aabb_box([-5.0, 3.0, 0.0], [5.0, 3.0, 0.0], lo, hi), RayBox::Miss));
    assert!(matches!(ray_aabb_box([-5.0, 0.0, 0.0], [-3.0, 0.0, 0.0], lo, hi), RayBox::Miss));
    assert!(matches!(ray_aabb_box([0.0; 3], [5.0, 0.0, 0.0], lo, hi), RayBox::Inside));
}

fn unit_bone_at(center: [f32; 3], part: u8) -> AuthorityDObjCollisionBone {
    AuthorityDObjCollisionBone {
        bone: 0,
        part_classification: part,
        center,
        axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        half_size: [2.0, 2.0, 2.0],
    }
}

fn world_collider() -> ColliderId {
    ColliderId::World { surface_flags: 0, contents: CONTENTS_SOLID, glass_encoded: 0 }
}

#[test]
fn oriented_box_hit_matches_the_axis_aligned_one_when_unrotated() {
    let bone = unit_bone_at([10.0, 0.0, 0.0], 3);
    let hit = ray_obb([0.0; 3], [20.0, 0.0, 0.0], &bone, world_collider(), None).expect("a hit");
    assert!((hit.fraction - 0.4).abs() < 1e-5, "{hit:?}");
    assert_eq!(hit.normal, [-1.0, 0.0, 0.0]);
}

#[test]
fn oriented_box_follows_its_axes() {
    // Turned a quarter turn about z, the box's local x axis points along world y.
    let mut bone = unit_bone_at([0.0, 10.0, 0.0], 3);
    bone.axes = [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
    bone.half_size = [2.0, 5.0, 2.0];
    let hit = ray_obb([0.0; 3], [0.0, 20.0, 0.0], &bone, world_collider(), None).expect("a hit");
    assert!((hit.fraction - 0.4).abs() < 1e-5, "{hit:?}");
    assert_eq!(hit.normal, [0.0, -1.0, 0.0]);
    // 3 units to the side along world x is inside the 5-unit local y extent, 6 is outside.
    assert!(ray_obb([3.0, 0.0, 0.0], [3.0, 20.0, 0.0], &bone, world_collider(), None).is_some());
    assert!(ray_obb([6.0, 0.0, 0.0], [6.0, 20.0, 0.0], &bone, world_collider(), None).is_none());
}

#[test]
fn a_line_that_begins_inside_a_part_box_gets_no_hit_unless_it_heads_for_the_middle() {
    let bone = unit_bone_at([0.0; 3], 3);
    let from = [1.0, 0.0, 0.0];
    // Leaving the box: nothing.
    assert!(ray_obb(from, [20.0, 0.0, 0.0], &bone, world_collider(), Some([0.0; 3])).is_none());
    // Heading in toward the body's middle and ending inside: a hit right at the start, facing away from the middle.
    let hit = ray_obb(from, [0.5, 0.0, 0.0], &bone, world_collider(), Some([0.0; 3])).expect("a hit");
    assert_eq!(hit.fraction, 0.0);
    assert_eq!(hit.normal, [1.0, 0.0, 0.0]);
    // Without a middle to head for, nothing.
    assert!(ray_obb(from, [0.5, 0.0, 0.0], &bone, world_collider(), None).is_none());
}

// ---- colliders ----------------------------------------------------------------------------------------------

#[test]
fn a_player_is_flesh_and_a_shield_is_its_own_surface() {
    let flesh = ColliderId::Player { client: ClientId(1), life: LifeSequence(1), hitloc: 2 };
    let shield = ColliderId::Player { client: ClientId(1), life: LifeSequence(1), hitloc: HITLOC_SHIELD };
    assert_eq!(shield.surf_type(), SURF_TYPE_RIOTSHIELD as u8);
    assert_ne!(flesh.surf_type(), shield.surf_type());
    assert_eq!(shield.surface_flags(), SURF_TYPE_RIOTSHIELD << 20);
    assert!(shield.is_shield() && !flesh.is_shield());
    assert!(flesh.is_damageable() && !world_collider().is_damageable());
    assert_eq!(flesh.client(), Some(ClientId(1)));
    assert_eq!(world_collider().client(), None);
}

#[test]
fn only_world_surfaces_carry_contents_and_panes() {
    let glass = ColliderId::World { surface_flags: 0, contents: 0x10, glass_encoded: 5 };
    assert_eq!(glass.contents(), 0x10);
    assert_eq!(glass.pane(), Some(4));
    assert_eq!(world_collider().pane(), None);
    let model = ColliderId::EntityLinkedBrush {
        owner: AuthorityModelOwner::ScriptModel(ScriptModelId(9)),
        cmodel_handle: 1,
        surface_flags: 0,
    };
    assert_eq!(model.contents(), 0);
    assert_eq!(model.script_model(), Some(ScriptModelId(9)));
    assert_eq!(model.pane(), None);
}

#[test]
fn glass_hits_are_told_apart_from_other_entity_hits() {
    let glass = ColliderId::World { surface_flags: 0, contents: 0, glass_encoded: 5 };
    assert_eq!(glass.hit_kind(false).1, 5);
    assert_eq!(glass_piece_from_hit(glass.hit_kind(false).0, 5), Some(4));
    // Started inside the pane: reported as the world, not as glass.
    assert_eq!(glass_piece_from_hit(glass.hit_kind(true).0, 5), None);
    assert_eq!(glass_piece_from_hit(glass.hit_kind(false).0, 0), None);
}

#[test]
fn sky_is_told_by_its_surface_bit() {
    assert!(is_sky(ColliderId::World { surface_flags: SURF_SKY | 0x100, contents: 0, glass_encoded: 0 }));
    assert!(!is_sky(world_collider()));
}

#[test]
fn contents_with_nothing_set_match_any_mask() {
    assert!(contents_match_mask(None, MASK_SHOT));
    assert!(contents_match_mask(Some(0), 0));
    assert!(contents_match_mask(Some(CONTENTS_SOLID), MASK_SHOT));
    assert!(!contents_match_mask(Some(CONTENTS_BODY), CONTENTS_SOLID));
}

// ---- stored frames ------------------------------------------------------------------------------------------

fn frame_of(tick: u32, clients: &[u32]) -> HistoryFrame {
    HistoryFrame {
        tick: Tick(tick),
        phase: HistoryPhase::PostMovement,
        poses: clients.iter().map(|&c| standing_pose(c, None)).collect(),
    }
}

#[test]
fn history_keeps_only_the_newest_frames() {
    let mut history = CollisionHistory::with_capacity(3);
    for tick in 1..=5 {
        history.push_frame(frame_of(tick, &[1]));
    }
    assert!(history.frame_at(Tick(2)).is_none());
    assert!(history.frame_at(Tick(3)).is_some());
    assert_eq!(history.latest().map(|f| f.tick), Some(Tick(5)));
    assert_eq!(history.capacity(), 3);
}

#[test]
fn a_frame_for_the_tick_already_stored_replaces_it() {
    let mut history = CollisionHistory::with_capacity(4);
    history.push_frame(frame_of(7, &[1]));
    history.push_frame(frame_of(7, &[1, 2]));
    assert_eq!(history.frame_at(Tick(7)).map(|f| f.poses.len()), Some(2));
    history.push(Tick(8), vec![standing_pose(4, None)]);
    let (tick, poses) = history.latest_poses().expect("a frame");
    assert_eq!((tick, poses.len()), (Tick(8), 1));
    history.clear();
    assert!(history.latest().is_none());
}

#[test]
fn history_never_has_zero_capacity() {
    assert_eq!(CollisionHistory::with_capacity(0).capacity(), 1);
    assert_eq!(EntityCollisionHistory::with_capacity(0).capacity(), 1);
    assert_eq!(CollisionHistory::default().capacity(), COLLISION_HISTORY_TICKS);
}

#[test]
fn entity_history_files_frames_the_same_way() {
    let mut history = EntityCollisionHistory::with_capacity(2);
    for tick in [10, 11, 12] {
        history.push_frame(EntityCollisionFrame { tick: Tick(tick), phase: HistoryPhase::PostMovement, rows: Vec::new() });
    }
    assert!(history.frame_at(Tick(10)).is_none());
    assert!(history.frame_at(Tick(11)).is_some());
    assert_eq!(history.latest().map(|f| f.tick), Some(Tick(12)));
}

// ---- a shot's claim about what it saw -----------------------------------------------------------------------

fn claim(left: u32, right: u32, alpha: f32, quality: ShotSampleQuality) -> ShotSampleProvenance {
    ShotSampleProvenance { left: Tick(left), right: Tick(right), alpha, quality }
}

#[test]
fn a_claim_holds_up_when_its_frames_are_in_order_and_not_from_the_future() {
    let now = Tick(20);
    assert!(claim(18, 19, 0.5, ShotSampleQuality::Interpolated).is_valid_for(now));
    assert!(claim(20, 20, 0.0, ShotSampleQuality::Exact).is_valid_for(now));
    assert!(!claim(19, 18, 0.5, ShotSampleQuality::Interpolated).is_valid_for(now));
    assert!(!claim(18, 21, 0.5, ShotSampleQuality::Interpolated).is_valid_for(now));
    assert!(!claim(18, 19, 1.5, ShotSampleQuality::Interpolated).is_valid_for(now));
    assert!(!claim(18, 19, f32::NAN, ShotSampleQuality::Interpolated).is_valid_for(now));
}

#[test]
fn only_exact_and_interpolated_claims_ask_for_history() {
    let now = Tick(20);
    for quality in [ShotSampleQuality::None, ShotSampleQuality::Held, ShotSampleQuality::Starved] {
        assert!(!quality.claims_history());
        assert!(!claim(18, 19, 0.5, quality).is_valid_for(now));
    }
    assert!(!ShotSampleProvenance::NO_CLAIM.is_valid_for(now));
    assert_eq!(claim(18, 19, 0.5, ShotSampleQuality::Exact).requested_tick(), Tick(18));
}
