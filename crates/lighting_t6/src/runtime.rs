//! bo2zm: Black Ops II lighting a moving thing samples at run time (the
//! first-person gun and arms, effects): the map's light grid, the sky grid
//! volumes outside it, and the scale the props measured (the grid's
//! coefficient scale is not stored with it). Installed once per map load.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

/// What lights a point: the grid's light there (linear, before exposure,
/// times the measured scale), the primary light most of its corners carry
/// and that light's visibility (0..1).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct T6PointLight {
    pub light: [f32; 3],
    pub primary: u8,
    pub seen_share: f32,
}

/// Whether the segment between two points is clear for a grid sight check
/// (the map's collision, light grid content mask).
pub type SightCheck = Arc<dyn Fn([f32; 3], [f32; 3]) -> bool + Send + Sync>;

pub struct T6LightSampler {
    pub grid: Option<crate::DetachedLightGrid>,
    /// Grid bounds in cells (x, y at 32 units, z at 64), for the in-grid
    /// test the props use.
    pub low: [u16; 3],
    pub high: [u16; 3],
    /// Sky volumes: bounds, base light, primary light and visibility.
    pub sky: Vec<([f32; 3], [f32; 3], [f32; 3], u8, u8)>,
    pub scale: f32,
    pub sunlight_primary_flag_index: u8,
    /// bo2zm M3 fix list 1: the map's collision, for the grid's sight
    /// checks: a grid point marked "needs trace" counts only when it can be
    /// seen from the lit point (BO2's R_LightGridLookup); without them a
    /// zombie near the ground took the light of the points under it (black).
    pub sight: Option<SightCheck>,
    /// Sight-checked lights by 8-unit cell (the traces cost; a zombie's
    /// body and head, the gun and effects read the same cells frame after
    /// frame).
    pub cache: Mutex<HashMap<[i32; 3], T6PointLight>>,
}

fn slot() -> &'static RwLock<Option<Arc<T6LightSampler>>> {
    static SLOT: RwLock<Option<Arc<T6LightSampler>>> = RwLock::new(None);
    &SLOT
}

pub fn install_t6_light_sampler(sampler: Option<T6LightSampler>) {
    if let Ok(mut s) = slot().write() {
        *s = sampler.map(Arc::new);
    }
}

pub fn t6_light_sampler() -> Option<Arc<T6LightSampler>> {
    slot().read().ok()?.clone()
}

impl T6LightSampler {
    /// Where the grid has no sample and no sky volume holds the point:
    /// Black Ops' engine lights it with one of the grid's colour sets
    /// (`index`: 1, or 0 under the grid's columns) in full view of the sun
    /// (R_ExtrapolateLightingAtPoint, from the lookup's default entry).
    /// Before, such a point got no light at all: BO2's animated vines high on
    /// Castaway's cliff and the seagulls over its sea drew black.
    fn predicted_ahead(&self, index: u32) -> T6PointLight {
        let Some(row) = self
            .grid
            .as_ref()
            .and_then(|g| g.view().colour_samples_at(index).map(|r| [r[0], r[1], r[2]]))
        else {
            return T6PointLight::default();
        };
        T6PointLight {
            light: row.map(|b| {
                let v = f32::from(b) / 255.0;
                v * v * 32.0 * self.scale
            }),
            primary: self.sunlight_primary_flag_index,
            seen_share: 1.0,
        }
    }

    fn in_grid(&self, p: [f32; 3]) -> bool {
        let cell = [
            (p[0].floor() as i32).wrapping_add(0x20000) >> 5,
            (p[1].floor() as i32).wrapping_add(0x20000) >> 5,
            (p[2].floor() as i32).wrapping_add(0x20000) >> 6,
        ];
        (0..3).all(|a| cell[a] >= i32::from(self.low[a]) && cell[a] <= i32::from(self.high[a]))
    }

    /// The light at `pos` (with sight checks: once per 8-unit cell, read at
    /// its centre).
    pub fn at(&self, pos: [f32; 3]) -> T6PointLight {
        if self.sight.is_none() {
            return self.light_at(pos);
        }
        let key = pos.map(|v| (v / 8.0).floor() as i32);
        if let Ok(cache) = self.cache.lock()
            && let Some(hit) = cache.get(&key)
        {
            return *hit;
        }
        let centre = key.map(|k| k as f32 * 8.0 + 4.0);
        let lit = self.light_at(centre);
        if let Ok(mut cache) = self.cache.lock() {
            if cache.len() > 16_384 {
                cache.clear();
            }
            cache.insert(key, lit);
        }
        lit
    }

    fn light_at(&self, where_at: [f32; 3]) -> T6PointLight {
        if !self.in_grid(where_at) {
            if let Some(v) = self
                .sky
                .iter()
                .find(|v| (0..3).all(|a| where_at[a] >= v.0[a] && where_at[a] <= v.1[a]))
            {
                // (A volume's colour set is in the grid's units: the
                // props outside the grid take it times the same scale.)
                return T6PointLight {
                    light: v.2.map(|c| c * self.scale),
                    primary: v.3,
                    seen_share: f32::from(v.4) / 255.0,
                };
            }
            return self.predicted_ahead(1);
        }
        let Some(grid) = self.grid.as_ref() else {
            return T6PointLight::default();
        };
        let sampled = match self.sight.as_deref() {
            Some(clear) => crate::grid_sample_with_line_of_sight(&grid.view(), where_at, Some(&clear)),
            None => crate::probe_light_cells(&grid.view(), where_at),
        }
        .ok();
        // No corner of the grid answers here (above or beside its columns):
        // the colour set the lookup fell back to, in full view of the sun
        // (BO's R_ExtrapolateLightingAtPoint).
        if let Some(s) = sampled
            .as_ref()
            .filter(|s| s.path == crate::GridProbeFallbackBranch::UnassignedUseSlot)
        {
            return T6PointLight {
                light: crate::t6_sampled_light(s).map(|c| c * self.scale),
                primary: self.sunlight_primary_flag_index,
                seen_share: 1.0,
            };
        }
        let light = sampled
            .as_ref()
            .map(|s| crate::t6_sampled_light(s).map(|c| c * self.scale))
            .unwrap_or_default();
        // The primary light most of the surrounding entries carry.
        let primary = sampled
            .as_ref()
            .and_then(|s| {
                // Each light's summed weight, the lights in the order the
                // corners first name them; a tie keeps the earlier light.
                let mut order: Vec<u8> = Vec::new();
                let mut share = [0.0f32; 256];
                for (light, w) in s.corner_dominant_lights.iter().zip(s.mix) {
                    if let Some(p) = *light {
                        if !order.contains(&p) {
                            order.push(p);
                        }
                        share[usize::from(p)] += w;
                    }
                }
                order.into_iter().reduce(|a, b| if share[usize::from(b)] > share[usize::from(a)] { b } else { a })
            })
            .unwrap_or(self.sunlight_primary_flag_index);
        let visibility = sampled
            .as_ref()
            .and_then(|s| crate::t6_sampled_visibility(s, primary))
            .map_or(0.0, |v| f32::from(v) / 255.0);
        T6PointLight {
            light,
            primary,
            seen_share: visibility,
        }
    }
}

/// bo2zm fix list 2: the map's lightmaps on the CPU, for what is lit like
/// the world but drawn later: a decal on a wall (BO2's `wc_` mark
/// materials) takes the wall's own lightmap, not the light grid in front of
/// it. Each image holds three pages stacked (base light, directional light,
/// direction + the primary light's baked visibility), as the world shader
/// reads them.
pub struct T6LightmapPage {
    pub width: u32,
    pub height: u32,
    /// RGBA8, `width * height * 4` bytes.
    pub rgba: Vec<u8>,
}

fn lightmap_slot() -> &'static RwLock<Vec<Option<Arc<T6LightmapPage>>>> {
    static SLOT: RwLock<Vec<Option<Arc<T6LightmapPage>>>> = RwLock::new(Vec::new());
    &SLOT
}

static LIGHTMAP_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn install_t6_lightmaps(pages: Vec<Option<T6LightmapPage>>) {
    if let Ok(mut s) = lightmap_slot().write() {
        *s = pages.into_iter().map(|p| p.map(Arc::new)).collect();
    }
    LIGHTMAP_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// bo2mp: bumped whenever the lightmaps are replaced, so a cache of their
/// light knows to start over.
pub fn t6_lightmap_generation() -> u64 {
    LIGHTMAP_GENERATION.load(std::sync::atomic::Ordering::Relaxed)
}

impl T6LightmapPage {
    /// One texel channel set, filtered as the GPU's linear sampler with
    /// clamped edges does.
    fn bilinear(&self, u: f32, val: f32) -> [f32; 4] {
        let (w, h) = (self.width as i64, self.height as i64);
        let x = u * self.width as f32 - 0.5;
        let y = val * self.height as f32 - 0.5;
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let texel = |xi: i64, yi: i64| -> [f32; 4] {
            let xi = xi.clamp(0, w - 1) as usize;
            let yi = yi.clamp(0, h - 1) as usize;
            let place = (yi * self.width as usize + xi) * 4;
            self.rgba
                .get(place..place + 4)
                .map_or([0.0; 4], |p| [0, 1, 2, 3].map(|c| f32::from(p[c]) / 255.0))
        };
        let (xi, yi) = (x0 as i64, y0 as i64);
        let (a, b, c, d) = (
            texel(xi, yi),
            texel(xi + 1, yi),
            texel(xi, yi + 1),
            texel(xi + 1, yi + 1),
        );
        [0, 1, 2, 3].map(|k| {
            let top = a[k] + (b[k] - a[k]) * fx;
            let bottom = c[k] + (d[k] - c[k]) * fx;
            top + (bottom - top) * fy
        })
    }
}

/// The world shader's lightmap light at lightmap `index`, coordinate `uv`
/// (one page's u, v) for normal `n`: base + directional * saturate(N.dir),
/// and the baked visibility of the surface's primary light. `None` when the
/// map has no such lightmap.
pub fn t6_lightmap_light(light_no: u8, uv: [f32; 2], num: [f32; 3]) -> Option<([f32; 3], f32)> {
    let (flat_light, aimed_light, ndl, seen) = t6_lightmap_parts(light_no, uv, num)?;
    let light = [0, 1, 2].map(|c| flat_light[c] + aimed_light[c] * ndl);
    Some((light, seen))
}

/// The lightmap's parts at a point: base light, directional light, the
/// directional term's N.dir, and the primary light's baked visibility.
pub fn t6_lightmap_parts(
    index: u8,
    uv: [f32; 2],
    n: [f32; 3],
) -> Option<([f32; 3], [f32; 3], f32, f32)> {
    let pages = lightmap_slot().read().ok()?;
    let page = pages.get(usize::from(index))?.as_ref()?;
    let v = uv[1] / 3.0;
    let p0 = page.bilinear(uv[0], v);
    let p1 = page.bilinear(uv[0], v + 1.0 / 3.0);
    let p2 = page.bilinear(uv[0], v + 2.0 / 3.0);
    let dir = [p2[0] * 2.0 - 1.0, p2[1] * 2.0 - 1.0, p2[2] * 2.0 - 1.0];
    let ndl = (dir[0] * n[0] + dir[1] * n[1] + dir[2] * n[2]).clamp(0.0, 1.0);
    let flat_light = [0, 1, 2].map(|c| p0[c] / (p0[3] + 0.000001));
    let aimed_light = [0, 1, 2].map(|c| p1[c] / (p1[3] + 0.000001));
    Some((flat_light, aimed_light, ndl, p2[3]))
}

/// bo2zm: the world's reflection probe origins, for what moves (the
/// first-person gun reflects the probe nearest the eye).
fn probe_slot() -> &'static std::sync::RwLock<Vec<[f32; 3]>> {
    static SLOT: std::sync::OnceLock<std::sync::RwLock<Vec<[f32; 3]>>> = std::sync::OnceLock::new();
    SLOT.get_or_init(Default::default)
}

pub fn install_t6_probe_origins(origins: Vec<[f32; 3]>) {
    if let Ok(mut s) = probe_slot().write() {
        *s = origins;
    }
}

/// The reflection probe nearest `p` (probe 0 is a stand-in and never picked
/// while another exists).
pub fn t6_nearest_probe(p: [f32; 3]) -> Option<u32> {
    let origins = probe_slot().read().ok()?;
    origins
        .iter()
        .enumerate()
        .skip(usize::from(origins.len() > 1))
        .min_by(|(_, a), (_, b)| {
            let d = |o: &[f32; 3]| (0..3).map(|i| (o[i] - p[i]) * (o[i] - p[i])).sum::<f32>();
            d(a).total_cmp(&d(b))
        })
        .map(|(i, _)| i as u32)
}

/// bo2mp: one placed model's light as Black Ops II hands it to the model's
/// shader: the 56 directional colours its own light grid coefficient set
/// gives (`colorsIndex`; `t6_grid_cube`, bytes as `t6_light_byte`), which
/// the game writes to the model's 4x4x4 block of the model lighting
/// texture, and the three SH constants of the same set (`t6_grid_sh`).
#[derive(Clone, Copy, Debug)]
pub struct T6ModelLight {
    pub cube: [[u8; 3]; 56],
    pub sh: [[f32; 4]; 3],
}

/// Model lights in the model lighting texture's layout: 128 blocks of
/// 4x4x4 texels to a row of 512, handle `h` (from 1) at block `h - 1`;
/// within a block the 56 colours fill the shell in the grid's direction
/// order (z, then y, then x) and the 8 inner texels take their mean.
/// Returns width, height, depth and RGBA8 texels (alpha 255).
pub fn t6_model_light_texels(lights: &[T6ModelLight]) -> (u32, u32, u32, Vec<u8>) {
    const W: usize = 512;
    let rows = lights.len().div_ceil(128).max(1);
    let h = rows * 4;
    let mut texels = vec![0u8; W * h * 4 * 4];
    for (i, light) in lights.iter().enumerate() {
        let (bx, by) = ((i & 127) * 4, (i >> 7) * 4);
        let mut mean = [0u32; 3];
        for rgb in &light.cube {
            for c in 0..3 {
                mean[c] += u32::from(rgb[c]);
            }
        }
        let inner = mean.map(|m| ((m + 28) / 56) as u8);
        let mut n = 0;
        for z in 0..4 {
            for y in 0..4 {
                for x in 0..4 {
                    let rgb = if [x, y, z].iter().all(|i| (1..3).contains(i)) {
                        inner
                    } else {
                        n += 1;
                        light.cube[n - 1]
                    };
                    let at = ((z * h + by + y) * W + bx + x) * 4;
                    texels[at..at + 4].copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
                }
            }
        }
    }
    (W as u32, h as u32, 4, texels)
}

fn model_light_slot() -> &'static RwLock<Arc<Vec<T6ModelLight>>> {
    static SLOT: std::sync::OnceLock<RwLock<Arc<Vec<T6ModelLight>>>> = std::sync::OnceLock::new();
    SLOT.get_or_init(Default::default)
}

static MODEL_LIGHT_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Installed once per map load; a placement's `t6_light` is its index here
/// plus one.
pub fn install_t6_model_lights(lights: Vec<T6ModelLight>) {
    if let Ok(mut s) = model_light_slot().write() {
        *s = Arc::new(lights);
    }
    MODEL_LIGHT_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// The model lights and their generation (bumped on every install).
pub fn t6_model_lights() -> (u64, Arc<Vec<T6ModelLight>>) {
    // The generation first: an install in between is then seen next time.
    let generation = MODEL_LIGHT_GENERATION.load(std::sync::atomic::Ordering::Relaxed);
    let lights = model_light_slot()
        .read()
        .map(|s| s.clone())
        .unwrap_or_default();
    (generation, lights)
}
