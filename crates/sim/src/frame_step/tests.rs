//! Tests of the step's movement rules, its hazards and the collision a move is lent.

use bevy_ecs::prelude::World;
use movement_t6::bridge::{SolidProbe, SweepAsk};
use trace_t6::{Trace, TraceHitType};

use super::bo2_map_collision;
use super::solids::{BodyBox, SweepCollisionProvider, OtherVehicle, VehicleClip};
use super::feel::{bo2_feel, bo2_feel_rules};
use super::hazard::{breath_step, fall_damage};
use crate::shot_trace::{AttachedSolidShape, BODY_BOX_UPPER, BODY_BOX_LOWER};
use crate::ent_slots::MoverSlot;
use crate::sim_parts::{SolidBrush, CollisionTreeData, CollisionTriangleData};

mod sent_feel_tests {
    use super::*;

    fn zombies_feel() -> movement_t6::bridge::Bo2Feel {
        movement_t6::bridge::Bo2Feel {
            zombies: true,
            g_speed: 175.0,
            ..movement_t6::bridge::Bo2Feel::BO2_MP
        }
    }

    #[test]
    fn a_second_pc_guesses_with_the_rules_the_game_sent() {
        let feel = zombies_feel();
        assert_eq!(bo2_feel(&World::new(), true, Some(feel)), feel);
    }

    #[test]
    fn the_snapshot_carries_the_rules_to_the_other_pc() {
        let feel = zombies_feel();
        let mut host = crate::sim_whole::WholeSim::default();
        host.frame().set_feel(feel);
        let snapshot = host.snapshot(crate::sim_parts::StepNo(1));
        assert_eq!(snapshot.meta.feel, Some(feel));
        let mut guess = crate::sim_whole::WholeSim::default();
        guess.take_over_from(&snapshot);
        assert_eq!(guess.frame().feel(), Some(feel));
    }
}

mod fall_damage_tests {
    use super::fall_damage;

    #[test]
    fn falls_cost_a_share_of_max_health_rounded_down() {
        // Worked out in floats: event parm 15 on 100 health is 14 damage.
        assert_eq!(fall_damage(15, 100), 14);
        assert_eq!(fall_damage(0, 100), 0);
        assert_eq!(fall_damage(100, 100), 110);
        assert_eq!(fall_damage(250, 150), 165);
    }

    #[test]
    fn a_player_under_water_breathes_for_16_seconds_then_drowns_each_second() {
        use super::breath_step;
        let mut out = 0;
        // Swimming with the head up (level 4) is fine.
        assert!(!breath_step(&mut out, 4, 50_000));
        assert_eq!(out, 50_000);
        // Under: 16 s of breath, the first hurt just after.
        let hurts: Vec<i32> = (1..=400)
            .map(|f| 50_000 + f * 50)
            .filter(|&t| breath_step(&mut out, 6, t))
            .collect();
        assert_eq!(hurts, vec![66_050, 67_050, 68_050, 69_050]);
        // Coming up for one frame refills the breath.
        assert!(!breath_step(&mut out, 1, 70_050));
        assert!(!breath_step(&mut out, 5, 86_050));
        assert!(breath_step(&mut out, 5, 86_100));
    }
}

mod mover_ride_tests {
    use super::*;
    use crate::sim_parts::clip_move_to_bmodels;
    use movement_t6::Bo2Collision;

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-3)
    }

    /// A 64-unit floor 8 thick, its top at its origin.
    fn floor() -> SolidBrush {
        SolidBrush {
            sides: vec![
                [1.0, 0.0, 0.0, 32.0],
                [-1.0, 0.0, 0.0, 32.0],
                [0.0, 1.0, 0.0, 32.0],
                [0.0, -1.0, 0.0, 32.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, -1.0, 8.0],
            ],
            solid_bits: crate::shot_trace::HARD_SURFACE_FLAG,
            plane_material_flags: vec![0; 6],
            glass_hit_code: 0,
            vertex_points: Vec::new(),
            hit_surface_flags: 0,
        }
    }

    #[test]
    fn the_water_surface_is_found_over_and_under_the_feet() {
        use movement_t6::water::{CONTENTS_WATER, NO_WATER};
        // A pool 100 deep, its surface at 0.
        let mut pool = floor();
        pool.solid_bits = CONTENTS_WATER;
        pool.sides[5] = [0.0, 0.0, -1.0, 100.0];
        let brushes = [pool];
        let (bsp, mesh) = (CollisionTreeData::default(), CollisionTriangleData::default());
        let world = bo2_map_collision(&brushes, &bsp, &mesh, false);
        for feet in [-50.0, -99.0, 30.0] {
            let height = world.water_height([0.0, 0.0, feet]);
            assert!(height.abs() < 0.5, "feet {feet}: {height}");
        }
        assert_eq!(world.water_height([100.0, 0.0, -50.0]), NO_WATER, "beside it");
        assert_eq!(world.water_height([0.0, 0.0, 250.0]), NO_WATER, "too high over it");
        // Solid ground is not water.
        let solid = [floor()];
        let world = bo2_map_collision(&solid, &bsp, &mesh, false);
        assert_eq!(world.water_height([0.0, 0.0, 10.0]), NO_WATER);
    }

    #[test]
    fn a_lift_on_its_path_has_a_pose_at_each_command() {
        let mut lift = MoverSlot::default();
        lift.state.slot_index = 40;
        lift.state.lerp.pos.tr_type = entity_t6::flight::path_kind::STRAIGHT;
        lift.state.lerp.pos.began_at = 1000;
        lift.state.lerp.pos.base_point = [0.0, 0.0, 100.0];
        lift.state.lerp.pos.rate = [0.0, 0.0, 200.0];
        lift.state.lerp.apos.tr_type = entity_t6::flight::path_kind::STRAIGHT;
        lift.state.lerp.apos.began_at = 1000;
        lift.state.lerp.apos.rate = [0.0, 90.0, 0.0];
        let mut still = MoverSlot::default();
        still.state.slot_index = 41;
        still.state.lerp.pos.base_point = [5.0, 0.0, 0.0];
        let movers = [lift, still];
        let (bsp, geom) = (CollisionTreeData::default(), CollisionTriangleData::default());
        let backend = SweepCollisionProvider {
            brushes: &[],
            bsp: &bsp,
            mesh: &geom,
            pane_harm: &[],
            player_boxes: &[],
            own_entity_slot: 0,
            cmodels: &[],
            attached_solids: &[],
            linked_owners: &[],
            struck: None,
            touched: None,
            blockers: &[],
            movers: &movers,
            vehicles: &[],
            gjk: false,
            slick: false,
        };
        let (spot, angles) = backend.mover_pose(40, 1500).expect("the lift has a pose");
        assert!(close(spot, [0.0, 0.0, 200.0]), "{spot:?}");
        assert!(close(angles, [0.0, 45.0, 0.0]), "{angles:?}");
        // Standing still: the same pose whenever it is asked.
        assert_eq!(backend.mover_pose(41, 1500), backend.mover_pose(41, 900));
        assert_eq!(backend.mover_pose(41, 1500).map(|p| p.0), Some([5.0, 0.0, 0.0]));
        // Not a script mover: no pose, nothing carried.
        assert_eq!(backend.mover_pose(42, 1500), None);
    }

    #[test]
    fn landing_on_a_brush_model_names_its_entity() {
        let brushes = [floor()];
        // The world (model 0), then the lift: the floor brush.
        let cmodels = [
            asset_world::CollisionSubmodel::default(),
            asset_world::CollisionSubmodel {
                lo: [-32.0, -32.0, -8.0],
                hi: [32.0, 32.0, 0.0],
                radius: 46.0,
                initial_convex_solid: 0,
                solid_volume_total: 1,
            },
        ];
        let store = crate::brush_clip::clip_store(&brushes, &cmodels, &[0]);
        let input = SweepAsk {
            start: [0.0, 0.0, 50.0],
            end: [0.0, 0.0, -50.0],
            lo_corner: crate::frame_step::STANDING_FOOTPRINT_LOW,
            hi_corner: crate::frame_step::STANDING_FOOTPRINT_HIGH,
            collide_filter: crate::shot_trace::HARD_SURFACE_FLAG,
        };
        let lift = AttachedSolidShape {
            collision_model_ref: 1,
            origin: [0.0; 3],
            turn: [0.0; 3],
        };
        let open = Trace::new();
        let hit = store.with_map(|map| clip_move_to_bmodels(open, Some(map), &Default::default(), [(40, &lift)], input));
        assert!(hit.fraction < 1.0, "{hit:?}");
        assert_eq!((hit.impact_category, hit.struck_entity_num), (TraceHitType::Entity, 40));
        // A brush model nobody numbered stays the world's.
        let hit = store.with_map(|map| {
            clip_move_to_bmodels(open, Some(map), &Default::default(), [(trace_t6::WORLD_ENTITY_SLOT, &lift)], input)
        });
        assert!(hit.fraction < 1.0, "{hit:?}");
        assert_eq!(
            trace_t6::entity_hit_slot(&hit),
            trace_t6::WORLD_ENTITY_SLOT
        );
    }

    /// bo2mp: on a BO2 map a body's temporary box stops a line at its
    /// near face, turned or not, with the hit's normal facing the line.
    #[test]
    fn a_bo2_temp_box_stops_a_line_at_its_near_face() {
        use crate::sim_parts::{T6Model, t6_transformed_trace};
        let map = clipmap_t6::ClipMap::default();
        let body = T6Model::TempBox {
            mins: [-5.0; 3],
            maxs: [5.0; 3],
            contents: 1,
        };
        for angles in [[0.0; 3], [0.0, 90.0, 0.0]] {
            let hit = t6_transformed_trace(
                &map,
                [0.0; 3],
                [20.0, 0.0, 0.0],
                [0.0; 3],
                [0.0; 3],
                body,
                u32::MAX,
                [10.0, 0.0, 0.0],
                angles,
            );
            assert!((hit.fraction - 0.25).abs() < 0.01, "{angles:?} {hit:?}");
            assert!(hit.normal[0] < -0.99, "{angles:?} {hit:?}");
            assert!((hit.finish_location([0.0; 3], [20.0, 0.0, 0.0])[0] - 5.0).abs() < 0.2, "{angles:?} {hit:?}");
        }
    }

    /// A driven Dragonfire flown at a player stops at his body (both
    /// collisions), and its own number's box never stops it.
    #[test]
    fn a_vehicle_stops_at_a_players_body_not_its_own() {
        use movement_t6::gjk::MoverBrush;
        use movement_t6::heli::{HeliCollision, HeliControls, HeliDef, HeliState, heli_move};
        let body = |entnum: u16, x: f32| BodyBox {
            entnum,
            origin: [x, 0.0, 0.0],
            bounds: None,
            owner: entnum,
            contents: crate::sim_parts::SOLID_PLAYER_BODY,
        };
        let def = HeliDef {
            top_velocity: 352.0,
            speed_up_rate: 440.0,
            max_speed_vertical: 193.6,
            accel_vertical: 369.6,
            rot_rate: 200.0,
            rot_accel: 1500.0,
            max_body_pitch: 20.0,
            max_body_roll: 25.0,
            tilt_from_accel: [0.01, 0.01],
            tilt_from_decel: [0.05, 0.2],
            tilt_speed: [5.0, 5.0],
            decel: [2.0, 2.0, 2.5],
            tilt_from_axes: 0.4,
            tilt_from_fwd_yaw: 0.5,
            vel_at_max_tilt: 0.75,
            tilt_momentum: 0.1,
            ..HeliDef::default()
        };
        let brush = MoverBrush::new(
            &(0..8)
                .map(|c: u32| {
                    std::array::from_fn(|k| if c >> k & 1 == 0 { [-16.0, -16.0, -10.0][k] } else { [16.0, 16.0, 10.0][k] })
                })
                .collect::<Vec<_>>(),
        )
        .expect("a brush");
        let (bsp, mesh) = (CollisionTreeData::default(), CollisionTriangleData::default());
        for gjk in [false, true] {
            // Player 3 stands 300 ahead; the drone (number 90) has a box of
            // its own where it starts.
            let clip = VehicleClip {
                bodies: vec![body(3, 300.0), body(90, 0.0)],
                attached_solids: Vec::new(),
                blockers: Vec::new(),
                movers: Vec::new(),
                pane_harm: Vec::new(),
                own_entity_slot: 90,
                vehicles: Vec::new(),
            };
            let backend = clip.backend(&[], &bsp, &mesh, &[], gjk);
            let flight = HeliCollision { collision: &backend, brush, gjk };
            let mut st = HeliState { origin: [0.0, 0.0, 40.0], ..HeliState::default() };
            let ahead = HeliControls { forward: 127, ..HeliControls::default() };
            for _ in 0..60 {
                heli_move(&def, &mut st, &ahead, 0.05, &flight);
            }
            // It left its own box and stopped at him (his body is a capsule
            // 15 round, so its corners reach a little past his side, 285).
            assert!(st.origin[0] > 250.0, "{gjk} it left its own box: {st:?}");
            assert!(st.origin[0] < 285.0 && st.velocity[0].abs() < 1.0, "{gjk} it passed him: {st:?}");
        }
    }

    /// A driven Dragonfire flown at another vehicle, a long one turned a
    /// quarter across its way, stops at its turned side (both collisions);
    /// a vehicle never stops at its own.
    #[test]
    fn a_vehicle_stops_at_another_vehicle_turned_to_its_angles() {
        use movement_t6::gjk::MoverBrush;
        use movement_t6::heli::{HeliCollision, HeliControls, HeliDef, HeliState, heli_move};
        let def = HeliDef {
            top_velocity: 352.0,
            speed_up_rate: 440.0,
            max_speed_vertical: 193.6,
            accel_vertical: 369.6,
            rot_rate: 200.0,
            rot_accel: 1500.0,
            max_body_pitch: 20.0,
            max_body_roll: 25.0,
            tilt_from_accel: [0.01, 0.01],
            tilt_from_decel: [0.05, 0.2],
            tilt_speed: [5.0, 5.0],
            decel: [2.0, 2.0, 2.5],
            tilt_from_axes: 0.4,
            tilt_from_fwd_yaw: 0.5,
            vel_at_max_tilt: 0.75,
            tilt_momentum: 0.1,
            ..HeliDef::default()
        };
        let corners = |lo: [f32; 3], hi: [f32; 3]| -> Vec<[f32; 3]> {
            (0..8)
                .map(|c: u32| std::array::from_fn(|k| if c >> k & 1 == 0 { lo[k] } else { hi[k] }))
                .collect()
        };
        let brush = MoverBrush::new(&corners([-16.0, -16.0, -10.0], [16.0, 16.0, 10.0])).expect("a brush");
        // 100 long, 32 wide, at 240 and turned a quarter: its near side is
        // at 224 (unturned it would be at 190).
        let other = OtherVehicle::new(
            9,
            [240.0, 0.0, 40.0],
            [0.0, 90.0, 0.0],
            corners([-50.0, -16.0, -10.0], [50.0, 16.0, 10.0]),
            true,
        )
        .expect("a vehicle");
        let (bsp, mesh) = (CollisionTreeData::default(), CollisionTriangleData::default());
        for gjk in [false, true] {
            for own in [90, 9] {
                let clip = VehicleClip {
                    bodies: Vec::new(),
                    attached_solids: Vec::new(),
                    blockers: Vec::new(),
                    movers: Vec::new(),
                    pane_harm: Vec::new(),
                    own_entity_slot: own,
                    vehicles: vec![other.clone()],
                };
                let backend = clip.backend(&[], &bsp, &mesh, &[], gjk);
                let flight = HeliCollision { collision: &backend, brush, gjk };
                let mut st = HeliState { origin: [0.0, 0.0, 40.0], ..HeliState::default() };
                let ahead = HeliControls { forward: 127, ..HeliControls::default() };
                for _ in 0..60 {
                    heli_move(&def, &mut st, &ahead, 0.05, &flight);
                }
                if own == 9 {
                    assert!(st.origin[0] > 300.0, "{gjk} its own stopped it: {st:?}");
                } else {
                    // Its brush reaches 16 ahead; the box traces' shape only
                    // 10 (theirs is a capsule as wide as it is tall).
                    let reach = if gjk { 16.0 } else { 10.0 };
                    assert!(st.origin[0] > 200.0, "{gjk} it stopped at the unturned box: {st:?}");
                    assert!(
                        st.origin[0] < 224.5 - reach && st.velocity[0].abs() < 1.0,
                        "{gjk} it passed it: {st:?}"
                    );
                }
            }
        }
        // Only the other one is kept.
        let mine = OtherVehicle::new(90, [0.0; 3], [0.0; 3], corners([-1.0; 3], [1.0; 3]), false);
        let clip = VehicleClip {
            bodies: Vec::new(),
            attached_solids: Vec::new(),
            blockers: Vec::new(),
            movers: Vec::new(),
            pane_harm: Vec::new(),
            own_entity_slot: 90,
            vehicles: Vec::new(),
        }
        .with_vehicles(vec![other, mine.expect("a vehicle")]);
        assert_eq!(clip.vehicles.iter().map(|v| v.entnum).collect::<Vec<_>>(), [9]);
        assert!(OtherVehicle::new(5, [0.0; 3], [0.0; 3], Vec::new(), false).is_none());
    }

    #[test]
    fn a_climbers_landing_stops_the_others_not_him() {
        let landing = BodyBox {
            entnum: trace_t6::WORLD_ENTITY_SLOT,
            origin: [0.0; 3],
            bounds: None,
            owner: 1,
            contents: crate::sim_parts::CONTENTS_PLAYERCLIP,
        };
        let masses = [landing];
        let (bsp, geom) = (CollisionTreeData::default(), CollisionTriangleData::default());
        let backend = |own_entity_slot| SweepCollisionProvider {
            brushes: &[],
            bsp: &bsp,
            mesh: &geom,
            pane_harm: &[],
            player_boxes: &masses,
            own_entity_slot,
            cmodels: &[],
            attached_solids: &[],
            linked_owners: &[],
            struck: None,
            touched: None,
            blockers: &[],
            movers: &[],
            vehicles: &[],
            gjk: false,
            slick: false,
        };
        let walk = |collide_filter| SweepAsk {
            start: [-100.0, 0.0, 0.0],
            end: [100.0, 0.0, 0.0],
            lo_corner: BODY_BOX_LOWER,
            hi_corner: BODY_BOX_UPPER,
            collide_filter,
        };
        // Another player walking across the spot stops at it, on no player.
        let hit = backend(2).trace(walk(0x0281_0011));
        assert!(hit.fraction < 0.5, "{hit:?}");
        assert_ne!(
            trace_t6::entity_hit_slot(&hit),
            2,
            "a hit on it names no player"
        );
        assert!(trace_t6::entity_hit_slot(&hit) >= 64);
        // The climber passes through his own.
        assert_eq!(backend(1).trace(walk(0x0281_0011)).fraction, 1.0);
        // It is player clip: a trace that only bodies stop ignores it.
        assert_eq!(backend(2).trace(walk(crate::sim_parts::SOLID_PLAYER_BODY)).fraction, 1.0);
    }
}

mod feel_tests {
    use super::bo2_feel_rules;

    /// With no scripts the rules are BO2 multiplayer's own defaults (what a
    /// guess made before the first snapshot now uses instead of MW2's).
    #[test]
    fn no_scripts_is_bo2_multiplayer() {
        let feel = bo2_feel_rules(|_| None, false);
        assert_eq!(feel, movement_t6::bridge::Bo2Feel::BO2_MP);
        assert!(!feel.zombies && !feel.omni && feel.jump_slowdown && feel.dive);
        assert_eq!(feel.sprint_strafe_speed_scale, 0.667);
        assert_eq!((feel.gravity, feel.g_speed, feel.leap_height), (800, 190.0, 39.0));
        assert_eq!((feel.fall_damage_min_height, feel.fall_damage_max_height), (128.0, 300.0));
        assert_eq!((feel.sprint_multiplier, feel.speed_multiplier, feel.fast_ads_multiplier), (2.0, 1.07, 0.5));
        assert_eq!(feel.fall_damage_perk, playerstate_t6::perks::PERK_FALLHEIGHT);
    }

    /// A game's scripts still win over every default, as before.
    #[test]
    fn script_dvars_win() {
        let dvars = [("mapname", "zm_transit"), ("bg_gravity", " 400 "), ("g_speed", "200"), ("dtp", "0")];
        let feel = bo2_feel_rules(|name| dvars.iter().find(|(n, _)| *n == name).map(|(_, v)| *v), true);
        assert!(feel.zombies && feel.omni && !feel.jump_slowdown && !feel.dive);
        assert_eq!((feel.gravity, feel.g_speed), (400, 200.0));
    }
}

mod bo2_collision_literals {
    /// The footstep tables and the bullet tables number the 32 surface types alike.
    #[test]
    fn the_footstep_surface_names_are_the_bo2_surface_names() {
        assert_eq!(movement_t6::bridge::SURFACE_KIND_LABELS, weapon_t6::surface::T6_SURFACE_TYPE_NAMES);
        assert_eq!(movement_t6::bridge::material_kind_number(31 << 20), 31);
    }

    /// `ClientThink` gives a living player 0x2818011 and the dead 0x810011; `BulletTrace` shoots
    /// with 0x280e833.
    #[test]
    fn the_player_and_shot_masks_are_the_bo2_masks() {
        use crate::shot_trace::{MASK_PLAYER_DEAD, PLAYER_BODY_MASK, MASK_SHOT};
        assert_eq!(PLAYER_BODY_MASK, 0x0281_8011);
        assert_eq!(MASK_PLAYER_DEAD, PLAYER_BODY_MASK & !(0x0200_0000 | 0x8000));
        assert_eq!(MASK_SHOT, 0x0280_e833);
    }
}

/// bo2mp: bot 11 at Frostbite (-750,-150) made no headway for seconds (run
/// avoid-after1, line 5162): its logged spot, view and stick, replayed through
/// the move code the game runs (the game's collision install, provider,
/// command context and 50 ms command), and on t6walk's world, mask and 16 ms
/// command. Needs T6_ZONE_DIR (BO2's zone/all); prints where each run got.
mod frostbite_replay_tests {
    use super::*;
    use movement_t6::bridge::{FixedClimbClipDuration, StillClimbOffset};
    use playerstate_t6::game::{AvatarState, InputCommand, NO_ENTITY_SLOT};
    use super::super::feel::{pmove_context, HandlingFacts};

    const START: [f32; 3] = [-752.7, -183.4, 60.3];
    const YAW: f32 = 20.0;
    const STICK: (i8, i8) = (116, 49);
    /// t6walk's trace mask (bo2zm_tools t6walk.rs PLAYER_MASK).
    const T6WALK_MASK: u32 = 0x0281_0011;

    struct Level {
        /// Every brush, as the match install takes them (after the model collision).
        game: Vec<SolidBrush>,
        /// The brushes t6walk keeps: the clip map's own, before anything is attached.
        t6walk: Vec<SolidBrush>,
        bsp: CollisionTreeData,
        mesh: CollisionTriangleData,
        /// The brush models with the models' collision appended (the install's list).
        cmodels: Vec<asset_world::CollisionSubmodel>,
        /// The destructible van ahead of him, as its entity links its model's collision.
        van: Option<crate::shot_trace::AttachedSolidShape>,
    }

    fn brush(b: &asset_world::CollisionSolid) -> SolidBrush {
        SolidBrush {
            sides: b.planes.clone(),
            solid_bits: b.contents,
            plane_material_flags: b.plane_material_flags.clone(),
            glass_hit_code: b.glass_hit_code,
            vertex_points: b.vertex_points.clone(),
            hit_surface_flags: b.hit_surface_flags,
        }
    }

    /// Frostbite's collision as assets/zone_io/t6.rs installs it (no glass panes).
    fn frostbite(dir: &str) -> Level {
        let capture = asset_t6::capture_zone(&std::path::Path::new(dir).join("mp_frostbite.ff")).expect("zone");
        let raw = capture.clip.as_ref().expect("clipMap");
        let mut clip = asset_world::build_t6_clip_collision(raw).expect("clip");
        let t6walk = clip.brushes.iter().map(brush).collect();
        asset_world::attach_t6_static_world(&mut clip, raw, &capture.xmodels).expect("static world");
        asset_world::attach_t6_model_collision(
            &mut clip,
            capture.xmodels.iter().map(|x| (x.name.as_str(), x.collmap_brushes.as_slice())),
        );
        // The map's entities near the spot: what the live match puts there that this does not.
        for e in capture.map_ents.iter().flat_map(|text| asset_t6::parse_entities(text)) {
            let Some(o) = e.vec3("origin") else { continue };
            if (o[0] - START[0]).hypot(o[1] - START[1]) < 200.0 {
                println!("entity {} model {:?} targetname {:?} at {o:?} angles {:?}", e.classname(), e.get("model"), e.get("targetname"), e.get("angles"));
            }
        }
        let game: Vec<SolidBrush> = clip.brushes.iter().map(brush).collect();
        let bsp = CollisionTreeData {
            brush_leaf_indices: clip.brush_leaf_indices.clone(),
            t6: clip.t6,
            t6_glass: CollisionTreeData::glass_of(&game),
            t6_model_brushes: clip.t6_model_brushes.clone(),
        };
        let van = clip.model_cmodels.get("veh_t6_dlc4_van_sprinter_snow_white_whole").map(|&collision_model_ref| {
            crate::shot_trace::AttachedSolidShape { collision_model_ref, origin: [-691.19, -188.57, 56.0], turn: [0.0, 244.699, 0.0] }
        });
        for (name, n) in &clip.model_cmodels { if name.contains("van") || name.contains("sprinter") || name.contains("veh") { println!("model cmodel {name} = {n}"); } }
        println!("van cmodel {:?} of {} (raw clip map {})", van.as_ref().map(|v| v.collision_model_ref), clip.cmodels.len(), clip.t6.map_or(0, |m| m.cmodels.len()));
        Level { game, t6walk, bsp, mesh: CollisionTriangleData { t6_static: clip.t6_static }, cmodels: clip.cmodels.clone(), van }
    }

    fn handling() -> HandlingFacts {
        HandlingFacts {
            weapon_scales: (1.0, 1.0, 1.0),
            sights_permitted: true,
            scoped_aiming: false,
            scope_raise_speed: 0.0,
            scope_lower_speed: 0.0,
            rack_bolt_when_scoped: false,
            ads_reload: (false, 0, false),
            fire_only_when_scoped: false,
            strike_wait_millis: 0,
            lunge_windup_wait_millis: 0,
            hud_crosshair_layer: 0,
            shellshock_movement_scale: 1.0,
        }
    }

    /// Walks the stick for `ms` milliseconds in `msec` commands; gives the spot after each.
    fn walk(world: &impl movement_t6::Bo2Collision, mask: u32, msec: i32, ms: i32) -> Vec<[f32; 3]> {
        let feel = bo2_feel_rules(|_| None, false);
        let mut ps = AvatarState {
            origin: START,
            last_cmd_ms: 1000,
            eye_height_goal: 60,
            eye_height_now: 60.0,
            standing_on: NO_ENTITY_SLOT,
            run_speed_modifier: 1.0,
            ..AvatarState::ZERO
        };
        ps.gravity = feel.gravity;
        let mut out = Vec::new();
        for i in 0..ms / msec {
            ps.speed = movement_t6::bridge::player_speed(&ps, feel);
            let mut context = pmove_context(0, &handling(), feel);
            context.bounds.collide_filter = mask;
            let mut cmd = InputCommand::default();
            cmd.authority_clock_ms = ps.last_cmd_ms + msec;
            // Two still commands first: it stood there.
            if i >= 2 {
                (cmd.thrust_ahead, cmd.thrust_lateral) = STICK;
            }
            cmd.angles[1] = (YAW * 65536.0 / 360.0) as i32 & 0xffff;
            movement_t6::pmove_with_body(&mut ps, &mut cmd, context, world, &FixedClimbClipDuration::default(), &StillClimbOffset, true);
            out.push(ps.origin);
        }
        out
    }

    fn line(name: &str, path: &[[f32; 3]]) -> String {
        let end = *path.last().unwrap();
        let gone = (end[0] - START[0]).hypot(end[1] - START[1]);
        let picks: Vec<String> = [0.25, 0.5, 0.75].iter().map(|f| {
            let p = path[(path.len() as f32 * f) as usize];
            format!("({:.0} {:.0} {:.0})", p[0], p[1], p[2])
        }).collect();
        format!("{name:<34} moved {gone:>6.1} to ({:.1} {:.1} {:.1})  via {}", end[0], end[1], end[2], picks.join(" "))
    }

    #[test]
    fn bot_11s_move_at_frostbite_on_the_game_code_and_on_t6walk() {
        let Ok(dir) = std::env::var("T6_ZONE_DIR") else {
            return;
        };
        let level = frostbite(&dir);
        let game = bo2_map_collision(&level.game, &level.bsp, &level.mesh, true);
        let t6walk = bo2_map_collision(&level.t6walk, &level.bsp, &level.mesh, true);
        let mask = crate::shot_trace::PLAYER_BODY_MASK;
        let runs = [
            ("game: world, mask, 50 ms", walk(&game, mask, 50, 2000)),
            ("game world, mask, 16 ms", walk(&game, mask, 16, 2000)),
            ("game world, t6walk mask, 50 ms", walk(&game, T6WALK_MASK, 50, 2000)),
            ("t6walk world, game mask, 50 ms", walk(&t6walk, mask, 50, 2000)),
            ("t6walk: world, mask, 16 ms", walk(&t6walk, T6WALK_MASK, 16, 2000)),
        ];
        println!("brushes: game {} t6walk {}", level.game.len(), level.t6walk.len());
        // The van linked as the live match links it: the move (BO2's own collision) and the
        // bots' clear-ahead check (trace_clip's world sweep, then the linked brush models).
        let van = level.van.clone().expect("van model collision");
        let attached = [(trace_t6::WORLD_ENTITY_SLOT, van.clone())];
        let with_van = SweepCollisionProvider {
            brushes: &level.game,
            bsp: &level.bsp,
            mesh: &level.mesh,
            pane_harm: &[],
            player_boxes: &[],
            own_entity_slot: 0,
            cmodels: &level.cmodels,
            attached_solids: &attached,
            linked_owners: &[],
            struck: None,
            touched: None,
            blockers: &[],
            movers: &[],
            vehicles: &[],
            gjk: true,
            slick: false,
        };
        println!("{}", line("game + van: move, 50 ms", &walk(&with_van, mask, 50, 2000)));
        use crate::shot_trace::{BODY_BOX_LOWER, BODY_BOX_UPPER};
        let (from, to) = ([START[0], START[1], START[2] + 18.0], [-547.0, -192.0, START[2] + 18.0]);
        let (lo, hi) = ([-15.0, -15.0, BODY_BOX_LOWER[2]], [15.0, 15.0, BODY_BOX_UPPER[2] - 18.0]);
        let probe = movement_t6::bridge::SweepAsk { start: from, end: to, lo_corner: lo, hi_corner: hi, collide_filter: mask };
        let world_only = crate::sim_parts::sweep_box_trace(&level.game, &level.bsp, &level.mesh, from, to, lo, hi, mask, &|_| false);
        let bots = crate::sim_parts::clip_move_to_bmodels(world_only.clone(), level.bsp.t6, &level.bsp.t6_model_brushes, [(trace_t6::WORLD_ENTITY_SLOT, &van)], probe);
        println!("bots' step trace to the lookahead: world frac {:.2}, with the van linked frac {:.2}", world_only.fraction, bots.fraction);
        // The van blocks the bots' clear-ahead check as BO2's CM_TempBrushModel clip handle does.
        assert!(bots.fraction < world_only.fraction, "the van does not block the bots' step trace");
        for (name, path) in &runs {
            println!("{}", line(name, path));
        }
    }
}
