//! Reliable messages from the host: sounds, card slots, menus, splashes, notices, scoreboard
//! text, forced weapon switches and the answers to the player's own action requests.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use super::gun_pick::WeaponSelect;
use super::stores::{PlayerGuaranteedReceipt, GuaranteedSimEvent};
use crate::host_tick::peer_mail::PlayerActionIntake;
use crate::client::frame_clock::FrameClock;
use crate::net_links::tick_frames::Frame;
use crate::{
    ActionRulingNotice, AbortStep, DirectoryLink, GuaranteedEnvelope, GuaranteedEntry, Scoreboard, JoinFailureCause,
    JoinSequenceStep, JoinProgressRecord, HostCardSlotMsg, CardSlotCommand, HostGameAnnouncement, HostScreenFlash,
    HostLocalSoundMsg, HostMenuOpenMsg, MenuOpenCommand, HostSoundCue,
};

/// The five message streams the host's small commands turn into on this side.
#[derive(SystemParam)]
pub struct HostFrameMessageSinks<'w> {
    sounds: MessageWriter<'w, HostLocalSoundMsg>,
    cards: MessageWriter<'w, CardSlotCommand>,
    menus: MessageWriter<'w, MenuOpenCommand>,
    splashes: MessageWriter<'w, HostScreenFlash>,
    notices: MessageWriter<'w, HostGameAnnouncement>,
}

impl HostFrameMessageSinks<'_> {
    pub(super) fn sound(&mut self, cmd: &HostSoundCue) {
        self.sounds.write(HostLocalSoundMsg { stop: cmd.halt, index: cmd.slot });
    }

    pub(super) fn card(&mut self, cmd: &HostCardSlotMsg) {
        self.cards.write(CardSlotCommand { client: cmd.client, card_slot: cmd.card_slot });
    }

    pub(super) fn menu(&mut self, cmd: &HostMenuOpenMsg) {
        self.menus.write(MenuOpenCommand {
            config_slot_number: cmd.config_slot_number,
        });
    }

    pub(super) fn splash(&mut self, cmd: HostScreenFlash) {
        self.splashes.write(cmd);
    }

    pub(super) fn notice(&mut self, cmd: HostGameAnnouncement) {
        self.notices.write(cmd);
    }
}

/// Everything a received reliable payload can touch.
#[derive(SystemParam)]
pub struct GuaranteedIntakeParams<'w> {
    intake_q: ResMut<'w, PlayerActionIntake>,
    confirm_receipt: ResMut<'w, PlayerGuaranteedReceipt>,
    notes: MessageWriter<'w, GuaranteedSimEvent>,
    svc: HostFrameMessageSinks<'w>,
    board: ResMut<'w, Scoreboard>,
    join_rec: ResMut<'w, JoinProgressRecord>,
    dir_link: Option<Res<'w, DirectoryLink>>,
}

/// Whether sequence number `a` comes after `b` when the 16-bit counter may have wrapped.
fn is_newer_seq(x: u16, y: u16) -> bool {
    // Ahead by 1..=0x7fff steps, read as a signed 16-bit gap.
    (x.wrapping_sub(y) as i16) > 0
}

fn first_failure(payload: &GuaranteedEnvelope) -> Option<&str> {
    payload.entries.iter().find_map(|(_, row)| match row {
        GuaranteedEntry::RefusalText(why) => Some(why.as_str()),
        _ => None,
    })
}

impl GuaranteedIntakeParams<'_> {
    /// Replaces the scoreboard with the text the host sent.
    pub(super) fn store_scores(&mut self, text: String) {
        self.board.parsed = crate::read_board_command(&text);
        self.board.command_text = Some(text);
    }

    /// Passes on the small commands that rode along with one tick: sounds, card slots,
    /// splashes, notices and menus become messages, the scoreboard text is stored.
    pub(super) fn take_frame_commands(&mut self, frame: Frame) {
        for cmd in &frame.host_sound_triggers {
            self.svc.sound(cmd);
        }
        for cmd in &frame.host_nameplate_entries {
            self.svc.card(cmd);
        }
        for cmd in frame.flash_rows {
            self.svc.splash(cmd);
        }
        for cmd in frame.alert_rows {
            self.svc.notice(cmd);
        }
        for cmd in &frame.host_menu_openings {
            self.svc.menu(cmd);
        }
        if let Some(text) = frame.board_text {
            self.store_scores(text);
        }
    }

    /// Ends the connection: drops what was still waiting on an answer, tells the master link and
    /// puts the sign-on into its failed phase.
    fn abort(&mut self, why: &str) {
        self.intake_q.clear();
        let link = self.dir_link.as_deref();
        let cause = JoinFailureCause::Transport {
            source: String::from(why),
            stage: AbortStep::Transport,
            game_session_key: crate::join_walk::current_round_key(link),
        };
        link.inspect(|dir_link| dir_link.mark_failed(why));
        self.join_rec.enter_phase(JoinSequenceStep::Failed(cause));
    }

    /// Takes in a payload: a host that lost rows or reported a failure ends the connection,
    /// otherwise each row not seen before is acted on in sequence order and acknowledged.
    pub(super) fn apply(&mut self, own_id: sim::ClientId, payload: &GuaranteedEnvelope) {
        if payload.evicted_stale_count != 0 {
            self.abort("ReliableControlOverflow: outcome unknown");
            return;
        }
        if let Some(why) = first_failure(payload) {
            self.abort(why);
            return;
        }

        let acked_at_start = self.confirm_receipt.0;
        let mut entries: Vec<&(u16, GuaranteedEntry)> = payload.entries.iter().collect();
        entries.sort_by_key(|(seq, _)| seq.wrapping_sub(acked_at_start));
        for &(seq, ref row) in entries {
            let fresh = is_newer_seq(seq, self.confirm_receipt.0);
            if fresh && !self.take_row(own_id, row) {
                return;
            }
            if fresh {
                self.confirm_receipt.0 = seq;
            }
        }
    }

    /// Acts on one row; `false` means the row ended the connection.
    fn take_row(&mut self, own_id: sim::ClientId, row: &GuaranteedEntry) -> bool {
        match row {
            GuaranteedEntry::RefusalText(why) => {
                self.abort(why);
                return false;
            }
            GuaranteedEntry::Sound(cmd) => self.svc.sound(cmd),
            GuaranteedEntry::Card(cmd) => self.svc.card(cmd),
            GuaranteedEntry::Menu(cmd) => self.svc.menu(cmd),
            GuaranteedEntry::Splash(cmd) => self.svc.splash(cmd.clone()),
            GuaranteedEntry::Notify(cmd) => self.svc.notice(cmd.clone()),
            GuaranteedEntry::Standings(text) => self.store_scores(text.clone()),
            GuaranteedEntry::Event(event) => {
                self.notes.write(GuaranteedSimEvent(*event));
            }
            GuaranteedEntry::CommandResolution { request_id, verdict } => match verdict {
                ActionRulingNotice::DuplicateIdDifferentBody => {
                    self.abort("ActionPayloadMismatch: connection retired");
                    return false;
                }
                ActionRulingNotice::TimedOut => {
                    self.abort("ActionOutcomeUnknown: request expired");
                    return false;
                }
                ActionRulingNotice::TakenIntoEffect | ActionRulingNotice::Declined => {
                    self.intake_q.resolve(own_id, *request_id);
                }
            },
        }
        true
    }
}

/// A switch the host forced (a streak weapon taken away, a respawn loadout) becomes the
/// request, so the next command asks for the gun that is about to be in hand.
pub fn apply_weapon_switch_requests(
    mut notes: MessageReader<GuaranteedSimEvent>,
    mut select: ResMut<WeaponSelect>,
    clock: Res<FrameClock>,
) {
    for event in notes.read() {
        if let sim::MatchOccurrence::WeaponSwitchRequested { weapon } = event.0 {
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
    struct Delivery(GuaranteedEnvelope);

    fn deliver_it(mut inbound: GuaranteedIntakeParams, delivery: Res<Delivery>) {
        inbound.apply(ME, &delivery.0);
    }

    fn app_with_inbound() -> App {
        let mut app = App::new();
        app.init_resource::<PlayerActionIntake>()
            .init_resource::<PlayerGuaranteedReceipt>()
            .init_resource::<Scoreboard>()
            .init_resource::<JoinProgressRecord>()
            .add_message::<GuaranteedSimEvent>()
            .add_message::<HostLocalSoundMsg>()
            .add_message::<CardSlotCommand>()
            .add_message::<MenuOpenCommand>()
            .add_message::<HostScreenFlash>()
            .add_message::<HostGameAnnouncement>()
            .add_systems(Update, deliver_it);
        app
    }

    fn payload(rows: Vec<(u16, GuaranteedEntry)>) -> GuaranteedEnvelope {
        GuaranteedEnvelope {
            confirmed_up_to: 0,
            entries: rows,
            evicted_stale_count: 0,
        }
    }

    fn deliver(app: &mut App, payload: GuaranteedEnvelope) {
        app.insert_resource(Delivery(payload));
        app.update();
    }

    fn ack_of(app: &App) -> u16 {
        app.world().resource::<PlayerGuaranteedReceipt>().0
    }

    fn sounds_heard(app: &mut App) -> Vec<(bool, u8)> {
        app.world_mut()
            .resource_mut::<Messages<HostLocalSoundMsg>>()
            .drain()
            .map(|sound| (sound.stop, sound.index))
            .collect()
    }

    fn failure_text(app: &App) -> Option<String> {
        match &app.world().resource::<JoinProgressRecord>().phase {
            JoinSequenceStep::Failed(JoinFailureCause::Transport {
                source,
                stage: AbortStep::Transport,
                ..
            }) => Some(source.clone()),
            _ => None,
        }
    }

    fn sound_row(seq: u16, index: u8) -> (u16, GuaranteedEntry) {
        (seq, GuaranteedEntry::Sound(HostSoundCue { halt: false, slot: index }))
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
        app.insert_resource(PlayerGuaranteedReceipt(u16::MAX - 1));
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
                (1, GuaranteedEntry::Card(HostCardSlotMsg { client: 7, card_slot: -2 })),
                (2, GuaranteedEntry::Menu(HostMenuOpenMsg { config_slot_number: 41 })),
                (
                    3,
                    GuaranteedEntry::Splash(HostScreenFlash {
                        key: "kills".into(),
                        card_slot: 1,
                        optional: 0,
                    }),
                ),
            ]),
        );
        let world = app.world_mut();
        let cards: Vec<_> = world.resource_mut::<Messages<CardSlotCommand>>().drain().collect();
        assert_eq!(cards, [CardSlotCommand { client: 7, card_slot: -2 }]);
        let menus: Vec<_> = world.resource_mut::<Messages<MenuOpenCommand>>().drain().collect();
        assert_eq!(menus, [MenuOpenCommand { config_slot_number: 41 }]);
        let splashes: Vec<_> = world.resource_mut::<Messages<HostScreenFlash>>().drain().collect();
        assert_eq!(splashes.len(), 1);
        assert_eq!(splashes[0].key, "kills");
    }

    #[test]
    fn scoreboard_text_replaces_the_stored_scoreboard() {
        let mut app = app_with_inbound();
        deliver(&mut app, payload(vec![(1, GuaranteedEntry::Standings("first".into()))]));
        deliver(&mut app, payload(vec![(2, GuaranteedEntry::Standings("second".into()))]));
        assert_eq!(app.world().resource::<Scoreboard>().command_text.as_deref(), Some("second"));
    }

    #[test]
    fn a_host_event_is_passed_on_as_a_control_event() {
        let mut app = app_with_inbound();
        deliver(
            &mut app,
            payload(vec![(1, GuaranteedEntry::Event(sim::MatchOccurrence::WeaponSwitchRequested { weapon: 9 }))]),
        );
        let notes: Vec<_> = app
            .world_mut()
            .resource_mut::<Messages<GuaranteedSimEvent>>()
            .drain()
            .collect();
        assert_eq!(notes.len(), 1);
        assert!(matches!(notes[0].0, sim::MatchOccurrence::WeaponSwitchRequested { weapon: 9 }));
    }

    #[test]
    fn a_host_that_dropped_rows_ends_the_connection_without_acting_on_the_payload() {
        let mut app = app_with_inbound();
        let mut lost = payload(vec![sound_row(1, 10)]);
        lost.evicted_stale_count = 2;
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
            payload(vec![sound_row(1, 10), (2, GuaranteedEntry::RefusalText("server said no".into()))]),
        );
        assert_eq!(failure_text(&app).as_deref(), Some("server said no"));
        assert!(sounds_heard(&mut app).is_empty());
        assert_eq!(ack_of(&app), 0);
    }

    #[test]
    fn a_failure_also_forgets_the_requests_still_waiting_for_an_answer() {
        let mut app = app_with_inbound();
        app.world_mut()
            .resource_mut::<PlayerActionIntake>()
            .push(ME, sim::PlayerIntent::JoinMatch { request_id: 5 })
            .unwrap();
        deliver(&mut app, payload(vec![(1, GuaranteedEntry::RefusalText("gone".into()))]));
        assert_eq!(app.world().resource::<PlayerActionIntake>().waiting_count(), 0);
    }

    #[test]
    fn an_applied_or_refused_outcome_retires_the_request_and_is_acknowledged() {
        for verdict in [ActionRulingNotice::TakenIntoEffect, ActionRulingNotice::Declined] {
            let mut app = app_with_inbound();
            let mut inbox = PlayerActionIntake::default();
            inbox.push(ME, sim::PlayerIntent::JoinMatch { request_id: 5 }).unwrap();
            inbox.push(ME, sim::PlayerIntent::SpawnClient { request_id: 6 }).unwrap();
            app.insert_resource(inbox);
            deliver(
                &mut app,
                payload(vec![(1, GuaranteedEntry::CommandResolution { request_id: 5, verdict })]),
            );
            let inbox = app.world().resource::<PlayerActionIntake>();
            assert_eq!(inbox.waiting_count(), 1, "{verdict:?}");
            assert_eq!(ack_of(&app), 1);
            assert!(failure_text(&app).is_none());
        }
    }

    #[test]
    fn an_expired_or_mismatched_outcome_ends_the_connection_and_is_not_acknowledged() {
        let cases = [
            (ActionRulingNotice::TimedOut, "ActionOutcomeUnknown: request expired"),
            (ActionRulingNotice::DuplicateIdDifferentBody, "ActionPayloadMismatch: connection retired"),
        ];
        for (verdict, text) in cases {
            let mut app = app_with_inbound();
            deliver(
                &mut app,
                payload(vec![
                    sound_row(1, 10),
                    (2, GuaranteedEntry::CommandResolution { request_id: 5, verdict }),
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

    fn pass_on(mut inbound: GuaranteedIntakeParams, mut carried: ResMut<Carried>) {
        if let Some(frame) = carried.0.take() {
            inbound.take_frame_commands(frame);
        }
    }

    #[test]
    fn the_commands_riding_on_a_tick_all_arrive() {
        let mut frame = crate::net_links::tick_frames::frame_for_tick(
            &mut crate::FrameDiffWriter::new(),
            &sim::StepInputs::default(),
            &sim::Snapshot::unpublished(sim::Tick(0)),
        );
        frame.host_sound_triggers = vec![HostSoundCue { halt: true, slot: 4 }];
        frame.host_nameplate_entries = vec![HostCardSlotMsg { client: 2, card_slot: 5 }];
        frame.host_menu_openings = vec![HostMenuOpenMsg { config_slot_number: 8 }];
        frame.board_text = Some("scores here".to_owned());

        let mut app = app_with_inbound();
        app.insert_resource(Delivery(payload(Vec::new())));
        app.insert_resource(Carried(Some(frame)));
        app.add_systems(Update, pass_on);
        app.update();

        assert_eq!(sounds_heard(&mut app), [(true, 4)]);
        let world = app.world_mut();
        assert_eq!(world.resource_mut::<Messages<CardSlotCommand>>().drain().count(), 1);
        assert_eq!(world.resource_mut::<Messages<MenuOpenCommand>>().drain().count(), 1);
        assert_eq!(world.resource::<Scoreboard>().command_text.as_deref(), Some("scores here"));
    }

    #[test]
    fn a_forced_weapon_switch_becomes_the_request() {
        let mut app = App::new();
        app.add_message::<GuaranteedSimEvent>()
            .init_resource::<WeaponSelect>()
            .init_resource::<FrameClock>()
            .add_systems(Update, apply_weapon_switch_requests);
        app.world_mut()
            .write_message(GuaranteedSimEvent(sim::MatchOccurrence::WeaponSwitchRequested { weapon: 12 }));
        app.update();
        let select = *app.world().resource::<WeaponSelect>();
        assert_eq!((select.index, select.mapped_index), (12, 12));
        assert_eq!(select.time, app.world().resource::<FrameClock>().time());
    }
}
