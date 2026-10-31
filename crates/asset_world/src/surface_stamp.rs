//! The per-surface draw keys a world carries, and stamping each surface's
//! material into its key.

use std::fmt;

use dpvs_t6::{SortedDraw, populate_surface_material_table};

use crate::FaceBlocksSunlight;

/// A world's surfaces as the renderer sorts them: one packed draw key for
/// each, and which of them cast the sun's shadow. The two always have the
/// same length.
#[derive(Clone, Debug, Default)]
pub struct LevelSnapshot {
    /// Material and light bits are filled in by
    /// [`stamp_compact_facet_materials`] once the materials are known.
    pub compact_facet_draws: Vec<SortedDraw>,
    pub casters: FaceBlocksSunlight,
}

impl LevelSnapshot {
    pub fn facet_tally(&self) -> usize {
        self.compact_facet_draws.len()
    }
}

/// Why materials could not be stamped into a capture's draw keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FacetMaterialStampError {
    /// The capture's draw keys and shadow casters cover different numbers of
    /// surfaces.
    KeyCasterCountDiffers { keys: usize, shadow_casters: usize },
    /// A per-surface input list is not as long as the capture.
    WrongPayloadSize {
        surfaces: usize,
        slots_wanted: usize,
        key_lights: usize,
    },
}

impl fmt::Display for FacetMaterialStampError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::KeyCasterCountDiffers { keys, shadow_casters: casters } => {
                write!(f, "capture covers {keys} draw keys but {casters} casters")
            }
            Self::WrongPayloadSize { surfaces, slots_wanted, key_lights } => {
                let head = format!("capture has {surfaces} surfaces but {slots_wanted} material slots");
                write!(f, "{head} and {key_lights} primary lights")
            }
        }
    }
}

impl std::error::Error for FacetMaterialStampError {}

/// A capture whose draw keys are all blank, for the given shadow casters.
pub fn snapshot_from_sun_blockers(casters: FaceBlocksSunlight) -> LevelSnapshot {
    let blank = SortedDraw::unpack_from_bits(0);
    let compact_facet_draws = vec![blank; casters.len()];
    LevelSnapshot { casters, compact_facet_draws }
}

/// Stamping a capture's draw keys, or why it was refused.
type Stamped = Result<(), FacetMaterialStampError>;

/// Writes each surface's material and primary light into its draw key.
/// `baked` gives the key bits a material contributes.
pub fn stamp_compact_facet_materials(
    slots: &[Option<usize>],
    lights: &[u8],
    baked: &[Option<SortedDraw>],
    target: &mut LevelSnapshot,
) -> Stamped {
    let (keys, caster_total) = (target.compact_facet_draws.len(), target.casters.len());
    if keys != caster_total {
        return Err(FacetMaterialStampError::KeyCasterCountDiffers { keys, shadow_casters: caster_total });
    }
    let (slots_wanted, key_lights) = (slots.len(), lights.len());
    if [slots_wanted, key_lights].iter().any(|&n| n != keys) {
        return Err(FacetMaterialStampError::WrongPayloadSize { surfaces: keys, slots_wanted, key_lights });
    }
    populate_surface_material_table(slots, lights, baked, &mut target.compact_facet_draws);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_capture_has_a_blank_key_for_every_caster() {
        let made = snapshot_from_sun_blockers(FaceBlocksSunlight::with_len(4));
        assert_eq!(made.casters.len(), 4);
        assert_eq!(made.facet_tally(), 4);
    }

    #[test]
    fn stamping_refuses_inputs_of_the_wrong_length() {
        let mut held = snapshot_from_sun_blockers(FaceBlocksSunlight::with_len(2));
        let outcome = stamp_compact_facet_materials(&[None], &[0, 0], &[], &mut held);
        let expected = FacetMaterialStampError::WrongPayloadSize {
            surfaces: 2,
            slots_wanted: 1,
            key_lights: 2,
        };
        assert_eq!(outcome, Err(expected));
    }

    #[test]
    fn stamping_refuses_a_capture_whose_halves_disagree() {
        let mut held = snapshot_from_sun_blockers(FaceBlocksSunlight::with_len(2));
        held.casters = FaceBlocksSunlight::with_len(3);
        let outcome = stamp_compact_facet_materials(&[None; 2], &[0; 2], &[], &mut held);
        let expected = FacetMaterialStampError::KeyCasterCountDiffers { keys: 2, shadow_casters: 3 };
        assert_eq!(outcome, Err(expected));
    }

    #[test]
    fn stamping_matching_inputs_succeeds() {
        let mut held = snapshot_from_sun_blockers(FaceBlocksSunlight::with_len(2));
        let outcome = stamp_compact_facet_materials(&[None; 2], &[0; 2], &[], &mut held);
        assert_eq!(outcome, Ok(()));
    }
}
