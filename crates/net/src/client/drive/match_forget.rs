//! Forgetting a match. When one is ended down the client drops everything it learned from it, so
//! the next match starts from the same blank state as the first.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use frame::GameSessionClosed;

use super::gun_pick::WeaponSelect;
use super::outbox::QueuedOutgoingPlayerMessages;
use super::stores::{
    ClientGuessState, PlayerGuaranteedReceipt, NewestTakenSnapshot, PendingDebris, QueuedPelletEffect,
    StagedShownEntityEvents, ArrivedStepQueue, OtherPlayerSmootherHolder, GuaranteedSimEvent,
};
use super::ticker::PlayerTimeStepper;
use super::uplink::UplinkPacer;
use crate::host_tick::peer_mail::PlayerActionIntake;
use crate::client::frame_clock::{FrameClock, GameActive, GameJoinCensus};
use crate::client::presentation::entity_slots::EntitySpawnTally;
use crate::client::presentation::shown_frame::{OwnViewerTally, ShownWorldFrame};

/// Everything about the connection: sign-on, what is queued to send, what was acknowledged.
#[derive(SystemParam)]
pub struct LinkMemory<'w> {
    join_rec: ResMut<'w, crate::JoinProgressRecord>,
    admission: ResMut<'w, crate::PlayerEntryTicket>,
    sends: ResMut<'w, QueuedOutgoingPlayerMessages>,
    pacer: ResMut<'w, UplinkPacer>,
    confirm_receipt: ResMut<'w, PlayerGuaranteedReceipt>,
    held_events: ResMut<'w, Messages<GuaranteedSimEvent>>,
    received: ResMut<'w, ArrivedStepQueue>,
    actions: Option<ResMut<'w, PlayerActionIntake>>,
}

impl LinkMemory<'_> {
    fn forget(&mut self) {
        *self.join_rec = crate::JoinProgressRecord::default();
        *self.admission = crate::PlayerEntryTicket::default();
        *self.sends = QueuedOutgoingPlayerMessages::default();
        *self.pacer = UplinkPacer::default();
        *self.confirm_receipt = PlayerGuaranteedReceipt::default();
        self.held_events.clear();
        self.received.0.clear();
        if let Some(queue) = self.actions.as_mut() {
            queue.clear();
        }
    }
}

/// What the client knows about the world: clocks, snapshots, what is drawn.
#[derive(SystemParam)]
pub struct WorldMemory<'w> {
    frame_clock: ResMut<'w, FrameClock>,
    client_clock: ResMut<'w, PlayerTimeStepper>,
    active: ResMut<'w, GameActive>,
    join: ResMut<'w, GameJoinCensus>,
    adopted: ResMut<'w, NewestTakenSnapshot>,
    forecast: ResMut<'w, ClientGuessState>,
    proxy: ResMut<'w, OtherPlayerSmootherHolder>,
    shown: ResMut<'w, ShownWorldFrame>,
    census: ResMut<'w, OwnViewerTally>,
    select: ResMut<'w, WeaponSelect>,
    board: ResMut<'w, crate::Scoreboard>,
    births: Option<ResMut<'w, EntitySpawnTally>>,
}

impl WorldMemory<'_> {
    fn forget(&mut self) {
        self.frame_clock.restart_blank();
        *self.client_clock = PlayerTimeStepper::default();
        self.active.0 = false;
        self.join.clear();
        *self.adopted = NewestTakenSnapshot::default();
        self.forecast.0.stand_down();
        *self.proxy = OtherPlayerSmootherHolder::default();
        self.shown.clear();
        *self.census = OwnViewerTally::default();
        *self.select = WeaponSelect::default();
        *self.board = crate::Scoreboard::default();
        if let Some(births) = self.births.as_mut() {
            **births = EntitySpawnTally::default();
        }
    }
}

/// Effects and events that were waiting to be played.
#[derive(SystemParam)]
pub struct EffectMemory<'w> {
    notes: ResMut<'w, StagedShownEntityEvents>,
    cursor: ResMut<'w, crate::EntityEventReadPos>,
    pellets: ResMut<'w, QueuedPelletEffect>,
    debris: ResMut<'w, PendingDebris>,
}

impl EffectMemory<'_> {
    fn forget(&mut self) {
        *self.notes = StagedShownEntityEvents::default();
        *self.cursor = crate::EntityEventReadPos::default();
        self.pellets.0.clear();
        self.debris.0.clear();
    }
}

pub fn wipe_presentation_after_match_end(
    mut ended: MessageReader<GameSessionClosed>,
    mut link: LinkMemory,
    mut world: WorldMemory,
    mut effects: EffectMemory,
) {
    if ended.read().count() == 0 {
        return;
    }
    link.forget();
    world.forget();
    effects.forget();
    perf::view_freeze_mark("no_next", 0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::run_ahead::InputCommandNumber;
    use frame::{GameSessionStamp, ShutdownCause, LevelGeneration};
    use playerstate_t6::game::InputCommand;

    fn app() -> App {
        let mut app = App::new();
        app.add_message::<GameSessionClosed>()
            .add_message::<GuaranteedSimEvent>()
            .init_resource::<crate::JoinProgressRecord>()
            .init_resource::<crate::PlayerEntryTicket>()
            .init_resource::<QueuedOutgoingPlayerMessages>()
            .init_resource::<UplinkPacer>()
            .init_resource::<PlayerGuaranteedReceipt>()
            .init_resource::<ArrivedStepQueue>()
            .init_resource::<PlayerActionIntake>()
            .init_resource::<FrameClock>()
            .init_resource::<PlayerTimeStepper>()
            .init_resource::<GameActive>()
            .init_resource::<GameJoinCensus>()
            .init_resource::<NewestTakenSnapshot>()
            .init_resource::<ClientGuessState>()
            .init_resource::<OtherPlayerSmootherHolder>()
            .init_resource::<ShownWorldFrame>()
            .init_resource::<OwnViewerTally>()
            .init_resource::<WeaponSelect>()
            .init_resource::<crate::Scoreboard>()
            .init_resource::<EntitySpawnTally>()
            .init_resource::<StagedShownEntityEvents>()
            .init_resource::<crate::EntityEventReadPos>()
            .init_resource::<QueuedPelletEffect>()
            .init_resource::<PendingDebris>()
            .add_systems(Update, wipe_presentation_after_match_end);
        app
    }

    /// Puts something into every store that the teardown is meant to empty.
    fn dirty(app: &mut App) {
        let world = app.world_mut();
        world.resource_mut::<crate::JoinProgressRecord>().admitted = true;
        world.resource_mut::<QueuedOutgoingPlayerMessages>().enqueue(
            InputCommandNumber(4),
            InputCommand::default(),
            Default::default(),
        );
        world.resource_mut::<UplinkPacer>().note_sent(900, Some(InputCommandNumber(4)), 7);
        world.resource_mut::<PlayerGuaranteedReceipt>().0 = 12;
        world.write_message(GuaranteedSimEvent(sim::MatchOccurrence::WeaponSwitchRequested { weapon: 12 }));
        world.resource_mut::<FrameClock>().set_host_ms(5000);
        world.resource_mut::<PlayerTimeStepper>().carryover_millis = 33.0;
        world.resource_mut::<GameActive>().0 = true;
        world.resource_mut::<GameJoinCensus>().note_join(10, 2, 30);
        world.resource_mut::<NewestTakenSnapshot>().taken_in_current_step = true;
        world.resource_mut::<OwnViewerTally>().picked_view = Some("predicted");
        world.resource_mut::<WeaponSelect>().index = 9;
        world.resource_mut::<crate::Scoreboard>().command_text = Some("scores".into());
        world.resource_mut::<EntitySpawnTally>().receiver_spawn_count = 3;
        world.resource_mut::<StagedShownEntityEvents>().replaying_final_death = true;
        world.resource_mut::<QueuedPelletEffect>().0.push(Default::default());
        world.resource_mut::<PendingDebris>().0.push(Default::default());
    }

    fn tear_down(app: &mut App) {
        app.world_mut().write_message(GameSessionClosed {
            reason: ShutdownCause::Disconnect,
            level_generation: LevelGeneration(None),
            game_session_key: GameSessionStamp::NONE,
            game_generation: 1,
        });
    }

    fn assert_blank(app: &App) {
        let world = app.world();
        assert!(!world.resource::<crate::JoinProgressRecord>().admitted);
        assert!(world.resource::<QueuedOutgoingPlayerMessages>().is_empty());
        assert_eq!(world.resource::<PlayerGuaranteedReceipt>().0, 0);
        assert!(world.resource::<Messages<GuaranteedSimEvent>>().is_empty());
        assert_eq!(*world.resource::<FrameClock>(), FrameClock::default());
        assert_eq!(world.resource::<PlayerTimeStepper>().carryover_millis, 0.0);
        assert!(!world.resource::<GameActive>().0);
        assert_eq!(*world.resource::<GameJoinCensus>(), GameJoinCensus::default());
        assert!(!world.resource::<NewestTakenSnapshot>().taken_in_current_step);
        assert_eq!(world.resource::<OwnViewerTally>().picked_view, None);
        assert_eq!(*world.resource::<WeaponSelect>(), WeaponSelect::default());
        assert_eq!(world.resource::<crate::Scoreboard>().command_text, None);
        assert_eq!(world.resource::<EntitySpawnTally>().receiver_spawn_count, 0);
        assert!(!world.resource::<StagedShownEntityEvents>().replaying_final_death);
        assert!(world.resource::<QueuedPelletEffect>().0.is_empty());
        assert!(world.resource::<PendingDebris>().0.is_empty());
    }

    #[test]
    fn a_torn_down_match_leaves_nothing_behind() {
        let mut app = app();
        dirty(&mut app);
        tear_down(&mut app);
        app.update();
        assert_blank(&app);
    }

    #[test]
    fn the_send_pacer_starts_over_too() {
        let mut app = app();
        dirty(&mut app);
        tear_down(&mut app);
        app.update();
        let pacer = app.world().resource::<UplinkPacer>();
        assert!(pacer.packet_due(0, Some(InputCommandNumber(1))));
        assert!(pacer.next_packet(&QueuedOutgoingPlayerMessages::default()).0.is_empty());
    }

    #[test]
    fn nothing_is_forgotten_without_a_teardown() {
        let mut app = app();
        dirty(&mut app);
        app.update();
        let world = app.world();
        assert!(world.resource::<crate::JoinProgressRecord>().admitted);
        assert_eq!(world.resource::<QueuedOutgoingPlayerMessages>().len(), 1);
        assert_eq!(world.resource::<PlayerGuaranteedReceipt>().0, 12);
        assert!(world.resource::<GameActive>().0);
        assert_eq!(world.resource::<WeaponSelect>().index, 9);
    }

    #[test]
    fn a_teardown_is_acted_on_once() {
        let mut app = app();
        tear_down(&mut app);
        app.update();
        dirty(&mut app);
        app.update();
        assert!(app.world().resource::<GameActive>().0);
    }

    #[test]
    fn the_stores_that_may_be_missing_are_not_required() {
        let mut app = app();
        app.world_mut().remove_resource::<PlayerActionIntake>();
        app.world_mut().remove_resource::<EntitySpawnTally>();
        tear_down(&mut app);
        app.update();
    }
}
