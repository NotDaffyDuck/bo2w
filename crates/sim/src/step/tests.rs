//! Tests of the step's movement rules, its hazards and the collision a move is lent.

use bevy_ecs::prelude::World;
use movement_t6::bridge::{CollisionBackend, GroundTraceInput};
use trace_t6::{Trace, TraceHitType};

use super::bo2_map_collision;
use super::clip::{BodyBox, ClipBackend, OtherVehicle, VehicleClip};
use super::feel::{bo2_feel, bo2_feel_rules};
use super::hazard::{breath_step, fall_damage};
use crate::bullet_collision::{LinkedBrushCollisionBrush, PLAYER_MAXS, PLAYER_MINS};
use crate::gentity::ScriptMoverGentity;
use crate::world::{SimBrush, SimClipBsp, SimClipMesh};

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
        let mut host = crate::carrier::SimWorld::default();
        host.frame().set_feel(feel);
        let snapshot = host.snapshot(crate::world::Tick(1));
        assert_eq!(snapshot.meta.feel, Some(feel));
        let mut guess = crate::carrier::SimWorld::default();
        guess.adopt_snapshot(&snapshot);
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
    use crate::world::clip_move_to_bmodels;
    use movement_t6::Bo2Collision;

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-3)
    }

    /// A 64-unit floor 8 thick, its top at its origin.
    fn floor() -> SimBrush {
        SimBrush {
            planes: vec![
                [1.0, 0.0, 0.0, 32.0],
                [-1.0, 0.0, 0.0, 32.0],
                [0.0, 1.0, 0.0, 32.0],
                [0.0, -1.0, 0.0, 32.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, -1.0, 8.0],
            ],
            contents: crate::bullet_collision::CONTENTS_SOLID,
            plane_surface_flags: vec![0; 6],
            glass_encoded: 0,
            verts: Vec::new(),
            hit_surface_flags: 0,
        }
    }

    #[test]
    fn the_water_surface_is_found_over_and_under_the_feet() {
        use movement_t6::water::{CONTENTS_WATER, NO_WATER};
        // A pool 100 deep, its surface at 0.
        let mut pool = floor();
        pool.contents = CONTENTS_WATER;
        pool.planes[5] = [0.0, 0.0, -1.0, 100.0];
        let brushes = [pool];
        let (bsp, mesh) = (SimClipBsp::default(), SimClipMesh::default());
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
        let mut lift = ScriptMoverGentity::default();
        lift.state.number = 40;
        lift.state.lerp.pos.tr_type = entity_t6::trajectory::tr_type::LINEAR;
        lift.state.lerp.pos.tr_time = 1000;
        lift.state.lerp.pos.tr_base = [0.0, 0.0, 100.0];
        lift.state.lerp.pos.tr_delta = [0.0, 0.0, 200.0];
        lift.state.lerp.apos.tr_type = entity_t6::trajectory::tr_type::LINEAR;
        lift.state.lerp.apos.tr_time = 1000;
        lift.state.lerp.apos.tr_delta = [0.0, 90.0, 0.0];
        let mut still = ScriptMoverGentity::default();
        still.state.number = 41;
        still.state.lerp.pos.tr_base = [5.0, 0.0, 0.0];
        let movers = [lift, still];
        let (bsp, mesh) = (SimClipBsp::default(), SimClipMesh::default());
        let backend = ClipBackend {
            brushes: &[],
            bsp: &bsp,
            mesh: &mesh,
            glass_damage: &[],
            bodies: &[],
            self_entnum: 0,
            cmodels: &[],
            linked_brushes: &[],
            linked_owners: &[],
            struck: None,
            touched: None,
            blockers: &[],
            movers: &movers,
            vehicles: &[],
            gjk: false,
        };
        let (origin, angles) = backend.mover_pose(40, 1500).expect("the lift has a pose");
        assert!(close(origin, [0.0, 0.0, 200.0]), "{origin:?}");
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
            asset_world::ClipCmodel::default(),
            asset_world::ClipCmodel {
                mins: [-32.0, -32.0, -8.0],
                maxs: [32.0, 32.0, 0.0],
                radius: 46.0,
                first_brush: 0,
                num_brushes: 1,
            },
        ];
        let store = crate::clip_fallback::clip_store(&brushes, &cmodels, &[0]);
        let input = GroundTraceInput {
            start: [0.0, 0.0, 50.0],
            end: [0.0, 0.0, -50.0],
            mins: [-15.0, -15.0, 0.0],
            maxs: [15.0, 15.0, 70.0],
            tracemask: crate::bullet_collision::CONTENTS_SOLID,
        };
        let lift = LinkedBrushCollisionBrush {
            cmodel_handle: 1,
            origin: [0.0; 3],
            angles: [0.0; 3],
        };
        let open = Trace::new();
        let hit = store.with_map(|map| clip_move_to_bmodels(open, Some(map), [(40, &lift)], input));
        assert!(hit.fraction < 1.0, "{hit:?}");
        assert_eq!((hit.hit_type, hit.hit_id), (TraceHitType::Entity, 40));
        // A brush model nobody numbered stays the world's.
        let hit = store.with_map(|map| {
            clip_move_to_bmodels(open, Some(map), [(trace_t6::ENTITYNUM_WORLD, &lift)], input)
        });
        assert!(hit.fraction < 1.0, "{hit:?}");
        assert_eq!(
            trace_t6::trace_get_entity_hit_id(&hit),
            trace_t6::ENTITYNUM_WORLD
        );
    }

    /// bo2mp: on a BO2 map a body's temporary box stops a line at its
    /// near face, turned or not, with the hit's normal facing the line.
    #[test]
    fn a_bo2_temp_box_stops_a_line_at_its_near_face() {
        use crate::world::{T6Model, t6_transformed_trace};
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
            assert!((hit.end_pos([0.0; 3], [20.0, 0.0, 0.0])[0] - 5.0).abs() < 0.2, "{angles:?} {hit:?}");
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
            contents: crate::world::CONTENTS_BODY,
        };
        let def = HeliDef {
            max_speed: 352.0,
            accel: 440.0,
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
        let (bsp, mesh) = (SimClipBsp::default(), SimClipMesh::default());
        for gjk in [false, true] {
            // Player 3 stands 300 ahead; the drone (number 90) has a box of
            // its own where it starts.
            let clip = VehicleClip {
                bodies: vec![body(3, 300.0), body(90, 0.0)],
                linked_brushes: Vec::new(),
                blockers: Vec::new(),
                movers: Vec::new(),
                glass_damage: Vec::new(),
                self_entnum: 90,
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
            max_speed: 352.0,
            accel: 440.0,
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
        let (bsp, mesh) = (SimClipBsp::default(), SimClipMesh::default());
        for gjk in [false, true] {
            for own in [90, 9] {
                let clip = VehicleClip {
                    bodies: Vec::new(),
                    linked_brushes: Vec::new(),
                    blockers: Vec::new(),
                    movers: Vec::new(),
                    glass_damage: Vec::new(),
                    self_entnum: own,
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
            linked_brushes: Vec::new(),
            blockers: Vec::new(),
            movers: Vec::new(),
            glass_damage: Vec::new(),
            self_entnum: 90,
            vehicles: Vec::new(),
        }
        .with_vehicles(vec![other, mine.expect("a vehicle")]);
        assert_eq!(clip.vehicles.iter().map(|v| v.entnum).collect::<Vec<_>>(), [9]);
        assert!(OtherVehicle::new(5, [0.0; 3], [0.0; 3], Vec::new(), false).is_none());
    }

    #[test]
    fn a_climbers_landing_stops_the_others_not_him() {
        let landing = BodyBox {
            entnum: trace_t6::ENTITYNUM_WORLD,
            origin: [0.0; 3],
            bounds: None,
            owner: 1,
            contents: crate::world::CONTENTS_PLAYERCLIP,
        };
        let bodies = [landing];
        let (bsp, mesh) = (SimClipBsp::default(), SimClipMesh::default());
        let backend = |self_entnum| ClipBackend {
            brushes: &[],
            bsp: &bsp,
            mesh: &mesh,
            glass_damage: &[],
            bodies: &bodies,
            self_entnum,
            cmodels: &[],
            linked_brushes: &[],
            linked_owners: &[],
            struck: None,
            touched: None,
            blockers: &[],
            movers: &[],
            vehicles: &[],
            gjk: false,
        };
        let walk = |tracemask| GroundTraceInput {
            start: [-100.0, 0.0, 0.0],
            end: [100.0, 0.0, 0.0],
            mins: PLAYER_MINS,
            maxs: PLAYER_MAXS,
            tracemask,
        };
        // Another player walking across the spot stops at it, on no player.
        let hit = backend(2).trace(walk(0x0281_0011));
        assert!(hit.fraction < 0.5, "{hit:?}");
        assert_ne!(
            trace_t6::trace_get_entity_hit_id(&hit),
            2,
            "a hit on it names no player"
        );
        assert!(trace_t6::trace_get_entity_hit_id(&hit) >= 64);
        // The climber passes through his own.
        assert_eq!(backend(1).trace(walk(0x0281_0011)).fraction, 1.0);
        // It is player clip: a trace that only bodies stop ignores it.
        assert_eq!(backend(2).trace(walk(crate::world::CONTENTS_BODY)).fraction, 1.0);
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
        assert_eq!((feel.gravity, feel.g_speed, feel.jump_height), (800, 190.0, 39.0));
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
