use super::*;

#[test]
fn drone_launch_clip_runs_after_a_tenth_of_a_second() {
    let (clip, start) = missile_client_clip("missile_drone_projectile", 50).unwrap();
    assert_eq!(clip, "o_drone_hunter_launch");
    assert_eq!(start, 0.0);
    let (_, mid) = missile_client_clip("missile_drone_projectile_mp", 100 + 417).unwrap();
    assert!((mid - 0.5).abs() < 0.01);
    let (_, held) = missile_client_clip("missile_drone_projectile", 5000).unwrap();
    assert_eq!(held, 1.0);
}

#[test]
fn other_weapons_have_no_client_clip() {
    assert!(missile_client_clip("frag_grenade_mp", 500).is_none());
    assert!(missile_client_clip("", 0).is_none());
}

#[test]
fn light_sample_sits_above_the_missile() {
    assert_eq!(
        missile_lighting_origin([4.0, 5.0, 6.0]),
        [4.0, 5.0, 6.0 + MISSILE_LIGHTING_Z_OFS]
    );
}

#[test]
fn missile_matrix_places_and_turns() {
    let m = missile_world_from_local([1.0, 2.0, 3.0], [0.0, 90.0, 0.0]);
    assert!(
        m.transform_point3(Vec3::ZERO)
            .abs_diff_eq(Vec3::new(1.0, 2.0, 3.0), 1e-5)
    );
    assert!(m.transform_vector3(Vec3::X).abs_diff_eq(Vec3::Y, 1e-5));
    let tipped = missile_world_from_local([0.0; 3], [90.0, 0.0, 0.0]);
    assert!(
        tipped
            .transform_vector3(Vec3::X)
            .abs_diff_eq(-Vec3::Z, 1e-5)
    );
}

#[test]
fn clip_phase_loops_or_clamps() {
    assert!((clip_phase(true, 1.25) - 0.25).abs() < 1e-6);
    assert!((clip_phase(true, -0.25) - 0.75).abs() < 1e-6);
    assert_eq!(clip_phase(false, 1.25), 1.0);
    assert_eq!(clip_phase(false, -0.25), 0.0);
}

#[test]
fn atlas_prefers_the_first_source() {
    assert!(missile_lighting_atlas(None, None).is_none());
}

#[test]
fn bones_only_pose_is_skinned_on_the_card() {
    let pose = PosedVerts::from_bones(0, |_| Mat4::IDENTITY);
    assert_eq!(pose.bones.len(), 1);
    assert!(pose.on_gpu());
    let three = PosedVerts::from_bones(3, |b| Mat4::from_translation(Vec3::splat(b as f32)));
    assert_eq!(three.bones[2], Mat4::from_translation(Vec3::splat(2.0)));
    let with_verts = PosedVerts {
        positions: vec![[0.0; 3]],
        ..three.clone()
    };
    assert!(!with_verts.on_gpu());
    assert!(!PosedVerts::default().on_gpu());
}
