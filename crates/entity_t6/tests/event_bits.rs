//! Pins the client's event answers to what they were before the rewrite of 10-23: the effect of every event number
//! from just below zero to past our own numbers, landing surfaces, what the wire carries, the shot and strike events,
//! which entity types keep a ring, and the ring reader on seeded states. Everything is folded into one FNV-1a hash.

use entity_t6::{
    EventCode, EntitySnapshot, bullet_hit_event, drain_entity_events, entity_event_action, gunner_fire_event,
    packet_entity_uses_event_ring, predicted_weapon_fire_event,
};

// Re-pinned 10-24 when the ring reader moved to BO2's 8-bit sequence wrap (77857a3f).
const EXPECTED: u64 = 0x871b_41db_e36d_9584;

struct Fold(u64);

impl Fold {
    fn eat(&mut self, word: u32) {
        for byte in word.to_le_bytes() {
            self.0 = (self.0 ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    fn eat_text(&mut self, chunk: &str) {
        chunk.bytes().for_each(|b| self.eat(b.into()));
    }
    fn eat_maybe(&mut self, maybe_num: Option<i32>) {
        self.eat(maybe_num.map_or(u32::MAX, |v| v as u32));
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
    fn below(&mut self, n: u64) -> i32 {
        ((self.roll() >> 33) % n) as i32
    }
}

fn table(out: &mut Fold) {
    for number in (-8..=0x120).chain([i32::MIN, i32::MAX]) {
        let event = EventCode(number);
        match entity_event_action(event) {
            Ok(action) => out.eat_text(&format!("{action:?}")),
            Err(unsupported) => out.eat(!(unsupported.0.0 as u32)),
        }
        out.eat_maybe(event.touchdown_material_index());
        out.eat(u32::from(event.on_the_wire()) | u32::from(event.hurts_on_landing()) << 1);
    }
    for impact in -2..=12 {
        for is_local in [false, true] {
            out.eat_maybe(bullet_hit_event(impact, is_local).map(|e| e.0));
        }
    }
    for hand in -1..=2 {
        for prior_shot in [false, true] {
            out.eat(predicted_weapon_fire_event(hand, prior_shot).0 as u32);
        }
    }
    for gunner in 0..4 {
        for second in [false, true] {
            out.eat(gunner_fire_event(gunner, second).0 as u32);
        }
    }
    for e_type in -4..=40 {
        out.eat(packet_entity_uses_event_ring(e_type).into());
    }
}

fn ring(out: &mut Fold) {
    let mut dice = Dice(0x9e37_79b9_7f4a_7c15);
    for _ in 0..20_000 {
        let mut state = EntitySnapshot::default();
        state.event_sequence = if dice.below(9) == 0 { 0 } else { (dice.below(2_200) - 60) as i16 };
        for slot in 0..4 {
            state.events[slot] = dice.roll() as u8;
            state.event_parms[slot] = dice.roll() as u32;
        }
        let latest = i32::from(state.event_sequence);
        let mut seen = match dice.below(4) {
            0 => latest,
            1 => latest + dice.below(17) - 8,
            2 => 0,
            _ => dice.below(10_000) - 5_000,
        };
        drain_entity_events(&state, &mut seen, |e| {
            out.eat(e.ring_seq as u32);
            out.eat(e.code.0 as u32);
            out.eat(e.parm_value as u32);
        });
        out.eat(seen as u32);
    }
}

#[test]
fn event_answers_match_the_recorded_bits() {
    let mut out = Fold(0xcbf2_9ce4_8422_2325);
    table(&mut out);
    ring(&mut out);
    let got = out.0;
    assert_eq!(got, EXPECTED, "event fingerprint is now {got:#018x}");
}
