//! Reliable messages from the host: sounds, card slots, menus, splashes, notices, scoreboard
//! text, forced weapon switches and the answers to the player's own action requests.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use super::gun_pick::WeaponSelect;
use super::stores::{ClientReliableAck, ReliableControlEvent};
use crate::authority::inbox::ClientActionInbox;
use crate::client::frame_clock::FrameClock;
use crate::transport::frame::Frame;
use crate::{
    ActionVerdict, FailStage, MasterBridge, ReliablePayload, ReliableRow, Scoreboard, SignonFailReason,
    SignonPhase, SignonState, SvcCardSlot, SvcCardSlotCmd, SvcGameNotify, SvcHudSplash,
    SvcLocalSound, SvcOpenMenu, SvcOpenMenuCmd, SvcSound,
};

/// The five message streams the host's small commands turn into on this side.
#[derive(SystemParam)]
pub struct SvcFrameWriters<'w> {
    sounds: MessageWriter<'w, SvcLocalSound>,
    cards: MessageWriter<'w, SvcCardSlotCmd>,
    menus: MessageWriter<'w, SvcOpenMenuCmd>,
    splashes: MessageWriter<'w, SvcHudSplash>,
    notices: MessageWriter<'w, SvcGameNotify>,
}

impl SvcFrameWriters<'_> {
    pub(super) fn sound(&mut self, cmd: &SvcSound) {
        self.sounds.write(SvcLocalSound {
            stop: cmd.stop,
            index: cmd.index,
        });
    }

    pub(super) fn card(&mut self, cmd: &SvcCardSlot) {
        self.cards.write(SvcCardSlotCmd {
            client: cmd.client,
            slot: cmd.slot,
        });
    }

    pub(super) fn menu(&mut self, cmd: &SvcOpenMenu) {
        self.menus.write(SvcOpenMenuCmd {
            cs_index: cmd.cs_index,
        });
    }

    pub(super) fn splash(&mut self, cmd: SvcHudSplash) {
        self.splashes.write(cmd);
    }

    pub(super) fn notice(&mut self, cmd: SvcGameNotify) {
        self.notices.write(cmd);
    }
}

/// Everything a received reliable payload can touch.
#[derive(SystemParam)]
pub struct ReliableInbound<'w> {
    actions: ResMut<'w, ClientActionInbox>,
    ack: ResMut<'w, ClientReliableAck>,
    events: MessageWriter<'w, ReliableControlEvent>,
    svc: SvcFrameWriters<'w>,
    scores: ResMut<'w, Scoreboard>,
    signon: ResMut<'w, SignonState>,
    bridge: Option<Res<'w, MasterBridge>>,
}

/// Whether sequence number `a` comes after `b` when the 16-bit counter may have wrapped.
fn is_newer_seq(a: u16, b: u16) -> bool {
    a != b && a.wrapping_sub(b) < 0x8000
}

fn first_failure(payload: &ReliablePayload) -> Option<&str> {
    payload.rows.iter().find_map(|(_, row)| match row {
        ReliableRow::Failure(reason) => Some(reason.as_str()),
        _ => None,
    })
}

impl ReliableInbound<'_> {
    /// Replaces the scoreboard with the text the host sent.
    pub(super) fn store_scores(&mut self, text: String) {
        self.scores.parsed = crate::parse_scoreboard_cmd(&text);
        self.scores.cmd = Some(text);
    }

    /// Passes on the small commands that rode along with one tick: sounds, card slots,
    /// splashes, notices and menus become messages, the scoreboard text is stored.
    pub(super) fn take_frame_commands(&mut self, frame: Frame) {
        for cmd in &frame.svc_sounds {
            self.svc.sound(cmd);
        }
        for cmd in &frame.svc_card_slots {
            self.svc.card(cmd);
        }
        for cmd in frame.svc_hud_splashes {
            self.svc.splash(cmd);
        }
        for cmd in frame.svc_game_notifies {
            self.svc.notice(cmd);
        }
        for cmd in &frame.svc_open_menus {
            self.svc.menu(cmd);
        }
        if let Some(text) = frame.svc_scores {
            self.store_scores(text);
        }
    }

    /// Ends the connection: drops what was still waiting on an answer, tells the master link and
    /// puts the sign-on into its failed phase.
    fn abort(&mut self, reason: &str) {
        self.actions.clear();
        let match_key = crate::signon::live_match_key(self.bridge.as_deref());
        if let Some(bridge) = &self.bridge {
            bridge.fail(reason);
        }
        self.signon.set_phase(SignonPhase::Failed(SignonFailReason::Transport {
            source: reason.to_owned(),
            stage: FailStage::Transport,
            match_key,
        }));
    }

    /// Takes in a payload: a host that lost rows or reported a failure ends the connection,
    /// otherwise each row not seen before is acted on in sequence order and acknowledged.
    pub(super) fn apply(&mut self, local: sim::ClientId, payload: &ReliablePayload) {
        if payload.dropped_oldest != 0 {
            self.abort("ReliableControlOverflow: outcome unknown");
            return;
        }
        if let Some(reason) = first_failure(payload) {
            self.abort(reason);
            return;
        }

        let acked_at_start = self.ack.0;
        let mut rows: Vec<&(u16, ReliableRow)> = payload.rows.iter().collect();
        rows.sort_by_key(|(seq, _)| seq.wrapping_sub(acked_at_start));
        for (seq, row) in rows {
            if !is_newer_seq(*seq, self.ack.0) {
                continue;
            }
            if !self.take_row(local, row) {
                return;
            }
            self.ack.0 = *seq;
        }
    }

    /// Acts on one row; `false` means the row ended the connection.
    fn take_row(&mut self, local: sim::ClientId, row: &ReliableRow) -> bool {
        match row {
            ReliableRow::Failure(reason) => {
                self.abort(reason);
                return false;
            }
            ReliableRow::Sound(cmd) => self.svc.sound(cmd),
            ReliableRow::Card(cmd) => self.svc.card(cmd),
            ReliableRow::Menu(cmd) => self.svc.menu(cmd),
            ReliableRow::Splash(cmd) => self.svc.splash(cmd.clone()),
            ReliableRow::Notify(cmd) => self.svc.notice(cmd.clone()),
            ReliableRow::Scores(text) => self.store_scores(text.clone()),
            ReliableRow::Event(event) => {
                self.events.write(ReliableControlEvent(*event));
            }
            ReliableRow::ActionOutcome { request_id, verdict } => match verdict {
                ActionVerdict::PayloadMismatch => {
                    self.abort("ActionPayloadMismatch: connection retired");
                    return false;
                }
                ActionVerdict::Expired => {
                    self.abort("ActionOutcomeUnknown: request expired");
                    return false;
                }
                ActionVerdict::Applied | ActionVerdict::Refused => {
                    self.actions.retire(local, *request_id);
                }
            },
        }
        true
    }
}

/// A switch the host forced (a streak weapon taken away, a respawn loadout) becomes the
/// request, so the next command asks for the gun that is about to be in hand.
pub fn apply_weapon_switch_requests(
    mut events: MessageReader<ReliableControlEvent>,
    mut select: ResMut<WeaponSelect>,
    clock: Res<FrameClock>,
) {
    for event in events.read() {
        if let sim::SimEvent::WeaponSwitchRequested { weapon } = event.0 {
            select.index = weapon;
            select.mapped_index = weapon;
            select.time = clock.time();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: sim::ClientId = sim::ClientId(3);

    #[derive(Resource)]
    struct Delivery(ReliablePayload);

    fn deliver_it(mut inbound: ReliableInbound, delivery: Res<Delivery>) {
        inbound.apply(ME, &delivery.0);
    }

    fn app_with_inbound() -> App {
        let mut app = App::new();
        app.init_resource::<ClientActionInbox>()
            .init_resource::<ClientReliableAck>()
            .init_resource::<Scoreboard>()
            .init_resource::<SignonState>()
            .add_message::<ReliableControlEvent>()
            .add_message::<SvcLocalSound>()
            .add_message::<SvcCardSlotCmd>()
            .add_message::<SvcOpenMenuCmd>()
            .add_message::<SvcHudSplash>()
            .add_message::<SvcGameNotify>()
            .add_systems(Update, deliver_it);
        app
    }

    fn payload(rows: Vec<(u16, ReliableRow)>) -> ReliablePayload {
        ReliablePayload {
            ack_through: 0,
            rows,
            dropped_oldest: 0,
        }
    }

    fn deliver(app: &mut App, payload: ReliablePayload) {
        app.insert_resource(Delivery(payload));
        app.update();
    }

    fn ack_of(app: &App) -> u16 {
        app.world().resource::<ClientReliableAck>().0
    }

    fn sounds_heard(app: &mut App) -> Vec<(bool, u8)> {
        app.world_mut()
            .resource_mut::<Messages<SvcLocalSound>>()
            .drain()
            .map(|sound| (sound.stop, sound.index))
            .collect()
    }

    fn failure_text(app: &App) -> Option<String> {
        match &app.world().resource::<SignonState>().phase {
            SignonPhase::Failed(SignonFailReason::Transport {
                source,
                stage: FailStage::Transport,
                ..
            }) => Some(source.clone()),
            _ => None,
        }
    }

    fn sound_row(seq: u16, index: u8) -> (u16, ReliableRow) {
        (seq, ReliableRow::Sound(SvcSound { stop: false, index }))
    }

    #[test]
    fn rows_are_acted_on_once_and_move_the_ack_to_the_last_one() {
        let mut app = app_with_inbound();
        deliver(&mut app, payload(vec![sound_row(1, 10), sound_row(2, 11)]));
        assert_eq!(sounds_heard(&mut app), [(false, 10), (false, 11)]);
        assert_eq!(ack_of(&app), 2);

        deliver(&mut app, payload(vec![sound_row(1, 10), sound_row(2, 11), sound_row(3, 12)]));
        assert_eq!(sounds_heard(&mut app), [(false, 12)]);
        assert_eq!(ack_of(&app), 3);
    }

    #[test]
    fn rows_that_arrive_out_of_order_are_taken_in_sequence_order() {
        let mut app = app_with_inbound();
        deliver(&mut app, payload(vec![sound_row(3, 30), sound_row(1, 10), sound_row(2, 20)]));
        assert_eq!(sounds_heard(&mut app), [(false, 10), (false, 20), (false, 30)]);
        assert_eq!(ack_of(&app), 3);
    }

    #[test]
    fn the_sequence_counter_may_wrap_around() {
        let mut app = app_with_inbound();
        app.insert_resource(ClientReliableAck(u16::MAX - 1));
        deliver(&mut app, payload(vec![sound_row(1, 3), sound_row(u16::MAX, 1), sound_row(0, 2)]));
        assert_eq!(sounds_heard(&mut app), [(false, 1), (false, 2), (false, 3)]);
        assert_eq!(ack_of(&app), 1);
    }

    #[test]
    fn newer_sequence_numbers_are_judged_within_half_the_counter_range() {
        assert!(is_newer_seq(5, 4));
        assert!(!is_newer_seq(4, 4));
        assert!(!is_newer_seq(4, 5));
        assert!(is_newer_seq(0, u16::MAX));
        assert!(is_newer_seq(0x7fff, 0));
        assert!(!is_newer_seq(0x8000, 0));
    }

    #[test]
    fn small_commands_become_their_own_messages() {
        let mut app = app_with_inbound();
        deliver(
            &mut app,
            payload(vec![
                (1, ReliableRow::Card(SvcCardSlot { client: 7, slot: -2 })),
                (2, ReliableRow::Menu(SvcOpenMenu { cs_index: 41 })),
                (
                    3,
                    ReliableRow::Splash(SvcHudSplash {
                        key: "kills".into(),
                        slot: 1,
                        optional: 0,
                    }),
                ),
            ]),
        );
        let world = app.world_mut();
        let cards: Vec<_> = world.resource_mut::<Messages<SvcCardSlotCmd>>().drain().collect();
        assert_eq!(cards, [SvcCardSlotCmd { client: 7, slot: -2 }]);
        let menus: Vec<_> = world.resource_mut::<Messages<SvcOpenMenuCmd>>().drain().collect();
        assert_eq!(menus, [SvcOpenMenuCmd { cs_index: 41 }]);
        let splashes: Vec<_> = world.resource_mut::<Messages<SvcHudSplash>>().drain().collect();
        assert_eq!(splashes.len(), 1);
        assert_eq!(splashes[0].key, "kills");
    }

    #[test]
    fn scoreboard_text_replaces_the_stored_scoreboard() {
        let mut app = app_with_inbound();
        deliver(&mut app, payload(vec![(1, ReliableRow::Scores("first".into()))]));
        deliver(&mut app, payload(vec![(2, ReliableRow::Scores("second".into()))]));
        assert_eq!(app.world().resource::<Scoreboard>().cmd.as_deref(), Some("second"));
    }

    #[test]
    fn a_host_event_is_passed_on_as_a_control_event() {
        let mut app = app_with_inbound();
        deliver(
            &mut app,
            payload(vec![(1, ReliableRow::Event(sim::SimEvent::WeaponSwitchRequested { weapon: 9 }))]),
        );
        let events: Vec<_> = app
            .world_mut()
            .resource_mut::<Messages<ReliableControlEvent>>()
            .drain()
            .collect();
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0].0, sim::SimEvent::WeaponSwitchRequested { weapon: 9 }));
    }

    #[test]
    fn a_host_that_dropped_rows_ends_the_connection_without_acting_on_the_payload() {
        let mut app = app_with_inbound();
        let mut lost = payload(vec![sound_row(1, 10)]);
        lost.dropped_oldest = 2;
        deliver(&mut app, lost);
        assert_eq!(failure_text(&app).as_deref(), Some("ReliableControlOverflow: outcome unknown"));
        assert!(sounds_heard(&mut app).is_empty());
        assert_eq!(ack_of(&app), 0);
    }

    #[test]
    fn a_failure_row_ends_the_connection_before_any_other_row_is_used() {
        let mut app = app_with_inbound();
        deliver(
            &mut app,
            payload(vec![sound_row(1, 10), (2, ReliableRow::Failure("server said no".into()))]),
        );
        assert_eq!(failure_text(&app).as_deref(), Some("server said no"));
        assert!(sounds_heard(&mut app).is_empty());
        assert_eq!(ack_of(&app), 0);
    }

    #[test]
    fn a_failure_also_forgets_the_requests_still_waiting_for_an_answer() {
        let mut app = app_with_inbound();
        app.world_mut()
            .resource_mut::<ClientActionInbox>()
            .push(ME, sim::ClientAction::JoinMatch { request_id: 5 })
            .unwrap();
        deliver(&mut app, payload(vec![(1, ReliableRow::Failure("gone".into()))]));
        assert_eq!(app.world().resource::<ClientActionInbox>().pending_len(), 0);
    }

    #[test]
    fn an_applied_or_refused_outcome_retires_the_request_and_is_acknowledged() {
        for verdict in [ActionVerdict::Applied, ActionVerdict::Refused] {
            let mut app = app_with_inbound();
            let mut inbox = ClientActionInbox::default();
            inbox.push(ME, sim::ClientAction::JoinMatch { request_id: 5 }).unwrap();
            inbox.push(ME, sim::ClientAction::SpawnClient { request_id: 6 }).unwrap();
            app.insert_resource(inbox);
            deliver(
                &mut app,
                payload(vec![(1, ReliableRow::ActionOutcome { request_id: 5, verdict })]),
            );
            let inbox = app.world().resource::<ClientActionInbox>();
            assert_eq!(inbox.pending_len(), 1, "{verdict:?}");
            assert_eq!(ack_of(&app), 1);
            assert!(failure_text(&app).is_none());
        }
    }

    #[test]
    fn an_expired_or_mismatched_outcome_ends_the_connection_and_is_not_acknowledged() {
        let cases = [
            (ActionVerdict::Expired, "ActionOutcomeUnknown: request expired"),
            (ActionVerdict::PayloadMismatch, "ActionPayloadMismatch: connection retired"),
        ];
        for (verdict, text) in cases {
            let mut app = app_with_inbound();
            deliver(
                &mut app,
                payload(vec![
                    sound_row(1, 10),
                    (2, ReliableRow::ActionOutcome { request_id: 5, verdict }),
                    sound_row(3, 12),
                ]),
            );
            assert_eq!(failure_text(&app).as_deref(), Some(text));
            assert_eq!(sounds_heard(&mut app), [(false, 10)]);
            assert_eq!(ack_of(&app), 1);
        }
    }

    #[derive(Resource)]
    struct Carried(Option<Frame>);

    fn pass_on(mut inbound: ReliableInbound, mut carried: ResMut<Carried>) {
        if let Some(frame) = carried.0.take() {
            inbound.take_frame_commands(frame);
        }
    }

    #[test]
    fn the_commands_riding_on_a_tick_all_arrive() {
        let mut frame = crate::transport::frame::frame_from_tick(
            &mut crate::SnapshotEncoder::new(),
            &sim::TickInput::default(),
            &sim::Snapshot::unpublished(sim::Tick(0)),
        );
        frame.svc_sounds = vec![SvcSound { stop: true, index: 4 }];
        frame.svc_card_slots = vec![SvcCardSlot { client: 2, slot: 5 }];
        frame.svc_open_menus = vec![SvcOpenMenu { cs_index: 8 }];
        frame.svc_scores = Some("scores here".to_owned());

        let mut app = app_with_inbound();
        app.insert_resource(Delivery(payload(Vec::new())));
        app.insert_resource(Carried(Some(frame)));
        app.add_systems(Update, pass_on);
        app.update();

        assert_eq!(sounds_heard(&mut app), [(true, 4)]);
        let world = app.world_mut();
        assert_eq!(world.resource_mut::<Messages<SvcCardSlotCmd>>().drain().count(), 1);
        assert_eq!(world.resource_mut::<Messages<SvcOpenMenuCmd>>().drain().count(), 1);
        assert_eq!(world.resource::<Scoreboard>().cmd.as_deref(), Some("scores here"));
    }

    #[test]
    fn a_forced_weapon_switch_becomes_the_request() {
        let mut app = App::new();
        app.add_message::<ReliableControlEvent>()
            .init_resource::<WeaponSelect>()
            .init_resource::<FrameClock>()
            .add_systems(Update, apply_weapon_switch_requests);
        app.world_mut()
            .write_message(ReliableControlEvent(sim::SimEvent::WeaponSwitchRequested { weapon: 12 }));
        app.update();
        let select = *app.world().resource::<WeaponSelect>();
        assert_eq!((select.index, select.mapped_index), (12, 12));
        assert_eq!(select.time, app.world().resource::<FrameClock>().time());
    }
}
