//! Forgetting a match. When one is torn down the client drops everything it learned from it, so
//! the next match starts from the same blank state as the first.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use frame::MatchTornDown;

use super::gun_pick::WeaponSelect;
use super::outbox::PendingClientSends;
use super::stores::{
    ClientPredictionState, ClientReliableAck, LastAdoptedSnapshot, PendingDebris, PendingPelletFx,
    PendingPresentedEntityEvents, ReceivedTicks, RemoteProxyState, ReliableControlEvent,
};
use super::ticker::ClientClock;
use super::uplink::UplinkPacer;
use crate::authority::inbox::ClientActionInbox;
use crate::client::frame_clock::{FrameClock, GameActive, GameJoinCensus};
use crate::client::presentation::entities::CEntityBirthCensus;
use crate::client::presentation::presented::{PresentLocalCensus, PresentedSnapshot};

/// Everything about the connection: sign-on, what is queued to send, what was acknowledged.
#[derive(SystemParam)]
pub struct LinkMemory<'w> {
    signon: ResMut<'w, crate::SignonState>,
    admission: ResMut<'w, crate::ClientAdmission>,
    sends: ResMut<'w, PendingClientSends>,
    pacer: ResMut<'w, UplinkPacer>,
    ack: ResMut<'w, ClientReliableAck>,
    reliable: ResMut<'w, Messages<ReliableControlEvent>>,
    received: ResMut<'w, ReceivedTicks>,
    actions: Option<ResMut<'w, ClientActionInbox>>,
}

impl LinkMemory<'_> {
    fn forget(&mut self) {
        *self.signon = crate::SignonState::default();
        *self.admission = crate::ClientAdmission::default();
        *self.sends = PendingClientSends::default();
        *self.pacer = UplinkPacer::default();
        *self.ack = ClientReliableAck::default();
        self.reliable.clear();
        self.received.0.clear();
        if let Some(actions) = self.actions.as_mut() {
            actions.clear();
        }
    }
}

/// What the client knows about the world: clocks, snapshots, what is drawn.
#[derive(SystemParam)]
pub struct WorldMemory<'w> {
    frame_clock: ResMut<'w, FrameClock>,
    client_clock: ResMut<'w, ClientClock>,
    active: ResMut<'w, GameActive>,
    join: ResMut<'w, GameJoinCensus>,
    adopted: ResMut<'w, LastAdoptedSnapshot>,
    prediction: ResMut<'w, ClientPredictionState>,
    proxy: ResMut<'w, RemoteProxyState>,
    presented: ResMut<'w, PresentedSnapshot>,
    census: ResMut<'w, PresentLocalCensus>,
    select: ResMut<'w, WeaponSelect>,
    scores: ResMut<'w, crate::Scoreboard>,
    births: Option<ResMut<'w, CEntityBirthCensus>>,
}

impl WorldMemory<'_> {
    fn forget(&mut self) {
        self.frame_clock.reset();
        *self.client_clock = ClientClock::default();
        self.active.0 = false;
        self.join.clear();
        *self.adopted = LastAdoptedSnapshot::default();
        self.prediction.0.disarm();
        *self.proxy = RemoteProxyState::default();
        self.presented.clear();
        *self.census = PresentLocalCensus::default();
        *self.select = WeaponSelect::default();
        *self.scores = crate::Scoreboard::default();
        if let Some(births) = self.births.as_mut() {
            **births = CEntityBirthCensus::default();
        }
    }
}

/// Effects and events that were waiting to be played.
#[derive(SystemParam)]
pub struct EffectMemory<'w> {
    events: ResMut<'w, PendingPresentedEntityEvents>,
    cursor: ResMut<'w, crate::EntityEventCursor>,
    pellets: ResMut<'w, PendingPelletFx>,
    debris: ResMut<'w, PendingDebris>,
}

impl EffectMemory<'_> {
    fn forget(&mut self) {
        *self.events = PendingPresentedEntityEvents::default();
        *self.cursor = crate::EntityEventCursor::default();
        self.pellets.0.clear();
        self.debris.0.clear();
    }
}

pub fn reset_cgame_on_match_torn_down(
    mut torn: MessageReader<MatchTornDown>,
    mut link: LinkMemory,
    mut world: WorldMemory,
    mut effects: EffectMemory,
) {
    if torn.read().count() == 0 {
        return;
    }
    link.forget();
    world.forget();
    effects.forget();
    perf::cgame_hold("no_next", 0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::predict::CmdSeq;
    use frame::{MatchKey, TeardownReason, WorldGeneration};
    use playerstate_t6::game::UserCmd;

    fn app() -> App {
        let mut app = App::new();
        app.add_message::<MatchTornDown>()
            .add_message::<ReliableControlEvent>()
            .init_resource::<crate::SignonState>()
            .init_resource::<crate::ClientAdmission>()
            .init_resource::<PendingClientSends>()
            .init_resource::<UplinkPacer>()
            .init_resource::<ClientReliableAck>()
            .init_resource::<ReceivedTicks>()
            .init_resource::<ClientActionInbox>()
            .init_resource::<FrameClock>()
            .init_resource::<ClientClock>()
            .init_resource::<GameActive>()
            .init_resource::<GameJoinCensus>()
            .init_resource::<LastAdoptedSnapshot>()
            .init_resource::<ClientPredictionState>()
            .init_resource::<RemoteProxyState>()
            .init_resource::<PresentedSnapshot>()
            .init_resource::<PresentLocalCensus>()
            .init_resource::<WeaponSelect>()
            .init_resource::<crate::Scoreboard>()
            .init_resource::<CEntityBirthCensus>()
            .init_resource::<PendingPresentedEntityEvents>()
            .init_resource::<crate::EntityEventCursor>()
            .init_resource::<PendingPelletFx>()
            .init_resource::<PendingDebris>()
            .add_systems(Update, reset_cgame_on_match_torn_down);
        app
    }

    /// Puts something into every store that the teardown is meant to empty.
    fn dirty(app: &mut App) {
        let world = app.world_mut();
        world.resource_mut::<crate::SignonState>().admitted = true;
        world.resource_mut::<PendingClientSends>().push(
            CmdSeq(4),
            UserCmd::default(),
            Default::default(),
        );
        world.resource_mut::<UplinkPacer>().note_sent(900, Some(CmdSeq(4)), 7);
        world.resource_mut::<ClientReliableAck>().0 = 12;
        world.write_message(ReliableControlEvent(sim::SimEvent::WeaponSwitchRequested { weapon: 12 }));
        world.resource_mut::<FrameClock>().assign_server_time(5000);
        world.resource_mut::<ClientClock>().accumulator_ms = 33.0;
        world.resource_mut::<GameActive>().0 = true;
        world.resource_mut::<GameJoinCensus>().latch(10, 2, 30);
        world.resource_mut::<LastAdoptedSnapshot>().applied_this_frame = true;
        world.resource_mut::<PresentLocalCensus>().choice = Some("predicted");
        world.resource_mut::<WeaponSelect>().index = 9;
        world.resource_mut::<crate::Scoreboard>().cmd = Some("scores".into());
        world.resource_mut::<CEntityBirthCensus>().client_births = 3;
        world.resource_mut::<PendingPresentedEntityEvents>().in_killcam = true;
        world.resource_mut::<PendingPelletFx>().0.push(Default::default());
        world.resource_mut::<PendingDebris>().0.push(Default::default());
    }

    fn tear_down(app: &mut App) {
        app.world_mut().write_message(MatchTornDown {
            reason: TeardownReason::Disconnect,
            world_generation: WorldGeneration(None),
            match_key: MatchKey::NONE,
            match_epoch: 1,
        });
    }

    fn assert_blank(app: &App) {
        let world = app.world();
        assert!(!world.resource::<crate::SignonState>().admitted);
        assert!(world.resource::<PendingClientSends>().is_empty());
        assert_eq!(world.resource::<ClientReliableAck>().0, 0);
        assert!(world.resource::<Messages<ReliableControlEvent>>().is_empty());
        assert_eq!(*world.resource::<FrameClock>(), FrameClock::default());
        assert_eq!(world.resource::<ClientClock>().accumulator_ms, 0.0);
        assert!(!world.resource::<GameActive>().0);
        assert_eq!(*world.resource::<GameJoinCensus>(), GameJoinCensus::default());
        assert!(!world.resource::<LastAdoptedSnapshot>().applied_this_frame);
        assert_eq!(world.resource::<PresentLocalCensus>().choice, None);
        assert_eq!(*world.resource::<WeaponSelect>(), WeaponSelect::default());
        assert_eq!(world.resource::<crate::Scoreboard>().cmd, None);
        assert_eq!(world.resource::<CEntityBirthCensus>().client_births, 0);
        assert!(!world.resource::<PendingPresentedEntityEvents>().in_killcam);
        assert!(world.resource::<PendingPelletFx>().0.is_empty());
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
        assert!(pacer.packet_due(0, Some(CmdSeq(1))));
        assert!(pacer.next_packet(&PendingClientSends::default()).0.is_empty());
    }

    #[test]
    fn nothing_is_forgotten_without_a_teardown() {
        let mut app = app();
        dirty(&mut app);
        app.update();
        let world = app.world();
        assert!(world.resource::<crate::SignonState>().admitted);
        assert_eq!(world.resource::<PendingClientSends>().len(), 1);
        assert_eq!(world.resource::<ClientReliableAck>().0, 12);
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
        app.world_mut().remove_resource::<ClientActionInbox>();
        app.world_mut().remove_resource::<CEntityBirthCensus>();
        tear_down(&mut app);
        app.update();
    }
}
