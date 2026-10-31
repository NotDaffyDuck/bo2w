//! bo2mp: measure Black Ops II's model vertex (`GfxPackedVertex`), read-only.
//! `t6vert <zone.ff>...` (default Nuketown) captures each zone's XModels and
//! reports, over every first-LOD vertex: the row size, the binormal-sign
//! values, the colour and uv_semantic bytes, how the normal and tangent words
//! pack (top bits, component range, decoded length per candidate decode),
//! and per triangle how the packed normal, tangent and binormal sign line up
//! with the geometry and the texcoords. A few whole rows are printed in hex
//! (three per binormal sign, each from a different model) for tests. The
//! world vertex buffers (vd0, vd1) are swept: for each candidate row size,
//! rows laid from every surface's vertexDataOffset, how each 4-byte slot
//! reads (unit 10:10:10 word, f32 sign, f32 inside the world box).

use std::collections::BTreeMap;
use std::path::PathBuf;

use glam::Vec3;

use asset_t6::{WorldRef, WorldSurface, ZoneCapture, capture_zone, short_float_to_full, pack_unit_vec};
use fastfile_t6::layout_gen::GfxPackedVertex;

fn default_zone() -> PathBuf {
    bo2zm_tools::zone_file("mp_nuketown_2020.ff")
}

fn rd_u32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// Three 10-bit fields, two's complement, over `div`.
fn signed10(w: u32, div: f32) -> Vec3 {
    let c = |v: u32| {
        let v = (v & 0x3ff) as i32;
        (if v >= 512 { v - 1024 } else { v }) as f32 / div
    };
    Vec3::new(c(w), c(w >> 10), c(w >> 20))
}

/// Three 10-bit fields, unsigned, biased to -1..1.
fn biased10(w: u32) -> Vec3 {
    let c = |v: u32| (v & 0x3ff) as f32 / 1023.0 * 2.0 - 1.0;
    Vec3::new(c(w), c(w >> 10), c(w >> 20))
}

/// Four bytes, unsigned, biased to -1..1 (the legacy packing).
fn biased8(w: u32) -> Vec3 {
    let c = |v: u32| (v & 0xff) as f32 / 127.5 - 1.0;
    Vec3::new(c(w), c(w >> 8), c(w >> 16))
}

#[derive(Default, Clone, Copy)]
struct Lengths {
    n: u64,
    min: f32,
    max: f32,
    sum: f64,
    near_one: u64,
}

impl Lengths {
    fn tally(&mut self, l: f32) {
        let (min, max) = if self.n == 0 { (l, l) } else { (self.min, self.max) };
        *self = Lengths {
            n: self.n + 1,
            min: min.min(l),
            max: max.max(l),
            sum: self.sum + f64::from(l),
            near_one: self.near_one + u64::from((l - 1.0).abs() <= 0.01),
        };
    }

    fn line(&self, what: &str) -> String {
        if self.n == 0 {
            return format!("{what}: none");
        }
        format!(
            "{what}: len min {:.4} max {:.4} mean {:.4}, within 0.01 of 1: {}/{} ({:.2}%)",
            self.min,
            self.max,
            self.sum / self.n as f64,
            self.near_one,
            self.n,
            100.0 * self.near_one as f64 / self.n as f64
        )
    }
}

/// One packed unit-vector word (normal or tangent).
#[derive(Default)]
struct Word {
    top2: BTreeMap<u32, u64>,
    comp_min: i32,
    comp_max: i32,
    s511: Lengths,
    s512: Lengths,
    b1023: Lengths,
    b8: Lengths,
    /// `pack_unit_vec(signed10 /511)` gives the word back.
    repack_same: u64,
}

impl Word {
    fn tally(&mut self, w: u32) {
        *self.top2.entry(w >> 30).or_default() += 1;
        for k in 0..3 {
            let v = ((w >> (10 * k)) & 0x3ff) as i32;
            let v = if v >= 512 { v - 1024 } else { v };
            self.comp_min = self.comp_min.min(v);
            self.comp_max = self.comp_max.max(v);
        }
        self.s511.tally(signed10(w, 511.0).length());
        self.s512.tally(signed10(w, 512.0).length());
        self.b1023.tally(biased10(w).length());
        self.b8.tally(biased8(w).length());
        self.repack_same += u64::from(pack_unit_vec(signed10(w, 511.0).to_array()) == w);
    }

    fn print(&self, what: &str) {
        println!("  {what} @{}: bits 30..31 values {:?}", if what == "normal" { GfxPackedVertex::normal } else { GfxPackedVertex::tangent }, self.top2);
        println!("    signed 10-bit components: min {} max {}", self.comp_min, self.comp_max);
        println!("    {line}", line = self.s511.line("signed10 /511"));
        println!("    {line}", line = self.s512.line("signed10 /512"));
        println!("    {line}", line = self.b1023.line("unsigned10 biased"));
        println!("    {line}", line = self.b8.line("bytes biased"));
        println!("    asset_t6::pack_unit_vec(signed10 /511) == word: {}", pct(self.repack_same, self.s511.n - self.repack_same));
    }
}

#[derive(Default)]
struct Report {
    models: u64,
    surfaces: u64,
    short_surfaces: u64,
    stride: BTreeMap<String, u64>,
    vertex_points: u64,
    sign_bits: BTreeMap<u32, u64>,
    alpha_19_ff: u64,
    alpha_16_ff: u64,
    uv_nonfinite: u64,
    uv_lo: (f32, f32),
    uv_hi: (f32, f32),
    normal: Word,
    tangent: Word,
    n_dot_t: Lengths,
    triangle_list: u64,
    face_with_normal: u64,
    face_against_normal: u64,
    tan_uv_lo: (u64, u64),
    tan_uv_hi: (u64, u64),
    bin_ntx_dv: (u64, u64),
    bin_tnx_dv: (u64, u64),
    mixed_sign_tris: u64,
    /// Sample rows, keyed by binormal sign bits.
    samples: BTreeMap<u32, Vec<SampleRow>>,
}

/// One whole vertex row and the model it came from.
struct SampleRow {
    from_model: String,
    bytes: Vec<u8>,
}

fn measure(capture: &ZoneCapture, r: &mut Report) {
    r.models += capture.xmodels.len() as u64;
    for model in &capture.xmodels {
        for srf in &model.lod0 {
            r.surfaces += 1;
            let n = usize::from(srf.vertex_tally);
            if n == 0 {
                continue;
            }
            let key = if srf.vertex_points.len() % n == 0 {
                format!("{per}", per = srf.vertex_points.len() / n)
            } else {
                format!("{} bytes / {} verts", srf.vertex_points.len(), n)
            };
            *r.stride.entry(key).or_default() += 1;
            if srf.vertex_points.len() < n * GfxPackedVertex::SIZE {
                r.short_surfaces += 1;
                continue;
            }
            let rows: Vec<&[u8]> = srf.vertex_points.chunks_exact(GfxPackedVertex::SIZE).take(n).collect();
            let sign = rd_u32(rows[0], GfxPackedVertex::binormalSign);
            let picked = r.samples.entry(sign).or_default();
            if picked.len() < 3 && picked.iter().all(|s| s.from_model != model.name) {
                picked.push(SampleRow { from_model: model.name.clone(), bytes: rows[0].to_vec() });
            }
            for v in &rows {
                r.vertex_points += 1;
                *r.sign_bits.entry(rd_u32(v, GfxPackedVertex::binormalSign)).or_default() += 1;
                r.alpha_19_ff += u64::from(v[GfxPackedVertex::color + 3] == 0xff);
                r.alpha_16_ff += u64::from(v[GfxPackedVertex::color] == 0xff);
                let tex = rd_u32(v, GfxPackedVertex::texCoord);
                let (lo, hi) = (short_float_to_full(tex as u16), short_float_to_full((tex >> 16) as u16));
                if !lo.is_finite() || !hi.is_finite() {
                    r.uv_nonfinite += 1;
                } else {
                    r.uv_lo = (r.uv_lo.0.min(lo), r.uv_lo.1.max(lo));
                    r.uv_hi = (r.uv_hi.0.min(hi), r.uv_hi.1.max(hi));
                }
                let (nw, tw) = (rd_u32(v, GfxPackedVertex::normal), rd_u32(v, GfxPackedVertex::tangent));
                r.normal.tally(nw);
                r.tangent.tally(tw);
                r.n_dot_t.tally(signed10(nw, 511.0).dot(signed10(tw, 511.0)).abs());
            }
            for t in srf.indices.chunks_exact(3) {
                let ids = [usize::from(t[0]), usize::from(t[1]), usize::from(t[2])];
                if ids.iter().any(|&i| i >= n) {
                    continue;
                }
                triangle(&ids.map(|i| rows[i]), r);
            }
        }
    }
}

fn triangle(v: &[&[u8]; 3], r: &mut Report) {
    let pos = |row: &[u8]| Vec3::from_array([0, 4, 8].map(|o| f32::from_bits(rd_u32(row, GfxPackedVertex::xyz + o))));
    let uv = |row: &[u8]| -> [f32; 2] {
        let tex = rd_u32(row, GfxPackedVertex::texCoord);
        [short_float_to_full(tex as u16), short_float_to_full((tex >> 16) as u16)]
    };
    let p = v.map(pos);
    let face = (p[1] - p[0]).cross(p[2] - p[0]);
    if face.length() < 1e-6 {
        return;
    }
    r.triangle_list += 1;
    let normal = v.iter().fold(Vec3::ZERO, |a, row| a + signed10(rd_u32(row, GfxPackedVertex::normal), 511.0));
    let tangent = v.iter().fold(Vec3::ZERO, |a, row| a + signed10(rd_u32(row, GfxPackedVertex::tangent), 511.0));
    if face.dot(normal) >= 0.0 {
        r.face_with_normal += 1;
    } else {
        r.face_against_normal += 1;
    }

    // dP/du and dP/dv over the triangle, with u the low half (lo) or the
    // high half (hi) of the uv_semantic word.
    let t = v.map(uv);
    let (e1, e2) = (p[1] - p[0], p[2] - p[0]);
    let grads = |a: usize, b: usize| -> Option<(Vec3, Vec3)> {
        let (du1, dv1) = (t[1][a] - t[0][a], t[1][b] - t[0][b]);
        let (du2, dv2) = (t[2][a] - t[0][a], t[2][b] - t[0][b]);
        let det = du1 * dv2 - du2 * dv1;
        if !det.is_finite() || det.abs() < 1e-8 {
            return None;
        }
        let dpdu = (e1 * dv2 - e2 * dv1) * (1.0 / det);
        let dpdv = (e2 * du1 - e1 * du2) * (1.0 / det);
        Some((dpdu, dpdv))
    };
    let count = |pair: &mut (u64, u64), d: f32| {
        if d >= 0.0 { pair.0 += 1 } else { pair.1 += 1 }
    };
    if let Some((dpdu, dpdv)) = grads(0, 1) {
        count(&mut r.tan_uv_lo, tangent.dot(dpdu));
        let signs = v.map(|row| f32::from_bits(rd_u32(row, GfxPackedVertex::binormalSign)));
        if signs.iter().all(|&s| s == signs[0]) && signs[0] != 0.0 {
            let s = signs[0].signum();
            count(&mut r.bin_ntx_dv, s * normal.cross(tangent).dot(dpdv));
            count(&mut r.bin_tnx_dv, s * tangent.cross(normal).dot(dpdv));
        } else {
            r.mixed_sign_tris += 1;
        }
    }
    if let Some((dpdu, _)) = grads(1, 0) {
        count(&mut r.tan_uv_hi, tangent.dot(dpdu));
    }
}

fn pct(a: u64, b: u64) -> String {
    format!("{a}/{} ({:.2}%)", a + b, 100.0 * a as f64 / (a + b).max(1) as f64)
}

fn print(zone: &str, r: &Report, capture: &ZoneCapture) {
    println!("{zone}: {} xmodels, {} lod0 surfaces, {} vertices, {} triangles", r.models, r.surfaces, r.vertex_points, r.triangle_list);
    println!(
        "  layout GfxPackedVertex SIZE {} xyz @{} binormalSign @{} color @{} texCoord @{} normal @{} tangent @{}",
        GfxPackedVertex::SIZE, GfxPackedVertex::xyz, GfxPackedVertex::binormalSign, GfxPackedVertex::color, GfxPackedVertex::texCoord, GfxPackedVertex::normal, GfxPackedVertex::tangent
    );
    println!("  bytes per vertex (verts.len() / vertCount), surfaces: {:?}; short: {}", r.stride, r.short_surfaces);
    let mut signs: Vec<_> = r.sign_bits.iter().collect();
    signs.sort_by(|a, b| b.1.cmp(a.1));
    let shown: Vec<String> = signs
        .iter()
        .take(6)
        .map(|(bits, c)| format!("{bits:#010x} (f32 {}) x{c}", f32::from_bits(**bits)))
        .collect();
    println!("  binormalSign @{}: {} distinct u32 values; top: {}", GfxPackedVertex::binormalSign, signs.len(), shown.join(", "));
    println!(
        "  color @{}: byte 3 is 0xff in {}, byte 0 is 0xff in {}",
        GfxPackedVertex::color,
        pct(r.alpha_19_ff, r.vertex_points - r.alpha_19_ff),
        pct(r.alpha_16_ff, r.vertex_points - r.alpha_16_ff)
    );
    println!(
        "  texCoord @{}: non-finite half in {} verts; low half {:.3}..{:.3}, high half {:.3}..{:.3}",
        GfxPackedVertex::texCoord, r.uv_nonfinite, r.uv_lo.0, r.uv_lo.1, r.uv_hi.0, r.uv_hi.1
    );
    r.normal.print("normal");
    r.tangent.print("tangent");
    println!("  |normal . tangent| (signed10 /511): mean {:.4}, max {:.4}", r.n_dot_t.sum / r.n_dot_t.n.max(1) as f64, r.n_dot_t.max);
    println!(
        "  triangles (v0,v1,v2), cross(v1-v0, v2-v0) . summed normal >= 0: {}",
        pct(r.face_with_normal, r.face_against_normal)
    );
    println!("  summed tangent . dP/du >= 0, u = low half: {}; u = high half: {}", pct(r.tan_uv_lo.0, r.tan_uv_lo.1), pct(r.tan_uv_hi.0, r.tan_uv_hi.1));
    println!(
        "  sign * cross(N, T) . dP/dv >= 0 (u low, v high): {}; sign * cross(T, N): {}; triangles with mixed or zero signs: {}",
        pct(r.bin_ntx_dv.0, r.bin_ntx_dv.1),
        pct(r.bin_tnx_dv.0, r.bin_tnx_dv.1),
        r.mixed_sign_tris
    );
    for (sign, table) in &r.samples {
        for SampleRow { from_model, bytes } in table {
            let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02x}")).collect();
            println!("  row sign {sign:#010x} {from_model}: {}", hex.join(""));
        }
    }
    if let Some(w) = &capture.world {
        world(w);
    }
}

/// How one 4-byte slot of a candidate world row reads, over every row.
#[derive(Default, Clone, Copy)]
struct Slot {
    n: u64,
    /// Signed 10:10:10 over 511 with bits 30..31 clear, unit length within 0.01.
    unit: u64,
    /// Exactly +1.0 or -1.0 as an f32.
    sign: u64,
    /// A finite f32 inside the world's box (grown by 1 unit).
    in_box: u64,
}

impl Slot {
    fn tally(&mut self, w: u32, lo: f32, hi: f32) {
        let is_unit = w >> 30 == 0 && (signed10(w, 511.0).length() - 1.0).abs() <= 0.01;
        let is_sign = matches!(w, 0x3f80_0000 | 0xbf80_0000);
        let inside = (lo..=hi).contains(&f32::from_bits(w));
        *self = Slot {
            n: self.n + 1,
            unit: self.unit + u64::from(is_unit),
            sign: self.sign + u64::from(is_sign),
            in_box: self.in_box + u64::from(inside),
        };
    }

    fn cell(&self, at: usize) -> String {
        let p = |a: u64| 100.0 * a as f64 / self.n.max(1) as f64;
        format!("@{at} u{:.1} s{:.1} b{:.1}", p(self.unit), p(self.sign), p(self.in_box))
    }
}

/// Standings every 4-byte offset of `stride`-byte rows laid from `starts`
/// (byte offset into `data`, row count); rows past the end are counted apart.
fn sweep(data: &[u8], starts: &[(usize, usize)], stride_bytes: usize, lo: f32, hi: f32) -> (Vec<Slot>, u64, u64) {
    let mut slots = vec![Slot::default(); stride_bytes / 4];
    let (mut rows, mut past_end) = (0u64, 0u64);
    for &(base, n) in starts {
        for i in 0..n {
            let at = base + i * stride_bytes;
            let Some(row) = data.get(at..at + stride_bytes) else {
                past_end += 1;
                continue;
            };
            rows += 1;
            for (k, slot) in slots.iter_mut().enumerate() {
                slot.tally(rd_u32(row, k * 4), lo, hi);
            }
        }
    }
    (slots, rows, past_end)
}

fn world(w: &WorldRef) {
    let count = w.vertex_count as usize;
    let per = |len: usize| len as f64 / count.max(1) as f64;
    println!(
        "  world: vertexCount {count}, vd0 {} bytes ({:.3} per vertex), vd1 {} bytes ({:.3} per vertex), {} surfaces",
        w.vd0.len(),
        per(w.vd0.len()),
        w.vd1.len(),
        per(w.vd1.len()),
        w.surfaces.len()
    );
    let lo = w.lo_corner.iter().copied().fold(f32::INFINITY, f32::min) - 1.0;
    let hi = w.hi_corner.iter().copied().fold(f32::NEG_INFINITY, f32::max) + 1.0;

    // Each distinct vertex range once: (vertexDataOffset, firstVertex, vertexCount).
    let ranges = |offset: fn(&WorldSurface) -> i32| -> Vec<(usize, usize, usize)> {
        let mut r: Vec<(usize, usize, usize)> = w
            .surfaces
            .iter()
            .filter(|s| offset(s) >= 0 && s.first_vertex >= 0 && s.vertex_count > 0)
            .map(|s| (offset(s) as usize, s.first_vertex as usize, usize::from(s.vertex_count)))
            .collect();
        r.sort_unstable();
        r.dedup();
        r
    };
    for (name, data, ranges, strides) in [
        ("vd0", &w.vd0, ranges(|s: &WorldSurface| s.vertex_data_offset0), &[32usize, 36, 40, 44, 48][..]),
        ("vd1", &w.vd1, ranges(|s: &WorldSurface| s.vertex_data_offset1), &[4usize, 8, 12, 16, 20, 24][..]),
    ] {
        // vertexDataOffset over firstVertex: one value means a fixed row size.
        let mut ratio: BTreeMap<String, u64> = BTreeMap::new();
        for &(off, first, _) in &ranges {
            let key = match (off, first) {
                (0, 0) => "both 0".to_string(),
                (_, 0) => "firstVertex 0".to_string(),
                _ if off % first == 0 => format!("{}", off / first),
                _ => format!("{:.3}", off as f64 / first as f64),
            };
            *ratio.entry(key).or_default() += 1;
        }
        let mut ratio: Vec<_> = ratio.into_iter().collect();
        ratio.sort_by(|a, b| b.1.cmp(&a.1));
        ratio.truncate(8);
        let end_vertex = ranges.iter().map(|&(_, f, n)| f + n).max().unwrap_or(0);
        println!(
            "  world {name}: {} distinct surface ranges, highest firstVertex + vertexCount {end_vertex}; vertexDataOffset / firstVertex (top 8): {ratio:?}",
            ranges.len()
        );
        // The gap to the next range's start over this range's vertex count.
        let mut gap: BTreeMap<String, u64> = BTreeMap::new();
        for pair in ranges.windows(2) {
            let ((a, _, n), (b, _, _)) = (pair[0], pair[1]);
            if b > a {
                let key = if (b - a) % n == 0 { format!("{per}", per = (b - a) / n) } else { "not whole".to_string() };
                *gap.entry(key).or_default() += 1;
            }
        }
        let mut gap: Vec<_> = gap.into_iter().collect();
        gap.sort_by(|a, b| b.1.cmp(&a.1));
        gap.truncate(8);
        println!("    (next start - start) / vertexCount (top 8): {gap:?}");
        let starts: Vec<(usize, usize)> = ranges.iter().map(|&(off, _, n)| (off, n)).collect();
        for &stride in strides {
            let (slots, rows, past_end) = sweep(data, &starts, stride, lo, hi);
            let cells: Vec<String> = slots.iter().enumerate().map(|(k, s)| s.cell(k * 4)).collect();
            println!("    stride {stride} from vertexDataOffset ({rows} rows, {past_end} past the end): {}", cells.join(" | "));
        }
    }
    // vd0 as one array from byte 0, the reading the first probe took.
    let (slots, rows, _) = sweep(&w.vd0, &[(0, count)], 36, lo, hi);
    let cells: Vec<String> = slots.iter().enumerate().map(|(k, s)| s.cell(k * 4)).collect();
    println!("  world vd0 stride 36 from byte 0 ({rows} rows): {}", cells.join(" | "));
    println!("  (u = unit 10:10:10 word, s = f32 +-1, b = f32 inside the world box; percent of rows)");
}

/// The zones named on the command line, or the default one.
fn zones_to_read(named: impl Iterator<Item = String>) -> Vec<PathBuf> {
    let zones: Vec<PathBuf> = named.map(PathBuf::from).collect();
    match zones.as_slice() {
        [] => vec![default_zone()],
        _ => zones,
    }
}

/// Reads one zone and prints its report; false when the zone would not load.
fn report_zone(path: &PathBuf) -> bool {
    let capture = match capture_zone(path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("t6vert: {}: {e}", path.to_string_lossy());
            return false;
        }
    };
    let mut r = Report::default();
    measure(&capture, &mut r);
    print(&path.display().to_string(), &r, &capture);
    true
}

fn main() {
    let loaded: Vec<bool> = zones_to_read(std::env::args().skip(1))
        .iter()
        .map(report_zone)
        .collect();
    std::process::exit(if loaded.contains(&false) { 1 } else { 0 });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_give_vec3_per_field() {
        assert_eq!(signed10(0x0010_0401 | (1023 << 20), 1.0).to_array(), [1.0, 1.0, -1.0]);
        assert_eq!(biased8(0x00ff_0000).to_array(), [-1.0, -1.0, 1.0]);
    }

    #[test]
    fn lengths_track_extremes_and_near_one() {
        let mut l = Lengths::default();
        assert_eq!(l.line("n"), "n: none");
        for v in [0.5, 1.005, 2.0] {
            l.tally(v);
        }
        assert_eq!((l.n, l.min, l.max, l.near_one), (3, 0.5, 2.0, 1));
        assert!((l.sum - 3.505).abs() < 1e-6);
    }

    #[test]
    fn no_zone_named_means_the_default() {
        assert_eq!(zones_to_read(std::iter::empty()), vec![default_zone()]);
        assert_eq!(zones_to_read(["a.ff".to_owned()].into_iter()), vec![PathBuf::from("a.ff")]);
    }

    #[test]
    fn the_sweep_finds_each_slot_at_its_offset() {
        // Two ranges of 36-byte rows: x y z, sign, colour, uv, normal, tangent, baked_light_slot.
        let row = |x: f32| {
            let mut r = Vec::new();
            for f in [x, 2.0, 3.0, -1.0] {
                r.extend_from_slice(&f.to_le_bytes());
            }
            r.extend_from_slice(&[0xff; 8]);
            r.extend_from_slice(&pack_unit_vec([0.0, 0.0, 1.0]).to_le_bytes());
            r.extend_from_slice(&pack_unit_vec([1.0, 0.0, 0.0]).to_le_bytes());
            r.extend_from_slice(&[0; 4]);
            r
        };
        let data: Vec<u8> = (0..5).flat_map(|i| row(i as f32)).collect();
        let (slots, rows, past_end) = sweep(&data, &[(0, 2), (72, 3)], 36, -10.0, 10.0);
        assert_eq!((rows, past_end), (5, 0));
        assert_eq!([0, 4, 8].map(|k| slots[k / 4].in_box), [5; 3]);
        assert_eq!(slots[3].sign, 5);
        assert_eq!((slots[6].unit, slots[7].unit), (5, 5));
        assert_eq!(slots[2].unit + slots[5].unit, 0);
        // At the wrong stride the normal does not sit at 24 in later rows; the last row runs past the end.
        let (slots, rows, past_end) = sweep(&data, &[(0, 5)], 40, -10.0, 10.0);
        assert_eq!((rows, past_end), (4, 1));
        assert!(slots[6].unit < 4);
    }
}
