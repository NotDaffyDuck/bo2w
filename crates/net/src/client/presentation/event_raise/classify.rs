//! Turning one event into the Bevy event for its kind of effect.

use bevy::prelude::*;
use entity_t6::{EntityEventResponse, EventCode, EntityEventNotHandled, entity_event_action};

use super::raised_events::*;
use super::unsupported::UnhandledEntityEvents;
use super::walk::InstalledEntityEventTraversal;

/// What a frame's dispatch works with, apart from the events it dispatches.
pub(super) struct Dispatcher<'a, 'w, 's> {
    pub(super) commands: &'a mut Commands<'w, 's>,
    pub(super) local_number: i32,
    pub(super) walk: &'a mut InstalledEntityEventTraversal,
    pub(super) unsupported: &'a mut UnhandledEntityEvents,
}

impl Dispatcher<'_, '_, '_> {
    /// Look up what `event` asks for and raise it on `entity`. `replaying_final_death` is set for events that
    /// come out of the archive being replayed.
    pub(super) fn dispatch(
        &mut self,
        target: Entity,
        event: RoutedEntityEvent,
        from_archive: bool,
    ) {
        let action = match entity_event_action(event.event) {
            Ok(EntityEventResponse::None) => return,
            Ok(action) => Some(action),
            Err(EntityEventNotHandled(kind)) => {
                self.unsupported.note(kind);
                None
            }
        };
        if let Some(action) = action {
            self.raise(action, target, event, from_archive);
        }
        self.walk.handed_on = self.walk.handed_on.saturating_add(1);
    }

    fn raise(
        &mut self,
        kind: EntityEventResponse,
        target: Entity,
        event: RoutedEntityEvent,
        from_archive: bool,
    ) {
        let commands = &mut *self.commands;
        match kind {
            // Nothing to raise; `dispatch` already returned for this one.
            EntityEventResponse::None => {}
            EntityEventResponse::GunShot => {
                let ours = u32::from(event.payload.number == self.local_number);
                self.walk.self_trigger_pull = self.walk.self_trigger_pull.saturating_add(ours);
                commands.trigger(GunFiredAt { entity: target, event });
            }
            EntityEventResponse::Rumble => commands.trigger(EntityRumble {
                entity: target,
                event,
                replaying_final_death: from_archive,
            }),
            EntityEventResponse::Obituary => commands.trigger(EntityKillNotice {
                entity: target,
                event,
                replaying_final_death: from_archive,
            }),
            EntityEventResponse::Sound => commands.trigger(SoundCueAt { entity: target, event }),
            EntityEventResponse::ShellEject => commands.trigger(ShellCaseOut { entity: target, event }),
            EntityEventResponse::BulletHit => commands.trigger(BulletStrike { entity: target, event }),
            EntityEventResponse::GrenadeBounceImpact => commands.trigger(GrenadeBounceAt { entity: target, event }),
            EntityEventResponse::BoltImpact => commands.trigger(EntityBoltImpact { entity: target, event }),
            EntityEventResponse::RemoteMissileBoost => commands.trigger(EntityRemoteMissileBoost { entity: target, event }),
            EntityEventResponse::Earthquake => commands.trigger(EntityEarthquake { entity: target, event }),
            EntityEventResponse::ShieldBlast => commands.trigger(EntityShieldBlast { entity: target, event }),
            EntityEventResponse::FireMelee => commands.trigger(EntityFireMelee { entity: target, event }),
            EntityEventResponse::Explosion => commands.trigger(BlastAt { entity: target, event }),
            EntityEventResponse::PlayFx => commands.trigger(EffectAt { entity: target, event }),
            EntityEventResponse::FootstepNoise => commands.trigger(StepSoundAt { entity: target, event }),
            EntityEventResponse::ResetAds => commands.trigger(EntitySightsLowered { entity: target, event }),
            EntityEventResponse::MeleeGore => commands.trigger(MeleeBloodAt { entity: target, event }),
        }
    }
}

/// Events about no entity the client tracks: when their number names none, they go to the
/// placeholder entity.
pub(super) fn event_without_centity(event: EventCode) -> bool {
    let is_alias = event == EventCode::SOUND_ALIAS;
    is_alias
        || matches!(
            entity_event_action(event),
            Ok(EntityEventResponse::PlayFx | EntityEventResponse::Obituary | EntityEventResponse::Rumble | EntityEventResponse::Earthquake)
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::world::CommandQueue;
    use sim::{EntityEventPayload, EventSequence, Tick};

    #[derive(Resource, Default)]
    struct Heard {
        sounds: u32,
        fires: u32,
    }

    /// A dispatched event of `kind` that happened on entity number `number`.
    fn dispatched(kind: EventCode, number: i32) -> RoutedEntityEvent {
        let payload = EntityEventPayload {
            number,
            ..Default::default()
        };
        RoutedEntityEvent {
            order_no: EventSequence(1),
            tick: Tick(1),
            event: kind,
            payload,
        }
    }

    /// The first event kind whose classification satisfies `wanted`.
    fn find_kind(
        wanted: impl Fn(&Result<EntityEventResponse, EntityEventNotHandled>) -> bool,
    ) -> Option<EventCode> {
        (0..4096)
            .map(EventCode)
            .find(|&kind| wanted(&entity_event_action(kind)))
    }

    struct Rig {
        world: World,
        walk: InstalledEntityEventTraversal,
        unsupported: UnhandledEntityEvents,
    }

    impl Rig {
        fn listening() -> Self {
            let mut world = World::new();
            world.init_resource::<Heard>();
            world.add_observer(|_: On<SoundCueAt>, mut heard: ResMut<Heard>| {
                heard.sounds += 1;
            });
            world.add_observer(|_: On<GunFiredAt>, mut heard: ResMut<Heard>| {
                heard.fires += 1;
            });
            Self {
                world,
                walk: InstalledEntityEventTraversal::default(),
                unsupported: UnhandledEntityEvents::default(),
            }
        }

        /// Dispatch every event on the placeholder entity, with the local player being number 3.
        fn feed(&mut self, events: &[RoutedEntityEvent]) {
            let mut queue = CommandQueue::default();
            let mut commands = Commands::new(&mut queue, &self.world);
            let mut dispatcher = Dispatcher {
                commands: &mut commands,
                local_number: 3,
                walk: &mut self.walk,
                unsupported: &mut self.unsupported,
            };
            events
                .iter()
                .for_each(|&event| dispatcher.dispatch(Entity::PLACEHOLDER, event, false));
            queue.apply(&mut self.world);
        }

        fn heard(&self) -> &Heard {
            self.world.resource::<Heard>()
        }
    }

    #[test]
    fn world_wide_kinds_are_exactly_the_ones_without_an_entity() {
        let without = [
            EventCode::PLAY_FX,
            EventCode::EARTHQUAKE,
            EventCode::SOUND_ALIAS,
            EventCode::OBITUARY,
        ];
        assert!(without.into_iter().all(event_without_centity));
        let with = [
            EventCode::NOAMMO,
            EventCode::BOMB_BURST,
            EventCode::NONE,
        ];
        assert!(with.into_iter().all(|kind| !event_without_centity(kind)));
    }

    #[test]
    fn sound_events_reach_the_sound_observer_and_are_tallied() {
        let mut rig = Rig::listening();
        let sound = dispatched(EventCode::NOAMMO, 3);
        assert!(matches!(
            entity_event_action(sound.event),
            Ok(EntityEventResponse::Sound)
        ));
        rig.feed(&[sound, sound]);
        assert_eq!(
            (
                rig.heard().sounds,
                rig.walk.handed_on,
                rig.unsupported.count_all
            ),
            (2, 2, 0)
        );
    }

    #[test]
    fn the_none_kind_neither_raises_nor_counts() {
        let mut rig = Rig::listening();
        rig.feed(&[dispatched(EventCode::NONE, 3)]);
        assert_eq!(rig.walk.handed_on, 0);
        assert_eq!(rig.heard().sounds + rig.heard().fires, 0);
    }

    #[test]
    fn an_unhandled_kind_is_noted_as_unsupported_and_still_tallied() {
        let Some(kind) = find_kind(Result::is_err) else {
            return;
        };
        let mut rig = Rig::listening();
        rig.feed(&[dispatched(kind, 3)]);
        assert_eq!(rig.unsupported.count_all, 1);
        assert_eq!(rig.unsupported.earliest, Some(kind));
        assert_eq!(rig.walk.handed_on, 1);
    }

    #[test]
    fn only_fire_events_of_the_local_number_count_as_local_fire() {
        let Some(fire) = find_kind(|action| matches!(action, Ok(EntityEventResponse::GunShot)))
        else {
            return;
        };
        let mut rig = Rig::listening();
        rig.feed(&[dispatched(fire, 3), dispatched(fire, 4)]);
        assert_eq!(rig.walk.self_trigger_pull, 1);
        assert_eq!(rig.heard().fires, 2);
    }
}
