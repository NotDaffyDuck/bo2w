//! Telling the match what the player is called.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use frame::PlayerPrefsBundle;
use frame::LevelPresent;
use frame::ProcessDuty;
use frame::LevelGeneration;
use net::{ActionRequestCounter, PlayerActionIntake, HostViewClient, PlayerUdpSession};
use sim::ClientId;

/// Whether there is a match to name ourselves in, and which client we are.
#[derive(SystemParam)]
pub(crate) struct Presence<'w> {
    generation: Res<'w, LevelGeneration>,
    level_is_up: Res<'w, LevelPresent>,
    duty: Res<'w, ProcessDuty>,
    viewer: Option<Res<'w, HostViewClient>>,
    link: Option<Res<'w, PlayerUdpSession>>,
}

impl Presence<'_> {
    /// The client to name, once it is really in the match: a replay has no
    /// player, and a network client must have been assigned its slot.
    fn client(&self) -> Option<ClientId> {
        if !self.level_is_up.0 || *self.duty == ProcessDuty::Replay {
            return None;
        }
        let local = self.viewer.as_deref()?.0;
        if let Some(link) = self.link.as_deref()
            && (link.connection.is_none()
                || link.granted_player_slot != Some(local)
                || !link.is_inside_match())
        {
            return None;
        }
        Some(local)
    }
}

/// Asks the match to rename the local client whenever its name, the match or
/// the client changes, and not again until one of those does.
pub(crate) fn rename_local_player(
    presence: Presence,
    settings: Res<PlayerPrefsBundle>,
    inbox: Option<ResMut<PlayerActionIntake>>,
    mut ids: ResMut<ActionRequestCounter>,
    mut sent: Local<Option<(LevelGeneration, ClientId, [u8; 16])>>,
) {
    let Some(client) = presence.client() else {
        *sent = None;
        return;
    };
    let Some(mut inbox) = inbox else { return };
    let name = entity_t6::client_info::strncpyz::<16>(&settings.shown_name());
    let this = (*presence.generation, client, name);
    if *sent == Some(this) {
        return;
    }
    let serial = ids.allocate();
    match inbox.push(client, sim::PlayerIntent::SetName { request_id: serial, name }) {
        Ok(()) => *sent = Some(this),
        Err(why) => diag::warn!(
            Console,
            "name: request_id={serial} not queued — {why}"
        ),
    }
}

#[cfg(test)]
mod tests {
    use sim::PlayerIntent;

    use crate::prefs::name::*;

    const US: ClientId = ClientId(3);

    fn name_app() -> App {
        let mut app = App::default();
        app.init_resource::<PlayerPrefsBundle>()
            .init_resource::<ActionRequestCounter>()
            .init_resource::<PlayerActionIntake>()
            .insert_resource(LevelGeneration(Some(1)))
            .insert_resource(LevelPresent(true))
            .insert_resource(ProcessDuty::Listen)
            .insert_resource(HostViewClient(US))
            .add_systems(bevy::app::Update, rename_local_player);
        app.world_mut().resource_mut::<PlayerPrefsBundle>().shown_name = "Sam".into();
        app
    }

    fn rename(app: &mut App, called: &str) {
        app.world_mut().resource_mut::<PlayerPrefsBundle>().shown_name = called.into();
    }

    /// The names asked for so far, as the sim will read them.
    fn asked(app: &App) -> Vec<[u8; 16]> {
        app.world()
            .resource::<PlayerActionIntake>()
            .pending()
            .iter()
            .filter_map(|(who, intent)| match intent {
                PlayerIntent::SetName { name, .. } if *who == US => Some(*name),
                _ => None,
            })
            .collect::<Vec<[u8; 16]>>()
    }

    fn name_of(text: &str) -> [u8; 16] {
        entity_t6::client_info::strncpyz::<16>(text)
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
        app.insert_resource(LevelGeneration(Some(2)));
        app.update();
        assert_eq!(asked(&app).len(), 2);
    }

    #[test]
    fn leaving_the_match_forgets_it_so_the_next_one_is_told() {
        let mut app = name_app();
        app.update();
        app.insert_resource(LevelPresent(false));
        app.update();
        app.insert_resource(LevelPresent(true));
        app.update();
        assert_eq!(asked(&app).len(), 2);
    }

    #[test]
    fn a_replay_has_no_player_to_name() {
        let mut app = name_app();
        app.insert_resource(ProcessDuty::Replay);
        app.update();
        assert!(asked(&app).is_empty());
    }

    #[test]
    fn nothing_is_asked_before_a_world_exists() {
        let mut app = name_app();
        app.insert_resource(LevelPresent(false));
        app.update();
        assert!(asked(&app).is_empty());
    }

    #[test]
    fn nothing_is_asked_without_a_local_client() {
        let mut app = name_app();
        app.world_mut().remove_resource::<HostViewClient>();
        app.update();
        assert!(asked(&app).is_empty());
    }

    #[test]
    fn nothing_is_asked_without_an_inbox() {
        let mut app = name_app();
        app.world_mut().remove_resource::<PlayerActionIntake>();
        app.update();
        app.world_mut().init_resource::<PlayerActionIntake>();
        app.update();
        assert_eq!(asked(&app), [name_of("Sam")], "asked once the inbox exists");
    }
}
