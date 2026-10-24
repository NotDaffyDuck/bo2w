//! Telling the match what the player is called.

use bevy::{ecs::system::SystemParam, prelude::*};
use frame::{GameSettings, HasWorld, RuntimeRole, WorldGeneration};
use net::{ActionRequestIds, ClientActionInbox, LocalPresentClient, UdpClientLink};
use sim::ClientId;

/// Whether there is a match to name ourselves in, and which client we are.
#[derive(SystemParam)]
pub(crate) struct Presence<'w> {
    generation: Res<'w, WorldGeneration>,
    has_world: Res<'w, HasWorld>,
    role: Res<'w, RuntimeRole>,
    local: Option<Res<'w, LocalPresentClient>>,
    link: Option<Res<'w, UdpClientLink>>,
}

impl Presence<'_> {
    /// The client to name, once it is really in the match: a replay has no
    /// player, and a network client must have been assigned its slot.
    fn client(&self) -> Option<ClientId> {
        if !self.has_world.0 || *self.role == RuntimeRole::Replay {
            return None;
        }
        let local = self.local.as_deref()?.0;
        if let Some(link) = self.link.as_deref()
            && (link.connection.is_none()
                || link.assigned_client != Some(local)
                || !link.has_entered_match())
        {
            return None;
        }
        Some(local)
    }
}

/// Asks the match to rename the local client whenever its name, the match or
/// the client changes, and not again until one of those does.
pub(crate) fn sync_player_name(
    presence: Presence,
    settings: Res<GameSettings>,
    inbox: Option<ResMut<ClientActionInbox>>,
    mut ids: ResMut<ActionRequestIds>,
    mut sent: Local<Option<(WorldGeneration, ClientId, [u8; 16])>>,
) {
    let Some(client) = presence.client() else {
        *sent = None;
        return;
    };
    let Some(mut inbox) = inbox else { return };
    let name = entity_t6::client_state::strncpyz::<16>(&settings.player_name);
    let this = (*presence.generation, client, name);
    if *sent == Some(this) {
        return;
    }
    let request_id = ids.allocate();
    match inbox.push(client, sim::ClientAction::SetName { request_id, name }) {
        Ok(()) => *sent = Some(this),
        Err(error) => diag::warn!(
            Console,
            "name: request_id={request_id} not queued — {error}"
        ),
    }
}

#[cfg(test)]
mod tests {
    use sim::ClientAction;

    use super::*;

    const US: ClientId = ClientId(3);

    fn name_app() -> App {
        let mut app = App::new();
        app.init_resource::<GameSettings>()
            .init_resource::<ActionRequestIds>()
            .init_resource::<ClientActionInbox>()
            .insert_resource(WorldGeneration(Some(1)))
            .insert_resource(HasWorld(true))
            .insert_resource(RuntimeRole::Listen)
            .insert_resource(LocalPresentClient(US))
            .add_systems(Update, sync_player_name);
        app.world_mut().resource_mut::<GameSettings>().player_name = "Sam".into();
        app
    }

    fn rename(app: &mut App, name: &str) {
        app.world_mut().resource_mut::<GameSettings>().player_name = name.into();
    }

    /// The names asked for so far, as the sim will read them.
    fn asked(app: &App) -> Vec<[u8; 16]> {
        app.world()
            .resource::<ClientActionInbox>()
            .pending()
            .iter()
            .filter_map(|(client, action)| match action {
                ClientAction::SetName { name, .. } if *client == US => Some(*name),
                _ => None,
            })
            .collect()
    }

    fn name_of(text: &str) -> [u8; 16] {
        entity_t6::client_state::strncpyz::<16>(text)
    }

    #[test]
    fn the_name_is_asked_for_once() {
        let mut app = name_app();
        for _ in 0..3 {
            app.update();
        }
        assert_eq!(asked(&app), [name_of("Sam")]);
    }

    #[test]
    fn a_new_name_is_asked_for_again() {
        let mut app = name_app();
        app.update();
        rename(&mut app, "Alex");
        app.update();
        assert_eq!(asked(&app), [name_of("Sam"), name_of("Alex")]);
    }

    #[test]
    fn a_new_match_is_told_the_name_again() {
        let mut app = name_app();
        app.update();
        app.insert_resource(WorldGeneration(Some(2)));
        app.update();
        assert_eq!(asked(&app).len(), 2);
    }

    #[test]
    fn leaving_the_match_forgets_it_so_the_next_one_is_told() {
        let mut app = name_app();
        app.update();
        app.insert_resource(HasWorld(false));
        app.update();
        app.insert_resource(HasWorld(true));
        app.update();
        assert_eq!(asked(&app).len(), 2);
    }

    #[test]
    fn a_replay_has_no_player_to_name() {
        let mut app = name_app();
        app.insert_resource(RuntimeRole::Replay);
        app.update();
        assert!(asked(&app).is_empty());
    }

    #[test]
    fn nothing_is_asked_before_a_world_exists() {
        let mut app = name_app();
        app.insert_resource(HasWorld(false));
        app.update();
        assert!(asked(&app).is_empty());
    }

    #[test]
    fn nothing_is_asked_without_a_local_client() {
        let mut app = name_app();
        app.world_mut().remove_resource::<LocalPresentClient>();
        app.update();
        assert!(asked(&app).is_empty());
    }

    #[test]
    fn nothing_is_asked_without_an_inbox() {
        let mut app = name_app();
        app.world_mut().remove_resource::<ClientActionInbox>();
        app.update();
        app.world_mut().init_resource::<ClientActionInbox>();
        app.update();
        assert_eq!(asked(&app), [name_of("Sam")], "asked once the inbox exists");
    }
}
