//! Black Ops II light grid: the SH coefficient sets decoded as the game does,
//! and what they give a point (light, primary light visibility).

/// Bytes in one T6 SH coefficient set (`GfxCompressedLightGridCoeffs`).
pub const T6_COEFF_SET_BYTES: usize = 54;

/// One T6 SH coefficient set decoded as the game does
/// (`R_SetLightGridColorsFromIndex`): nine RGB coefficients, each u16
/// `u * (1 / 65535) * 32 - 16`.
pub fn t6_grid_coeffs(set: &[u8]) -> Option<[[f32; 3]; 9]> {
    let set = set.get(..T6_COEFF_SET_BYTES)?;
    Some(core::array::from_fn(|k| {
        core::array::from_fn(|c| {
            let i = (k * 3 + c) * 2;
            f32::from(u16::from_le_bytes([set[i], set[i + 1]])) * 1.525_902_2e-5 * 32.0 - 16.0
        })
    }))
}

/// The game's 56 light grid directions (`GenerateLightGridBasisDirs`): the
/// shell of a 4x4x4 block, z outermost then y then x, each axis
/// `i * 2/3 - 1`, normalized. Same order as the shell of a model lighting
/// tile.
pub fn t6_grid_basis_dirs() -> [[f32; 3]; 56] {
    let mut dirs = [[0.0; 3]; 56];
    let mut n = 0;
    for z in 0..4 {
        for y in 0..4 {
            for x in 0..4 {
                if [x, y, z].iter().all(|i| (1..3).contains(i)) {
                    continue;
                }
                let v = [x, y, z].map(|i| i as f32 * 0.666_666_7 - 1.0);
                let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
                dirs[n] = v.map(|c| c / len);
                n += 1;
            }
        }
    }
    dirs
}

/// The light (linear, before exposure) a decoded set gives along each grid
/// direction (`R_CalculateLightGridColorFromCoeffs`): the SH sum, never
/// below zero.
pub fn t6_grid_cube(c: &[[f32; 3]; 9]) -> [[f32; 3]; 56] {
    t6_grid_basis_dirs().map(|[x, y, z]| {
        let basis = [
            1.0,
            x,
            y,
            z,
            z * x,
            z * y,
            x * y,
            3.0 * z * z - 1.0,
            x * x - y * y,
        ];
        core::array::from_fn(|ch| {
            basis
                .iter()
                .zip(c)
                .map(|(b, k)| b * k[ch])
                .sum::<f32>()
                .max(0.0)
        })
    })
}

/// The three SH constants the game hands a model's shader for its grid
/// light along the vertex normal (the tail of `R_SetLightGridColorsFromIndex`,
/// stored through `R_EncodeLightingSH`'s 16-bit round trip): with
/// `lum(c) = c.r/4 + c.g/2 + c.b/4`, `V0 = (c0 / (lum c0 + 1e-4), 3 lum c7)`,
/// `V1 = (lum c1, lum c2, lum c3, lum c0 + 1e-4 - lum c7)`,
/// `V2 = (lum c4, lum c5, lum c6, lum c8)`.
pub fn t6_grid_sh(c: &[[f32; 3]; 9]) -> [[f32; 4]; 3] {
    let lum = |k: usize| c[k][0] * 0.25 + c[k][1] * 0.5 + c[k][2] * 0.25;
    let l0 = lum(0) + 0.0001;
    let sh = [
        [c[0][0] / l0, c[0][1] / l0, c[0][2] / l0, 3.0 * lum(7)],
        [lum(1), lum(2), lum(3), l0 - lum(7)],
        [lum(4), lum(5), lum(6), lum(8)],
    ];
    let round_trip = |v: f32| {
        let q = ((v.clamp(-16.0, 16.0) + 16.0) * 2_047.968_8 + 0.5).floor();
        q * 1.525_902_2e-5 * 32.0 - 16.0
    };
    sh.map(|v| v.map(round_trip))
}

/// A grid light (linear) as a model lighting texel byte: the game's shader
/// decodes `(v / 255)^2 * 32`; the encode is BO1's (`8 / 255`, half-even).
pub fn t6_light_byte(light: f32) -> u8 {
    ((light * (8.0 / 255.0)).clamp(0.0, 1.0).sqrt() * 255.0).round_ties_even() as u8
}

/// A T6 light grid in the engine's legacy form, so its sampler can find the
/// entries around a point and blend them. T6 grids keep SH coefficient sets
/// where legacy keeps 56 directional RGB8 colours; each set becomes the 56
/// colours the game computes from it (`t6_grid_cube`), in the grid basis
/// order, encoded with `t6_light_byte`.
pub fn t6_light_grid(grid: &asset_t6::LightGridRef) -> Option<crate::OwnedLightGrid> {
    const SET: usize = T6_COEFF_SET_BYTES;
    const DIRS: usize = 56;
    if grid.row_axis > 2 || grid.col_axis > 2 || grid.coeffs.len() < SET {
        return None;
    }
    let mut colors = Vec::with_capacity(grid.coeffs.len() / SET * DIRS * 3);
    for set in grid.coeffs.chunks_exact(SET) {
        let cube = t6_grid_cube(&t6_grid_coeffs(set)?);
        for rgb in cube {
            colors.extend_from_slice(&rgb.map(t6_light_byte));
        }
    }
    Some(crate::OwnedLightGrid {
        mins: grid.mins,
        maxs: grid.maxs,
        row_axis: grid.row_axis as usize,
        col_axis: grid.col_axis as usize,
        color_count: (grid.coeffs.len() / SET) as u32,
        has_light_regions: false,
        sun_primary_light_index: grid.sun_primary_light_index,
        row_data_start: grid.row_data_start.clone(),
        raw_row_data: grid.raw_row_data.clone(),
        entries: grid.entries.clone(),
        colors,
        color_encoding: crate::LightGridColorEncoding::Rgb8,
    })
}

/// The light (linear, before exposure) a T6 grid gives at `pos`: the mean
/// of the sampled tile, decoded `(v / 255)^2 * 32`.
pub fn t6_grid_light(grid: &crate::OwnedLightGrid, pos: [f32; 3]) -> Option<[f32; 3]> {
    let sampled = crate::sample_light_grid(&grid.view(), pos).ok()?;
    Some(t6_sampled_light(&sampled))
}

/// `t6_grid_light` for a sample already taken.
pub fn t6_sampled_light(sampled: &crate::SampledLighting) -> [f32; 3] {
    let mean = crate::mean_tile_rgba01(&sampled.tile);
    mean.map(|v| v * v * 32.0)
}

/// The visibility (0..255) a T6 grid gives primary light `primary` at
/// `pos`: each grid entry carries a primary light and that light's
/// visibility; this is their weighted mean over the corners lit by
/// `primary`, or over every live corner when none is.
pub fn t6_grid_visibility(grid: &crate::OwnedLightGrid, pos: [f32; 3], primary: u8) -> Option<u8> {
    let sampled = crate::sample_light_grid(&grid.view(), pos).ok()?;
    t6_sampled_visibility(&sampled, primary)
}

/// `t6_grid_visibility` for a sample already taken.
pub fn t6_sampled_visibility(sampled: &crate::SampledLighting, primary: u8) -> Option<u8> {
    let mean = |only: Option<u8>| {
        let (mut sum, mut weight) = (0.0f32, 0.0f32);
        for i in 0..8 {
            let (Some(p), Some(v)) = (sampled.corner_primaries[i], sampled.corner_flags[i]) else {
                continue;
            };
            if only.is_some_and(|o| o != p) {
                continue;
            }
            sum += sampled.weights[i] * f32::from(v);
            weight += sampled.weights[i];
        }
        (weight > 1e-6).then(|| (sum / weight).round().clamp(0.0, 255.0) as u8)
    };
    mean(Some(primary)).or_else(|| mean(None))
}
