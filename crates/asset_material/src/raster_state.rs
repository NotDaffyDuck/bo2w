// Cull and alpha-test fields of a material's state bits (word 0).

pub use d3d9_state::{
    AlphaGate, COMPARE_PASS_IF_ABOVE, COMPARE_PASS_IF_ABOVE_OR_SAME, DEPTH_COMPARE_STRICT_BELOW, WINDING_CULL_COUNTERCLOCKWISE, FACE_CULL_CLOCKWISE,
    FACE_CULL_DISABLED, STATE_FACE_CULL,
};

pub const CULL_FIELD_SHIFT: u32 = 14;

pub const CULL_FIELD_MASK: u32 = 0b11 << 14;

pub const CULL_FIELD_OFF: u32 = 1 << 14;

pub const RASTER_DROP_REAR_FACES: u32 = 2 << 14;

pub const CULL_FIELD_FRONT: u32 = 3 << 14;

pub const CULL_MODE_FOR_FIELD: [u32; 4] = [
    0,
    FACE_CULL_DISABLED,
    WINDING_CULL_COUNTERCLOCKWISE,
    FACE_CULL_CLOCKWISE,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaceCullingMode {
    None,
    Back,
    Front,
}

impl FaceCullingMode {
    pub const fn read_from_member(field: u32) -> Self {
        let low_pair = field % 4;
        if low_pair == 2 {
            Self::Back
        } else if low_pair == 3 {
            Self::Front
        } else {
            Self::None
        }
    }

    pub const fn lookup_slot(self) -> usize {
        1 + self as usize
    }
}

/// Black Ops II keeps the alpha test in two single bits of word 0: bit 11 switches the test
/// off, bit 12 picks the comparison (clear: alpha above zero, set: alpha at least 128).
pub const CUTOFF_OFF_BIT: u32 = 1 << 11;

pub const CUTOFF_FIELD_SHIFT: u32 = 12;

pub const CUTOFF_FIELD_MASK: u32 = 1 << 12;

pub const CUTOFF_ABOVE_ZERO: u32 = 0;

pub const CUTOFF_HALF_OR_MORE: u32 = 1 << 12;

pub const S_ALPHA_TEST_REF_128: u8 = 128;

/// Indexed by `TransparencyCutoffMode::lookup_slot`: the compare function and reference byte.
pub const CUTOFF_FOR_FIELD: [(u32, u8); 2] = [(COMPARE_PASS_IF_ABOVE, 0), (COMPARE_PASS_IF_ABOVE_OR_SAME, S_ALPHA_TEST_REF_128)];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TransparencyCutoffMode {
    AboveZero,

    AtLeastHalfRange,
}

impl TransparencyCutoffMode {
    /// Reads the comparison bit; only its lowest bit counts.
    pub const fn read_from_member(field: u32) -> Self {
        match field % 2 {
            1 => Self::AtLeastHalfRange,
            _ => Self::AboveZero,
        }
    }

    pub const fn lookup_slot(self) -> usize {
        self as usize
    }
}

/// `None` when the material switches its alpha test off.
pub const fn cutout_flag_from_raster_bits(loading_flag_mask: [u32; 2]) -> Option<TransparencyCutoffMode> {
    let [dword0, _] = loading_flag_mask;
    match dword0 & CUTOFF_OFF_BIT {
        0 => Some(TransparencyCutoffMode::read_from_member(dword0 >> CUTOFF_FIELD_SHIFT)),
        _ => None,
    }
}

pub const fn face_culling_of(loading_flag_mask: [u32; 2]) -> FaceCullingMode {
    let [dword0, _] = loading_flag_mask;
    FaceCullingMode::read_from_member(dword0 >> CULL_FIELD_SHIFT)
}

pub const fn device_cull_mode_of(loading_flag_mask: [u32; 2]) -> u32 {
    let cull_field = (loading_flag_mask[0] >> CULL_FIELD_SHIFT) % 4;
    CULL_MODE_FOR_FIELD[cull_field as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_alpha_test_reads_its_two_bits() {
        let test_of = |dword0: u32| cutout_flag_from_raster_bits([dword0, 0]);
        assert_eq!(test_of(0), Some(TransparencyCutoffMode::AboveZero));
        assert_eq!(test_of(CUTOFF_HALF_OR_MORE), Some(TransparencyCutoffMode::AtLeastHalfRange));
        assert_eq!(test_of(CUTOFF_OFF_BIT), None);
        assert_eq!(test_of(CUTOFF_OFF_BIT | CUTOFF_HALF_OR_MORE), None);
    }

    #[test]
    fn the_cull_field_picks_face_and_d3d_mode() {
        let faces = [
            (0u32, FaceCullingMode::None, 0),
            (CULL_FIELD_OFF, FaceCullingMode::None, FACE_CULL_DISABLED),
            (RASTER_DROP_REAR_FACES, FaceCullingMode::Back, WINDING_CULL_COUNTERCLOCKWISE),
            (CULL_FIELD_FRONT, FaceCullingMode::Front, FACE_CULL_CLOCKWISE),
        ];
        for (dword0, face, d3d) in faces {
            assert_eq!(face_culling_of([dword0, 0]), face);
            assert_eq!(device_cull_mode_of([dword0, 0]), d3d);
        }
    }
}
