use weapon_t6::penetration::{PierceCapability, BulletPierceDepthChart, FACE_KIND_BODY};

use super::ray_boxes::{RayBox, ray_aabb_box, ray_obb};
use super::*;
use crate::match_ids::{LifeCount, ScriptedModel};
use crate::sim_parts::{SeatNo, SolidBrush, CollisionTreeData, CollisionSubmodels, CollisionTriangleData, StepNo};

const NOT_GLASS: &dyn Fn(u16) -> bool = &|_| false;

fn query(start: [f32; 3], till: [f32; 3]) -> ShotRayRequest {
    ShotRayRequest { from: start, to: till, bits: MASK_SHOT, ignore: None, skip_client_hit: None, ignore_model: None }
}

fn box_brush(lo: [f32; 3], hi: [f32; 3], surface: u32) -> SolidBrush {
    SolidBrush {
        sides: vec![
            [-1.0, 0.0, 0.0, -lo[0]],
            [1.0, 0.0, 0.0, hi[0]],
            [0.0, -1.0, 0.0, -lo[1]],
            [0.0, 1.0, 0.0, hi[1]],
            [0.0, 0.0, -1.0, -lo[2]],
            [0.0, 0.0, 1.0, hi[2]],
        ],
        solid_bits: HARD_SURFACE_FLAG,
        plane_material_flags: vec![surface; 6],
        glass_hit_code: 0,
        vertex_points: Vec::new(),
        hit_surface_flags: surface,
    }
}

fn standing_pose(client: u32, shield: Option<ShieldCover>) -> PlayerHitboxPose {
    PlayerHitboxPose {
        client: SeatNo(client),
        origin: [0.0; 3],
        yaw: 0.0,
        lo: crate::frame_step::STANDING_FOOTPRINT_LOW,
        hi: [15.0, 15.0, 72.0],
        life_sequence: LifeCount(1),
        hit_volume: HitboxShapeKind::Standing,
        joints: Vec::new(),
        shield,
        shield_tag: None,
    }
}

// ---- blending between two kept frames ------------------------------------------------------------------------

fn turned_pose(origin: [f32; 3], yaw: f32) -> PlayerHitboxPose {
    let (sin, cos) = yaw.to_radians().sin_cos();
    let bone = xmodel_runtime::HitShapeBone {
        bone: 1,
        body_part_category: 0,
        center: [origin[0] + 10.0 * cos, origin[1] + 10.0 * sin, origin[2] + 60.0],
        axes: [[cos, sin, 0.0], [-sin, cos, 0.0], [0.0, 0.0, 1.0]],
        half_ext: [3.0, 3.0, 3.0],
    };
    PlayerHitboxPose {
        origin,
        yaw,
        joints: vec![bone.clone()],
        shield: Some(ShieldCover { facing: [cos, sin], cos_limit: 0.5 }),
        shield_tag: Some(bone),
        ..standing_pose(1, None)
    }
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

fn near3(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| near(*x, y))
}

#[test]
fn a_blend_moves_and_turns_the_body_part_way() {
    let blended = turned_pose([0.0, 0.0, 0.0], 0.0).blended_toward(&turned_pose([40.0, 0.0, 8.0], 90.0), 0.25);
    let expected = turned_pose([10.0, 0.0, 2.0], 22.5);
    assert!(near3(blended.origin, expected.origin));
    assert!(near(blended.yaw, 22.5));
    // The boxes are carried with him; only BO2's origin and angles move.
    let (got, want) = (&blended.joints[0], &expected.joints[0]);
    assert!(near3(got.center, want.center));
    assert!(got.axes.iter().zip(want.axes).all(|(a, b)| near3(*a, b)));
    let facing = blended.shield.expect("shield").facing;
    assert!(near(facing[0], 22.5_f32.to_radians().cos()) && near(facing[1], 22.5_f32.to_radians().sin()));
    assert!(near3(blended.shield_tag.expect("tag").center, want.center));
    assert_eq!((blended.lo, blended.hi), (expected.lo, expected.hi));
}

#[test]
fn a_blend_turns_the_short_way_round() {
    let from = turned_pose([0.0; 3], 170.0);
    let to = turned_pose([0.0; 3], -170.0);
    assert!(near(from.blended_toward(&to, 0.25).yaw, 175.0));
    assert!(near(from.blended_toward(&to, 0.5).yaw.abs(), 180.0));
    assert!(near(from.blended_toward(&to, 0.75).yaw, -175.0));
}

#[test]
fn a_blend_at_either_end_is_that_end() {
    let from = turned_pose([0.0, 0.0, 0.0], 30.0);
    let to = turned_pose([20.0, -10.0, 0.0], 120.0);
    for (progress, end) in [(0.0, &from), (1.0, &to)] {
        let blended = from.blended_toward(&to, progress);
        assert!(near3(blended.origin, end.origin));
        assert!(near(blended.yaw, end.yaw));
        assert!(near3(blended.joints[0].center, end.joints[0].center));
    }
}

// ---- a mover between two kept frames -------------------------------------------------------------------------

/// Script model 7 at `at` facing `yaw`: one brush bolted at the same place, one box 10 units in front of it, and its
/// triangles placed there.
fn mover(at: [f32; 3], yaw: f32) -> EntityRayShape {
    let (sin, cos) = yaw.to_radians().sin_cos();
    let placement = super::posed_coll::matrix_from_angles(at.into(), [0.0, yaw, 0.0]);
    EntityRayShape {
        placed_at: Some((at, [0.0, yaw, 0.0])),
        attached_solids: vec![AttachedSolidShape { collision_model_ref: 3, origin: at, turn: [0.0, yaw, 0.0] }],
        collision: Some(ServerModelCollision {
            bones: vec![xmodel_runtime::HitShapeBone {
                bone: 1,
                body_part_category: 0,
                center: [at[0] + 10.0 * cos, at[1] + 10.0 * sin, at[2]],
                axes: [[cos, sin, 0.0], [-sin, cos, 0.0], [0.0, 0.0, 1.0]],
                half_ext: [2.0, 4.0, 6.0],
            }],
            coll: Some(ServerModelHitTrace {
                model: std::sync::Arc::new(DObjColl { collision_detail_tier: 0, solid_bits: 0, triangle_list: Vec::new(), surfs: Vec::new() }),
                placement_matrix: placement,
                bones: Vec::new(),
                hidden_parts: [0; xmodel_runtime::HiddenParts::BIT_CHUNK_COUNT],
            }),
            face_bits: 0,
        }),
        ..body(7, 0.0)
    }
}

fn same_mover(got: &EntityRayShape, want: &EntityRayShape) -> bool {
    let (Some((at, aim)), Some((want_at, want_aim))) = (got.placed_at, want.placed_at) else { return false };
    let (got_box, want_box) = (&got.collision.as_ref().unwrap().bones[0], &want.collision.as_ref().unwrap().bones[0]);
    let matrix = |row: &EntityRayShape| row.collision.as_ref().unwrap().coll.as_ref().unwrap().placement_matrix;
    let (solid, want_solid) = (&got.attached_solids[0], &want.attached_solids[0]);
    near3(at, want_at)
        && near3(aim, want_aim)
        && near3(got_box.center, want_box.center)
        && got_box.axes.iter().zip(want_box.axes).all(|(a, b)| near3(*a, b))
        && got_box.half_ext == want_box.half_ext
        && matrix(got).abs_diff_eq(matrix(want), 1e-3)
        && near3(solid.origin, want_solid.origin)
        && near3(solid.turn, want_solid.turn)
}

#[test]
fn a_mover_blend_moves_and_turns_its_brush_box_and_triangles() {
    let blended = mover([0.0, 0.0, 0.0], 0.0).blended_toward(&mover([40.0, 20.0, 8.0], 60.0), 0.25);
    assert!(same_mover(&blended, &mover([10.0, 5.0, 2.0], 15.0)));
}

#[test]
fn a_mover_blend_turns_the_short_way_round() {
    let blended = mover([0.0; 3], 170.0).blended_toward(&mover([0.0; 3], -170.0), 0.5);
    let yaw = blended.placed_at.unwrap().1[1];
    assert!(near(yaw.rem_euclid(360.0), 180.0));
    assert!(near(blended.attached_solids[0].turn[1].rem_euclid(360.0), 180.0));
}

#[test]
fn a_mover_that_did_not_move_is_left_exactly_as_it_was() {
    let still = mover([12.0, -4.0, 3.0], 270.0);
    assert_eq!(still.blended_toward(&still.clone(), 0.5), still);
}

#[test]
fn a_mover_blend_at_either_end_is_that_end() {
    let (from, to) = (mover([0.0; 3], 30.0), mover([20.0, -10.0, 0.0], 120.0));
    assert!(same_mover(&from.blended_toward(&to, 0.0), &from));
    assert!(same_mover(&from.blended_toward(&to, 1.0), &to));
}

// ---- a riot shield ------------------------------------------------------------------------------------------

/// A bullet flying through the player's chest from `from_x` to the opposite side; returns the part it hit.
fn part_hit_from(players: &[PlayerHitboxPose], from_x: f32) -> Option<u8> {
    match trace_shot(&[], players, &query([from_x, 0.0, 40.0], [-from_x, 0.0, 40.0]), NOT_GLASS) {
        RayResult::Hit { struck: HitTargetId::Player { hitloc, .. }, .. } => Some(hitloc),
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
    let pen = PierceCapability { wall_pierce_class: 1, wall_pierce_scale: 1.0, long_gun_round: false, impact_explode: false };
    let (legs, _) = trace_shot_legs(
        &brushes,
        &[],
        &query([0.0; 3], [1000.0, 0.0, 0.0]),
        pen,
        &BulletPierceDepthChart::empty(),
        NOT_GLASS,
    );
    legs.iter().map(|leg| leg.to[0]).fold(0.0, f32::max)
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

// ---- passing through walls ---------------------------------------------------------------------------------

/// The legs of a class-3 bullet fired down +x through walls spanning x `lo..hi`, each of a surface 40 units deep for
/// it.
fn legs_through_walls(walls: &[(f32, f32)]) -> Vec<ShotPathLeg> {
    let brushes: Vec<_> = walls.iter().map(|&(lo, hi)| box_brush([lo, -50.0, -50.0], [hi, 50.0, 50.0], 2 << 20)).collect();
    let mut table = BulletPierceDepthChart::empty();
    table.put_depth(3, 2, 40.0);
    let pen = PierceCapability { wall_pierce_class: 3, wall_pierce_scale: 1.0, long_gun_round: false, impact_explode: false };
    trace_shot_legs(&brushes, &[], &query([0.0; 3], [2000.0, 0.0, 0.0]), pen, &table, NOT_GLASS).0
}

/// How far short of a brush face a trace stops (trace_t6 SURFACE_CLIP_SLACK, BO2's SURFACE_CLIP_EPSILON); a bullet
/// measures a wall this much thicker on each side.
const CLIP: f32 = 0.125;

/// A leg that ends where the bullet came out of a surface.
fn leaves_a_surface(leg: &ShotPathLeg) -> bool {
    leg.face_bits & fx_t6::runtime::HIT_ON_WAY_OUT_BIT != 0
}

#[test]
fn a_bullet_strikes_six_thin_walls_and_pays_for_five() {
    // BO2: one pass per wall and five passes; the wall the fifth pass reaches is struck but not paid for, and the
    // seventh is never reached.
    let walls: Vec<_> = (1..=7).map(|n| (n as f32 * 100.0, n as f32 * 100.0 + 0.5)).collect();
    let legs = legs_through_walls(&walls);
    let struck: Vec<f32> = legs.iter().filter(|leg| !leaves_a_surface(leg)).map(|leg| leg.to[0]).collect();
    assert_eq!(struck.len(), 6, "{struck:?}");
    assert!(struck.iter().zip(1..).all(|(x, n)| (x - (n as f32 * 100.0 - CLIP)).abs() < 0.01), "{struck:?}");
    let paid: Vec<_> = legs.iter().filter(|leg| leaves_a_surface(leg)).collect();
    assert_eq!(paid.len(), 5);
    // Three quarters of a unit (half a unit plus the clip on each face) counts as the thinnest a surface can be, 1.
    assert!(paid.iter().all(|leg| (leg.depth - 1.0).abs() < 1e-4 && (leg.to[0] - leg.from[0] - (0.5 + 2.0 * CLIP)).abs() < 0.01));
    let last = legs.last().expect("legs");
    assert!((last.to[0] - (600.0 - CLIP)).abs() < 0.01 && (last.harm_multiplier - (1.0 - 5.0 / 40.0)).abs() < 1e-4);
}

#[test]
fn ten_units_of_a_forty_unit_surface_cost_a_quarter() {
    // A wall 9.75 thick measures 10 with the clip on both faces.
    let legs = legs_through_walls(&[(100.0, 109.75), (200.0, 200.5)]);
    let exit = legs.iter().find(|leg| leaves_a_surface(leg)).expect("an exit");
    assert!((exit.depth - 10.0).abs() < 1e-3 && (exit.to[0] - (109.75 + CLIP)).abs() < 0.01, "{exit:?}");
    let next = legs.iter().find(|leg| !leaves_a_surface(leg) && (leg.to[0] - (200.0 - CLIP)).abs() < 0.01).expect("the next wall");
    assert!((next.harm_multiplier - 0.75).abs() < 1e-4, "{next:?}");
}

#[test]
fn a_bullet_that_cannot_penetrate_goes_through_at_most_twelve_panes() {
    let panes: Vec<_> = (1..=14)
        .map(|n| {
            let x = n as f32 * 10.0;
            SolidBrush {
                solid_bits: HARD_SURFACE_FLAG | weapon_t6::penetration::BRUSH_IS_PANE,
                ..box_brush([x, -50.0, -50.0], [x + 0.1, 50.0, 50.0], 0)
            }
        })
        .collect();
    let (legs, _) = trace_shot_legs(
        &panes,
        &[],
        &query([0.0; 3], [1000.0, 0.0, 0.0]),
        PierceCapability::default(),
        &BulletPierceDepthChart::empty(),
        NOT_GLASS,
    );
    let reached: Vec<f32> = legs.iter().map(|leg| leg.to[0]).collect();
    assert_eq!(reached.len(), 12, "{reached:?}");
    assert!((reached[11] - (120.0 - CLIP)).abs() < 0.01, "{reached:?}");
}

// ---- single traces ------------------------------------------------------------------------------------------

#[test]
fn nearest_of_two_brushes_is_hit() {
    let brushes = [
        box_brush([300.0, -50.0, -50.0], [320.0, 50.0, 50.0], 0),
        box_brush([100.0, -50.0, -50.0], [150.0, 50.0, 50.0], 0),
    ];
    match trace_shot(&brushes, &[], &query([0.0; 3], [1000.0, 0.0, 0.0]), NOT_GLASS) {
        RayResult::Hit { fraction, end, normal, struck: collider } => {
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
    let outcome = trace_shot(&[], &[], &query([0.0; 3], [10.0, 20.0, 30.0]), NOT_GLASS);
    assert_eq!(outcome, RayResult::Miss { end: [10.0, 20.0, 30.0] });
}

#[test]
fn a_line_that_is_not_a_number_is_refused() {
    let outcome = trace_shot(&[], &[], &query([f32::NAN, 0.0, 0.0], [1.0, 0.0, 0.0]), NOT_GLASS);
    assert_eq!(outcome, RayResult::Invalid { reason: RayRejectCause::InfiniteOrNanInput });
}

#[test]
fn the_shooter_is_never_hit_and_a_player_already_hit_is_skipped() {
    let bystander = [standing_pose(7, None)];
    let mut q = query([200.0, 0.0, 40.0], [-200.0, 0.0, 40.0]);
    assert!(matches!(trace_shot(&[], &bystander, &q, NOT_GLASS), RayResult::Hit { .. }));
    q.ignore = Some(SeatNo(7));
    assert!(matches!(trace_shot(&[], &bystander, &q, NOT_GLASS), RayResult::Miss { .. }));
    q.ignore = None;
    q.skip_client_hit = Some(SeatNo(7));
    assert!(matches!(trace_shot(&[], &bystander, &q, NOT_GLASS), RayResult::Miss { .. }));
}

#[test]
fn a_mask_without_the_body_bit_goes_through_players() {
    let bystander = [standing_pose(7, None)];
    let mut q = query([200.0, 0.0, 40.0], [-200.0, 0.0, 40.0]);
    q.bits = SHOT_WORLD_FILTER & !SOLID_PLAYER_BODY;
    assert!(matches!(trace_shot(&[], &bystander, &q, NOT_GLASS), RayResult::Miss { .. }));
}

#[test]
fn a_player_in_front_of_a_wall_is_hit_first() {
    let wall = [box_brush([-300.0, -50.0, -50.0], [-280.0, 50.0, 100.0], 0)];
    let people = [standing_pose(2, None)];
    match trace_shot(&wall, &people, &query([200.0, 0.0, 40.0], [-400.0, 0.0, 40.0]), NOT_GLASS) {
        RayResult::Hit { struck, .. } => assert!(struck.is_player()),
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
            assert!((hit.share - 0.4).abs() < 1e-6);
            assert_eq!(hit.face_dir, [-1.0, 0.0, 0.0]);
            assert!((hit.finish_point[0] + 1.0).abs() < 1e-5 && hit.finish_point[1] == 0.0);
        }
        _ => panic!("expected a hit"),
    }
    match ray_aabb_box([0.0, 5.0, 0.0], [0.0, -5.0, 0.0], lo, hi) {
        RayBox::Hit(hit) => assert_eq!(hit.face_dir, [0.0, 1.0, 0.0]),
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

fn unit_bone_at(center: [f32; 3], part: u8) -> xmodel_runtime::HitShapeBone {
    xmodel_runtime::HitShapeBone {
        bone: 0,
        body_part_category: part,
        center,
        axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        half_ext: [2.0, 2.0, 2.0],
    }
}

fn world_collider() -> HitTargetId {
    HitTargetId::World { surface_flags: 0, contents: HARD_SURFACE_FLAG, glass_hit_code: 0 }
}

#[test]
fn oriented_box_hit_matches_the_axis_aligned_one_when_unrotated() {
    let bone = unit_bone_at([10.0, 0.0, 0.0], 3);
    let hit = ray_obb([0.0; 3], [20.0, 0.0, 0.0], &bone, world_collider(), None).expect("a hit");
    assert!((hit.share - 0.4).abs() < 1e-5, "{hit:?}");
    assert_eq!(hit.face_dir, [-1.0, 0.0, 0.0]);
}

#[test]
fn oriented_box_follows_its_axes() {
    // Turned a quarter turn about z, the box's local x axis points along world y.
    let mut bone = unit_bone_at([0.0, 10.0, 0.0], 3);
    bone.axes = [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
    bone.half_ext = [2.0, 5.0, 2.0];
    let hit = ray_obb([0.0; 3], [0.0, 20.0, 0.0], &bone, world_collider(), None).expect("a hit");
    assert!((hit.share - 0.4).abs() < 1e-5, "{hit:?}");
    assert_eq!(hit.face_dir, [0.0, -1.0, 0.0]);
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
    assert_eq!(hit.share, 0.0);
    assert_eq!(hit.face_dir, [1.0, 0.0, 0.0]);
    // Without a middle to head for, nothing.
    assert!(ray_obb(from, [0.5, 0.0, 0.0], &bone, world_collider(), None).is_none());
}

// ---- colliders ----------------------------------------------------------------------------------------------

#[test]
fn a_player_is_flesh_and_a_shield_is_its_own_surface() {
    let flesh = HitTargetId::Player { client: SeatNo(1), life: LifeCount(1), hitloc: 2 };
    let shield = HitTargetId::Player { client: SeatNo(1), life: LifeCount(1), hitloc: HITLOC_SHIELD };
    assert_eq!(shield.surf_type(), SURF_TYPE_RIOTSHIELD as u8);
    assert_ne!(flesh.surf_type(), shield.surf_type());
    assert_eq!(shield.surface_flags(), SURF_TYPE_RIOTSHIELD << 20);
    assert!(shield.is_shield() && !flesh.is_shield());
    assert!(flesh.is_damageable() && !world_collider().is_damageable());
    assert_eq!(flesh.client(), Some(SeatNo(1)));
    assert_eq!(world_collider().client(), None);
}

#[test]
fn only_world_surfaces_carry_contents_and_panes() {
    let glass = HitTargetId::World { surface_flags: 0, contents: 0x10, glass_hit_code: 5 };
    assert_eq!(glass.contents(), 0x10);
    assert_eq!(glass.pane(), Some(4));
    assert_eq!(world_collider().pane(), None);
    let model = HitTargetId::AttachedBrush {
        owner: ServerModelSource::ScriptModel(ScriptedModel(9)),
        collision_model_ref: 1,
        surface_flags: 0,
    };
    assert_eq!(model.contents(), 0);
    assert_eq!(model.script_model(), Some(ScriptedModel(9)));
    assert_eq!(model.pane(), None);
}

#[test]
fn glass_hits_are_told_apart_from_other_entity_hits() {
    let glass = HitTargetId::World { surface_flags: 0, contents: 0, glass_hit_code: 5 };
    assert_eq!(glass.hit_kind(false).1, 5);
    assert_eq!(pane_from_hit(glass.hit_kind(false).0, 5), Some(4));
    // Started inside the pane: reported as the world, not as glass.
    assert_eq!(pane_from_hit(glass.hit_kind(true).0, 5), None);
    assert_eq!(pane_from_hit(glass.hit_kind(false).0, 0), None);
}

#[test]
fn sky_is_told_by_its_surface_bit() {
    assert!(is_sky(HitTargetId::World { surface_flags: SURF_SKY | 0x100, contents: 0, glass_hit_code: 0 }));
    assert!(!is_sky(world_collider()));
}

#[test]
fn contents_with_nothing_set_match_any_mask() {
    assert!(contents_match_mask(None, MASK_SHOT));
    assert!(contents_match_mask(Some(0), 0));
    assert!(contents_match_mask(Some(HARD_SURFACE_FLAG), MASK_SHOT));
    assert!(!contents_match_mask(Some(SOLID_PLAYER_BODY), HARD_SURFACE_FLAG));
}

// ---- stored frames ------------------------------------------------------------------------------------------

fn frame_of(tick: u32, clients: &[u32]) -> RewindFrame {
    RewindFrame {
        tick: StepNo(tick),
        phase: RewindStage::AfterPlayerMove,
        poses: clients.iter().map(|&c| standing_pose(c, None)).collect(),
    }
}

#[test]
fn history_keeps_only_the_newest_frames() {
    let mut history = HitboxTimeline::with_capacity(3);
    for tick in 1..=5 {
        history.record_sample(frame_of(tick, &[1]));
    }
    assert!(history.sample_for(StepNo(2)).is_none());
    assert!(history.sample_for(StepNo(3)).is_some());
    assert_eq!(history.latest().map(|f| f.tick), Some(StepNo(5)));
    assert_eq!(history.capacity(), 3);
}

#[test]
fn a_frame_for_the_tick_already_stored_replaces_it() {
    let mut history = HitboxTimeline::with_capacity(4);
    history.record_sample(frame_of(7, &[1]));
    history.record_sample(frame_of(7, &[1, 2]));
    assert_eq!(history.sample_for(StepNo(7)).map(|f| f.poses.len()), Some(2));
    history.push(StepNo(8), vec![standing_pose(4, None)]);
    let (tick, poses) = history.latest_poses().expect("a frame");
    assert_eq!((tick, poses.len()), (StepNo(8), 1));
    history.clear();
    assert!(history.latest().is_none());
}

#[test]
fn history_never_has_zero_capacity() {
    assert_eq!(HitboxTimeline::with_capacity(0).capacity(), 1);
    assert_eq!(EntityHitboxTimeline::with_capacity(0).capacity(), 1);
    assert_eq!(HitboxTimeline::default().capacity(), HITBOX_TIMELINE_LENGTH);
}

#[test]
fn entity_history_files_frames_the_same_way() {
    let mut history = EntityHitboxTimeline::with_capacity(2);
    for tick in [10, 11, 12] {
        history.record_sample(EntityHitboxSnapshot { tick: StepNo(tick), phase: RewindStage::AfterPlayerMove, rows: Vec::new() });
    }
    assert!(history.sample_for(StepNo(10)).is_none());
    assert!(history.sample_for(StepNo(11)).is_some());
    assert_eq!(history.latest().map(|f| f.tick), Some(StepNo(12)));
}

// ---- a shot's claim about what it saw -----------------------------------------------------------------------

fn claim(left: u32, right: u32, alpha: f32, quality: ShotRecordGrade) -> ShotRecordOrigin {
    ShotRecordOrigin { left: StepNo(left), right: StepNo(right), opacity: alpha, quality }
}

#[test]
fn a_claim_holds_up_when_its_frames_are_in_order_and_not_from_the_future() {
    let now = StepNo(20);
    assert!(claim(18, 19, 0.5, ShotRecordGrade::SmoothedBetweenFrames).still_fresh_at(now));
    assert!(claim(20, 20, 0.0, ShotRecordGrade::Exact).still_fresh_at(now));
    assert!(!claim(19, 18, 0.5, ShotRecordGrade::SmoothedBetweenFrames).still_fresh_at(now));
    assert!(!claim(18, 21, 0.5, ShotRecordGrade::SmoothedBetweenFrames).still_fresh_at(now));
    assert!(!claim(18, 19, 1.5, ShotRecordGrade::SmoothedBetweenFrames).still_fresh_at(now));
    assert!(!claim(18, 19, f32::NAN, ShotRecordGrade::SmoothedBetweenFrames).still_fresh_at(now));
}

#[test]
fn only_exact_and_interpolated_claims_ask_for_history() {
    let now = StepNo(20);
    for quality in [ShotRecordGrade::None, ShotRecordGrade::Held, ShotRecordGrade::RanDry] {
        assert!(!quality.asserts_past_record());
        assert!(!claim(18, 19, 0.5, quality).still_fresh_at(now));
    }
    assert!(!ShotRecordOrigin::UNCLAIMED.still_fresh_at(now));
    assert_eq!(claim(18, 19, 0.5, ShotRecordGrade::Exact).asked_for_tick(), StepNo(18));
}

// ---- bo2zm: bullets through zombies ----------------------------------------------------------------------

/// A zombie-sized body: one hit box 16 deep, at `x` on the shot line.
fn body(id: u32, x: f32) -> EntityRayShape {
    EntityRayShape {
        placed_at: None,
        owner: ServerModelSource::ScriptModel(ScriptedModel(id)),
        epoch: EntityCollisionMoment::LiveMoment,
        collision: Some(ServerModelCollision {
            bones: vec![xmodel_runtime::HitShapeBone {
                bone: 1,
                body_part_category: 2,
                center: [x, 0.0, 40.0],
                axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
                half_ext: [8.0, 12.0, 30.0],
            }],
            coll: None,
            face_bits: 0,
        }),
        model_content_mask: None,
        mesh_asset_name: None,
        attached_solids: Vec::new(),
    }
}

/// The bodies a shot hits down the line (id, damage multiplier), exit marks left out.
fn bodies_hit(pen_type: i32) -> Vec<(u32, f32)> {
    // Black Ops II's flesh depths (info/bullet_penetration_mp).
    let mut table = BulletPierceDepthChart::empty();
    table.put_depth(1, FACE_KIND_BODY as usize, 32.0);
    table.put_depth(2, FACE_KIND_BODY as usize, 64.0);
    let pen = PierceCapability { wall_pierce_class: pen_type, wall_pierce_scale: 1.0, long_gun_round: false, impact_explode: false };
    let models = [body(1, 100.0), body(2, 150.0), body(3, 200.0)];
    let (legs, _) = shot_legs_against_models(
        &[],
        &CollisionTreeData::default(),
        &CollisionSubmodels::default(),
        &CollisionTriangleData::default(),
        &[],
        &models,
        &query([0.0, 0.0, 40.0], [1000.0, 0.0, 40.0]),
        pen,
        &table,
        NOT_GLASS,
    );
    legs
        .iter()
        .filter(|s| s.face_bits & fx_t6::runtime::HIT_ON_WAY_OUT_BIT == 0)
        .filter_map(|s| Some((s.collider?.script_model()?.0, s.harm_multiplier)))
        .collect()
}

#[test]
fn a_pistol_bullet_goes_through_a_zombie_into_the_next() {
    let hits = bodies_hit(1);
    assert_eq!(hits.iter().map(|h| h.0).collect::<Vec<_>>(), [1, 2], "{hits:?}");
    // 16 of 32 units of flesh used: half damage to the second.
    assert!((hits[1].1 - 0.5).abs() < 0.05, "{hits:?}");
}

#[test]
fn a_rifle_bullet_goes_through_three() {
    let hits = bodies_hit(2);
    assert_eq!(hits.iter().map(|h| h.0).collect::<Vec<_>>(), [1, 2, 3], "{hits:?}");
}
