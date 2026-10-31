//! bo2mp: Black Ops II's final colour, from the map's own vision file
//! (`vision/<map>.vision`) and colour table (`GfxWorld::lutMaterial`).
//!
//! The game builds a 32x32x32 colour table each frame with
//! `hdr_create_lut2dv` (fxc disassembly of its pixel shader) and looks the
//! frame up in it after bloom and the highlight roll-off (`hdr_bloom_apply`):
//!
//! - start from the map's own table (band 0 of the lut material's image,
//!   gamma values; the band picked when no LUT volume holds the eye),
//! - weights by its luminance (Rec. 709, on the gamma values): shadows a
//!   falling ramp `vc_RS.x..vc_RE.x`, highlights a rising ramp `y`,
//!   midtones a rising ramp `z` times a falling ramp `w`; each smoothstep,
//!   the three normalised,
//! - in linear (squared): the shadow, midtone and highlight joint_transforms
//!   (`vc_SM*`, `vc_MM*`, `vc_HM*`, rows with an offset) mixed by those
//!   weights, clamped; then per channel the power `vc_FGM`, the saturation
//!   `vc_FSM` (weights, amount), the blend back toward the input `vc_FBM`,
//!   then the code's colour matrix (identity: the vision's levels are),
//!   clamped and square-rooted.
//!
//! The vision's bloom rows (`vc_RGBH`, `vc_RGBL`, `vc_YH`, `vc_YL`) are the
//! tints `hdr_bloom_combine_hilo` puts the high and low bloom levels'
//! colour and luminance back with.

use std::sync::{Arc, RwLock};

/// The `vc_*` rows of a Black Ops II vision file (four floats each).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct T6Vision {
    pub film_enable: bool,
    pub levels_in_black: [f32; 4],
    pub levels_in_white: [f32; 4],
    pub levels_in_gamma: [f32; 4],
    pub levels_out_black: [f32; 4],
    pub levels_out_white: [f32; 4],
    pub bloom_rgb_hi: [f32; 4],
    pub bloom_rgb_lo: [f32; 4],
    pub bloom_y_hi: [f32; 4],
    pub bloom_y_lo: [f32; 4],
    pub range_start: [f32; 4],
    pub range_end: [f32; 4],
    pub shadow: [[f32; 4]; 3],
    pub highlight: [[f32; 4]; 3],
    pub midtone: [[f32; 4]; 3],
    pub final_gamma: [f32; 4],
    pub final_saturation: [f32; 4],
    pub final_blend: [f32; 4],
}

impl Default for T6Vision {
    /// What changes nothing: identity levels and joint_transforms, full saturation.
    fn default() -> Self {
        let row = |i: usize| {
            let mut r = [0.0; 4];
            r[i] = 1.0;
            r
        };
        let identity = [row(0), row(1), row(2)];
        Self {
            film_enable: false,
            levels_in_black: [0.0; 4],
            levels_in_white: [32.0; 4],
            levels_in_gamma: [1.0; 4],
            levels_out_black: [0.0; 4],
            levels_out_white: [32.0; 4],
            bloom_rgb_hi: [0.0; 4],
            bloom_rgb_lo: [0.0; 4],
            bloom_y_hi: [0.0; 4],
            bloom_y_lo: [0.0; 4],
            range_start: [0.0, 0.0, 0.0, 0.5],
            range_end: [1.0, 1.0, 0.5, 1.0],
            shadow: identity,
            highlight: identity,
            midtone: identity,
            final_gamma: [1.0, 1.0, 1.0, 0.0],
            final_saturation: [T6_LUMA[0], T6_LUMA[1], T6_LUMA[2], 1.0],
            final_blend: [1.0, 1.0, 1.0, 0.0],
        }
    }
}

/// The luminance weights BO2's post shaders use.
pub const T6_LUMA: [f32; 3] = [0.212585, 0.715195, 0.07222];

/// Parse a vision file's text: `name "values"` per line. Rows it does not
/// name keep their identity value; `None` when it names no `vc_` row.
pub fn parse_t6_vision(text: &str) -> Option<T6Vision> {
    let mut v = T6Vision::default();
    let mut rows_set = 0usize;
    for (name, value) in text.lines().filter_map(vision_row) {
        if name == "r_filmenable" {
            v.film_enable = value.trim() != "0";
            continue;
        }
        if let (Some(four), Some(slot)) = (leading_four(value), vision_slot(&mut v, &name)) {
            *slot = four;
            rows_set += 1;
        }
    }
    (rows_set > 0).then_some(v)
}

/// One `name "values"` line as (lowercased name, unquoted value text).
fn vision_row(line: &str) -> Option<(String, &str)> {
    let (name, rest) = line.trim().split_once(char::is_whitespace)?;
    Some((name.to_ascii_lowercase(), rest.trim().trim_matches('"')))
}

/// The first four numbers of `value`; `None` unless all four parse.
fn leading_four(value: &str) -> Option<[f32; 4]> {
    let mut parts = value.split_whitespace().map(|p| p.parse::<f32>().ok());
    Some([parts.next()??, parts.next()??, parts.next()??, parts.next()??])
}

/// The row of `v` a `vc_` name stands for.
fn vision_slot<'a>(v: &'a mut T6Vision, name: &str) -> Option<&'a mut [f32; 4]> {
    Some(match name {
        "vc_lib" => &mut v.levels_in_black,
        "vc_liw" => &mut v.levels_in_white,
        "vc_lig" => &mut v.levels_in_gamma,
        "vc_lob" => &mut v.levels_out_black,
        "vc_low" => &mut v.levels_out_white,
        "vc_rgbh" => &mut v.bloom_rgb_hi,
        "vc_rgbl" => &mut v.bloom_rgb_lo,
        "vc_yh" => &mut v.bloom_y_hi,
        "vc_yl" => &mut v.bloom_y_lo,
        "vc_rs" => &mut v.range_start,
        "vc_re" => &mut v.range_end,
        "vc_smr" => &mut v.shadow[0],
        "vc_smg" => &mut v.shadow[1],
        "vc_smb" => &mut v.shadow[2],
        "vc_hmr" => &mut v.highlight[0],
        "vc_hmg" => &mut v.highlight[1],
        "vc_hmb" => &mut v.highlight[2],
        "vc_mmr" => &mut v.midtone[0],
        "vc_mmg" => &mut v.midtone[1],
        "vc_mmb" => &mut v.midtone[2],
        "vc_fgm" => &mut v.final_gamma,
        "vc_fsm" => &mut v.final_saturation,
        "vc_fbm" => &mut v.final_blend,
        _ => return None,
    })
}

/// Side of the colour table.
pub const T6_LUT_SIZE: usize = 32;

/// One colour (gamma values, the table's input) through `hdr_create_lut2dv`
/// with `alpha` the base table's alpha. Returns gamma values.
pub fn t6_grade_colour(v: &T6Vision, c: [f32; 3], alpha: f32) -> [f32; 3] {
    let dot3 = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let l = dot3(c, T6_LUMA);
    // The code passes each ramp as scale and bias (mad_sat): rising from
    // start to end, or falling.
    let ramp = |i: usize, rising: bool| {
        let (s, e) = (v.range_start[i], v.range_end[i]);
        let t = if (e - s).abs() < 1e-6 {
            if l >= s { 1.0 } else { 0.0 }
        } else {
            ((l - s) / (e - s)).clamp(0.0, 1.0)
        };
        if rising { t } else { 1.0 - t }
    };
    let smooth = |x: f32| x * x * (3.0 - 2.0 * x);
    let mut w = [
        smooth(ramp(0, false)),
        smooth(ramp(2, true) * ramp(3, false)),
        smooth(ramp(1, true)),
    ];
    let sum = w[0] + w[1] + w[2];
    if sum > 0.0 {
        w = w.map(|x| x / sum);
    }
    let lin = c.map(|x| x * x);
    let row = |r: [f32; 4]| r[0] * lin[0] + r[1] * lin[1] + r[2] * lin[2] + r[3] * alpha;
    let mut out: [f32; 3] = core::array::from_fn(|ch| {
        let graded =
            w[0] * row(v.shadow[ch]) + w[1] * row(v.midtone[ch]) + w[2] * row(v.highlight[ch]);
        graded.clamp(0.0, 1.0).powf(v.final_gamma[ch])
    });
    let fs = v.final_saturation;
    let lum = dot3(out, [fs[0], fs[1], fs[2]]);
    for (ch, o) in out.iter_mut().enumerate() {
        let saturated = lum + fs[3] * (*o - lum);
        let blended = lin[ch] + v.final_blend[ch] * (saturated - lin[ch]);
        *o = blended.clamp(0.0, 1.0).sqrt();
    }
    out
}

/// The 32x32x32 table, RGBA8, red fastest then green then blue (a 3D
/// texture's layout). `base` is the map's own table: band 0 of its lut
/// image (1024x32 RGBA8, texel (b * 32 + r, g)); `None` = identity.
pub fn t6_grade_lut(v: &T6Vision, base: Option<&[u8]>) -> Vec<u8> {
    let n = T6_LUT_SIZE;
    let base = base.filter(|b| b.len() >= n * n * n * 4);
    let mut out = Vec::with_capacity(n * n * n * 4);
    for b in 0..n {
        for g in 0..n {
            for r in 0..n {
                let (c, a) = match base {
                    Some(px) => {
                        let o = (g * n * n + b * n + r) * 4;
                        (
                            [0, 1, 2].map(|i| f32::from(px[o + i]) / 255.0),
                            f32::from(px[o + 3]) / 255.0,
                        )
                    }
                    None => ([r, g, b].map(|x| x as f32 / (n - 1) as f32), 1.0),
                };
                let graded = t6_grade_colour(v, c, a);
                for x in graded {
                    out.push((x * 255.0).round().clamp(0.0, 255.0) as u8);
                }
                out.push(255);
            }
        }
    }
    out
}

/// What the renderer's final pass draws a Black Ops II map with.
#[derive(Clone, Debug)]
pub struct T6Grade {
    /// `t6_grade_lut`'s table.
    pub lut: Vec<u8>,
    pub vision: T6Vision,
}

fn grade_slot() -> &'static RwLock<(u64, Option<Arc<T6Grade>>)> {
    static SLOT: std::sync::OnceLock<RwLock<(u64, Option<Arc<T6Grade>>)>> =
        std::sync::OnceLock::new();
    SLOT.get_or_init(Default::default)
}

/// Installed once per Black Ops II map load (`None`: the map has no vision).
pub fn install_t6_grade(grade: Option<T6Grade>) {
    let grade = grade.map(Arc::new);
    if let Ok(mut n) = naked_slot().write() {
        *n = (grade.clone(), None);
    }
    put_grade(grade);
}

fn put_grade(grade: Option<Arc<T6Grade>>) {
    if let Ok(mut s) = grade_slot().write() {
        s.0 += 1;
        s.1 = grade;
    }
}

/// bo2mp vehicle screens: the naked vision's grade (the map's own, or a
/// script's `visionsetnaked`) and the server's vision for the local player
/// over it (`setvisionsetforplayer` while `useservervisionset` is on).
fn naked_slot() -> &'static RwLock<(Option<Arc<T6Grade>>, Option<String>)> {
    static SLOT: std::sync::OnceLock<RwLock<(Option<Arc<T6Grade>>, Option<String>)>> = std::sync::OnceLock::new();
    SLOT.get_or_init(Default::default)
}

fn named_grade(name: &str) -> Option<Arc<T6Grade>> {
    let (vision, base) = vision_registry()
        .read()
        .ok()
        .and_then(|r| r.0.get(&name.to_ascii_lowercase()).map(|v| (*v, r.1.clone())))?;
    Some(Arc::new(T6Grade { lut: t6_grade_lut(&vision, base.as_deref()), vision }))
}

/// bo2mp vehicle screens: the local player's server vision ("" back to the
/// naked one). False when no zone holds the name (the naked one stays).
pub fn set_t6_player_vision(name: &str) -> bool {
    let name = name.trim();
    let over = if name.is_empty() { None } else { named_grade(name) };
    let found = name.is_empty() || over.is_some();
    let naked = naked_slot().write().ok().map(|mut n| {
        n.1 = over.is_some().then(|| name.to_ascii_lowercase());
        n.0.clone()
    });
    put_grade(over.or(naked.flatten()));
    found
}

/// bo2mp: every named vision the zones hold (`vision/<name>.vision`, lower
/// case) and the map's colour table band, so a script's `visionsetnaked`
/// (`mpOutro` at a match's end, back to the map's own after) can switch the
/// frame's grade.
fn vision_registry() -> &'static RwLock<(std::collections::HashMap<String, T6Vision>, Option<Vec<u8>>)> {
    static REG: std::sync::OnceLock<
        RwLock<(std::collections::HashMap<String, T6Vision>, Option<Vec<u8>>)>,
    > = std::sync::OnceLock::new();
    REG.get_or_init(Default::default)
}

/// Add named visions (a later one of a name wins).
pub fn register_t6_visions(visions: Vec<(String, T6Vision)>) {
    if let Ok(mut r) = vision_registry().write() {
        for (name, v) in visions {
            r.0.insert(name.to_ascii_lowercase(), v);
        }
    }
}

/// The installed map's colour table band 0 (what every vision's table is
/// built over), `None` when the map has none.
pub fn set_t6_vision_base(base: Option<Vec<u8>>) {
    if let Ok(mut r) = vision_registry().write() {
        r.1 = base;
    }
}

/// `visionsetnaked <name>`: install that vision's grade. False when no
/// zone holds the name (the grade stays).
pub fn set_t6_vision(name: &str) -> bool {
    let Some(grade) = named_grade(name) else {
        return false;
    };
    // (A server vision for the local player stays over it until cleared.)
    let over = naked_slot().write().ok().is_some_and(|mut n| {
        n.0 = Some(grade.clone());
        n.1.is_some()
    });
    if !over {
        put_grade(Some(grade));
    }
    true
}

/// bo2mp vision fades: the vision a zone holds under `name`.
pub fn named_t6_vision(name: &str) -> Option<T6Vision> {
    vision_registry().read().ok()?.0.get(&name.to_ascii_lowercase()).copied()
}

/// bo2mp vision fades: the naked vision now (where a fade starts).
pub fn t6_naked_vision() -> Option<T6Vision> {
    naked_slot().read().ok()?.0.as_ref().map(|g| g.vision)
}

/// bo2mp vision fades: one step of `visionsetnaked( name, time )`, the
/// vision `a` part of the way `t` (0..1) to `b`, each row mixed.
pub fn lerp_t6_vision(a: &T6Vision, b: &T6Vision, t: f32) -> T6Vision {
    let t = t.clamp(0.0, 1.0);
    let row = |x: [f32; 4], y: [f32; 4]| -> [f32; 4] { std::array::from_fn(|i| x[i] + (y[i] - x[i]) * t) };
    let mat = |x: [[f32; 4]; 3], y: [[f32; 4]; 3]| -> [[f32; 4]; 3] { std::array::from_fn(|i| row(x[i], y[i])) };
    T6Vision {
        film_enable: a.film_enable || b.film_enable,
        levels_in_black: row(a.levels_in_black, b.levels_in_black),
        levels_in_white: row(a.levels_in_white, b.levels_in_white),
        levels_in_gamma: row(a.levels_in_gamma, b.levels_in_gamma),
        levels_out_black: row(a.levels_out_black, b.levels_out_black),
        levels_out_white: row(a.levels_out_white, b.levels_out_white),
        bloom_rgb_hi: row(a.bloom_rgb_hi, b.bloom_rgb_hi),
        bloom_rgb_lo: row(a.bloom_rgb_lo, b.bloom_rgb_lo),
        bloom_y_hi: row(a.bloom_y_hi, b.bloom_y_hi),
        bloom_y_lo: row(a.bloom_y_lo, b.bloom_y_lo),
        range_start: row(a.range_start, b.range_start),
        range_end: row(a.range_end, b.range_end),
        shadow: mat(a.shadow, b.shadow),
        highlight: mat(a.highlight, b.highlight),
        midtone: mat(a.midtone, b.midtone),
        final_gamma: row(a.final_gamma, b.final_gamma),
        final_saturation: row(a.final_saturation, b.final_saturation),
        final_blend: row(a.final_blend, b.final_blend),
    }
}

/// bo2mp vision fades: install vision `v` (a step of a fade) as the naked
/// one, as `set_t6_vision` does a named one. The grade's generation after
/// it (unchanged while a server vision for the local player is over it).
pub fn set_t6_vision_values(v: T6Vision) -> u64 {
    let base = vision_registry().read().ok().and_then(|r| r.1.clone());
    let grade = Arc::new(T6Grade { lut: t6_grade_lut(&v, base.as_deref()), vision: v });
    let over = naked_slot().write().ok().is_some_and(|mut n| {
        n.0 = Some(grade.clone());
        n.1.is_some()
    });
    if !over {
        put_grade(Some(grade));
    }
    t6_grade().0
}

/// The installed grade and its generation (bumped by every install).
pub fn t6_grade() -> (u64, Option<Arc<T6Grade>>) {
    grade_slot()
        .read()
        .map(|s| (s.0, s.1.clone()))
        .unwrap_or((0, None))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fade_runs_from_one_vision_to_the_other() {
        let a = T6Vision::default();
        let b = T6Vision { levels_in_white: [16.0; 4], film_enable: true, ..T6Vision::default() };
        assert_eq!(lerp_t6_vision(&a, &b, 0.0).levels_in_white, a.levels_in_white);
        assert_eq!(lerp_t6_vision(&a, &b, 1.0), b);
        assert_eq!(lerp_t6_vision(&a, &b, 0.5).levels_in_white, [24.0; 4]);
        assert_eq!(lerp_t6_vision(&a, &b, 2.0), b);
    }

    const NUKETOWN_MP: &str = "r_filmEnable       \"1\"\n\
vc_LIB \"0.000000 0.000000 0.000000 0.000000\"\n\
vc_RGBH \"0.090000 0.090000 0.090000 0.650000\"\n\
vc_RS \"0.000000 0.000000 0.000000 0.500000\"\n\
vc_RE \"1.000000 1.000000 0.500000 1.000000\"\n\
vc_SMR \"1.164402 0.000000 0.000000 0.002557\"\n\
vc_FSM \"0.212585 0.715195 0.072220 0.880859\"\n";

    #[test]
    fn parses_rows() {
        let v = parse_t6_vision(NUKETOWN_MP).unwrap();
        assert!(v.film_enable);
        assert_eq!(v.bloom_rgb_hi, [0.09, 0.09, 0.09, 0.65]);
        assert_eq!(v.shadow[0], [1.164402, 0.0, 0.0, 0.002557]);
        assert_eq!(v.shadow[1], [0.0, 1.0, 0.0, 0.0]);
        assert_eq!(v.final_saturation[3], 0.880859);
        assert!(parse_t6_vision("r_filmEnable \"1\"\n").is_none());
    }

    #[test]
    fn vision_edge_rows() {
        let v = parse_t6_vision("r_filmenable 0\nvc_yh \"1 2 3\"\nvc_yl \"1 2 3 4 5\"\nvc_rs \"1 x 3 4\"\nbogus \"1 2 3 4\"\nvc_hmg 1 2 3 4\nlone\n").unwrap();
        assert!(!v.film_enable);
        assert_eq!(v.bloom_y_hi, T6Vision::default().bloom_y_hi);
        assert_eq!(v.bloom_y_lo, [1.0, 2.0, 3.0, 4.0]);
        assert_eq!(v.range_start, T6Vision::default().range_start);
        assert_eq!(v.highlight[1], [1.0, 2.0, 3.0, 4.0]);
        assert!(parse_t6_vision("r_filmenable 0\nvc_yh \"1 2 3\"\n").is_none());
    }

    #[test]
    fn default_vision_is_identity() {
        let v = T6Vision::default();
        for c in [
            [0.0, 0.0, 0.0],
            [0.25, 0.5, 0.75],
            [1.0, 1.0, 1.0],
            [0.9, 0.1, 0.3],
        ] {
            let o = t6_grade_colour(&v, c, 1.0);
            for i in 0..3 {
                assert!((o[i] - c[i]).abs() < 1e-5, "{c:?} -> {o:?}");
            }
        }
        let lut = t6_grade_lut(&v, None);
        assert_eq!(lut.len(), 32 * 32 * 32 * 4);
        // red fastest, then green, then blue
        assert_eq!(&lut[31 * 4..31 * 4 + 4], &[255, 0, 0, 255]);
        assert_eq!(&lut[32 * 31 * 4..32 * 31 * 4 + 4], &[0, 255, 0, 255]);
        assert_eq!(
            &lut[32 * 32 * 31 * 4..32 * 32 * 31 * 4 + 4],
            &[0, 0, 255, 255]
        );
    }

    #[test]
    fn saturation_pulls_toward_luminance() {
        let mut v = T6Vision::default();
        v.final_saturation[3] = 0.5;
        let o = t6_grade_colour(&v, [1.0, 0.0, 0.0], 1.0);
        let lum = T6_LUMA[0];
        assert!((o[0] - (lum + 0.5 * (1.0 - lum)).sqrt()).abs() < 1e-5);
        assert!((o[1] - (lum * 0.5).sqrt()).abs() < 1e-5);
    }
}
