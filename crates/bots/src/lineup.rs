//! The bots this host speaks for. BO2's bot script adds each one through `addtestclient` on a client number it
//! picks; the host then joins that client to the match, names it, and gives it a default class at every spawn.

use bevy::prelude::Resource;
use sim::ClientId;
use sim::LifeSequence;

/// Mixes every class pick; a constant so picks repeat from match to match.
const PICK_SEED: u64 = 0xb075_0001;

/// The default class a bot takes on its `pick`-th choice (0 = first). A fixed 64-bit mix of the seed, the client
/// and the pick count; the sim's bot class choice turns the byte into a class.
pub(crate) fn class_for(seed: u64, bot: ClientId, pick: u32) -> u8 {
    let blend = [
        (seed, 0x9E37_79B9_7F4A_7C15_u64),
        (u64::from(bot.0), 0xBF58_476D_1CE4_E5B9),
        (u64::from(pick), 0x94D0_49BB_1331_11EB),
    ]
    .into_iter()
    .fold(0u64, |sum, (part, weight)| sum.wrapping_add(part.wrapping_mul(weight)));
    (blend >> 33) as u8
}

#[derive(Debug)]
pub(crate) struct ChairSlot {
    pub(crate) client: ClientId,
    pub(crate) gamertag: String,
    /// JoinMatch, the first class and the name have been sent.
    pub(crate) booted: bool,
    /// Classes chosen so far; feeds [`class_for`].
    pub(crate) picks: u32,
    /// The life the last class was chosen for, so each spawn chooses once.
    pub(crate) chose_for: Option<LifeSequence>,
}

/// Every bot client on this host.
#[derive(Resource, Debug)]
pub struct BotTeamList {
    pub(crate) seats: Vec<ChairSlot>,
    pub(crate) seed: u64,
}

impl Default for BotTeamList {
    fn default() -> Self {
        BotTeamList { seats: Vec::new(), seed: PICK_SEED }
    }
}

impl BotTeamList {
    /// Is `who` one of the host's bots?
    pub fn seats_bot(&self, who: ClientId) -> bool {
        self.seats.iter().any(|seat| seat.client == who)
    }

    /// ChairSlot a bot the game script made; false when that client already has a seat.
    pub(crate) fn seat(&mut self, newcomer: ClientId, gamertag: String) -> bool {
        if self.seats_bot(newcomer) {
            return false;
        }
        self.seats.push(ChairSlot { client: newcomer, gamertag, booted: false, picks: 0, chose_for: None });
        true
    }

    /// Drop a bot's seat; false when it had none.
    pub(crate) fn unseat(&mut self, leaver: ClientId) -> bool {
        let count = self.seats.len();
        self.seats.retain(|seat| seat.client != leaver);
        self.seats.len() != count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_picks_match_the_recorded_values() {
        let mut got = Vec::new();
        for client in 1..=3 {
            for pick in 0..3 {
                got.push(class_for(PICK_SEED, ClientId(client), pick));
            }
        }
        assert_eq!(got, [191, 157, 122, 118, 83, 49, 44, 10, 232]);
    }

    #[test]
    fn a_client_is_seated_once_and_can_leave() {
        let mut roster = BotTeamList::default();
        assert!(roster.seat(ClientId(4), "a".into()));
        assert!(!roster.seat(ClientId(4), "b".into()));
        assert!(roster.seats_bot(ClientId(4)));
        assert!(!roster.seats_bot(ClientId(5)));
        assert!(roster.unseat(ClientId(4)));
        assert!(!roster.unseat(ClientId(4)));
        assert!(!roster.seats_bot(ClientId(4)));
    }

    #[test]
    fn a_new_seat_starts_unjoined_on_its_first_pick() {
        let mut roster = BotTeamList::default();
        roster.seat(ClientId(2), "x".into());
        let seat = &roster.seats[0];
        assert_eq!((seat.booted, seat.picks, seat.chose_for), (false, 0, None));
        assert_eq!(class_for(roster.seed, seat.client, seat.picks), 118);
    }
}
