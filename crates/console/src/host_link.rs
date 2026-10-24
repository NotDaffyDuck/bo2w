//! What a command needs to ask the host for something: who the local player
//! is, whether the host allows cheats, and the host's queue of requests.

use bevy::{ecs::system::SystemParam, prelude::*};
use net::{ActionRequestIds, AuthorityWorld, ClientActionInbox, LocalPresentClient};
use sim::{ActionRequestId, ClientAction, ClientId, SimWorld};

#[derive(SystemParam)]
pub(crate) struct HostLink<'w> {
    local: Res<'w, LocalPresentClient>,
    authority: Option<ResMut<'w, AuthorityWorld>>,
    inbox: Option<ResMut<'w, ClientActionInbox>>,
    ids: ResMut<'w, ActionRequestIds>,
}

impl HostLink<'_> {
    pub(crate) fn client(&self) -> ClientId {
        self.local.0
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
        action: impl FnOnce(ActionRequestId) -> ClientAction,
    ) -> Result<ActionRequestId, String> {
        let Some(inbox) = self.inbox.as_deref_mut() else {
            return Err(format!("{verb}: no action inbox (not a listen host)"));
        };
        let request_id = self.ids.allocate();
        inbox
            .push(self.local.0, action(request_id))
            .map_err(|why| format!("{verb}: {why}"))?;
        Ok(request_id)
    }
}

/// An app with the resources a link reads, nothing queued, cheats off, and no
/// host world unless asked for.
#[cfg(test)]
pub(crate) mod testing {
    use net::{MAX_PENDING_ACTIONS_PER_CLIENT, PresentedSnapshot};
    use playerstate_t6::game::PlayerState;
    use sim::{ClientLifecycle, ClientSnapshotMeta, MatchBootstrap, Snapshot, Tick};

    use super::*;

    pub(crate) const FULL: usize = MAX_PENDING_ACTIONS_PER_CLIENT;

    pub(crate) fn host_app() -> App {
        let mut app = crate::feature_dispatch::testing::console_app();
        app.init_resource::<LocalPresentClient>()
            .init_resource::<ActionRequestIds>()
            .init_resource::<ClientActionInbox>();
        app
    }

    /// Makes the app the host of a world that allows cheats or not.
    pub(crate) fn host_world(app: &mut App, cheats: bool) {
        let mut world = AuthorityWorld::default();
        world
            .0
            .bootstrap(MatchBootstrap {
                allow_debug_actions: cheats,
                ..MatchBootstrap::default()
            })
            .expect("a fresh world takes a bootstrap");
        app.insert_resource(world);
    }

    /// Shows the local player to the screen as standing at `origin` looking
    /// along `angles`, alive or not, with a name.
    pub(crate) fn present_player(
        app: &mut App,
        origin: [f32; 3],
        angles: [f32; 3],
        alive: bool,
        name: &str,
    ) {
        let state = PlayerState {
            origin,
            viewangles: angles,
            view_height_current: 60.0,
            ground_entity_num: 5,
            ..PlayerState::ZERO
        };
        present(app, state, alive, name);
    }

    /// Shows the local player alive with `weapon` in hand.
    pub(crate) fn present_holding(app: &mut App, weapon: u32) {
        let state = PlayerState {
            weapon,
            ..PlayerState::ZERO
        };
        present(app, state, true, "holder");
    }

    fn present(app: &mut App, state: PlayerState, alive: bool, name: &str) {
        let client = app.world().resource::<LocalPresentClient>().0;
        let mut snapshot = Snapshot::unpublished(Tick(1));
        snapshot.players.push((client, state));
        snapshot.meta.clients.push((
            client,
            ClientSnapshotMeta {
                lifecycle: if alive {
                    ClientLifecycle::Alive
                } else {
                    ClientLifecycle::default()
                },
                name: entity_t6::client_state::strncpyz::<16>(name),
                ..ClientSnapshotMeta::default()
            },
        ));
        let mut presented = PresentedSnapshot::default();
        presented.publish_decoded(snapshot);
        app.insert_resource(presented);
    }

    /// What the local player has asked of the host and not had answered.
    pub(crate) fn queued(app: &App) -> Vec<ClientAction> {
        let local = app.world().resource::<LocalPresentClient>().0;
        app.world()
            .resource::<ClientActionInbox>()
            .pending_for(local)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use bevy::ecs::system::RunSystemOnce;

    use super::testing::{FULL, host_app, host_world, queued};
    use super::*;

    fn ask(app: &mut App, verb: &str) -> Result<ActionRequestId, String> {
        let verb = verb.to_owned();
        app.world_mut()
            .run_system_once(move |mut link: HostLink| {
                link.queue(&verb, |request_id| ClientAction::ForceDeath { request_id })
            })
            .expect("the link's resources are there")
    }

    #[test]
    fn requests_are_numbered_from_one_and_queued_for_the_local_player() {
        let mut app = host_app();
        assert_eq!(ask(&mut app, "kill"), Ok(1));
        assert_eq!(ask(&mut app, "kill"), Ok(2));
        assert_eq!(
            queued(&app),
            [
                ClientAction::ForceDeath { request_id: 1 },
                ClientAction::ForceDeath { request_id: 2 }
            ]
        );
    }

    #[test]
    fn a_full_queue_and_a_missing_one_are_said_in_the_commands_name() {
        let mut app = host_app();
        for _ in 0..FULL {
            ask(&mut app, "kill").unwrap();
        }
        assert_eq!(
            ask(&mut app, "kill"),
            Err("kill: action queue full (32 unresolved requests)".into())
        );

        let mut app = crate::feature_dispatch::testing::console_app();
        app.init_resource::<LocalPresentClient>()
            .init_resource::<ActionRequestIds>();
        assert_eq!(
            ask(&mut app, "kill"),
            Err("kill: no action inbox (not a listen host)".into())
        );
    }

    fn cheats(app: &mut App) -> Result<(), String> {
        app.world_mut()
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
        assert_eq!(
            refused.as_deref(),
            Some("nudge: no authority world (not a listen host)")
        );
        host_world(&mut app, true);
        let allowed = app
            .world_mut()
            .run_system_once(|mut link: HostLink| link.host_mut("nudge").is_ok())
            .unwrap();
        assert!(allowed);
    }
}
