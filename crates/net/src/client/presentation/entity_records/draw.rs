//! Decides, per remote player or corpse, whether its body gets drawn this frame, and why not.

use entity_t6::et::{PLAYER, PLAYER_CORPSE};
use playerstate_t6::game::{BODY_ENTITY_FIRST_SLOT, entity_flag_bits, secondary_state_bits};

use super::pose::{ClientEntityPoseState, DEAD_ENTITY_FLAG};

/// Describes what the viewer looks out of, which tells which body is their own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayerDrawGate {
    pub viewer_entity: i32,
    pub secondary_state_bits: u32,
    pub showing_outside_camera: bool,
}

impl PlayerDrawGate {
    /// Flags marking the view as a player's: alive, or dead in a third-person kill cam.
    pub const SELF_ENTITY_MASK: u32 = secondary_state_bits::PLAYER | secondary_state_bits::DEAD_KILLCAM_TPV;

    /// True when `slot_idx` is the entity the view looks out of.
    pub fn is_participant_camera(self, slot_idx: u16) -> bool {
        let owns_view = self.secondary_state_bits & Self::SELF_ENTITY_MASK != 0;
        owns_view && self.viewer_entity == i32::from(slot_idx)
    }

    /// True for the viewer's own body while seen from inside it; that body is never drawn.
    pub fn omit_self_first_person(self, slot_idx: u16) -> bool {
        if self.showing_outside_camera {
            return false;
        }
        self.is_participant_camera(slot_idx)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OtherPlayerDrawRoute {
    NotInSnap,
    Cadaver,
    Dead,
    SelfFpv,
    /// The bo2mp mod menu's "Invisible": nobody else draws this player.
    Hidden,
    Player,
}

impl OtherPlayerDrawRoute {
    /// Corpses and live players are the only kinds that get drawn.
    pub fn submits(self) -> bool {
        [Self::Cadaver, Self::Player].contains(&self)
    }

    /// How the entity dump spells this kind.
    pub fn trace_dump_key(self) -> &'static str {
        const SPELLINGS: [(OtherPlayerDrawRoute, &str); 6] = [
            (OtherPlayerDrawRoute::Player, "ok"),
            (OtherPlayerDrawRoute::Cadaver, "corpse"),
            (OtherPlayerDrawRoute::Dead, "dead"),
            (OtherPlayerDrawRoute::NotInSnap, "not_in_snap"),
            (OtherPlayerDrawRoute::SelfFpv, "self_fpv"),
            (OtherPlayerDrawRoute::Hidden, "hidden"),
        ];
        SPELLINGS
            .iter()
            .find_map(|&(kind, word)| (kind == self).then_some(word))
            .unwrap_or("ok")
    }
}

/// Works out how entity `slot_idx` is treated. Cadaver slots (`BODY_ENTITY_FIRST_SLOT` and up)
/// only ever draw as corpses. A player slot draws unless it left the snapshot, is dead, is the
/// viewer's own seen from inside, or is hidden from everyone except its owner.
pub fn peer_avatar_submission_class(
    slot_idx: u16,
    runtime: &ClientEntityPoseState,
    gate: PlayerDrawGate,
) -> OtherPlayerDrawRoute {

    if !runtime.present_in_newer_frame() {
        return OtherPlayerDrawRoute::NotInSnap;
    }
    let latest = &runtime.upcoming_condition;
    let typed_as = |wanted: i16| runtime.pose_entity_kind == wanted as u8 || latest.e_type == wanted;
    let flags = latest.lerp.e_flags;

    if i32::from(slot_idx) >= BODY_ENTITY_FIRST_SLOT {
        return match typed_as(PLAYER_CORPSE) {
            true => OtherPlayerDrawRoute::Cadaver,
            false => OtherPlayerDrawRoute::NotInSnap,
        };
    }
    match () {
        _ if flags & DEAD_ENTITY_FLAG != 0 => OtherPlayerDrawRoute::Dead,
        _ if gate.omit_self_first_person(slot_idx) => OtherPlayerDrawRoute::SelfFpv,
        _ if flags & entity_flag_bits::HIDDEN as i32 != 0 && !gate.is_participant_camera(slot_idx) => OtherPlayerDrawRoute::Hidden,
        _ if typed_as(PLAYER) => OtherPlayerDrawRoute::Player,
        _ => OtherPlayerDrawRoute::NotInSnap,
    }
}

pub fn remote_body_draws(slot_idx: u16, runtime: &ClientEntityPoseState, view: PlayerDrawGate) -> bool {
    peer_avatar_submission_class(slot_idx, runtime, view).submits()
}

#[cfg(test)]
mod tests {
    use entity_t6::EntitySnapshot;

    use super::super::pose::restart_body;
    use super::*;

    const NO_VIEWER: PlayerDrawGate = PlayerDrawGate {
        viewer_entity: -1,
        secondary_state_bits: 0,
        showing_outside_camera: false,
    };

    /// A runtime that has just been reset onto an entity of the given type and flags.
    fn live(e_type: i16, flags: i32) -> ClientEntityPoseState {
        let mut entity = EntitySnapshot::default();
        entity.lerp.e_flags = flags;
        entity.e_type = e_type;
        let mut rt = ClientEntityPoseState::default();
        restart_body(&mut rt, entity, 0, true);
        rt
    }

    fn viewer_in(entity: i32, third_person: bool) -> PlayerDrawGate {
        PlayerDrawGate {
            showing_outside_camera: third_person,
            secondary_state_bits: secondary_state_bits::PLAYER,
            viewer_entity: entity,
        }
    }

    #[test]
    fn a_snapshot_player_is_drawn() {
        let rt = live(PLAYER, 0);
        let kind = peer_avatar_submission_class(3, &rt, NO_VIEWER);
        assert!(remote_body_draws(3, &rt, NO_VIEWER));
        assert_eq!((kind, kind.trace_dump_key()), (OtherPlayerDrawRoute::Player, "ok"));
    }

    #[test]
    fn absent_and_non_player_entities_are_not_drawn() {
        let gone = peer_avatar_submission_class(3, &ClientEntityPoseState::default(), NO_VIEWER);
        assert!(!gone.submits());
        assert_eq!((gone, gone.trace_dump_key()), (OtherPlayerDrawRoute::NotInSnap, "not_in_snap"));
        assert_eq!(
            OtherPlayerDrawRoute::NotInSnap,
            peer_avatar_submission_class(3, &live(5, 0), NO_VIEWER)
        );
    }

    #[test]
    fn a_dead_player_is_reported_dead() {
        let kind = peer_avatar_submission_class(3, &live(PLAYER, DEAD_ENTITY_FLAG), NO_VIEWER);
        assert!(!kind.submits());
        assert_eq!((kind, kind.trace_dump_key()), (OtherPlayerDrawRoute::Dead, "dead"));
    }

    #[test]
    fn the_viewers_own_body_is_hidden_from_inside_only() {
        let rt = live(PLAYER, 0);
        let seen = |slot: u16, gate| peer_avatar_submission_class(slot, &rt, gate);
        assert_eq!(seen(3, viewer_in(3, false)), OtherPlayerDrawRoute::SelfFpv);
        assert_eq!(seen(3, viewer_in(3, true)), OtherPlayerDrawRoute::Player);
        // Another player's body shows no matter whose eyes these are.
        assert_eq!(seen(4, viewer_in(3, false)), OtherPlayerDrawRoute::Player);
        // Without either view flag the eyes belong to nobody.
        let neither = PlayerDrawGate {
            secondary_state_bits: 0,
            ..viewer_in(3, false)
        };
        assert!(!neither.is_participant_camera(3));
    }

    #[test]
    fn hidden_players_show_to_their_owner_alone() {
        let rt = live(PLAYER, entity_flag_bits::HIDDEN as i32);
        assert_eq!(peer_avatar_submission_class(3, &rt, NO_VIEWER), OtherPlayerDrawRoute::Hidden);
        assert_eq!(
            peer_avatar_submission_class(3, &rt, viewer_in(3, true)),
            OtherPlayerDrawRoute::Player
        );
    }

    #[test]
    fn corpse_numbers_only_ever_draw_corpses() {
        let slot_idx = BODY_ENTITY_FIRST_SLOT as u16;
        let body = peer_avatar_submission_class(slot_idx, &live(PLAYER_CORPSE, 0), NO_VIEWER);
        assert!(body.submits());
        assert_eq!((body, body.trace_dump_key()), (OtherPlayerDrawRoute::Cadaver, "corpse"));
        assert_eq!(
            OtherPlayerDrawRoute::NotInSnap,
            peer_avatar_submission_class(slot_idx, &live(PLAYER, 0), NO_VIEWER)
        );
    }

    #[test]
    fn dump_tokens_for_the_rest_and_who_submits() {
        let silent = [OtherPlayerDrawRoute::Dead, OtherPlayerDrawRoute::Hidden, OtherPlayerDrawRoute::SelfFpv];
        assert!(silent.iter().all(|kind| !kind.submits()));
        assert_eq!(OtherPlayerDrawRoute::SelfFpv.trace_dump_key(), "self_fpv");
        assert_eq!(OtherPlayerDrawRoute::Hidden.trace_dump_key(), "hidden");
    }
}
