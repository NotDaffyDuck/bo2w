//! Bolts: how an effect is attached to a moving entity bone, and the small
//! rules around following it, stopping it and iterating over effects.
//!
//! An effect's bolt is a packed word: the entity (or view model) it follows,
//! whether that entity teleported, and which bone. A bolt that can no longer
//! be followed is marked lost by blanking its entity and bone.

use crate::runtime::status::{FX_STATUS_OWNED_EFFECTS_MASK, FX_STATUS_REF_COUNT_LOW_MASK};

/// A packed bolt's entity is its low twelve bits; all ones means none.
const DOBJ_MASK: u32 = 0xfff;
pub const FX_BOLT_HANDLE_NONE: u32 = 0xfff;
/// The teleport flag is the bit above the entity.
const TELEPORT_SHIFT: u32 = 12;
/// The bone is eleven bits from here up.
pub const FX_BOLT_BONE_SHIFT: u32 = 13;
const BONE_MASK: u32 = 0x7ff;
/// ORed over a packed bolt: entity and bone all ones, teleport flag kept.
const LOST_BITS: u32 = 0x00ff_efff;

/// Entities below this are world entities with a state of their own to check.
const CENTITY_LIMIT: u32 = 0x7fe;
/// View model objects number up from here.
pub const FX_BOLT_VIEWMODEL_DOBJ_BASE: u32 = 0x800;
/// The entity state flag that says the entity teleported.
const CENTITY_TELEPORT_FLAG: u32 = 2;

/// The orientation of a bolt's parent before it has been placed.
pub const FX_BOLT_PARENT_IDENTITY_QUAT: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

/// How many bolt records there are.
pub const FX_BOLT_RECORD_CAPACITY: u32 = 0xff;
/// The last record linked into the free list when it is first built.
const INIT_LAST: i32 = 0xfe;
/// The free list's end.
pub const FX_BOLT_FREE_NONE: i32 = -1;

/// An effect whose bolt record number is this has no bolt.
const NO_BOLT_RECORD: u8 = 0xff;

/// Where a bone's orientation comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FxGetBoneOrientationRoute {
    /// A negative bone: the entity's own pose.
    EntityPose,
    /// A bone of the entity's skeleton.
    DObjBone(u16),
}

/// Why a bone's orientation could not be found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoneOrientationRefuse {
    CentityInvalid,
    DObjMissing,
    BoneOutOfRange,
}

/// What an effect does with its bolt on an update.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FxUpdateEffectBolt {
    /// There is no bolt to follow.
    Skip,
    /// The bolt is still good; take the bone's new orientation.
    Refresh,
    /// The entity teleported or the bone is gone; the bolt is lost.
    Lost,
}

/// The entity a packed bolt follows.
pub const fn bolt_dobj(packed: u32) -> u32 {
    packed & DOBJ_MASK
}

/// Whether the packed bolt remembers its entity teleporting.
const fn bolt_teleport_bit(packed: u32) -> bool {
    (packed >> TELEPORT_SHIFT) & 1 != 0
}

/// The bone a packed bolt follows.
pub const fn bolt_bone(packed: u32) -> u32 {
    (packed >> FX_BOLT_BONE_SHIFT) & BONE_MASK
}

/// Whether the packed bolt has no entity.
pub const fn bolt_handle_is_none(packed: u32) -> bool {
    bolt_dobj(packed) == FX_BOLT_HANDLE_NONE
}

/// The bolt with its entity and bone blanked.
pub const fn bolt_mark_lost(packed: u32) -> u32 {
    packed | LOST_BITS
}

/// A bolt from its entity, teleport flag and bone; each is cut to its field.
pub const fn bolt_pack(dobj: u32, teleport: bool, bone: u32) -> u32 {
    (dobj & DOBJ_MASK)
        | ((teleport as u32) << TELEPORT_SHIFT)
        | ((bone & BONE_MASK) << FX_BOLT_BONE_SHIFT)
}

/// The teleport flag to compare a bolt's against: only world entities have
/// one, everything else counts as not teleported.
pub const fn bolt_centity_teleport_for_compare(dobj: u32, teleport: bool) -> bool {
    dobj < CENTITY_LIMIT && teleport
}

/// The teleport flag a new bolt on `dobj` starts with, from the entity's
/// state flags.
pub const fn bolt_spawn_teleport_bit(dobj: u32, next_state_eflags: u32) -> bool {
    bolt_centity_teleport_for_compare(dobj, next_state_eflags & CENTITY_TELEPORT_FLAG != 0)
}

/// Where the orientation of `bone` on `dobj` is to be read from.
///
/// `dobj_bone_count` is the bone count of the entity's skeleton, if it has
/// one.
pub fn get_bone_orientation_route(
    dobj: u32,
    current_valid: bool,
    bone: i32,
    dobj_bone_count: Option<u8>,
) -> Result<FxGetBoneOrientationRoute, BoneOrientationRefuse> {
    if dobj < CENTITY_LIMIT && !current_valid {
        return Err(BoneOrientationRefuse::CentityInvalid);
    }
    if bone < 0 {
        return Ok(FxGetBoneOrientationRoute::EntityPose);
    }
    match dobj_bone_count {
        None => Err(BoneOrientationRefuse::DObjMissing),
        Some(count) if bone >= i32::from(count) => Err(BoneOrientationRefuse::BoneOutOfRange),
        Some(_) => Ok(FxGetBoneOrientationRoute::DObjBone(bone as u16)),
    }
}

/// Whether a plain stop (not one that recurses into owned effects) acts on an
/// effect with this status: it must hold a reference.
pub const fn stop_effect_non_recursive_allows(status: u32) -> bool {
    status & FX_STATUS_REF_COUNT_LOW_MASK != 0
}

/// Whether the effect with this status owns other effects.
pub const fn stop_effect_has_owned(status: u32) -> bool {
    status & FX_STATUS_OWNED_EFFECTS_MASK != 0
}

/// The iterator count after one more pass starts: a negative count starts
/// over from zero.
pub fn begin_iterating_over_effects_exclusive(iterator_count: i32) -> i32 {
    iterator_count.max(0).wrapping_add(1)
}

/// The iterator count after a pass ends.
pub fn end_iterating_over_effects(iterator_count: i32) -> i32 {
    iterator_count.wrapping_sub(1)
}

/// Whether the pass that just ended was the last and effects are waiting to
/// be cleaned up.
pub fn end_iterating_runs_gc(iterator_after_dec: i32, needs_gc: bool) -> bool {
    iterator_after_dec == 0 && needs_gc
}

/// The product of two rotations in the order bolts compose them: `a` is the
/// parent's, `b` the bone's.
pub fn quat_mul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let [ax, ay, az, aw] = a;
    let [bx, by, bz, bw] = b;
    [
        ((az * by + bx * aw) + ax * bw) - ay * bz,
        (bz * ax + by * aw) + (ay * bw - az * bx),
        bz * aw + ((ay * bx + az * bw) - ax * by),
        ((aw * bw - ax * bx) - ay * by) - bz * az,
    ]
}

/// A vector turned by a unit rotation.
fn quat_transform_vec(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let [qx, qy, qz, qw] = q;
    let [vx, vy, vz] = v;
    [
        vx + (((qw * qy + qx * qz) * vz + vy * (qy * qx - qw * qz)) + vx * (-qy * qy + -qz * qz))
            * 2.0,
        (((qy * qz - qw * qx) * vz + vx * (qw * qz + qy * qx)) + vy * (-qx * qx + -qz * qz)) * 2.0
            + vy,
        2.0 * ((vx * (qx * qz - qw * qy) + vy * (qw * qx + qy * qz)) + vz * (-qy * qy + -qx * qx))
            + vz,
    ]
}

/// The orientation a bolt's parent starts with: no rotation, at the origin.
pub const fn bolt_init_parent_orientation() -> ([f32; 4], [f32; 3]) {
    (FX_BOLT_PARENT_IDENTITY_QUAT, [0.0; 3])
}

/// The record a free-list link points to while the records are first being
/// chained together: the next, or the end after the last.
pub const fn bolt_init_next_index(i: i32) -> i32 {
    if i < INIT_LAST {
        i + 1
    } else {
        FX_BOLT_FREE_NONE
    }
}

/// Takes the record at the head of the free list. Gives it and the new head
/// (`next_at_taken`, the link the taken record held), or nothing when the list
/// is empty.
pub const fn bolt_alloc(first_free: i32, next_at_taken: i32) -> Option<(i32, i32)> {
    if first_free == FX_BOLT_FREE_NONE {
        None
    } else {
        Some((first_free, next_at_taken))
    }
}

/// A bone's orientation in the world: the bone's rotation and offset carried
/// through its parent's.
pub fn bolt_compose_orientation(
    parent_quat: [f32; 4],
    parent_origin: [f32; 3],
    bone_quat: [f32; 4],
    bone_origin: [f32; 3],
) -> ([f32; 4], [f32; 3]) {
    let carried = quat_transform_vec(parent_quat, bone_origin);
    (
        quat_mul(parent_quat, bone_quat),
        [
            parent_origin[0] + carried[0],
            parent_origin[1] + carried[1],
            parent_origin[2] + carried[2],
        ],
    )
}

/// What an effect does with its bolt this update.
///
/// `effect_bolt` is its bolt record number, `packed` the record's word,
/// `centity_teleport` whether the entity's state now says teleported, and
/// `bone_ok` whether the bone's orientation could be read.
pub fn update_effect_bolt(
    effect_bolt: u8,
    packed: u32,
    centity_teleport: bool,
    bone_ok: bool,
) -> FxUpdateEffectBolt {
    if effect_bolt == NO_BOLT_RECORD || bolt_handle_is_none(packed) {
        return FxUpdateEffectBolt::Skip;
    }
    let teleport = bolt_centity_teleport_for_compare(bolt_dobj(packed), centity_teleport);
    if teleport == bolt_teleport_bit(packed) && bone_ok {
        FxUpdateEffectBolt::Refresh
    } else {
        FxUpdateEffectBolt::Lost
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
            FX_BOLT_VIEWMODEL_DOBJ_BASE,
            true
        ));
        assert!(!bolt_centity_teleport_for_compare(3, false));
    }

    #[test]
    fn a_new_bolt_starts_teleported_when_the_entity_state_says_so() {
        assert!(bolt_spawn_teleport_bit(10, 2));
        assert!(bolt_spawn_teleport_bit(10, 7));
        assert!(!bolt_spawn_teleport_bit(10, 5));
        assert!(!bolt_spawn_teleport_bit(FX_BOLT_VIEWMODEL_DOBJ_BASE, 2));
    }

    #[test]
    fn a_world_entity_that_is_not_current_has_no_bone_orientation() {
        assert_eq!(
            get_bone_orientation_route(3, false, 0, Some(4)),
            Err(BoneOrientationRefuse::CentityInvalid)
        );
        assert_eq!(
            get_bone_orientation_route(3, false, -1, None),
            Err(BoneOrientationRefuse::CentityInvalid)
        );
    }

    #[test]
    fn a_negative_bone_is_the_entity_pose() {
        assert_eq!(
            get_bone_orientation_route(3, true, -1, None),
            Ok(FxGetBoneOrientationRoute::EntityPose)
        );
        assert_eq!(
            get_bone_orientation_route(FX_BOLT_VIEWMODEL_DOBJ_BASE, false, -5, None),
            Ok(FxGetBoneOrientationRoute::EntityPose)
        );
    }

    #[test]
    fn a_bone_needs_a_skeleton_and_must_be_inside_it() {
        assert_eq!(
            get_bone_orientation_route(3, true, 2, None),
            Err(BoneOrientationRefuse::DObjMissing)
        );
        assert_eq!(
            get_bone_orientation_route(3, true, 4, Some(4)),
            Err(BoneOrientationRefuse::BoneOutOfRange)
        );
        assert_eq!(
            get_bone_orientation_route(3, true, 3, Some(4)),
            Ok(FxGetBoneOrientationRoute::DObjBone(3))
        );
        assert_eq!(
            get_bone_orientation_route(FX_BOLT_VIEWMODEL_DOBJ_BASE, false, 0, Some(1)),
            Ok(FxGetBoneOrientationRoute::DObjBone(0))
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
        assert_eq!(
            bolt_init_parent_orientation(),
            (FX_BOLT_PARENT_IDENTITY_QUAT, [0.0; 3])
        );
    }

    #[test]
    fn the_free_list_chains_the_records_in_order_and_ends() {
        assert_eq!(bolt_init_next_index(0), 1);
        assert_eq!(bolt_init_next_index(0xfd), 0xfe);
        assert_eq!(bolt_init_next_index(0xfe), FX_BOLT_FREE_NONE);
        assert_eq!(FX_BOLT_RECORD_CAPACITY as i32 - 1, INIT_LAST);
    }

    #[test]
    fn allocating_takes_the_head_and_moves_it_on() {
        assert_eq!(bolt_alloc(4, 5), Some((4, 5)));
        assert_eq!(bolt_alloc(FX_BOLT_FREE_NONE, 5), None);
    }

    #[test]
    fn an_effect_with_no_bolt_or_no_entity_skips() {
        let packed = bolt_pack(5, false, 1);
        assert_eq!(
            update_effect_bolt(0xff, packed, false, true),
            FxUpdateEffectBolt::Skip
        );
        assert_eq!(
            update_effect_bolt(3, bolt_mark_lost(packed), false, true),
            FxUpdateEffectBolt::Skip
        );
    }

    #[test]
    fn a_steady_bolt_refreshes() {
        let packed = bolt_pack(5, false, 1);
        assert_eq!(
            update_effect_bolt(3, packed, false, true),
            FxUpdateEffectBolt::Refresh
        );
        let teleported = bolt_pack(5, true, 1);
        assert_eq!(
            update_effect_bolt(3, teleported, true, true),
            FxUpdateEffectBolt::Refresh
        );
    }

    #[test]
    fn a_new_teleport_or_a_missing_bone_loses_the_bolt() {
        let packed = bolt_pack(5, false, 1);
        assert_eq!(
            update_effect_bolt(3, packed, true, true),
            FxUpdateEffectBolt::Lost
        );
        assert_eq!(
            update_effect_bolt(3, packed, false, false),
            FxUpdateEffectBolt::Lost
        );
    }

    #[test]
    fn view_model_bolts_ignore_the_entity_teleport_flag() {
        let packed = bolt_pack(FX_BOLT_VIEWMODEL_DOBJ_BASE & DOBJ_MASK, false, 1);
        assert_eq!(
            update_effect_bolt(3, packed, true, true),
            FxUpdateEffectBolt::Refresh
        );
    }
}
