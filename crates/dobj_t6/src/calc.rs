use crate::PartSet;

/// The part `DObjCalcAnim` always adds to the ignore mask (bit 0 of word
/// 4), so anim data never writes it.
pub const IGNORED_PART: usize = 159;

/// The scale of a stored int16 rotation component: full range is -1..1.
pub const QUAT16_SCALE: f32 = 3.051851e-05;

/// A stored base-pose or sample rotation (four int16, x y z w) as floats.
/// Not normalised.
pub fn quat16(q: [i16; 4]) -> [f32; 4] {
    q.map(|c| f32::from(c) * QUAT16_SCALE)
}

/// The part sets a DObj's skeleton keeps between calcs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SkelPartBits {
    /// Parts whose anim pose is already computed for this skeleton time.
    pub anim: PartSet,
    /// Parts script or a controller overrides (tag angles, IK).
    pub control: PartSet,
    /// Parts whose world matrix is up to date.
    pub bone_rig: PartSet,
}

/// The two masks one anim calc walks the tree with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CalcMasks {
    /// Parts not wanted, or already computed: nothing writes them.
    pub skip: PartSet,
    /// The skip mask plus [`IGNORED_PART`], for the tree walk.
    pub ignore: PartSet,
}

impl SkelPartBits {
    /// `DObjCalcAnim`'s start: `None` when every part is either unwanted or
    /// already computed (the call returns at once). Otherwise the wanted
    /// parts are marked computed and the masks returned.
    pub fn begin_calc(&mut self, wanted: &PartSet) -> Option<CalcMasks> {
        let skip = wanted.not().or(&self.anim);
        if skip.is_all() {
            return None;
        }
        self.anim = self.anim.or(wanted);
        let mut ignore = skip;
        ignore.set(IGNORED_PART);
        Some(CalcMasks { skip, ignore })
    }
}

impl CalcMasks {
    /// The parts to reset after the walk: not skipped and touched by no
    /// leaf (see [`touch_leaf`] and [`reset_part`]).
    pub fn untouched(&self, touched: &PartSet) -> PartSet {
        self.skip.or(touched).not()
    }
}

/// One anim leaf's share of the walk: the parts its bone mask drives that
/// are not skipped join `touched`.
pub fn touch_leaf(touched: &mut PartSet, skip: &PartSet, anim_mask: &PartSet) {
    *touched = touched.or(&skip.not().and(anim_mask));
}

/// One part's pose while the leaves blend into it (`DObjAnimMat`): the
/// weighted rotation sum, the weighted translation sum and the weight
/// total of the translation samples.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AnimMat {
    pub rot_sum: [f32; 4],
    pub shift_offset: [f32; 3],
    pub move_blend_amount: f32,
}

impl AnimMat {
    pub const ZERO: Self = Self {
        rot_sum: [0.0; 4],
        shift_offset: [0.0; 3],
        move_blend_amount: 0.0,
    };

    /// A bone with no rotation data in this anim (group 0): the identity
    /// sample, added as `w += weight`.
    pub fn blend_identity(&mut self, share: f32) {
        self.rot_sum[3] += share;
    }

    /// A rotation sample (groups 1-4): flipped when it points away from the
    /// sum so far, then added times the weight (a sum, not a slerp).
    pub fn blend_rotation(&mut self, q: [f32; 4], share: f32) {
        let dot: f32 = (0..4).map(|i| q[i] * self.rot_sum[i]).sum();
        let w = if dot < 0.0 { -share } else { share };
        for (sum, c) in self.rot_sum.iter_mut().zip(q) {
            *sum += c * w;
        }
    }

    /// A translation sample (groups 5-7). Groups 5 and 6 add the weight to
    /// the translation total; group 7 is taken the same way [inferred: the
    /// facts list only `position += value * weight` for it].
    pub fn blend_translation(&mut self, t: [f32; 3], share: f32) {
        for (sum, c) in self.shift_offset.iter_mut().zip(t) {
            *sum += c * share;
        }
        self.move_blend_amount += share;
    }

    /// A bone with a translation weight and no translation (group 8).
    pub fn blend_translation_share(&mut self, share: f32) {
        self.move_blend_amount += share;
    }

    /// The blended local pose: the rotation sum normalised (identity when
    /// nothing was added) and the translation sum over its weight total.
    /// The game's exact normalise step is not in the facts (UNCLASSIFIED); this
    /// is the plain one.
    pub fn finish(&self) -> ([f32; 4], [f32; 3]) {
        let q = self.rot_sum;
        let magnitude_squared = q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3];
        let quat = if magnitude_squared > 1e-12 {
            let stow_kind = 1.0 / magnitude_squared.sqrt();
            q.map(|c| c * stow_kind)
        } else {
            [0.0, 0.0, 0.0, 1.0]
        };
        let shift_offset = if self.move_blend_amount > 0.0 {
            let stow_kind = 1.0 / self.move_blend_amount;
            self.shift_offset.map(|c| c * stow_kind)
        } else {
            self.shift_offset
        };
        (quat, shift_offset)
    }
}

/// The pose of a part no anim touched: a root bone gets the identity and
/// no translation; any other bone its model's base-pose rotation (`base`,
/// four int16) and no translation.
pub fn reset_part(base: Option<[i16; 4]>) -> ([f32; 4], [f32; 3]) {
    let quat = base.map_or([0.0, 0.0, 0.0, 1.0], quat16);
    (quat, [0.0; 3])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PartSet;

    #[test]
    fn calc_skips_unwanted_and_computed_parts() {
        let mut bone_rig = SkelPartBits::default();
        let wanted = PartSet::first(4);
        let masks = bone_rig.begin_calc(&wanted).unwrap();
        assert_eq!(masks.skip, wanted.not());
        assert_eq!(masks.ignore, masks.skip);
        assert_eq!(bone_rig.anim, wanted);
        // Every part wanted: only part 159 is ignored, and only for the walk.
        let all = SkelPartBits::default().begin_calc(&PartSet::ALL).unwrap();
        assert!(all.skip.is_empty());
        assert_eq!(all.ignore.iter().collect::<Vec<_>>(), [IGNORED_PART]);
        // The same parts again are already computed.
        assert!(bone_rig.begin_calc(&wanted).is_none());
        // One more part: only it is computed.
        let masks = bone_rig.begin_calc(&PartSet::first(5)).unwrap();
        assert_eq!(masks.skip.not().iter().collect::<Vec<_>>(), [4]);
    }

    #[test]
    fn untouched_parts_are_the_wanted_ones_no_leaf_drove() {
        let mut bone_rig = SkelPartBits::default();
        let masks = bone_rig.begin_calc(&PartSet::first(4)).unwrap();
        let mut touched = PartSet::NONE;
        let mut leaf = PartSet::NONE;
        leaf.set(1);
        leaf.set(9);
        touch_leaf(&mut touched, &masks.skip, &leaf);
        assert_eq!(touched.iter().collect::<Vec<_>>(), [1]);
        assert_eq!(masks.untouched(&touched).iter().collect::<Vec<_>>(), [0, 2, 3]);
    }

    #[test]
    fn rotation_sum_flips_samples_that_point_away() {
        let mut m = AnimMat::ZERO;
        m.blend_rotation([0.0, 0.0, 0.0, 1.0], 0.5);
        m.blend_rotation([0.0, 0.0, 0.0, -1.0], 0.5);
        assert_eq!(m.rot_sum, [0.0, 0.0, 0.0, 1.0]);
        let mut id = AnimMat::ZERO;
        id.blend_identity(0.25);
        assert_eq!(id.finish().0, [0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn translation_divides_by_the_translation_weight_only() {
        let mut m = AnimMat::ZERO;
        m.blend_rotation([0.0, 0.0, 0.0, 1.0], 0.5);
        m.blend_translation([2.0, 4.0, 6.0], 0.5);
        m.blend_identity(0.5);
        assert_eq!(m.finish().1, [2.0, 4.0, 6.0]);
        m.blend_translation_share(0.5);
        assert_eq!(m.finish().1, [1.0, 2.0, 3.0]);
        assert_eq!(AnimMat::ZERO.finish(), ([0.0, 0.0, 0.0, 1.0], [0.0; 3]));
    }

    #[test]
    fn reset_uses_identity_for_roots_and_the_base_pose_otherwise() {
        assert_eq!(reset_part(None), ([0.0, 0.0, 0.0, 1.0], [0.0; 3]));
        let (q, t) = reset_part(Some([0, 0, 32767, 0]));
        assert!((q[2] - 1.0).abs() < 1e-6 && t == [0.0; 3]);
    }
}
