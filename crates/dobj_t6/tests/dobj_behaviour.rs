//! What a DObj does, pinned through the crate's public face only: building
//! a skeleton from model sources, walking it to world joint_transforms, hiding
//! parts, and the player controller's tag angles.

use dobj_t6::*;
use glam::{Mat4, Vec3, Quat, EulerRot};
const NO_PLACEMENT: Mat4 = Mat4::IDENTITY;

const NEAR: f32 = 1e-4;

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < NEAR
}

fn close3(a: Vec3, b: Vec3) -> bool {
    (a - b).length() < NEAR
}

fn same_rotation(a: [f32; 4], b: [f32; 4]) -> bool {
    let qa = Quat::from_array(a);
    let qb = Quat::from_array(b);
    qa.dot(qb).abs() > 1.0 - 1e-5
}

/// A model of `names.len()` bones whose first `roots` are roots. Every
/// non-root bone hangs off the bone just before it and sits one unit along
/// x from it; bind poses are identity rotations at the matching offsets.
fn chain(name: &str, names: &[&str], roots: usize) -> PoseDonorModel {
    let count = names.len();
    let children = count - roots;
    PoseDonorModel {
        name: name.to_string(),
        skeleton_joint_total: count,
        top_joint_total: roots,
        size_scale: 1.0,
        fixed_size_parts: [0; 5],
        joint_labels: names.iter().map(|n| n.to_string()).collect(),
        ancestor_chain: vec![1; children],
        rotation_quaternions: vec![[0, 0, 0, 32767]; children],
        shift_offset: vec![[1.0, 0.0, 0.0]; children],
        foundation_surface: (0..count)
            .map(|i| {
                let along = i.saturating_sub(roots.saturating_sub(1)) as f32;
                (Quat::default(), Vec3::X * along)
            })
            .collect(),
    }
}

fn build_one(model: &PoseDonorModel) -> PosedSkeleton {
    PosedSkeleton::build(&[(model, None)]).expect("a single chain builds")
}

#[test]
fn bones_get_parents_names_and_models() {
    let model = chain("body", &["root", "mid", "tip"], 1);
    let dobj = build_one(&model);
    assert_eq!(dobj.joint_total(), 3);
    let parents: Vec<_> = dobj.bones.iter().map(|b| b.up).collect();
    assert_eq!(parents, [None, Some(0), Some(1)]);
    assert_eq!(dobj.find("tip"), Some(2));
    assert_eq!(dobj.find("nope"), None);
    assert_eq!(dobj.models.len(), 1);
    assert_eq!(dobj.models[0].joint_total, 3);
    assert!(dobj.mirrored_part_pairs.is_empty());
    assert_eq!(dobj.full_part_set(), PartSet::first(3));
}

#[test]
fn bind_rotations_are_normalised_and_a_zero_one_is_the_identity() {
    let mut model = chain("body", &["root", "a", "b"], 1);
    model.rotation_quaternions[0] = [0, 0, 16000, 16000];
    model.rotation_quaternions[1] = [0, 0, 0, 0];
    let dobj = build_one(&model);
    let r = dobj.bones[1].rest_orientation;
    assert!(close(r.length(), 1.0));
    assert!(close(r.z, r.w));
    assert!(dobj.bones[2].rest_orientation.abs_diff_eq(Quat::IDENTITY, NEAR));
    assert_eq!(dobj.bones[1].rest_offset, Vec3::X);
}

#[test]
fn composing_the_bind_pose_stacks_the_offsets() {
    let model = chain("body", &["root", "mid", "tip"], 1);
    let dobj = build_one(&model);
    let rest = dobj.rest_local_pose();
    let lifted_to = Mat4::from_translation(Vec3::Y * 10.0);
    let world = dobj.world_pose(&rest, lifted_to);
    assert_eq!(world.len(), 3);
    assert!(close3(world[0].w_axis.truncate(), Vec3::Y * 10.0));
    assert!(close3(world[1].w_axis.truncate(), Vec3::new(1.0, 10.0, 0.0)));
    assert!(close3(world[2].w_axis.truncate(), Vec3::new(2.0, 10.0, 0.0)));
}

#[test]
fn model_scale_stretches_animated_translations_unless_the_bone_opts_out() {
    let mut model = chain("body", &["root", "mid", "tip"], 1);
    model.size_scale = 2.0;
    model.fixed_size_parts = [0x2000_0000, 0, 0, 0, 0];
    let dobj = build_one(&model);
    assert!(!dobj.bones[1].ignores_size_scaling && dobj.bones[2].ignores_size_scaling);
    let mut joint_poses = dobj.rest_local_pose();
    joint_poses[1].translation = [0.0, 3.0, 0.0];
    joint_poses[2].translation = [0.0, 3.0, 0.0];
    let world = dobj.world_pose(&joint_poses, NO_PLACEMENT);
    assert!(close3(world[1].w_axis.truncate(), Vec3::new(1.0, 6.0, 0.0)));
    assert!(close3(world[2].w_axis.truncate(), Vec3::new(2.0, 9.0, 0.0)));
}

#[test]
fn a_controlled_bone_turns_by_its_angle_about_its_parent() {
    let model = chain("body", &["root", "mid"], 1);
    let dobj = build_one(&model);
    let mut joint_poses = dobj.rest_local_pose();
    let quarter = Quat::from_rotation_z(core::f32::consts::FRAC_PI_2);
    steer_joint(&mut joint_poses, 1, [0.0, 90.0, 0.0]);
    let world = dobj.world_pose(&joint_poses, NO_PLACEMENT);
    let (_, rotation, translation) = world[1].to_scale_rotation_translation();
    assert!(rotation.dot(quarter).abs() > 1.0 - 1e-5);
    assert!(close3(translation, Vec3::X));
}

#[test]
fn a_model_attaches_to_a_named_bone_of_an_earlier_model() {
    let body = chain("body", &["root", "hand"], 1);
    let gun = chain("gun", &["gun_root", "tag_flash"], 1);
    let attach = MountedModel {
        host_model: 0,
        mount_tag: "hand".into(),
    };
    let dobj = PosedSkeleton::build(&[(&body, None), (&gun, Some(attach))]).unwrap();
    assert_eq!(dobj.joint_total(), 4);
    assert_eq!(dobj.bones[2].up, Some(1));
    assert_eq!(dobj.bones[3].up, Some(2));
    assert_eq!(dobj.bones[2].model, 1);
    assert_eq!(dobj.models[1].first_joint, 2);
}

#[test]
fn root_binds_offset_a_models_roots_from_the_attach_bone() {
    let body = chain("body", &["root", "hand"], 1);
    let gun = chain("gun", &["gun_root", "tag_flash"], 1);
    let attach = MountedModel {
        host_model: 0,
        mount_tag: "hand".into(),
    };
    let bind = Some((Quat::from_rotation_y(0.5), Vec3::Z * 4.0));
    let dobj = PosedSkeleton::build_with_root_binds(&[(&body, None), (&gun, Some(attach))], &[None, bind])
        .unwrap();
    assert_eq!(dobj.bones[2].rest_offset, Vec3::Z * 4.0);
    assert!(dobj.bones[2].rest_orientation.abs_diff_eq(Quat::from_rotation_y(0.5), NEAR));
    assert!(dobj.bones[3].rest_offset.abs_diff_eq(Vec3::X, NEAR));
}

#[test]
fn a_bone_name_the_body_already_has_is_copied_from_it() {
    let body = chain("body", &["root", "hand"], 1);
    let other = chain("other", &["root", "extra"], 1);
    let dobj = PosedSkeleton::build(&[(&body, None), (&other, None)]).unwrap();
    assert_eq!(dobj.mirrored_part_pairs, [(2, 0)]);
    let mut joint_poses = dobj.rest_local_pose();
    joint_poses[0].translation = [5.0, 6.0, 7.0];
    joint_poses.set_control_tag(0, Some([0.0, 0.0, 1.0, 0.0]));
    dobj.copy_mirrored_parts(&mut joint_poses);
    assert_eq!(joint_poses[2], joint_poses[0]);
    assert!(joint_poses.control.get(2));
}

#[test]
fn a_duplicate_of_the_attach_bone_itself_is_not_a_duplicate() {
    let body = chain("body", &["root", "hand"], 1);
    let gun = chain("gun", &["hand", "tag_flash"], 1);
    let attach = MountedModel {
        host_model: 0,
        mount_tag: "hand".into(),
    };
    let dobj = PosedSkeleton::build(&[(&body, None), (&gun, Some(attach))]).unwrap();
    assert!(dobj.mirrored_part_pairs.is_empty());
}

#[test]
fn build_refuses_models_it_cannot_make_sense_of() {
    let good = chain("body", &["root", "hand"], 1);

    let mut short = chain("short", &["root", "a"], 1);
    short.foundation_surface.pop();
    let err = PosedSkeleton::build(&[(&short, None)]).unwrap_err();
    assert!(matches!(err, SkeletonBuildFault::BadModel { ref model_name, .. } if model_name == "short"));

    let mut orphan = chain("orphan", &["root", "a"], 1);
    orphan.ancestor_chain.clear();
    assert!(matches!(
        PosedSkeleton::build(&[(&orphan, None)]),
        Err(SkeletonBuildFault::BadModel { .. })
    ));

    let mut loops = chain("loops", &["root", "a"], 1);
    loops.ancestor_chain = vec![0];
    assert!(matches!(
        PosedSkeleton::build(&[(&loops, None)]),
        Err(SkeletonBuildFault::BadModel { .. })
    ));

    let mut far = chain("far", &["root", "a"], 1);
    far.ancestor_chain = vec![9];
    assert!(matches!(
        PosedSkeleton::build(&[(&far, None)]),
        Err(SkeletonBuildFault::BadModel { .. })
    ));

    let mut no_quat = chain("noquat", &["root", "a"], 1);
    no_quat.rotation_quaternions.clear();
    assert!(matches!(
        PosedSkeleton::build(&[(&no_quat, None)]),
        Err(SkeletonBuildFault::BadModel { .. })
    ));

    let mut no_trans = chain("notrans", &["root", "a"], 1);
    no_trans.shift_offset.clear();
    assert!(matches!(
        PosedSkeleton::build(&[(&no_trans, None)]),
        Err(SkeletonBuildFault::BadModel { .. })
    ));

    let missing = MountedModel {
        host_model: 0,
        mount_tag: "tag_missing".into(),
    };
    let err = PosedSkeleton::build(&[(&good, None), (&good, Some(missing))]).unwrap_err();
    assert!(matches!(err, SkeletonBuildFault::MissingMountTag { host_model: 0, .. }));

    let early = MountedModel {
        host_model: 3,
        mount_tag: "hand".into(),
    };
    assert!(matches!(
        PosedSkeleton::build(&[(&good, Some(early))]),
        Err(SkeletonBuildFault::BadModel { .. })
    ));
}

#[test]
fn a_dobj_holds_at_most_160_bones() {
    let names: Vec<String> = (0..161).map(|i| format!("b{i}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let big = chain("big", &refs, 1);
    match PosedSkeleton::build(&[(&big, None)]) {
        Err(SkeletonBuildFault::BoneLimitExceeded { held, cap }) => {
            assert_eq!((held, cap), (161, 160));
        }
        other => panic!("expected TooManyBones, got {other:?}"),
    }
    let edge = chain("edge", &refs[..160], 1);
    assert_eq!(build_one(&edge).joint_total(), 160);
}

#[test]
fn errors_say_what_went_wrong() {
    let text = SkeletonBuildFault::BadModel {
        model_name: "m".into(),
        why: "d".into(),
    }
    .to_string();
    assert_eq!(text, "model \"m\": d");
    let text = SkeletonBuildFault::MissingMountTag {
        model_name: "gun".into(),
        tag_name: "t".into(),
        host_model: 2,
    }
    .to_string();
    assert_eq!(text, "attachment \"gun\": no bone named \"t\" in model 2");
    let limit_text = format!("{}", SkeletonBuildFault::BoneLimitExceeded { held: 9, cap: 4 });
    assert_eq!(limit_text, "a DObj holds at most 4 bones, got 9");
}

#[test]
fn nameless_bones_are_never_duplicates_and_find_nothing() {
    let mut body = chain("body", &["root", "a"], 1);
    body.joint_labels.truncate(1);
    let dobj = build_one(&body);
    assert_eq!(dobj.bones[1].name, "");
    assert_eq!(dobj.find(""), Some(1));
    let twin = PosedSkeleton::build(&[(&body, None), (&body, None)]).unwrap();
    assert_eq!(twin.mirrored_part_pairs, [(2, 0)]);
}

#[test]
fn skin_matrices_undo_the_bind_pose() {
    let model = chain("body", &["root", "mid"], 1);
    let dobj = build_one(&model);
    let world = dobj.world_pose(&dobj.rest_local_pose(), NO_PLACEMENT);
    for skin in dobj.skin_matrices(&world) {
        assert!(skin.abs_diff_eq(Mat4::IDENTITY, NEAR));
    }
    let moved = dobj.world_pose(&dobj.rest_local_pose(), Mat4::from_translation(Vec3::Z));
    let skin = dobj.skin_matrices(&moved);
    assert!(close3(skin[1].w_axis.truncate(), Vec3::Z));
}

#[test]
fn hide_bits_are_msb_first_across_five_words() {
    let mut hide = HiddenParts::default();
    hide.set(0);
    hide.set(33);
    hide.set(159);
    hide.set(160);
    assert_eq!(hide.words(), &[0x8000_0000, 0x4000_0000, 0, 0, 1]);
    assert!(hide.get(33) && !hide.get(32) && !hide.get(160) && !hide.get(10_000));
    assert_eq!(HiddenParts::from_bit_chunks([1, 2, 3, 4, 5]).words(), &[1, 2, 3, 4, 5]);
}

#[test]
fn a_surface_is_visible_unless_a_hidden_part_is_in_its_mask() {
    let body = chain("body", &["root", "hand"], 1);
    let gun = chain("gun", &["gun_root", "tag_flash"], 1);
    let attach = MountedModel {
        host_model: 0,
        mount_tag: "hand".into(),
    };
    let dobj = PosedSkeleton::build(&[(&body, None), (&gun, Some(attach))]).unwrap();
    let mut hide = HiddenParts::default();
    let surface = [0x8000_0000u32, 0, 0, 0, 0];
    assert!(dobj.facet_visible(0, &surface, &hide));
    hide.set(0);
    assert!(!dobj.facet_visible(0, &surface, &hide));
    assert!(dobj.facet_visible(1, &surface, &hide));
    hide.set(2);
    assert!(!dobj.facet_visible(1, &surface, &hide));
    assert!(!dobj.facet_visible(7, &surface, &HiddenParts::default()));
}

#[test]
fn bone_orientation_reads_origin_and_unit_axes() {
    let turned = Mat4::from_rotation_translation(Quat::from_rotation_z(0.5), Vec3::new(1.0, 2.0, 3.0))
        * Mat4::from_scale(Vec3::splat(2.0));
    let got = bone_orientation(&[Mat4::IDENTITY, turned], 1).unwrap();
    assert_eq!(got.at, [1.0, 2.0, 3.0]);
    let forward = Vec3::from_array(got.basis_dirs[0]);
    assert!(close(forward.length(), 1.0));
    assert!(close(forward.x, 0.5f32.cos()) && close(forward.y, 0.5f32.sin()));
    assert_eq!(
        bone_orientation(&[Mat4::IDENTITY], 4),
        Err(BonePlacementFault::BoneIndexInvalid { joint: 4, count: 1 })
    );
    let nan = Mat4::from_translation(Vec3::new(f32::NAN, 0.0, 0.0));
    assert_eq!(
        bone_orientation(&[nan], 0),
        Err(BonePlacementFault::NonFinite { joint: 0 })
    );
    let flat = Mat4::from_scale(Vec3::new(1.0, 0.0, 1.0));
    assert_eq!(
        bone_orientation(&[flat], 0),
        Err(BonePlacementFault::CollapsedAxis { joint: 0, which_axis: 1 })
    );
    let blown = Mat4::from_scale(Vec3::new(1.0, 1.0, f32::INFINITY));
    assert_eq!(
        bone_orientation(&[blown], 0),
        Err(BonePlacementFault::NonFinite { joint: 0 })
    );
}

#[test]
fn an_empty_attach_tag_follows_the_childs_root_when_the_parent_has_it() {
    let parent = chain("zombie", &["j_spine4", "j_neck"], 1);
    let head = chain("head", &["j_neck", "j_head"], 1);
    let loose = chain("hat", &["hat_root"], 1);
    assert_eq!(empty_tag_attach(&parent, &head), "j_neck");
    assert_eq!(empty_tag_attach(&parent, &loose), "j_spine4");
    let bare = PoseDonorModel {
        joint_labels: Vec::new(),
        ..chain("bare", &["x"], 1)
    };
    assert_eq!(empty_tag_attach(&bare, &loose), "");
    assert_eq!(empty_tag_attach(&parent, &bare), "j_spine4");
}

#[test]
fn angles_make_the_yaw_pitch_roll_quaternion() {
    for angles in [
        [0.0, 0.0, 0.0],
        [0.0, 90.0, 0.0],
        [45.0, 0.0, 0.0],
        [0.0, 0.0, 30.0],
        [20.0, -70.0, 15.0],
        [-85.0, 179.0, -45.0],
    ] {
        let [lift, swing, bank] = angles.map(f32::to_radians);
        let want = Quat::from_euler(EulerRot::ZYX, swing, lift, bank);
        assert!(
            same_rotation(set_angles(angles), want.to_array()),
            "angles {angles:?}"
        );
    }
    assert_eq!(set_angles([0.0; 3]), [0.0, 0.0, 0.0, 1.0]);
}

#[test]
fn a_local_tag_sets_pose_and_releases_control() {
    let mut joint_poses = LocalPose::new(vec![PartPose::IDENTITY; 2]);
    steer_joint(&mut joint_poses, 1, [0.0, 90.0, 0.0]);
    assert!(joint_poses.control.get(1));
    assert!(same_rotation(joint_poses[1].rotation, set_angles([0.0, 90.0, 0.0])));
    set_local_tag(&mut joint_poses, 1, [1.0, 2.0, 3.0], [0.0, 0.0, 90.0]);
    assert_eq!(joint_poses[1].translation, [1.0, 2.0, 3.0]);
    assert!(same_rotation(joint_poses[1].rotation, set_angles([0.0, 0.0, 90.0])));
    assert!(!joint_poses.control.get(1));
    set_local_tag(&mut joint_poses, 5, [9.0; 3], [0.0; 3]);
    assert_eq!(joint_poses.len(), 2);
}

fn torso_dobj() -> PosedSkeleton {
    let model = chain(
        "body",
        &[
            "tag_origin",
            "back_low",
            "back_mid",
            "back_up",
            "pelvis",
            "torso_stabilizer",
            "j_spinelower",
            "j_spineupper",
            "j_spine4",
        ],
        1,
    );
    build_one(&model)
}

fn upright(pitch: f32) -> BodyAimInput {
    BodyAimInput {
        look_pitch_degrees: pitch,
        lying: false,
        crouched: false,
        body_lean_ratio: 0.0,
    }
}

#[test]
fn the_controller_tags_are_the_four_back_bones() {
    assert_eq!(SPINE_AIM_BONES, ["back_low", "back_mid", "back_up", "pelvis"]);
}

#[test]
fn standing_pitch_spreads_over_the_back_bones() {
    let dobj = torso_dobj();
    let mut joint_poses = dobj.rest_local_pose();
    let result = pose_body_aim(&dobj, &mut joint_poses, upright(30.0));
    assert_eq!(result.steered_count, 4);
    assert!(result.root_steered);
    assert_eq!(result.origin_tag_shift, [0.0; 3]);
    assert_eq!(result.origin_tag_turn, [0.0; 3]);
    let low = dobj.find("back_low").unwrap();
    let mid = dobj.find("back_mid").unwrap();
    let up = dobj.find("back_up").unwrap();
    let pelvis = dobj.find("pelvis").unwrap();
    assert!(same_rotation(joint_poses[low].rotation, set_angles([12.0, 0.0, 0.0])));
    assert!(same_rotation(joint_poses[mid].rotation, set_angles([18.0, 0.0, 0.0])));
    assert!(same_rotation(joint_poses[up].rotation, set_angles([30.0, 0.0, 0.0])));
    assert!(same_rotation(joint_poses[pelvis].rotation, [0.0, 0.0, 0.0, 1.0]));
    assert!(joint_poses.control.get(low) && joint_poses.control.get(pelvis));
    assert!(!joint_poses.control.get(0));
}

#[test]
fn standing_with_the_helper_matches_the_full_call() {
    let dobj = torso_dobj();
    let mut a = dobj.rest_local_pose();
    let mut b = dobj.rest_local_pose();
    let tags = pose_upright_body_aim(&dobj, &mut a, 25.0);
    pose_body_aim(&dobj, &mut b, upright(25.0));
    assert_eq!(tags, 4);
    assert_eq!(a, b);
}

#[test]
fn a_body_missing_back_bones_gets_the_ones_it_has() {
    let model = chain("body", &["tag_origin", "back_mid", "pelvis"], 1);
    let dobj = build_one(&model);
    let mut joint_poses = dobj.rest_local_pose();
    let result = pose_body_aim(&dobj, &mut joint_poses, upright(0.0));
    assert_eq!(result.steered_count, 2);
    let mut empty = LocalPose::new(Vec::new());
    let result = pose_body_aim(&dobj, &mut empty, upright(0.0));
    assert!(!result.root_steered);
}

#[test]
fn prone_pitch_is_damped_going_up_and_down_differently() {
    let mut input = upright(30.0);
    input.lying = true;
    let dobj = torso_dobj();
    let up = dobj.find("back_up").unwrap();
    let mut joint_poses = dobj.rest_local_pose();
    pose_body_aim(&dobj, &mut joint_poses, input);
    assert!(same_rotation(joint_poses[up].rotation, set_angles([30.0, 0.0, 0.0])));
    input.look_pitch_degrees = -30.0;
    let mut joint_poses = dobj.rest_local_pose();
    pose_body_aim(&dobj, &mut joint_poses, input);
    assert!(same_rotation(joint_poses[up].rotation, set_angles([-15.0, 0.0, 0.0])));
}

#[test]
fn a_pitch_past_straight_up_wraps_before_it_is_doubled() {
    let dobj = torso_dobj();
    let up = dobj.find("back_up").unwrap();
    let mut joint_poses = dobj.rest_local_pose();
    pose_body_aim(&dobj, &mut joint_poses, upright(270.0));
    assert!(same_rotation(joint_poses[up].rotation, set_angles([-90.0, 0.0, 0.0])));
}

#[test]
fn leaning_shifts_and_rolls_the_tag_origin() {
    let (offset, angles) = player_controller_tag_origin(BodyAimInput {
        body_lean_ratio: 1.0,
        ..upright(0.0)
    });
    assert_eq!(offset, [0.0, -2.5, 0.0]);
    assert_eq!(angles, [0.0, 0.0, 3.75]);
    let (offset, angles) = player_controller_tag_origin(BodyAimInput {
        body_lean_ratio: -1.0,
        ..upright(0.0)
    });
    assert_eq!(offset, [0.0, 5.0, 0.0]);
    assert_eq!(angles, [0.0, 0.0, -3.75]);
    let crouched = |lean| {
        player_controller_tag_origin(BodyAimInput {
            body_lean_ratio: lean,
            crouched: true,
            ..upright(0.0)
        })
        .0
    };
    assert_eq!(crouched(1.0), [0.0, -13.0, 0.0]);
    assert_eq!(crouched(-0.5), [0.0, 6.25, 0.0]);
}

#[test]
fn a_lean_rolls_the_back_bones_more_when_standing() {
    let dobj = torso_dobj();
    let low = dobj.find("back_low").unwrap();
    let up = dobj.find("back_up").unwrap();
    let mut joint_poses = dobj.rest_local_pose();
    pose_body_aim(
        &dobj,
        &mut joint_poses,
        BodyAimInput {
            body_lean_ratio: 1.0,
            ..upright(0.0)
        },
    );
    let roll = 46.25 * 1.25;
    assert!(same_rotation(joint_poses[low].rotation, set_angles([0.0, 0.0, roll * 0.5])));
    assert!(same_rotation(joint_poses[up].rotation, set_angles([0.0, 0.0, roll * -0.6])));
    let mut crouched = dobj.rest_local_pose();
    pose_body_aim(
        &dobj,
        &mut crouched,
        BodyAimInput {
            body_lean_ratio: 1.0,
            crouched: true,
            ..upright(0.0)
        },
    );
    let roll = 46.25;
    assert!(same_rotation(crouched[low].rotation, set_angles([0.0, 0.0, roll * 0.5])));
}

#[test]
fn prone_with_no_lean_keeps_the_tag_origin_still() {
    let (offset, angles) = player_controller_tag_origin(BodyAimInput {
        lying: true,
        ..upright(0.0)
    });
    assert_eq!(offset, [0.0; 3]);
    assert_eq!(angles, [0.0; 3]);
}

#[test]
fn legs_yaw_turns_the_pelvis_and_counter_turns_the_stabiliser() {
    let dobj = torso_dobj();
    let pelvis = dobj.find("pelvis").unwrap();
    let stab = dobj.find("torso_stabilizer").unwrap();
    let mut joint_poses = dobj.rest_local_pose();
    turn_legs(&dobj, &mut joint_poses, 0.4);
    let turned = |index: usize| Quat::from_array(joint_poses[index].rotation);
    assert!(turned(pelvis).dot(Quat::from_rotation_z(0.4)).abs() > 1.0 - 1e-5);
    assert!(turned(stab).dot(Quat::from_rotation_z(-0.4)).abs() > 1.0 - 1e-5);
    let mut still = dobj.rest_local_pose();
    turn_legs(&dobj, &mut still, 1e-7);
    assert_eq!(still, dobj.rest_local_pose());
}

#[test]
fn aim_pitch_is_clamped_and_shared_down_the_spine() {
    assert!(close(AIM_PITCH_CLAMP_RAD, 85.0f32.to_radians()));
    let dobj = torso_dobj();
    let lower = dobj.find("j_spinelower").unwrap();
    let upper = dobj.find("j_spineupper").unwrap();
    let top = dobj.find("j_spine4").unwrap();
    let mut joint_poses = dobj.rest_local_pose();
    pitch_spine_aim(&dobj, &mut joint_poses, 0.9, 0.3);
    let about_y = |index: usize, angle: f32| {
        Quat::from_array(joint_poses[index].rotation).dot(Quat::from_rotation_y(angle)).abs() > 1.0 - 1e-5
    };
    assert!(about_y(lower, -0.3));
    assert!(about_y(upper, -0.3));
    assert!(about_y(top, -0.3));
    let mut huge = dobj.rest_local_pose();
    pitch_spine_aim(&dobj, &mut huge, 10.0, 10.0);
    let clamp = AIM_PITCH_CLAMP_RAD;
    assert!(Quat::from_array(huge[lower].rotation).dot(Quat::from_rotation_y(-clamp)).abs() > 1.0 - 1e-5);
    assert_eq!(huge[upper], PartPose::at_rest(dobj.bones[upper].rest_orientation.to_array()));
}

#[test]
fn yaw_and_pitch_ignore_missing_bones() {
    let dobj = torso_dobj();
    let mut joint_poses = dobj.rest_local_pose();
    yaw_joint(&dobj, &mut joint_poses, "no_such_bone", 1.0);
    pitch_joint(&dobj, &mut joint_poses, "no_such_bone", 1.0);
    pitch_joint(&dobj, &mut joint_poses, "pelvis", 0.0);
    assert_eq!(joint_poses, dobj.rest_local_pose());
}

#[test]
fn the_gun_goes_on_the_first_weapon_tag_the_body_has() {
    let names = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(THIRD_PERSON_GUN_BONES, ["tag_weapon_right", "tag_inhand", "tag_weapon_left"]);
    assert_eq!(
        tp_weapon_attach_tag(&names(&["tag_weapon_left", "tag_inhand"])),
        Some("tag_inhand")
    );
    assert_eq!(
        tp_weapon_attach_tag(&names(&["tag_weapon_right", "tag_inhand"])),
        Some("tag_weapon_right")
    );
    assert_eq!(tp_weapon_attach_tag(&names(&["pelvis"])), None);
    assert_eq!(THIRD_PERSON_HEAD_BONE, "j_spine4");
    assert_eq!(tp_head_attach_tag(&names(&["j_spine4"])), Some("j_spine4"));
    assert_eq!(tp_head_attach_tag(&names(&["j_spine"])), None);
}

#[test]
fn body_head_and_weapon_join_into_one_dobj() {
    let body = chain("body", &["tag_origin", "j_spine4", "tag_inhand"], 1);
    let head = chain("head", &["head_root"], 1);
    let gun = chain("gun", &["gun_root", "tag_flash"], 1);
    let (dobj, tag, flash) = assemble_body_head_gun_skeleton(&body, Some(&head), &gun).unwrap();
    assert_eq!(tag, "tag_inhand");
    assert_eq!(dobj.models.len(), 3);
    assert_eq!(dobj.bones[3].up, Some(1));
    assert_eq!(dobj.bones[4].up, Some(2));
    assert_eq!(flash, dobj.find("tag_flash"));
    assert_eq!(flash, Some(5));

    let (dobj, _, flash) = build_body_weapon_dobj(&body, &gun).unwrap();
    assert_eq!(dobj.models.len(), 2);
    assert_eq!(flash, Some(4));

    let plain = chain("gun2", &["gun_root"], 1);
    assert_eq!(build_body_weapon_dobj(&body, &plain).unwrap().2, None);

    let no_hand = chain("body", &["tag_origin", "j_spine4"], 1);
    assert!(build_body_weapon_dobj(&no_hand, &gun).is_none());
    let no_neck = chain("body", &["tag_origin", "tag_inhand"], 1);
    assert!(assemble_body_head_gun_skeleton(&no_neck, Some(&head), &gun).is_none());
    assert!(build_body_weapon_dobj(&no_neck, &gun).is_some());
}
