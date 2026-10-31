use core::ops::{Deref, DerefMut};
use crate::PartSet;

/// `DObjSetControlTagAngles` only takes a part under this index.
pub const CONTROL_TAG_LIMIT: usize = 0xFE;

/// One part's local pose once the calc's sums are finished: the rotation
/// (x y z w) and the translation the anims add to the bone's base offset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PartPose {
    pub rotation: [f32; 4],
    pub translation: [f32; 3],
}

impl PartPose {
    pub const IDENTITY: Self = Self::at_rest([0.0, 0.0, 0.0, 1.0]);

    /// A part held at `rotation` with no translation (what [`reset_part`]
    /// gives a part no anim touched).
    ///
    /// [`reset_part`]: crate::reset_part
    pub const fn at_rest(rotation: [f32; 4]) -> Self {
        Self {
            rotation,
            translation: [0.0; 3],
        }
    }
}

/// A DObj's part poses between the anim calc and the skeleton, with the
/// control-bit set `DObjSetControlTagAngles` marks (facts section 9). It
/// derefs to the parts, one per bone over all the DObj's models.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LocalPose {
    parts: Vec<PartPose>,
    pub control: PartSet,
}

impl LocalPose {
    pub fn new(parts: Vec<PartPose>) -> Self {
        Self {
            parts,
            control: PartSet::NONE,
        }
    }

    /// `DObjSetControlTagAngles`: marks `part` controlled and replaces its
    /// anim result with `rotation` (the identity when none is given) and no
    /// translation. A part at or past [`CONTROL_TAG_LIMIT`] is ignored.
    pub fn set_control_tag(&mut self, part: usize, rotation: Option<[f32; 4]>) {
        if part >= CONTROL_TAG_LIMIT {
            return;
        }
        let Some(pose) = self.parts.get_mut(part) else {
            return;
        };
        *pose = PartPose::at_rest(rotation.unwrap_or(PartPose::IDENTITY.rotation));
        self.control.set(part);
    }

    /// Copies part `source` (its pose and control bit) onto part `write_target`.
    pub fn copy_part(&mut self, write_target: usize, source: usize) {
        self.parts[write_target] = self.parts[source];
        if self.control.get(source) {
            self.control.set(write_target);
        } else {
            self.control.clear(write_target);
        }
    }
}

impl Deref for LocalPose {
    type Target = [PartPose];

    fn deref(&self) -> &[PartPose] {
        &self.parts
    }
}

impl DerefMut for LocalPose {
    fn deref_mut(&mut self) -> &mut [PartPose] {
        &mut self.parts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_control_tag_replaces_the_part_and_marks_it() {
        let mut pose = LocalPose::new(vec![PartPose::IDENTITY; 3]);
        pose[1].translation = [1.0, 2.0, 3.0];
        pose.set_control_tag(1, Some([0.0, 0.0, 1.0, 0.0]));
        assert_eq!(pose[1], PartPose::at_rest([0.0, 0.0, 1.0, 0.0]));
        assert!(pose.control.get(1));
        pose.set_control_tag(2, None);
        assert_eq!(pose[2], PartPose::IDENTITY);
        pose.set_control_tag(9, None);
        assert_eq!(pose.control.iter().collect::<Vec<_>>(), vec![1, 2]);
    }

    #[test]
    fn a_copied_part_takes_its_control_bit() {
        let mut pose = LocalPose::new(vec![PartPose::IDENTITY; 3]);
        pose.set_control_tag(0, Some([1.0, 0.0, 0.0, 0.0]));
        pose.copy_part(2, 0);
        assert_eq!(pose[2], pose[0]);
        assert!(pose.control.get(2));
        pose.copy_part(2, 1);
        assert!(!pose.control.get(2));
    }
}
