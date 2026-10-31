//! The Bevy events the dispatcher raises. Every one names the entity it concerns and holds the
//! event exactly as it was dispatched.

use bevy::prelude::*;
use entity_t6::EventCode;
use sim::{EntityEventPayload, EventSequence, Tick};

/// One entity event on its way to the observers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoutedEntityEvent {
    pub order_no: EventSequence,
    pub tick: Tick,
    pub event: EventCode,
    pub payload: EntityEventPayload,
}

/// Declares one Bevy event per name. Without `killcam`, it holds only the entity and the event;
/// with it, it also records whether the event came from the replayed archive.
macro_rules! declare_entity_events {
    (plain: $($(#[$doc:meta])* $name:ident),+ $(,)?) => {$(
        $(#[$doc])*
        #[derive(EntityEvent, Clone, Copy, Debug, PartialEq)]
        pub struct $name {
            pub entity: Entity,
            pub event: RoutedEntityEvent,
        }
    )+};
    (killcam: $($(#[$doc:meta])* $name:ident),+ $(,)?) => {$(
        $(#[$doc])*
        #[derive(EntityEvent, Clone, Copy, Debug, PartialEq)]
        pub struct $name {
            pub entity: Entity,
            pub event: RoutedEntityEvent,
            pub replaying_final_death: bool,
        }
    )+};
}

declare_entity_events! { plain:
    SoundCueAt,
    GunFiredAt,
    ShellCaseOut,
    BulletStrike,
    GrenadeBounceAt,
    /// A crossbow bolt or thrown blade landing: the entry point, the hit normal, the surface it
    /// struck (7 is a player's body, 31 a riot shield) and the player who was hit.
    EntityBoltImpact,
    /// The Hellstorm boosted by its rider, or started coasting still armed; the rider's view
    /// shakes.
    EntityRemoteMissileBoost,
    /// A script-driven earthquake. The view shakes by the scale (first value of the second
    /// origin) for the duration (the event parameter, in ms), fading out to the radius (second
    /// value of the second origin).
    EntityEarthquake,
    /// A missile hitting a riot shield, or a blast the shield only partly absorbed. The entity is
    /// the shield's bearer; for a blast the event parameter is the damage it would have caused
    /// without the shield.
    EntityShieldBlast,
    /// A melee swing. The entity swings, the origin is his eye, the direction his view angles and
    /// the weapon the melee weapon.
    EntityFireMelee,
    BlastAt,
    EffectAt,
    StepSoundAt,
    EntitySightsLowered,
    MeleeBloodAt,
}

declare_entity_events! { killcam:
    /// A script rumble: play or loop on an entity or at a spot, stop one, or stop all. The entity
    /// is the one it plays on (the world entity for a spot), the event parameter indexes the
    /// sound alias `rumble:<name>` and the origin is the spot.
    EntityRumble,
    EntityKillNotice,
}

/// Raised ahead of the first events from the other world, so that effects of the old world are
/// cleared before the replay starts them again.
#[derive(Event, Clone, Copy, Debug, PartialEq, Eq)]
pub struct KillcamFxTransition {
    pub entering: bool,
}
