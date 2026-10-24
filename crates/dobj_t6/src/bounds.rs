/// The attach byte of a model that is not attached to a bone.
pub const DOBJ_NO_ATTACH: u8 = 0xff;

/// One model of a DObj as `DObjComputeBounds` sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RadiusModel {
    /// The model's stored radius (`XModel::radius`).
    pub radius: f32,
    /// The bone of model 0 it attaches to, or [`DOBJ_NO_ATTACH`].
    pub attach: u8,
}

impl RadiusModel {
    /// A model on no bone: its radius alone.
    pub const fn unattached(radius: f32) -> Self {
        Self { radius, attach: DOBJ_NO_ATTACH }
    }
}

/// `DObjComputeBounds`: the largest of each model's radius, where a model
/// attached to a bone of model 0 adds the length of that bone's base-pose
/// translation (`model0_base_trans`, one per bone of model 0). An attach to
/// a bone past model 0's bones adds nothing. No models: 0.
pub fn compute_radius(models: &[RadiusModel], model0_base_trans: &[[f32; 3]]) -> f32 {
    models.iter().fold(0.0f32, |max, model| {
        let offset = if model.attach == DOBJ_NO_ATTACH {
            0.0
        } else {
            model0_base_trans
                .get(usize::from(model.attach))
                .map_or(0.0, |t| (t[0] * t[0] + t[1] * t[1] + t[2] * t[2]).sqrt())
        };
        max.max(model.radius + offset)
    })
}

/// `DObjGetBounds`: a cube of the radius around the DObj origin, as
/// (mins, maxs).
pub fn bounds(radius: f32) -> ([f32; 3], [f32; 3]) {
    ([-radius; 3], [radius; 3])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attached_models_add_their_bone_offset() {
        let trans = [[0.0; 3], [3.0, 4.0, 0.0]];
        let body = RadiusModel {
            radius: 10.0,
            attach: DOBJ_NO_ATTACH,
        };
        let gun = RadiusModel {
            radius: 8.0,
            attach: 1,
        };
        assert_eq!(compute_radius(&[body, gun], &trans), 13.0);
        // A bone model 0 does not have adds nothing.
        let far = RadiusModel {
            radius: 8.0,
            attach: 7,
        };
        assert_eq!(compute_radius(&[body, far], &trans), 10.0);
        assert_eq!(compute_radius(&[], &trans), 0.0);
    }

    #[test]
    fn bounds_are_a_cube() {
        assert_eq!(bounds(5.0), ([-5.0; 3], [5.0; 3]));
    }
}
