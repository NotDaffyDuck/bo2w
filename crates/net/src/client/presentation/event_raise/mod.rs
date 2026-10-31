//! Entity events that reach the client become one Bevy event per kind of effect (sound, weapon
//! fire, explosion, ...), which the render and audio crates observe.
//!
//! Each frame that adopted a snapshot feeds the dispatcher from two places: the events held in
//! every entity's own ring (consumed as that entity's state advances) and the event records the
//! server shipped with the snapshot, live or out of the archive while a killcam plays.

mod classify;
mod cursor;
mod raised_events;
mod observers;
mod system;
mod unsupported;
mod walk;

pub use cursor::EntityEventReadPos;
pub use raised_events::{
    RoutedEntityEvent, EntityBoltImpact, BulletStrike, EntityEarthquake, ShellCaseOut,
    SoundCueAt, BlastAt,
};
pub use raised_events::{
    EntityFireMelee, GrenadeBounceAt, MeleeBloodAt, StepSoundAt, EntityKillNotice,
    EffectAt, EntityRemoteMissileBoost, EntitySightsLowered, EntityRumble, EntityShieldBlast,
    GunFiredAt, KillcamFxTransition,
};
pub use unsupported::UnhandledEntityEvents;
pub use walk::InstalledEntityEventTraversal;

use bevy::prelude::*;

use crate::identity_slips::LinkIdentityDeficits;
use crate::tick_sets::ClientStep;

pub fn enroll_object_occurrence_routing(app: &mut App) {
    app.init_resource::<LinkIdentityDeficits>();
    app.init_resource::<UnhandledEntityEvents>();
    app.init_resource::<InstalledEntityEventTraversal>();
    app.init_resource::<EntityEventReadPos>();

    app.add_observer(observers::switch_when_out_of_ammo);
    app.add_observer(observers::set_ads_from_reset);

    let dispatch = system::dispatch_entity_events
        .after(crate::mirror_entities)
        .in_set(ClientStep::Reconcile);
    app.add_systems(Update, dispatch);
}
