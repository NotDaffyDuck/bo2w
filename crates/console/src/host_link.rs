//! What a command needs to ask the host for something: who the local player
//! is, whether the host allows cheats, and the host's queue of requests.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use net::{ActionRequestCounter, HostSimulationWorld, PlayerActionIntake, HostViewClient};
use sim::{IntentSerial, PlayerIntent};
use sim::ClientId;
use sim::SimWorld;

#[derive(SystemParam)]
pub(crate) struct HostLink<'w> {
    viewer: Res<'w, HostViewClient>,
    authority: Option<ResMut<'w, HostSimulationWorld>>,
    inbox: Option<ResMut<'w, PlayerActionIntake>>,
    ids: ResMut<'w, ActionRequestCounter>,
}

impl HostLink<'_> {
    pub(crate) fn client(&self) -> ClientId {
        self.viewer.0
    }

    /// The host's world, when this game is the host.
    pub(crate) fn world(&self) -> Option<&SimWorld> {
        self.authority.as_deref().map(|authority| &authority.0)
    }

    /// Fails when the host has cheats off. A game with no host world here is
    /// somebody else's client, and the host it asks decides for itself.
    pub(crate) fn allow_cheats(&self, verb: &str) -> Result<(), String> {
        match self.world() {
            Some(world) if !world.cheats_enabled() => Err(format!("{verb}: cheats are off")),
            _ => Ok(()),
        }
    }

    /// The host's world, for what only the host can answer.
    pub(crate) fn host(&self, verb: &str) -> Result<&SimWorld, String> {
        self.world()
            .ok_or_else(|| format!("{verb}: no authority world (not a listen host)"))
    }

    /// The host's world, for a change only the host can make.
    pub(crate) fn host_mut(&mut self, verb: &str) -> Result<&mut SimWorld, String> {
        match self.authority.as_deref_mut() {
            Some(authority) => Ok(&mut authority.0),
            None => Err(format!("{verb}: no authority world (not a listen host)")),
        }
    }

    /// Asks the host for the action the next request number names, for the
    /// local player.
    pub(crate) fn queue(
        &mut self,
        verb: &str,
        action: impl FnOnce(IntentSerial) -> PlayerIntent,
    ) -> Result<IntentSerial, String> {
        let Some(inbox) = self.inbox.as_deref_mut() else {
            return Err(format!("{verb}: no action inbox (not a listen host)"));
        };
        let request_id = self.ids.allocate();
        inbox
            .push(self.viewer.0, action(request_id))
            .map(|()| request_id)
            .map_err(|why| format!("{verb}: {why}"))
    }
}

/// An app with the resources a link reads, nothing queued, cheats off, and no
/// host world unless asked for.
#[cfg(test)]
pub(crate) mod testing {
    use crate::tool_cmds::testing::console_app;
    use net::{PLAYER_ACTION_QUEUE_LIMIT, ShownWorldFrame};
    use playerstate_t6::game::AvatarState;
    use sim::{MatchStartSetup, PlayerSessionStage};

    use crate::host_link::*;
    use sim::Snapshot;
    use sim::Tick;
    use sim::ClientSnapshotMeta;

    pub(crate) const FULL: usize = PLAYER_ACTION_QUEUE_LIMIT;

    pub(crate) fn host_app() -> App {
        let mut app = console_app();
        app.init_resource::<HostViewClient>()
            .init_resource::<ActionRequestCounter>()
            .init_resource::<PlayerActionIntake>();
        app
    }

    /// Makes the app the host of a world that allows cheats or not.
    pub(crate) fn host_world(app: &mut App, cheats: bool) {
        let mut world = HostSimulationWorld::default();
        world
            .0
            .bootstrap(MatchStartSetup {
                permit_cheat_requests: cheats,
                ..MatchStartSetup::default()
            })
            .expect("a fresh world takes a bootstrap");
        app.insert_resource(world);
    }

    /// Shows the local player to the screen as standing at `origin` looking
    /// along `angles`, alive or not, with a name.
    pub(crate) fn present_player(
        app: &mut App,
        at: [f32; 3],
        look: [f32; 3],
        living: bool,
        name: &str,
    ) {
        let state = AvatarState {
            origin: at,
            look_angles: look,
            eye_height_now: 60.0,
            standing_on: 5,
            ..AvatarState::ZERO
        };
        present(app, state, living, name);
    }

    /// Shows the local player alive with `held` in hand.
    pub(crate) fn present_holding(app: &mut App, held: u32) {
        let state = AvatarState {
            weapon: held,
            ..AvatarState::ZERO
        };
        present(app, state, true, "holder");
    }

    fn present(app: &mut App, state: AvatarState, standing: bool, called: &str) {
        let client = app.world().resource::<HostViewClient>().0;
        let mut snapshot = Snapshot::unpublished(Tick(1));
        snapshot.players.push((client, state));
        let lifecycle = if standing {
            PlayerSessionStage::Alive
        } else {
            PlayerSessionStage::default()
        };
        let meta = sim::ClientSnapshotMeta {
            lifecycle,
            name: entity_t6::client_info::strncpyz::<16>(called),
            ..ClientSnapshotMeta::default()
        };
        snapshot.meta.clients.push((client, meta));
        let mut presented = ShownWorldFrame::default();
        presented.announce_parsed(snapshot);
        app.insert_resource(presented);
    }

    /// What the local player has asked of the host and not had answered.
    pub(crate) fn queued(app: &App) -> Vec<PlayerIntent> {
        let local = app.world().resource::<HostViewClient>().0;
        app.world()
            .resource::<PlayerActionIntake>()
            .awaiting_from(local)
            .collect::<Vec<PlayerIntent>>()
    }
}

#[cfg(test)]
mod tests {
    use bevy::ecs::system::RunSystemOnce;

    use super::testing::{FULL, host_app, host_world, queued};
    use crate::tool_cmds::testing::console_app;
    use crate::host_link::*;

    fn ask(app: &mut App, verb: &str) -> Result<IntentSerial, String> {
        let verb = verb.to_owned();
        let world = app.world_mut();
        world
            .run_system_once(move |mut link: HostLink| {
                link.queue(&verb, |request_id| PlayerIntent::ForceDeath { request_id })
            })
            .expect("the link's resources are there")
    }

    #[test]
    fn requests_are_numbered_from_one_and_queued_for_the_local_player() {
        let mut app = host_app();
        assert_eq!(ask(&mut app, "kill"), Ok(1));
        assert_eq!(ask(&mut app, "kill"), Ok(2));
        let expected = [
            PlayerIntent::ForceDeath { request_id: 1 },
            PlayerIntent::ForceDeath { request_id: 2 }
        ];
        assert_eq!(queued(&app), expected);
    }

    #[test]
    fn a_full_queue_and_a_missing_one_are_said_in_the_commands_name() {
        let mut app = host_app();
        for _ in 0..FULL {
            ask(&mut app, "kill").unwrap();
        }
        let actual = ask(&mut app, "kill");
        assert_eq!(actual, Err("kill: action queue full (32 unresolved requests)".into()));

        let mut app = console_app();
        app.init_resource::<HostViewClient>()
            .init_resource::<ActionRequestCounter>();
        let actual = ask(&mut app, "kill");
        assert_eq!(actual, Err("kill: no action inbox (not a listen host)".into()));
    }

    fn cheats(app: &mut App) -> Result<(), String> {
        let world = app.world_mut();
        world
            .run_system_once(|link: HostLink| link.allow_cheats("damage"))
            .expect("the link's resources are there")
    }

    #[test]
    fn cheats_are_refused_only_by_a_host_that_has_them_off() {
        let mut app = host_app();
        assert_eq!(cheats(&mut app), Ok(()), "no host world here");
        host_world(&mut app, false);
        assert_eq!(cheats(&mut app), Err("damage: cheats are off".into()));
        host_world(&mut app, true);
        assert_eq!(cheats(&mut app), Ok(()));
    }

    #[test]
    fn a_change_only_the_host_can_make_needs_the_host_world() {
        let mut app = host_app();
        let refused = app
            .world_mut()
            .run_system_once(|mut link: HostLink| link.host_mut("nudge").err())
            .unwrap();
        let actual = refused.as_deref();
        assert_eq!(actual, Some("nudge: no authority world (not a listen host)"));
        host_world(&mut app, true);
        let allowed = app
            .world_mut()
            .run_system_once(|mut link: HostLink| link.host_mut("nudge").is_ok())
            .unwrap();
        assert!(allowed);
    }
}
