//! The corner layout of a camera-facing sprite quad.

/// The quad's corners in its own plane, as offsets from its centre, in draw
/// order.
pub const FX_SPRITE_QUAD_LOCAL_XY: [[f32; 2]; 4] =
    [[-0.5, -0.5], [-0.5, 0.5], [0.5, 0.5], [0.5, -0.5]];

/// The two triangles of a quad, as 16-bit indices from its first corner.
pub const FX_SPRITE_QUAD_INDICES: [u16; 6] = [0, 1, 2, 2, 3, 0];

/// The same two triangles for a quad whose first corner is vertex `base`.
pub fn sprite_quad_indices(base: u32) -> [u32; 6] {
    FX_SPRITE_QUAD_INDICES.map(|corner| base.wrapping_add(u32::from(corner)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indices_follow_the_base_vertex() {
        assert_eq!(sprite_quad_indices(10), [10, 11, 12, 12, 13, 10]);
        assert_eq!(sprite_quad_indices(0), [0, 1, 2, 2, 3, 0]);
    }

    #[test]
    fn the_index_wraps_instead_of_overflowing() {
        assert_eq!(sprite_quad_indices(u32::MAX)[1], 0);
    }

    #[test]
    fn the_corners_go_round_a_unit_square() {
        let sum: f32 = FX_SPRITE_QUAD_LOCAL_XY.iter().map(|c| c[0] + c[1]).sum();
        assert_eq!(sum, 0.0);
        assert_eq!(FX_SPRITE_QUAD_LOCAL_XY[0], [-0.5, -0.5]);
    }
}
