//! Pins every public function of game_math to the exact bits it returned before the rewrite of 10-23: each
//! function runs over a fixed stream of inputs (edge values plus a seeded generator) and its outputs are folded into
//! one FNV-1a hash per function. A changed hash means a changed result somewhere, even in the last bit.

/// FNV-1a over the raw bits of every float fed in.
struct Fold(u64);

impl Fold {
    fn seeded() -> Self {
        Fold(0xcbf2_9ce4_8422_2325)
    }
    fn f(&mut self, x: f32) {
        for byte in x.to_bits().to_le_bytes() {
            self.0 = (self.0 ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    fn v<const N: usize>(&mut self, xs: [f32; N]) {
        xs.into_iter().for_each(|x| self.f(x));
    }
    fn m(&mut self, rows: [[f32; 3]; 3]) {
        rows.into_iter().for_each(|row| self.v(row));
    }
}

/// Edge values every stream starts with.
const EDGES: [f32; 24] = [
    0.0, -0.0, 1.0, -1.0, 0.5, -0.5, 0.001, -0.001, 45.0, 90.0, -90.0, 179.99, 180.0, -180.0, 270.0, 359.999,
    360.0, -360.0, 720.5, -1000.25, 1.0e-40, 8_388_608.5, 16_777_216.0, -3.0e7,
];

/// xorshift64* turned into floats: a third in [-1, 1], a third in [-400, 400], a third in [-3000, 3000].
struct Stream {
    state: u64,
    edge: usize,
}

impl Stream {
    fn new(seed: u64) -> Self {
        Stream { state: seed | 1, edge: 0 }
    }
    fn next(&mut self) -> f32 {
        if self.edge < EDGES.len() {
            self.edge += 1;
            return EDGES[self.edge - 1];
        }
        self.state ^= self.state >> 12;
        self.state ^= self.state << 25;
        self.state ^= self.state >> 27;
        let r = self.state.wrapping_mul(0x2545_f491_4f6c_dd1d);
        let unit = (r >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0;
        match r & 3 {
            0 | 1 => unit * 400.0,
            2 => unit,
            _ => unit * 3000.0,
        }
    }
    fn three(&mut self) -> [f32; 3] {
        [self.next(), self.next(), self.next()]
    }
}

const ROUNDS: usize = 20_000;

fn hash(seed: u64, mut each: impl FnMut(&mut Stream, &mut Fold)) -> u64 {
    let (mut input, mut out) = (Stream::new(seed), Fold::seeded());
    for _ in 0..ROUNDS {
        each(&mut input, &mut out);
    }
    out.0
}

/// Every function's hash, in the order of [`EXPECTED`].
fn hashes() -> [(&'static str, u64); 18] {
    [
        ("angle_normalize_360", hash(1, |s, o| o.f(game_math::wrap_degrees(s.next())))),
        ("angle_subtract", hash(2, |s, o| o.f(game_math::degree_gap(s.next(), s.next())))),
        ("angle_vectors", hash(3, |s, o| {
            let (f, r, u) = game_math::euler_basis(s.three());
            o.m([f, r, u]);
        })),
        ("angles_to_axis", hash(4, |s, o| o.m(game_math::euler_to_rows(s.three())))),
        ("axis_to_angles", hash(5, |s, o| {
            let axis = game_math::euler_to_rows(s.three());
            o.v(game_math::rows_to_euler(axis));
            o.v(game_math::rows_to_euler([s.three(), s.three(), s.three()]));
        })),
        ("pitch_for_yaw_on_normal", hash(6, |s, o| o.f(game_math::slope_pitch(s.next(), s.three())))),
        ("snap_angles", hash(7, |s, o| {
            let near = s.three().map(|x| (x * 4.0).round() / 4.0 + x * 1.0e-6);
            o.v(game_math::round_degrees(near));
            o.v(game_math::round_degrees(s.three()));
        })),
        ("track", hash(8, |s, o| o.f(game_math::ease_toward(s.next(), s.next(), s.next() * 0.01, s.next() * 0.001)))),
        ("track_angle", hash(9, |s, o| {
            // Bounded: the wrap loops step 360 at a time.
            let (a, b) = (s.next().clamp(-5000.0, 5000.0), s.next().clamp(-5000.0, 5000.0));
            o.f(game_math::ease_angle_toward(a, b, s.next() * 0.01, s.next() * 0.001));
        })),
        ("vec_to_yaw", hash(10, |s, o| o.f(game_math::heading_of(s.next(), s.next())))),
        ("vect_to_angles", hash(11, |s, o| {
            o.v(game_math::dir_to_euler(s.three()));
            let [x, _, z] = s.three();
            o.v(game_math::dir_to_euler([0.0, 0.0, z]));
            o.v(game_math::dir_to_euler([x, 0.0, 0.0]));
        })),
        ("yaw_vectors_2d", hash(12, |s, o| {
            let (f, r) = game_math::flat_basis(s.next());
            o.v(f);
            o.v(r);
        })),
        ("add_lean_to_position", hash(13, |s, o| {
            let lean = if s.next() < 0.0 { 0.0 } else { s.next() };
            o.v(game_math::lean_offset(s.three(), s.next(), lean, s.next(), s.next()));
        })),
        ("get_lean_fraction", hash(14, |s, o| o.f(game_math::lean_amount(s.next())))),
        ("matrix_multiply", hash(15, |s, o| {
            o.m(game_math::axis_product([s.three(), s.three(), s.three()], [s.three(), s.three(), s.three()]));
        })),
        ("matrix_multiply43", hash(16, |s, o| {
            let first_axis = [s.three(), s.three(), s.three()];
            let first_origin = s.three();
            let second_axis = [s.three(), s.three(), s.three()];
            let second_origin = s.three();
            let combined = game_math::frame_product(first_axis, first_origin, second_axis, second_origin);
            o.m(combined.0);
            o.v(combined.1);
        })),
        ("matrix_transform_vector43", hash(17, |s, o| {
            o.v(game_math::frame_apply(s.three(), [s.three(), s.three(), s.three()], s.three()));
        })),
        ("vec3_length", hash(18, |s, o| o.f(game_math::vec3_norm(s.three())))),
    ]
}

/// Recorded on the code before the rewrite.
const EXPECTED: [u64; 18] = [
    0x0ae06f9427935f40, // wrap into 0..360
    0xee2493737fa7a06b, // signed degree gap
    0x264bcd350310e456, // forward, right, up from angles
    0xd53ccdd136418e7e, // angles to a 3x3 frame
    0xb24c604961ad5b9a, // 3x3 frame back to angles
    0xf96acdf533c0568e, // pitch on a slope for a heading
    0x0effafbe90cf1b2f, // angles rounded to whole degrees
    0x8307d4b059347094, // track
    0xa35fd21ef2a71ff4, // one angle eased toward another
    0x01ed7beff1a25d3a, // heading of a direction
    0x4694638cc7f0b2fe, // direction to pitch and yaw
    0xaef14a5cc5e7512d, // flat forward and right
    0xdd965d1adc819a60, // eye moved by a lean
    0x154c10bdaed004f8, // how far the lean has gone
    0x1009ca9cc70fadb0, // 3x3 times 3x3
    0xd8cdaec82d1c7a5b, // 4x3 times 4x3
    0x7fb884c9da656679, // point through a 4x3 frame
    0xe8b1851c5ed758f5, // vec3_length
];

#[test]
fn every_function_returns_the_recorded_bits() {
    let got = hashes();
    let wrong: Vec<_> = got.iter().zip(EXPECTED).filter(|((_, h), want)| *h != *want).collect();
    let table: Vec<String> = got.iter().map(|(name, h)| format!("{h:#018x}, // {name}")).collect();
    assert!(wrong.is_empty(), "changed: {wrong:?}\nall:\n{}", table.join("\n"));
}
