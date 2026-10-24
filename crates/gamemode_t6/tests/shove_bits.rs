//! Pins the body shove to the exact results it gave before the rewrite of 10-23. Seeded crowds of 2 to 6 rows (some
//! dead, some with the wrong contents, some standing still) are shoved row by row the way the server's frame does it,
//! and every row's velocity bits, timer, flags, the shared seed and the chosen pair are folded into one FNV-1a hash.

use gamemode_t6::{PrematchStep, StuckClient, stuck_in_client};

const EXPECTED: u64 = 0xf420_6399_3ac0_3497;

struct Fold(u64);

impl Fold {
    fn eat(&mut self, word: u32) {
        for byte in word.to_le_bytes() {
            self.0 = (self.0 ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}

/// xorshift64*.
struct Dice(u64);

impl Dice {
    fn roll(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    /// Uniform in [-span, span].
    fn spread(&mut self, span: f32) -> f32 {
        ((self.roll() >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0) * span
    }
    fn below(&mut self, n: u64) -> u64 {
        (self.roll() >> 33) % n
    }
}

fn body(d: &mut Dice) -> StuckClient {
    let origin = [d.spread(24.0), d.spread(24.0), d.spread(8.0)];
    let still = d.below(3) == 0;
    let velocity = if still { [0.0, 0.0, d.spread(5.0)] } else { [d.spread(300.0), d.spread(300.0), d.spread(50.0)] };
    let contents = [0x200_0000, 0x400_0000, 0x200_0001, 0, 0x600_0000][d.below(5) as usize % 5];
    let contents = if d.below(4) == 0 { contents } else { [0x200_0000, 0x400_0000][d.below(2) as usize] };
    StuckClient {
        origin,
        velocity,
        speed: d.below(400) as i32 - 50,
        player: d.below(8) != 0,
        playing: d.below(8) != 0,
        contents,
        health: d.below(110) as i32 - 10,
        pm_time: d.below(500) as i32,
        pm_flags: d.roll() as u32,
        maxs_x: 15.0 + d.spread(2.0),
        bounds_mid: [origin[0] + d.spread(0.5), origin[1] + d.spread(0.5), origin[2] + 36.0],
        bounds_half: [15.0 + d.spread(1.0), 15.0 + d.spread(1.0), 36.0 + d.spread(4.0)],
    }
}

fn fingerprint() -> u64 {
    let mut dice = Dice(0x9e37_79b9_7f4a_7c15);
    let mut out = Fold(0xcbf2_9ce4_8422_2325);
    let mut seed = 0x1234_5678_u32;
    for round in 0..20_000 {
        let count = 2 + dice.below(5) as usize;
        let mut crowd: Vec<StuckClient> = (0..count).map(|_| body(&mut dice)).collect();
        let eject = match round % 4 {
            0 => 25.0,
            1 => 0.0,
            2 => 0.000_05,
            _ => dice.spread(60.0),
        };
        for i in 0..=count {
            let pair = stuck_in_client(i, &mut crowd, eject, &mut seed);
            out.eat(pair.map_or(u32::MAX, |p| (p.self_idx * 16 + p.other_idx) as u32));
        }
        out.eat(seed);
        for row in &crowd {
            row.velocity.iter().for_each(|v| out.eat(v.to_bits()));
            out.eat(row.pm_time as u32);
            out.eat(row.pm_flags);
        }
    }
    out.0
}

#[test]
fn shove_matches_the_recorded_bits() {
    let got = fingerprint();
    assert_eq!(got, EXPECTED, "shove fingerprint is now {got:#018x}");
}

#[test]
fn warm_up_starts_waiting_at_zero() {
    assert_eq!(PrematchStep::default(), PrematchStep::Waiting { elapsed_ms: 0 });
}
