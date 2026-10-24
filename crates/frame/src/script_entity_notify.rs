//! Per-entity events the game scripts raise about a player.

use bevy::prelude::*;

macro_rules! entity_events {
    ($($(#[$meta:meta])* $name:ident;)+) => {
        $(
            $(#[$meta])*
            #[derive(EntityEvent, Clone, Copy, Debug, PartialEq, Eq)]
            pub struct $name {
                pub entity: Entity,
            }
        )+
    };
}

entity_events! {
    /// A kill cam starts for the player.
    BeginKillcam;
    /// The player spawned into the match.
    SpawnedPlayer;
    /// The kill cam was cut short.
    AbortKillcam;
    /// The kill cam ran to its end.
    KillcamEnded;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Resource, Default)]
    struct Seen(Vec<(&'static str, Entity)>);

    #[test]
    fn each_event_reaches_an_observer_on_its_entity() {
        let mut world = World::new();
        world.init_resource::<Seen>();
        let player = world.spawn_empty().id();
        world
            .entity_mut(player)
            .observe(|event: On<BeginKillcam>, mut seen: ResMut<Seen>| {
                seen.0.push(("begin", event.entity));
            });
        world
            .entity_mut(player)
            .observe(|event: On<KillcamEnded>, mut seen: ResMut<Seen>| {
                seen.0.push(("ended", event.entity));
            });

        world.trigger(BeginKillcam { entity: player });
        world.trigger(SpawnedPlayer { entity: player });
        world.trigger(KillcamEnded { entity: player });

        assert_eq!(
            world.resource::<Seen>().0,
            [("begin", player), ("ended", player)]
        );
    }

    #[test]
    fn events_compare_by_entity() {
        let mut world = World::new();
        let (a, b) = (world.spawn_empty().id(), world.spawn_empty().id());
        assert_eq!(AbortKillcam { entity: a }, AbortKillcam { entity: a });
        assert_ne!(AbortKillcam { entity: a }, AbortKillcam { entity: b });
    }
}
