//! Bolts: how an effect is attached to a moving entity bone, and the small
//! rules around following it, stopping it and iterating over effects.
//!
//! An effect's bolt is a packed word: the entity (or view model) it follows,
//! whether that entity teleported, and which bone. A bolt that can no longer
//! be followed is marked lost by blanking its entity and bone.

use crate::runtime::state_word::{EMITTER_CHILD_TALLY_BITS, FX_STATUS_REF_COUNT_LOW_MASK};

/// A packed bolt's entity is its low twelve bits; all ones means none.
const DOBJ_MASK: u32 = 0xfff;
pub const NO_ATTACH_TICKET: u32 = 0xfff;
/// The teleport flag is the bit above the entity.
const TELEPORT_SHIFT: u32 = 12;
/// The bone is eleven bits from here up.
pub const ATTACH_JOINT_BIT_OFFSET: u32 = 13;
const BONE_MASK: u32 = 0x7ff;
/// ORed over a packed bolt: entity and bone all ones, teleport flag kept.
const LOST_BITS: u32 = 0x00ff_efff;

/// Entities below this are world entities with a state of their own to check.
const CENTITY_LIMIT: u32 = 0x7fe;
/// View model objects number up from here.
pub const ATTACH_HELD_GUN_SKELETON_START: u32 = 0x800;
/// The entity state flag that says the entity teleported.
const CENTITY_TELEPORT_FLAG: u32 = 2;

/// The orientation of a bolt's parent before it has been placed.
pub const ATTACH_ROOT_NO_TURN: [f32; 4] = [0.0_f32, 0.0, 0.0, 1.0];

/// How many bolt records there are.
pub const ATTACH_RECORD_LIMIT: u32 = 0xff;
/// The last record linked into the free list when it is first built.
const INIT_LAST: i32 = 0xfe;
/// The free list's end.
pub const NO_SPARE_ATTACH: i32 = -1;

/// An effect whose bolt record number is this has no bolt.
const NO_BOLT_RECORD: u8 = 0xff;

/// Where a bone's orientation comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JointFacingPath {
    /// A negative bone: the entity's own pose.
    OwnPose,
    /// A bone of the entity's skeleton.
    SkeletonJoint(u16),
}

/// Why a bone's orientation could not be found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoneOrientationRefuse {
    ClientThingGone,
    SkeletonAbsent,
    BoneIndexInvalid,
}

/// What an effect does with its bolt on an update.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmitterAttachRefresh {
    /// There is no bolt to follow.
    Skip,
    /// The bolt is still good; take the bone's new orientation.
    Refresh,
    /// The entity teleported or the bone is gone; the bolt is lost.
    Lost,
}

/// The entity a packed bolt follows.
pub const fn bolt_dobj(packed: u32) -> u32 {
    field_of(packed, DOBJ_MASK, 0)
}

/// The value in the field `mask` wide that starts at bit `shift`.
const fn field_of(packed: u32, mask: u32, shift: u32) -> u32 {
    (packed >> shift) & mask
}

/// `word` with `value`, cut to `mask`, set into the field starting at `shift`.
const fn with_field(word: u32, value: u32, mask: u32, shift: u32) -> u32 {
    word | ((value & mask) << shift)
}

/// Whether the packed bolt remembers its entity teleporting.
const fn bolt_teleport_bit(packed: u32) -> bool {
    field_of(packed, 1, TELEPORT_SHIFT) == 1
}

/// The bone a packed bolt follows.
pub const fn bolt_bone(packed: u32) -> u32 {
    field_of(packed, BONE_MASK, ATTACH_JOINT_BIT_OFFSET)
}

/// Whether the packed bolt has no entity.
pub const fn bolt_handle_is_none(packed: u32) -> bool {
    bolt_dobj(packed) == NO_ATTACH_TICKET
}

/// The bolt with its entity and bone blanked.
pub const fn bolt_mark_lost(packed: u32) -> u32 {
    packed | LOST_BITS
}

/// A bolt from its entity, teleport flag and bone; each is cut to its field.
pub const fn bolt_pack(dobj: u32, snapped: bool, joint: u32) -> u32 {
    let word = with_field(0, dobj, DOBJ_MASK, 0);
    let word = with_field(word, snapped as u32, 1, TELEPORT_SHIFT);
    with_field(word, joint, BONE_MASK, ATTACH_JOINT_BIT_OFFSET)
}

/// The teleport flag to compare a bolt's against: only world entities have
/// one, everything else counts as not teleported.
pub const fn bolt_centity_teleport_for_compare(dobj: u32, snapped: bool) -> bool {
    dobj < CENTITY_LIMIT && snapped
}

/// The teleport flag a new bolt on `dobj` starts with, from the entity's
/// state flags.
pub const fn bolt_spawn_teleport_bit(dobj: u32, upcoming_body_bits: u32) -> bool {
    bolt_centity_teleport_for_compare(dobj, upcoming_body_bits & CENTITY_TELEPORT_FLAG != 0)
}

/// Where the orientation of `bone` on `dobj` is to be read from.
///
/// `bone_total` is the bone count of the entity's skeleton, if it has
/// one.
pub fn get_bone_orientation_route(
    dobj: u32,
    pose_now_flag: bool,
    bone: i32,
    bone_total: Option<u8>,
) -> Result<JointFacingPath, BoneOrientationRefuse> {
    // A world entity must be live; view models and the like always are.
    let stale = dobj < CENTITY_LIMIT && !pose_now_flag;
    match (stale, bone, bone_total) {
        (true, ..) => Err(BoneOrientationRefuse::ClientThingGone),
        (false, b, _) if b < 0 => Ok(JointFacingPath::OwnPose),
        (false, _, None) => Err(BoneOrientationRefuse::SkeletonAbsent),
        (false, b, Some(count)) => match u16::try_from(b) {
            Ok(index) if index < u16::from(count) => Ok(JointFacingPath::SkeletonJoint(index)),
            _ => Err(BoneOrientationRefuse::BoneIndexInvalid),
        },
    }
}

/// Whether a plain stop (not one that recurses into owned effects) acts on an
/// effect with this status: it must hold a reference.
pub const fn stop_effect_non_recursive_allows(status: u32) -> bool {
    status & FX_STATUS_REF_COUNT_LOW_MASK != 0
}

/// Whether the effect with this status owns other effects.
pub const fn stop_effect_has_owned(status: u32) -> bool {
    status & EMITTER_CHILD_TALLY_BITS != 0
}

/// The iterator count after one more pass starts: a negative count starts
/// over from zero.
pub fn begin_iterating_over_effects_exclusive(walker_total: i32) -> i32 {
    if walker_total < 0 {
        1
    } else {
        walker_total.wrapping_add(1)
    }
}

/// The iterator count after a pass ends.
pub fn end_iterating_over_effects(walker_total: i32) -> i32 {
    walker_total.wrapping_sub(1)
}

/// Whether the pass that just ended was the last and effects are waiting to
/// be cleaned up.
pub fn end_iterating_runs_gc(walker_post_step: i32, wants_sweep: bool) -> bool {
    walker_post_step == 0 && wants_sweep
}

/// The product of two rotations in the order bolts compose them: `a` is the
/// parent's, `b` the bone's.
pub fn quat_mul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let (av, aw) = split_quat(a);
    let (bv, bw) = split_quat(b);
    // Vector part: each side's vector scaled by the other's scalar, plus the
    // cross product with the bone's vector first.
    let swirl = wedge(bv, av);
    let vector: [f32; 3] = core::array::from_fn(|k| aw * bv[k] + bw * av[k] + swirl[k]);
    let inner = av.iter().zip(&bv).map(|(x, y)| x * y).sum::<f32>();
    [vector[0], vector[1], vector[2], aw * bw - inner]
}

fn split_quat(q: [f32; 4]) -> ([f32; 3], f32) {
    ([q[0], q[1], q[2]], q[3])
}

/// The cross product, each component from the two axes after it (cyclically).
fn wedge(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    core::array::from_fn(|k| {
        let (j, l) = ((k + 1) % 3, (k + 2) % 3);
        a[j] * b[l] - a[l] * b[j]
    })
}

/// A vector turned by a unit rotation.
fn quat_transform_vec(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    // v + w*t + u*t with t = 2 (u * v), u the rotation's vector part.
    let (u, w) = split_quat(q);
    let t = wedge(u, v).map(|c| 2.0 * c);
    let ut = wedge(u, t);
    core::array::from_fn(|k| v[k] + w * t[k] + ut[k])
}

/// The orientation a bolt's parent starts with: no rotation, at the origin.
pub const fn bolt_init_parent_orientation() -> ([f32; 4], [f32; 3]) {
    (ATTACH_ROOT_NO_TURN, [0.0; 3])
}

/// The record a free-list link points to while the records are first being
/// chained together: the next, or the end after the last.
pub const fn bolt_init_next_index(i: i32) -> i32 {
    match i {
        i32::MIN..INIT_LAST => i + 1,
        _ => NO_SPARE_ATTACH,
    }
}

/// Takes the record at the head of the free list. Gives it and the new head
/// (`taken_next`, the link the taken record held), or nothing when the list
/// is empty.
pub const fn bolt_alloc(first_free: i32, taken_next: i32) -> Option<(i32, i32)> {
    match first_free {
        NO_SPARE_ATTACH => None,
        head => Some((head, taken_next)),
    }
}

/// A bone's orientation in the world: the bone's rotation and offset carried
/// through its parent's.
pub fn bolt_compose_orientation(
    host_rot: [f32; 4],
    host_at: [f32; 3],
    joint_quat: [f32; 4],
    joint_origin: [f32; 3],
) -> ([f32; 4], [f32; 3]) {
    let carried = quat_transform_vec(host_rot, joint_origin);
    let origin = core::array::from_fn(|k| host_at[k] + carried[k]);
    (quat_mul(host_rot, joint_quat), origin)
}

/// What an effect does with its bolt this update.
///
/// `bolt_byte` is its bolt record number, `packed` the record's word,
/// `client_actor_teleport` whether the entity's state now says teleported, and
/// `joint_ok` whether the bone's orientation could be read.
pub fn update_effect_bolt(
    bolt_byte: u8,
    packed: u32,
    client_actor_teleport: bool,
    joint_ok: bool,
) -> EmitterAttachRefresh {
    let has_bolt = bolt_byte != NO_BOLT_RECORD && !bolt_handle_is_none(packed);
    let same_teleport = bolt_centity_teleport_for_compare(bolt_dobj(packed), client_actor_teleport)
        == bolt_teleport_bit(packed);
    match (has_bolt, same_teleport && joint_ok) {
        (false, _) => EmitterAttachRefresh::Skip,
        (true, true) => EmitterAttachRefresh::Refresh,
        (true, false) => EmitterAttachRefresh::Lost,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDENTITY: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
    const QUARTER_TURN_Z: [f32; 4] = [
        0.0,
        0.0,
        core::f32::consts::FRAC_1_SQRT_2,
        core::f32::consts::FRAC_1_SQRT_2,
    ];

    fn near(a: &[f32], b: &[f32]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6)
    }

    #[test]
    fn a_packed_bolt_round_trips_its_fields() {
        let packed = bolt_pack(5, true, 9);
        assert_eq!(bolt_dobj(packed), 5);
        assert!(bolt_teleport_bit(packed));
        assert_eq!(bolt_bone(packed), 9);
        assert!(!bolt_handle_is_none(packed));
    }

    #[test]
    fn packing_cuts_each_field_to_its_width() {
        let packed = bolt_pack(0x1005, false, 0x1009);
        assert_eq!(bolt_dobj(packed), 5);
        assert!(!bolt_teleport_bit(packed));
        assert_eq!(bolt_bone(packed), 9);
    }

    #[test]
    fn a_lost_bolt_has_no_entity_or_bone_but_keeps_its_teleport_flag() {
        for teleport in [false, true] {
            let lost = bolt_mark_lost(bolt_pack(5, teleport, 9));
            assert!(bolt_handle_is_none(lost));
            assert_eq!(bolt_bone(lost), BONE_MASK);
            assert_eq!(bolt_teleport_bit(lost), teleport);
        }
    }

    #[test]
    fn only_world_entities_have_a_teleport_flag_to_compare() {
        assert!(bolt_centity_teleport_for_compare(0, true));
        assert!(bolt_centity_teleport_for_compare(0x7fd, true));
        assert!(!bolt_centity_teleport_for_compare(0x7fe, true));
        assert!(!bolt_centity_teleport_for_compare(
            ATTACH_HELD_GUN_SKELETON_START,
            true
        ));
        assert!(!bolt_centity_teleport_for_compare(3, false));
    }

    #[test]
    fn a_new_bolt_starts_teleported_when_the_entity_state_says_so() {
        assert!(bolt_spawn_teleport_bit(10, 2));
        assert!(bolt_spawn_teleport_bit(10, 7));
        assert!(!bolt_spawn_teleport_bit(10, 5));
        assert!(!bolt_spawn_teleport_bit(ATTACH_HELD_GUN_SKELETON_START, 2));
    }

    #[test]
    fn a_world_entity_that_is_not_current_has_no_bone_orientation() {
        assert_eq!(
            get_bone_orientation_route(3, false, 0, Some(4)),
            Err(BoneOrientationRefuse::ClientThingGone)
        );
        assert_eq!(
            get_bone_orientation_route(3, false, -1, None),
            Err(BoneOrientationRefuse::ClientThingGone)
        );
    }

    #[test]
    fn a_negative_bone_is_the_entity_pose() {
        assert_eq!(
            get_bone_orientation_route(3, true, -1, None),
            Ok(JointFacingPath::OwnPose)
        );
        assert_eq!(
            get_bone_orientation_route(ATTACH_HELD_GUN_SKELETON_START, false, -5, None),
            Ok(JointFacingPath::OwnPose)
        );
    }

    #[test]
    fn a_bone_needs_a_skeleton_and_must_be_inside_it() {
        assert_eq!(
            get_bone_orientation_route(3, true, 2, None),
            Err(BoneOrientationRefuse::SkeletonAbsent)
        );
        assert_eq!(
            get_bone_orientation_route(3, true, 4, Some(4)),
            Err(BoneOrientationRefuse::BoneIndexInvalid)
        );
        assert_eq!(
            get_bone_orientation_route(3, true, 3, Some(4)),
            Ok(JointFacingPath::SkeletonJoint(3))
        );
        assert_eq!(
            get_bone_orientation_route(ATTACH_HELD_GUN_SKELETON_START, false, 0, Some(1)),
            Ok(JointFacingPath::SkeletonJoint(0))
        );
    }

    #[test]
    fn a_plain_stop_needs_a_reference() {
        assert!(stop_effect_non_recursive_allows(1));
        assert!(stop_effect_non_recursive_allows(0x1fff));
        assert!(!stop_effect_non_recursive_allows(0));
        assert!(!stop_effect_non_recursive_allows(0x2000));
    }

    #[test]
    fn owned_effects_are_in_their_own_bits() {
        assert!(stop_effect_has_owned(0x0001_0000));
        assert!(stop_effect_has_owned(0x03ff_0001));
        assert!(!stop_effect_has_owned(0xfc00_ffff));
    }

    #[test]
    fn iterating_counts_passes_and_restarts_from_below_zero() {
        assert_eq!(begin_iterating_over_effects_exclusive(0), 1);
        assert_eq!(begin_iterating_over_effects_exclusive(2), 3);
        assert_eq!(begin_iterating_over_effects_exclusive(-7), 1);
        assert_eq!(end_iterating_over_effects(3), 2);
        assert_eq!(end_iterating_over_effects(i32::MIN), i32::MAX);
    }

    #[test]
    fn cleanup_waits_for_the_last_pass() {
        assert!(end_iterating_runs_gc(0, true));
        assert!(!end_iterating_runs_gc(0, false));
        assert!(!end_iterating_runs_gc(1, true));
    }

    #[test]
    fn the_identity_rotation_leaves_a_rotation_alone() {
        let q = [0.1, 0.2, 0.3, 0.9];
        assert!(near(&quat_mul(IDENTITY, q), &q));
        assert!(near(&quat_mul(q, IDENTITY), &q));
    }

    #[test]
    fn two_quarter_turns_make_a_half_turn() {
        let got = quat_mul(QUARTER_TURN_Z, QUARTER_TURN_Z);
        assert!(near(&got, &[0.0, 0.0, 1.0, 0.0]), "{got:?}");
    }

    #[test]
    fn a_quarter_turn_about_z_carries_x_to_y() {
        let got = quat_transform_vec(QUARTER_TURN_Z, [1.0, 0.0, 0.0]);
        assert!(near(&got, &[0.0, 1.0, 0.0]), "{got:?}");
        let up = quat_transform_vec(QUARTER_TURN_Z, [0.0, 0.0, 5.0]);
        assert!(near(&up, &[0.0, 0.0, 5.0]), "{up:?}");
    }

    #[test]
    fn the_identity_rotation_carries_nothing() {
        assert_eq!(
            quat_transform_vec(IDENTITY, [1.0, 2.0, 3.0]),
            [1.0, 2.0, 3.0]
        );
    }

    #[test]
    fn a_parent_places_its_bone_by_its_own_rotation_and_origin() {
        let (quat, origin) =
            bolt_compose_orientation(QUARTER_TURN_Z, [10.0, 0.0, 0.0], IDENTITY, [1.0, 0.0, 0.0]);
        assert!(near(&quat, &QUARTER_TURN_Z));
        assert!(near(&origin, &[10.0, 1.0, 0.0]), "{origin:?}");
    }

    #[test]
    fn a_new_parent_is_unrotated_at_the_origin() {
        let (quat, origin) = bolt_init_parent_orientation();
        assert_eq!(quat, IDENTITY);
        assert!(origin.iter().all(|&c| c == 0.0));
    }

    #[test]
    fn the_free_list_chains_the_records_in_order_and_ends() {
        assert_eq!(bolt_init_next_index(0), 1);
        assert_eq!(bolt_init_next_index(0xfd), 0xfe);
        assert_eq!(bolt_init_next_index(0xfe), NO_SPARE_ATTACH);
        assert_eq!(ATTACH_RECORD_LIMIT as i32 - 1, INIT_LAST);
    }

    #[test]
    fn allocating_takes_the_head_and_moves_it_on() {
        assert_eq!(bolt_alloc(4, 5), Some((4, 5)));
        assert_eq!(bolt_alloc(NO_SPARE_ATTACH, 5), None);
    }

    /// Each row: record number, packed word, entity teleported now, bone read ok, outcome.
    fn check_updates(rows: &[(u8, u32, bool, bool, EmitterAttachRefresh)]) {
        for &(record, packed, teleport_now, bone_ok, want) in rows {
            let got = update_effect_bolt(record, packed, teleport_now, bone_ok);
            assert_eq!(got, want, "record {record} packed {packed:#x} tp {teleport_now} bone {bone_ok}");
        }
    }

    #[test]
    fn an_effect_with_no_bolt_or_no_entity_skips() {
        let packed = bolt_pack(5, false, 1);
        check_updates(&[
            (NO_BOLT_RECORD, packed, false, true, EmitterAttachRefresh::Skip),
            (3, bolt_mark_lost(packed), false, true, EmitterAttachRefresh::Skip),
        ]);
    }

    #[test]
    fn a_steady_bolt_refreshes() {
        check_updates(&[
            (3, bolt_pack(5, false, 1), false, true, EmitterAttachRefresh::Refresh),
            (3, bolt_pack(5, true, 1), true, true, EmitterAttachRefresh::Refresh),
        ]);
    }

    #[test]
    fn a_new_teleport_or_a_missing_bone_loses_the_bolt() {
        let packed = bolt_pack(5, false, 1);
        check_updates(&[
            (3, packed, true, true, EmitterAttachRefresh::Lost),
            (3, packed, false, false, EmitterAttachRefresh::Lost),
        ]);
    }

    #[test]
    fn view_model_bolts_ignore_the_entity_teleport_flag() {
        let view_model = bolt_pack(ATTACH_HELD_GUN_SKELETON_START & DOBJ_MASK, false, 1);
        check_updates(&[(3, view_model, true, true, EmitterAttachRefresh::Refresh)]);
    }

    fn unit(x: f32, y: f32, z: f32, w: f32) -> [f32; 4] {
        let n = (x * x + y * y + z * z + w * w).sqrt();
        [x / n, y / n, z / n, w / n]
    }

    #[test]
    fn composing_two_turns_equals_turning_by_the_parent_then_the_bone() {
        let parent = unit(0.3, -0.5, 0.2, 0.8);
        let bone = unit(-0.6, 0.1, 0.4, 0.5);
        let v = [1.5, -2.0, 0.75];
        let once = quat_transform_vec(quat_mul(parent, bone), v);
        let twice = quat_transform_vec(bone, quat_transform_vec(parent, v));
        assert!(near(&once, &twice), "{once:?} vs {twice:?}");
    }

    #[test]
    fn a_turn_keeps_a_vectors_length_and_the_quarter_turn_is_ninety_degrees() {
        let q = unit(0.3, -0.5, 0.2, 0.8);
        let turned = quat_transform_vec(q, [3.0, 4.0, 12.0]);
        let len = (turned[0] * turned[0] + turned[1] * turned[1] + turned[2] * turned[2]).sqrt();
        assert!((len - 13.0).abs() < 1e-4, "{len}");
        assert!(near(&quat_transform_vec(QUARTER_TURN_Z, [1.0, 0.0, 0.0]), &[0.0, 1.0, 0.0]));
        assert!(near(&quat_mul(IDENTITY, q), &q) && near(&quat_mul(q, IDENTITY), &q));
    }
}
