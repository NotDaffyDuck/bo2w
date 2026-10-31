//! Which player-state fields cross the wire, how a prediction is checked against the server's
//! answer field by field, and the hash two peers compare to know their states agree.

mod table;
mod value;

use playerstate_t6::game::AvatarState;

use crate::net_links::byte_codec::ByteSinkWriter;

pub use table::PLAYER_STATE_WIRE_FIELDS;
pub(crate) use table::{value_changed, value_gap, extract_value, store_value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncPolicy {
    Synced,
    Withheld(&'static str),
}

/// How a client's prediction of a field is judged against the server's value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FieldCheck {
    /// Any difference is a misprediction.
    Exact,
    /// A difference up to `slack` is tolerated.
    AllowedSlack { slack: f32, cause_note: &'static str },
    /// The client cannot know this value; it takes the server's and never counts a difference.
    InstallWithoutCompare(&'static str),
}

#[derive(Clone, Copy, Debug)]
pub struct PlayerStateWireField {
    pub code_identifier: &'static str,
    pub sync_rule: SyncPolicy,
    pub check: FieldCheck,
}

impl PlayerStateWireField {
    pub const fn is_mirrored_to_peers(&self) -> bool {
        matches!(self.sync_rule, SyncPolicy::Synced)
    }

    pub const fn passed_verification(&self) -> bool {
        !matches!(self.check, FieldCheck::InstallWithoutCompare(_))
    }
}

pub const PLAYER_WIRE_FIELD_TOTAL: usize = PLAYER_STATE_WIRE_FIELDS.len();

/// The first field the prediction got wrong.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FieldDrift {
    pub name: &'static str,
    pub slot: usize,
    pub gap: f32,
}

/// Names of every table field in which the two states differ.
pub fn player_state_diff(a: &AvatarState, b: &AvatarState) -> Vec<&'static str> {
    (0..PLAYER_WIRE_FIELD_TOTAL)
        .filter(|&index| value_changed(a, b, index))
        .map(|index| PLAYER_STATE_WIRE_FIELDS[index].code_identifier)
        .collect()
}

/// The first validated field, in table order, where `guessed` is further from `hosted`
/// than its rule allows; `None` when the prediction held.
pub fn avatar_drift(guessed: &AvatarState, hosted: &AvatarState) -> Option<FieldDrift> {
    PLAYER_STATE_WIRE_FIELDS.iter().enumerate().find_map(|(index, row)| {
        let wrong = match row.check {
            FieldCheck::InstallWithoutCompare(_) => return None,
            FieldCheck::Exact => value_changed(guessed, hosted, index),
            FieldCheck::AllowedSlack { slack, .. } => {
                value_gap(guessed, hosted, index) > slack
            }
        };
        wrong.then(|| FieldDrift {
            name: row.code_identifier,
            slot: index,
            gap: value_gap(guessed, hosted, index),
        })
    })
}

/// Every adopt-only field with the reason the client cannot predict it.
pub fn adopted_field_names() -> Vec<(&'static str, &'static str)> {
    PLAYER_STATE_WIRE_FIELDS
        .iter()
        .filter_map(|row| match row.check {
            FieldCheck::InstallWithoutCompare(reason) => Some((row.code_identifier, reason)),
            _ => None,
        })
        .collect()
}

/// A hash of the validated fields of every listed player, in the order given.
pub fn players_hash(players: &[(sim::ClientId, AvatarState)]) -> u32 {
    let mut writer = ByteSinkWriter::new();
    for (peer, avatar) in players {
        writer.put_u32(peer.0);
        for (index, row) in PLAYER_STATE_WIRE_FIELDS.iter().enumerate() {
            if row.passed_verification() {
                store_value(&mut writer, avatar, index);
            }
        }
    }
    hash_fowler_noll_32(&writer.into_bytes())
}

/// 32-bit FNV-1a.
fn hash_fowler_noll_32(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811c_9dc5u32, |hash, byte| {
        (hash ^ u32::from(*byte)).wrapping_mul(0x0100_0193)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_origin_x(x: f32) -> AvatarState {
        let mut ps = AvatarState::ZERO;
        ps.origin[0] = x;
        ps
    }

    #[test]
    fn fnv1a_matches_the_published_vectors() {
        assert_eq!(hash_fowler_noll_32(b""), 0x811c_9dc5);
        assert_eq!(hash_fowler_noll_32(b"a"), 0xe40c_292c);
        assert_eq!(hash_fowler_noll_32(b"foobar"), 0xbf9c_f968);
    }

    #[test]
    fn the_table_is_all_replicated_and_the_count_matches() {
        assert_eq!(PLAYER_WIRE_FIELD_TOTAL, PLAYER_STATE_WIRE_FIELDS.len());
        assert!(PLAYER_STATE_WIRE_FIELDS.iter().all(PlayerStateWireField::is_mirrored_to_peers));
    }

    #[test]
    fn adopt_only_fields_are_the_unvalidated_ones_with_their_reasons() {
        let adopted = adopted_field_names();
        let names: Vec<_> = adopted.iter().map(|(name, _)| *name).collect();
        assert_eq!(
            names,
            [
                "damage_event",
                "damage_yaw",
                "damage_pitch",
                "hurt_amount",
                "health",
                "hint_kind",
                "hint_text",
                "hint_entity",
                "cursor_hint_dual_wield",
            ]
        );
        assert!(adopted.iter().all(|(_, reason)| !reason.is_empty()));
        let unvalidated = PLAYER_STATE_WIRE_FIELDS.iter().filter(|row| !row.passed_verification()).count();
        assert_eq!(unvalidated, adopted.len());
    }

    #[test]
    fn identical_states_have_no_diff_and_no_deviation() {
        let ps = with_origin_x(4.0);
        assert!(player_state_diff(&ps, &ps).is_empty());
        assert_eq!(avatar_drift(&ps, &ps), None);
    }

    #[test]
    fn the_diff_names_each_differing_field() {
        let mut other = with_origin_x(4.0);
        other.move_timer = 9;
        assert_eq!(
            player_state_diff(&AvatarState::ZERO, &other),
            ["move_timer", "origin"]
        );
    }

    #[test]
    fn the_deviation_is_the_first_wrong_field_with_its_distance() {
        let mut other = with_origin_x(4.0);
        other.move_timer = 9;
        let deviation = avatar_drift(&AvatarState::ZERO, &other).expect("they differ");
        assert_eq!(deviation.name, "move_timer");
        assert_eq!(deviation.gap, 9.0);
        assert_eq!(PLAYER_STATE_WIRE_FIELDS[deviation.slot].code_identifier, "move_timer");
    }

    #[test]
    fn adopt_only_fields_never_count_as_a_deviation() {
        let mut other = AvatarState::ZERO;
        other.health = 50;
        other.damage_event = 3;
        assert_eq!(avatar_drift(&AvatarState::ZERO, &other), None);
        assert_eq!(player_state_diff(&AvatarState::ZERO, &other), ["damage_event", "health"]);
    }

    #[test]
    fn the_hash_ignores_adopt_only_fields_but_sees_the_rest() {
        let client = sim::ClientId(3);
        let base = [(client, AvatarState::ZERO)];
        let mut adopted = AvatarState::ZERO;
        adopted.health = 1;
        let mut validated = AvatarState::ZERO;
        validated.move_timer = 1;
        let hash = players_hash(&base);
        assert_eq!(players_hash(&[(client, adopted)]), hash);
        assert_ne!(players_hash(&[(client, validated)]), hash);
        assert_ne!(players_hash(&[(sim::ClientId(4), AvatarState::ZERO)]), hash);
    }
}
