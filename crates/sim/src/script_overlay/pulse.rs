//! The pulse effect of a HUD element: its timing, and the sound id each client's next pulse element gets.

use hud_t6::overlay_piece::{HUDELEM_PULSE_SOUND_IDS, OverlayPiece};

/// How many clients own a pulse sound counter.
const COUNTED_CLIENTS: usize = 64;

/// Per client, the sound id given to the latest pulse-effect element. Ids count from 1 to
/// `HUDELEM_PULSE_SOUND_IDS - 1` and wrap, so elements next to each other sound different.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PulseEffectSoundSet {
    last: [i32; COUNTED_CLIENTS],
}

impl Default for PulseEffectSoundSet {
    fn default() -> Self {
        Self { last: [0; COUNTED_CLIENTS] }
    }
}

impl PulseEffectSoundSet {
    /// The id to give `client` next. A client without a counter always receives 1.
    pub(super) fn next(&mut self, client: usize) -> i32 {
        match self.last.get_mut(client) {
            Some(counter) => {
                *counter = ((*counter + 1) % HUDELEM_PULSE_SOUND_IDS).max(1);
                *counter
            }
            None => 1,
        }
    }
}

/// The timing of a pulse effect, in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PulseFx {
    /// When the effect starts.
    pub birth: i32,
    /// How long each letter takes to appear.
    pub letter: i32,
    /// When the decay starts, counted from the birth.
    pub decay_start: i32,
    /// How long the decay lasts.
    pub decay: i32,
}

/// Turns the pulse effect on for a client's element: the timing, and the next sound id of that client. The caller has
/// already turned down elements shown to every client.
pub fn set_pulse_fx(elem: &mut OverlayPiece, client: usize, fx: PulseFx, ids: &mut PulseEffectSoundSet) {
    elem.fx_born = fx.birth;
    elem.fx_char_ms = fx.letter;
    elem.fx_fade_from = fx.decay_start;
    elem.fx_fade_len = fx.decay;
    elem.sound_no = ids.next(client);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pulse_ids_count_up_from_one_and_wrap() {
        let mut ids = PulseEffectSoundSet::default();
        let run: Vec<i32> = (0..33).map(|_| ids.next(0)).collect();
        assert_eq!(&run[..2], &[1, 2]);
        assert_eq!(run[30], HUDELEM_PULSE_SOUND_IDS - 1);
        assert_eq!(&run[31..], &[1, 2]);
        assert!(!run.contains(&0));
    }

    #[test]
    fn every_client_has_its_own_pulse_counter() {
        let mut ids = PulseEffectSoundSet::default();
        assert_eq!([ids.next(0), ids.next(0), ids.next(1)], [1, 2, 1]);
        assert_eq!(ids.next(COUNTED_CLIENTS), 1, "past the counters the id is always 1");
        assert_eq!(ids.next(usize::MAX), 1);
    }

    #[test]
    fn the_pulse_fills_in_the_effect_fields() {
        let mut ids = PulseEffectSoundSet::default();
        let mut elem = OverlayPiece::script_default();
        let fx = PulseFx { birth: 5000, letter: 70, decay_start: 200, decay: 900 };
        set_pulse_fx(&mut elem, 3, fx, &mut ids);
        let fields = |e: &OverlayPiece| {
            (e.fx_born, e.fx_char_ms, e.fx_fade_from, e.fx_fade_len, e.sound_no)
        };
        assert_eq!(fields(&elem), (5000, 70, 200, 900, 1));
        set_pulse_fx(&mut elem, 3, PulseFx { birth: 6000, ..fx }, &mut ids);
        assert_eq!((elem.fx_born, elem.sound_no), (6000, 2));
    }
}
