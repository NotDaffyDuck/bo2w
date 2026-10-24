//! The per-surface draw keys a world carries, and stamping each surface's
//! material into its key.

use std::fmt;

use dpvs_t6::{GfxDrawSurf, fill_surface_materials};

use crate::SurfaceCastsSunShadow;

/// A world's surfaces as the renderer sorts them: one packed draw key for
/// each, and which of them cast the sun's shadow. The two always have the
/// same length.
#[derive(Clone, Debug, Default)]
pub struct WorldCapture {
    /// Material and light bits are filled in by
    /// [`stamp_packed_surface_materials`] once the materials are known.
    pub packed_draw_surfs: Vec<GfxDrawSurf>,
    pub casters: SurfaceCastsSunShadow,
}

impl WorldCapture {
    pub fn surface_count(&self) -> usize {
        self.packed_draw_surfs.len()
    }
}

/// Why materials could not be stamped into a capture's draw keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceMaterialStampError {
    /// The capture's draw keys and shadow casters cover different numbers of
    /// surfaces.
    CaptureLengthMismatch { packed: usize, casters: usize },
    /// A per-surface input list is not as long as the capture.
    InputLengthMismatch {
        authored: usize,
        material_slots: usize,
        primary_lights: usize,
    },
}

impl fmt::Display for SurfaceMaterialStampError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::CaptureLengthMismatch { packed, casters } => {
                write!(f, "capture covers {packed} draw keys but {casters} casters")
            }
            Self::InputLengthMismatch {
                authored,
                material_slots,
                primary_lights,
            } => write!(
                f,
                "capture has {authored} surfaces but {material_slots} material slots \
                 and {primary_lights} primary lights"
            ),
        }
    }
}

impl std::error::Error for SurfaceMaterialStampError {}

/// A capture whose draw keys are all blank, for the given shadow casters.
pub fn world_capture_from_casters(casters: SurfaceCastsSunShadow) -> WorldCapture {
    WorldCapture {
        packed_draw_surfs: vec![GfxDrawSurf::from_packed(0); casters.len()],
        casters,
    }
}

/// Writes each surface's material and primary light into its draw key.
/// `baked_by_material_slot` gives the key bits a material contributes.
pub fn stamp_packed_surface_materials(
    surface_material_slots: &[Option<usize>],
    surface_primary_lights: &[u8],
    baked_by_material_slot: &[Option<GfxDrawSurf>],
    capture: &mut WorldCapture,
) -> Result<(), SurfaceMaterialStampError> {
    let surfaces = capture.surface_count();
    if capture.casters.len() != surfaces {
        return Err(SurfaceMaterialStampError::CaptureLengthMismatch {
            packed: surfaces,
            casters: capture.casters.len(),
        });
    }
    if surface_material_slots.len() != surfaces || surface_primary_lights.len() != surfaces {
        return Err(SurfaceMaterialStampError::InputLengthMismatch {
            authored: surfaces,
            material_slots: surface_material_slots.len(),
            primary_lights: surface_primary_lights.len(),
        });
    }
    fill_surface_materials(
        surface_material_slots,
        surface_primary_lights,
        baked_by_material_slot,
        &mut capture.packed_draw_surfs,
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_capture_has_a_blank_key_for_every_caster() {
        let capture = world_capture_from_casters(SurfaceCastsSunShadow::with_len(4));
        assert_eq!(capture.surface_count(), 4);
        assert_eq!(capture.casters.len(), 4);
    }

    #[test]
    fn stamping_refuses_inputs_of_the_wrong_length() {
        let mut capture = world_capture_from_casters(SurfaceCastsSunShadow::with_len(2));
        let err = stamp_packed_surface_materials(&[None], &[0, 0], &[], &mut capture);
        assert_eq!(
            err,
            Err(SurfaceMaterialStampError::InputLengthMismatch {
                authored: 2,
                material_slots: 1,
                primary_lights: 2,
            })
        );
    }

    #[test]
    fn stamping_refuses_a_capture_whose_halves_disagree() {
        let mut capture = world_capture_from_casters(SurfaceCastsSunShadow::with_len(2));
        capture.casters = SurfaceCastsSunShadow::with_len(3);
        let err = stamp_packed_surface_materials(&[None; 2], &[0; 2], &[], &mut capture);
        assert_eq!(
            err,
            Err(SurfaceMaterialStampError::CaptureLengthMismatch {
                packed: 2,
                casters: 3,
            })
        );
    }

    #[test]
    fn stamping_matching_inputs_succeeds() {
        let mut capture = world_capture_from_casters(SurfaceCastsSunShadow::with_len(2));
        assert_eq!(
            stamp_packed_surface_materials(&[None; 2], &[0; 2], &[], &mut capture),
            Ok(())
        );
    }
}
