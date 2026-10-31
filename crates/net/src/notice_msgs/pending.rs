//! The server's list of notices for the frame being built.

use bevy::prelude::Resource;
use sim::ClientId;

use super::{HOST_MSG_LEAVE_ALERT, HOST_MSG_TEXT_LINE, SVC_PRINT_BOLD, HostGameAnnouncement};

/// Longest text a notice field can carry: its length is one byte on the wire.
const FIELD_MAX: usize = u8::MAX as usize;

#[derive(Resource, Debug, Default)]
pub struct StagedAnnouncements {
    id_counter: u32,
    /// Notices and who gets each; `None` means everyone.
    outbox: Vec<(Option<ClientId>, HostGameAnnouncement)>,
}

/// Cut `text` to at most [`FIELD_MAX`] bytes without splitting a character.
fn clip(mut text: String) -> String {
    if text.len() > FIELD_MAX {
        let mut end = FIELD_MAX;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
}

impl StagedAnnouncements {
    /// Tell everyone `leaver` left the game.
    pub fn enqueue_departure(&mut self, leaver: String) {
        self.stage(None, HOST_MSG_LEAVE_ALERT, leaver, hud_t6::gamemsg::LEFT_GAME_NOTICE.to_owned());
    }

    /// Take the printed lines the world queued.
    pub fn absorb_sim_queue(&mut self, game: &mut sim::SimWorld) {
        for print in game.take_pending_prints() {
            let tag = if print.bold { SVC_PRINT_BOLD } else { HOST_MSG_TEXT_LINE };
            self.stage(print.recipient, tag, print.arg, print.template);
        }
    }

    fn stage(&mut self, recipient: Option<ClientId>, tag: u8, shown_name: String, message_key: String) {
        let fresh = self.id_counter;
        self.id_counter = fresh.wrapping_add(1);
        let notice = HostGameAnnouncement { id: fresh, tag, name: clip(shown_name), key: clip(message_key) };
        self.outbox.push((recipient, notice));
    }

    /// The notices `listener` should see, its own and everyone's, in order. Nothing is removed.
    pub fn hand_to(&self, listener: ClientId) -> Vec<HostGameAnnouncement> {
        self.outbox
            .iter()
            .filter(|(recipient, _)| recipient.is_none_or(|only| only == listener))
            .map(|(_, notice)| notice.clone())
            .collect()
    }

    /// Drop everything once every client has been given its share.
    pub fn reset_post_broadcast(&mut self) {
        self.outbox.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.outbox.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn private_row(pending: &mut StagedAnnouncements, only: u32, field_text: &str) {
        pending.stage(Some(ClientId(only)), HOST_MSG_TEXT_LINE, field_text.to_owned(), "key".to_owned());
    }

    #[test]
    fn leaving_goes_to_everyone_with_the_left_game_key() {
        let mut pending = StagedAnnouncements::default();
        pending.enqueue_departure("Ada".to_owned());
        for viewer in [ClientId(0), ClientId(7)] {
            let seen = pending.hand_to(viewer);
            assert_eq!(seen.len(), 1);
            assert_eq!(seen[0].tag, HOST_MSG_LEAVE_ALERT);
            assert_eq!(seen[0].name, "Ada");
            assert_eq!(seen[0].key, hud_t6::gamemsg::LEFT_GAME_NOTICE);
        }
    }

    #[test]
    fn ids_count_up_from_zero_across_notices() {
        let mut pending = StagedAnnouncements::default();
        pending.enqueue_departure("a".to_owned());
        pending.enqueue_departure("b".to_owned());
        let ids: Vec<_> = pending.hand_to(ClientId(0)).iter().map(|n| n.id).collect();
        assert_eq!(ids, [0, 1]);
    }

    #[test]
    fn the_id_counter_wraps() {
        let mut pending = StagedAnnouncements { id_counter: u32::MAX, ..Default::default() };
        pending.enqueue_departure("a".to_owned());
        pending.enqueue_departure("b".to_owned());
        let ids: Vec<_> = pending.hand_to(ClientId(0)).iter().map(|n| n.id).collect();
        assert_eq!(ids, [u32::MAX, 0]);
    }

    #[test]
    fn a_private_notice_reaches_only_its_client() {
        let mut pending = StagedAnnouncements::default();
        private_row(&mut pending, 3, "for three");
        pending.enqueue_departure("for all".to_owned());
        let three: Vec<_> = pending.hand_to(ClientId(3)).into_iter().map(|n| n.name).collect();
        let four: Vec<_> = pending.hand_to(ClientId(4)).into_iter().map(|n| n.name).collect();
        assert_eq!(three, ["for three", "for all"]);
        assert_eq!(four, ["for all"]);
    }

    #[test]
    fn taking_does_not_remove_and_clearing_does() {
        let mut pending = StagedAnnouncements::default();
        assert!(pending.is_empty());
        pending.enqueue_departure("a".to_owned());
        assert_eq!(pending.hand_to(ClientId(0)).len(), 1);
        assert_eq!(pending.hand_to(ClientId(0)).len(), 1);
        pending.reset_post_broadcast();
        assert!(pending.is_empty());
        assert!(pending.hand_to(ClientId(0)).is_empty());
    }

    #[test]
    fn long_text_is_cut_to_255_bytes() {
        assert_eq!(clip("x".repeat(300)).len(), 255);
        assert_eq!(clip("x".repeat(255)).len(), 255);
        assert_eq!(clip("short".to_owned()), "short");
    }

    #[test]
    fn a_cut_never_splits_a_character() {
        // 254 ASCII bytes then a 2-byte character straddling the limit.
        let text = format!("{}é", "a".repeat(254));
        let clipped = clip(text);
        assert_eq!(clipped.len(), 254);
        assert!(clipped.chars().all(|c| c == 'a'));
    }

    #[test]
    fn a_queued_notice_has_its_fields_clipped() {
        let mut pending = StagedAnnouncements::default();
        pending.enqueue_departure("n".repeat(400));
        assert_eq!(pending.hand_to(ClientId(0))[0].name.len(), 255);
    }
}
