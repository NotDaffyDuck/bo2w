//! Which events the client acts on, and how. Every row is one event; the events are grouped by the
//! action they cause, which is how the table reads.

use entity_t6::{EntityEventResponse, EventCode};

use super::{EntityEventHandlerKind, EntityEventTableEntry};

/// Rows for the events in each `Action => [..]` group, named `EV_<event>`.
macro_rules! observed {
    ($($action:ident => [$($event:ident),+ $(,)?];)+) => {
        &[$($(EntityEventTableEntry {
            name: concat!("EV_", stringify!($event)),
            kind: EventCode::$event,
            dispatch: EntityEventHandlerKind::Observer(EntityEventResponse::$action),
        },)+)+]
    };
}

/// Events the observers handle.
pub(super) const OBSERVED: &[EntityEventTableEntry] = observed! {
    None => [NONE];
    Sound => [
        FOLIAGE_SOUND, GUN_SOUND_OFF, SOUND_ALIAS, ALL_SOUNDS_OFF, ITEM_PICKUP, AMMO_PICKUP,
        NOAMMO, RELOAD, RELOAD_FROM_EMPTY, RELOAD_START, RELOAD_END, RAISE_WEAPON,
        FIRST_RAISE_WEAPON, PUTAWAY_WEAPON, WEAPON_ALT, PULLBACK_WEAPON, RECHAMBER_WEAPON,
        MELEE_SWIPE, PREP_OFFHAND, USE_OFFHAND, MELEE_HIT, MELEE_MISS,
    ];
    ResetAds => [RESET_ADS];
    GunShot => [
        FIRE_WEAPON, GUN_LAST_ROUND, GUN_LEFT_HAND, GUN_LAST_ROUND_LEFT,
        FIRE_GUNNER_1, FIRE_GUNNER_2, FIRE_GUNNER_3, FIRE_GUNNER_4, FIRE_GUNNER_1A,
        FIRE_GUNNER_2A, FIRE_GUNNER_3A, FIRE_GUNNER_4A,
    ];
    ShellEject => [EJECT_BRASS];
    FireMelee => [FIRE_MELEE];
    MeleeGore => [MELEE_BLOOD];
    BulletHit => [
        ROUND_HIT, ROUND_HIT_SHIELD, ROUND_HIT_BODY_SMALL, ROUND_HIT_BODY_LARGE,
        BULLET_HIT_CLIENT_SHIELD,
    ];
    ShieldBlast => [EXPLOSIVE_IMPACT_ON_SHIELD, EXPLOSIVE_SPLASH_ON_SHIELD];
    GrenadeBounceImpact => [BOMB_HOP];
    Explosion => [BOMB_BURST, ROCKET_EXPLODE, ROCKET_EXPLODE_NOMARKS, FLASHBANG_EXPLODE];
    PlayFx => [PLAY_FX, SENSOR_LASER];
    Obituary => [OBITUARY];
    FootstepNoise => [
        FOOTSTEP_SPRINT, FOOTSTEP_RUN, FOOTSTEP_WALK, FOOTSTEP_PRONE, LEAP, MANTLE,
        FOOTSTEP_CROUCHRUN, FOOTSTEP_CROUCHWALK, LADDER_HAND_GRAB, SLIDE_START, SLIDE_STOP,
        LANDING_DEFAULT, LANDING_BARK, LANDING_BRICK, LANDING_CARPET, LANDING_CLOTH,
        LANDING_CONCRETE, LANDING_DIRT, LANDING_FLESH, LANDING_FOLIAGE, LANDING_GLASS,
        LANDING_GRASS, LANDING_GRAVEL, LANDING_ICE, LANDING_METAL, LANDING_MUD, LANDING_PAPER,
        LANDING_PLASTER, LANDING_ROCK, LANDING_SAND, LANDING_SNOW, LANDING_WATER, LANDING_WOOD,
        LANDING_ASPHALT, LANDING_CERAMIC, LANDING_PLASTIC, LANDING_RUBBER, LANDING_CUSHION,
        LANDING_FRUIT, LANDING_PAINTEDMETAL, LANDING_RIOTSHIELD, LANDING_SLUSH,
        LANDING_PAIN_DEFAULT, LANDING_PAIN_BARK, LANDING_PAIN_BRICK, LANDING_PAIN_CARPET,
        LANDING_PAIN_CLOTH, LANDING_PAIN_CONCRETE, LANDING_PAIN_DIRT, LANDING_PAIN_FLESH,
        LANDING_PAIN_FOLIAGE, LANDING_PAIN_GLASS, LANDING_PAIN_GRASS, LANDING_PAIN_GRAVEL,
        LANDING_PAIN_ICE, LANDING_PAIN_METAL, LANDING_PAIN_MUD, LANDING_PAIN_PAPER,
        LANDING_PAIN_PLASTER, LANDING_PAIN_ROCK, LANDING_PAIN_SAND, LANDING_PAIN_SNOW,
        LANDING_PAIN_WATER, LANDING_PAIN_WOOD, LANDING_PAIN_ASPHALT, LANDING_PAIN_CERAMIC,
        LANDING_PAIN_PLASTIC, LANDING_PAIN_RUBBER, LANDING_PAIN_CUSHION, LANDING_PAIN_FRUIT,
        LANDING_PAIN_PAINTEDMETAL, LANDING_PAIN_RIOTSHIELD, LANDING_PAIN_SLUSH,
    ];
    BoltImpact => [BOLT_IMPACT];
    RemoteMissileBoost => [MISSILE_REMOTE_BOOST];
    Earthquake => [EARTHQUAKE];
    Rumble => [
        PLAY_RUMBLE_ON_ENT, PLAY_RUMBLE_ON_POS, PLAY_RUMBLELOOP_ON_ENT, PLAY_RUMBLELOOP_ON_POS,
        STOP_RUMBLE, STOP_ALL_RUMBLES,
    ];
};

/// A row for an event the client deliberately leaves alone.
const fn unsupported_row(
    title: &'static str,
    kind: EventCode,
    cause: &'static str,
) -> EntityEventTableEntry {
    EntityEventTableEntry {
        name: title,
        kind,
        dispatch: EntityEventHandlerKind::NoHandler(cause),
    }
}

/// Events the client deliberately does not act on, each with its reason.
pub(super) const UNSUPPORTED: &[EntityEventTableEntry] = &[unsupported_row(
    "EV_RELOAD_ADDAMMO",
    EventCode::RELOAD_ADDAMMO,
    "applying the reload top-up here would double-count: clip and stock already arrive      authoritatively in ClientSnapshotMeta.ammo_clip / ammo_stock, so a client that also      applied this event would add the magazine twice",
)];
