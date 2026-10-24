//! Our engine's entity events over BO2's numbers (`crate::event`): the
//! named kinds, what the client does with each, and the packet entity's
//! event ring as the client reads it.

use crate::event as ev;

use crate::EntityState;

pub const ET_EVENTS: i16 = crate::et::EVENTS;
/// TODO(fact): MW2's sequence mask and wrap window; BO2's ring is
/// `crate::event_ring` (wrap 0x100, window 0x40), which the client reader
/// does not use yet.
pub const EVENT_SEQUENCE_MASK: i32 = 0x7ff;
pub const EVENT_SEQUENCE_WRAP_WINDOW: i32 = 0x200;
pub const EVENT_RING_LEN: i32 = 4;

pub const LOCAL_SOUND_ENTITY: i32 = crate::ENTITYNUM_NONE;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct EntityEventKind(pub i32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntityEventFact {
    pub name: &'static str,
    pub event: EntityEventKind,
}

macro_rules! entity_events {
    ($($name:ident = $value:expr;)*) => {
        impl EntityEventKind {
            $(pub const $name: Self = Self($value);)*

            pub const TAXONOMY: &'static [EntityEventFact] = &[
                $(EntityEventFact {
                    name: concat!("EV_", stringify!($name)),
                    event: EntityEventKind($value),
                },)*
            ];
        }
    };
}

entity_events! {
    NONE = ev::NONE as i32;
    FOLIAGE_SOUND = ev::FOLIAGE_SOUND as i32;
    STOP_WEAPON_SOUND = ev::STOP_WEAPON_SOUND as i32;
    SOUND_ALIAS = ev::SOUND_ALIAS as i32;
    STOPSOUNDS = ev::STOPSOUNDS as i32;
    ITEM_PICKUP = ev::ITEM_PICKUP as i32;
    AMMO_PICKUP = ev::AMMO_PICKUP as i32;
    NOAMMO = ev::NOAMMO as i32;
    RESET_ADS = ev::RESET_ADS as i32;
    RELOAD = ev::RELOAD as i32;
    RELOAD_FROM_EMPTY = ev::RELOAD_FROM_EMPTY as i32;
    RELOAD_START = ev::RELOAD_START as i32;
    RELOAD_END = ev::RELOAD_END as i32;
    RELOAD_ADDAMMO = ev::RELOAD_ADDAMMO as i32;
    RAISE_WEAPON = ev::RAISE_WEAPON as i32;
    FIRST_RAISE_WEAPON = ev::FIRST_RAISE_WEAPON as i32;
    PUTAWAY_WEAPON = ev::PUTAWAY_WEAPON as i32;
    WEAPON_ALT = ev::WEAPON_ALT as i32;
    PULLBACK_WEAPON = ev::PULLBACK_WEAPON as i32;
    FIRE_WEAPON = ev::FIRE_WEAPON as i32;
    FIRE_WEAPON_LASTSHOT = ev::FIRE_WEAPON_LASTSHOT as i32;
    FIRE_WEAPON_LEFT = ev::FIRE_WEAPON_LEFT as i32;
    FIRE_WEAPON_LASTSHOT_LEFT = ev::FIRE_WEAPON_LASTSHOT_LEFT as i32;
    RECHAMBER_WEAPON = ev::RECHAMBER_WEAPON as i32;
    EJECT_BRASS = ev::EJECT_BRASS as i32;
    MELEE_SWIPE = ev::MELEE_SWIPE as i32;
    FIRE_MELEE = ev::FIRE_MELEE as i32;
    PREP_OFFHAND = ev::PREP_OFFHAND as i32;
    USE_OFFHAND = ev::USE_OFFHAND as i32;
    MELEE_HIT = ev::MELEE_HIT as i32;
    MELEE_MISS = ev::MELEE_MISS as i32;
    MELEE_BLOOD = ev::MELEE_BLOOD as i32;
    BULLET_HIT = ev::BULLET_HIT as i32;
    BULLET_HIT_SHIELD = ev::BULLET_HIT_SHIELD as i32;
    BULLET_HIT_CLIENT_SHIELD = ev::BULLET_HIT_CLIENT_SHIELD as i32;
    EXPLOSIVE_IMPACT_ON_SHIELD = ev::EXPLOSIVE_IMPACT_ON_SHIELD as i32;
    EXPLOSIVE_SPLASH_ON_SHIELD = ev::EXPLOSIVE_SPLASH_ON_SHIELD as i32;
    BULLET_HIT_CLIENT_SMALL = ev::BULLET_HIT_CLIENT_SMALL as i32;
    BULLET_HIT_CLIENT_LARGE = ev::BULLET_HIT_CLIENT_LARGE as i32;
    GRENADE_BOUNCE = ev::GRENADE_BOUNCE as i32;
    GRENADE_EXPLODE = ev::GRENADE_EXPLODE as i32;
    ROCKET_EXPLODE = ev::ROCKET_EXPLODE as i32;
    ROCKET_EXPLODE_NOMARKS = ev::ROCKET_EXPLODE_NOMARKS as i32;
    FLASHBANG_EXPLODE = ev::FLASHBANG_EXPLODE as i32;
    PLAY_FX = ev::PLAY_FX as i32;
    EARTHQUAKE = ev::EARTHQUAKE as i32;
    DETONATE = ev::DETONATE as i32;
    MISSILE_REMOTE_BOOST = ev::MISSILE_REMOTE_BOOST as i32;
    PLAY_RUMBLE_ON_ENT = ev::PLAY_RUMBLE_ON_ENT as i32;
    PLAY_RUMBLE_ON_POS = ev::PLAY_RUMBLE_ON_POS as i32;
    PLAY_RUMBLELOOP_ON_ENT = ev::PLAY_RUMBLELOOP_ON_ENT as i32;
    PLAY_RUMBLELOOP_ON_POS = ev::PLAY_RUMBLELOOP_ON_POS as i32;
    STOP_RUMBLE = ev::STOP_RUMBLE as i32;
    STOP_ALL_RUMBLES = ev::STOP_ALL_RUMBLES as i32;
    OBITUARY = ev::OBITUARY as i32;
    FOOTSTEP_SPRINT = ev::FOOTSTEP_SPRINT as i32;
    FOOTSTEP_RUN = ev::FOOTSTEP_RUN as i32;
    FOOTSTEP_WALK = ev::FOOTSTEP_WALK as i32;
    FOOTSTEP_CROUCHRUN = ev::FOOTSTEP_CROUCHRUN as i32;
    FOOTSTEP_CROUCHWALK = ev::FOOTSTEP_CROUCHWALK as i32;
    FOOTSTEP_PRONE = ev::FOOTSTEP_PRONE as i32;
    LADDER_HAND_GRAB = ev::LADDER_HAND_GRAB as i32;
    MANTLE = ev::MANTLE as i32;
    JUMP = ev::JUMP as i32;
    // The landings, by our surface index (MW2's surface order until the
    // footsteps take BO2's 32 surfaces).
    LANDING_DEFAULT = ev::LANDING_FIRST as i32 + 0;
    LANDING_BARK = ev::LANDING_FIRST as i32 + 1;
    LANDING_BRICK = ev::LANDING_FIRST as i32 + 2;
    LANDING_CARPET = ev::LANDING_FIRST as i32 + 3;
    LANDING_CLOTH = ev::LANDING_FIRST as i32 + 4;
    LANDING_CONCRETE = ev::LANDING_FIRST as i32 + 5;
    LANDING_DIRT = ev::LANDING_FIRST as i32 + 6;
    LANDING_FLESH = ev::LANDING_FIRST as i32 + 7;
    LANDING_FOLIAGE = ev::LANDING_FIRST as i32 + 8;
    LANDING_GLASS = ev::LANDING_FIRST as i32 + 9;
    LANDING_GRASS = ev::LANDING_FIRST as i32 + 10;
    LANDING_GRAVEL = ev::LANDING_FIRST as i32 + 11;
    LANDING_ICE = ev::LANDING_FIRST as i32 + 12;
    LANDING_METAL = ev::LANDING_FIRST as i32 + 13;
    LANDING_MUD = ev::LANDING_FIRST as i32 + 14;
    LANDING_PAPER = ev::LANDING_FIRST as i32 + 15;
    LANDING_PLASTER = ev::LANDING_FIRST as i32 + 16;
    LANDING_ROCK = ev::LANDING_FIRST as i32 + 17;
    LANDING_SAND = ev::LANDING_FIRST as i32 + 18;
    LANDING_SNOW = ev::LANDING_FIRST as i32 + 19;
    LANDING_WATER = ev::LANDING_FIRST as i32 + 20;
    LANDING_WOOD = ev::LANDING_FIRST as i32 + 21;
    LANDING_ASPHALT = ev::LANDING_FIRST as i32 + 22;
    LANDING_CERAMIC = ev::LANDING_FIRST as i32 + 23;
    LANDING_PLASTIC = ev::LANDING_FIRST as i32 + 24;
    LANDING_RUBBER = ev::LANDING_FIRST as i32 + 25;
    LANDING_CUSHION = ev::LANDING_FIRST as i32 + 26;
    LANDING_FRUIT = ev::LANDING_FIRST as i32 + 27;
    LANDING_PAINTEDMETAL = ev::LANDING_FIRST as i32 + 28;
    LANDING_RIOTSHIELD = ev::LANDING_FIRST as i32 + 29;
    LANDING_SLUSH = ev::LANDING_FIRST as i32 + 30;
    LANDING_PAIN_DEFAULT = ev::LANDING_PAIN_FIRST as i32 + 0;
    LANDING_PAIN_BARK = ev::LANDING_PAIN_FIRST as i32 + 1;
    LANDING_PAIN_BRICK = ev::LANDING_PAIN_FIRST as i32 + 2;
    LANDING_PAIN_CARPET = ev::LANDING_PAIN_FIRST as i32 + 3;
    LANDING_PAIN_CLOTH = ev::LANDING_PAIN_FIRST as i32 + 4;
    LANDING_PAIN_CONCRETE = ev::LANDING_PAIN_FIRST as i32 + 5;
    LANDING_PAIN_DIRT = ev::LANDING_PAIN_FIRST as i32 + 6;
    LANDING_PAIN_FLESH = ev::LANDING_PAIN_FIRST as i32 + 7;
    LANDING_PAIN_FOLIAGE = ev::LANDING_PAIN_FIRST as i32 + 8;
    LANDING_PAIN_GLASS = ev::LANDING_PAIN_FIRST as i32 + 9;
    LANDING_PAIN_GRASS = ev::LANDING_PAIN_FIRST as i32 + 10;
    LANDING_PAIN_GRAVEL = ev::LANDING_PAIN_FIRST as i32 + 11;
    LANDING_PAIN_ICE = ev::LANDING_PAIN_FIRST as i32 + 12;
    LANDING_PAIN_METAL = ev::LANDING_PAIN_FIRST as i32 + 13;
    LANDING_PAIN_MUD = ev::LANDING_PAIN_FIRST as i32 + 14;
    LANDING_PAIN_PAPER = ev::LANDING_PAIN_FIRST as i32 + 15;
    LANDING_PAIN_PLASTER = ev::LANDING_PAIN_FIRST as i32 + 16;
    LANDING_PAIN_ROCK = ev::LANDING_PAIN_FIRST as i32 + 17;
    LANDING_PAIN_SAND = ev::LANDING_PAIN_FIRST as i32 + 18;
    LANDING_PAIN_SNOW = ev::LANDING_PAIN_FIRST as i32 + 19;
    LANDING_PAIN_WATER = ev::LANDING_PAIN_FIRST as i32 + 20;
    LANDING_PAIN_WOOD = ev::LANDING_PAIN_FIRST as i32 + 21;
    LANDING_PAIN_ASPHALT = ev::LANDING_PAIN_FIRST as i32 + 22;
    LANDING_PAIN_CERAMIC = ev::LANDING_PAIN_FIRST as i32 + 23;
    LANDING_PAIN_PLASTIC = ev::LANDING_PAIN_FIRST as i32 + 24;
    LANDING_PAIN_RUBBER = ev::LANDING_PAIN_FIRST as i32 + 25;
    LANDING_PAIN_CUSHION = ev::LANDING_PAIN_FIRST as i32 + 26;
    LANDING_PAIN_FRUIT = ev::LANDING_PAIN_FIRST as i32 + 27;
    LANDING_PAIN_PAINTEDMETAL = ev::LANDING_PAIN_FIRST as i32 + 28;
    LANDING_PAIN_RIOTSHIELD = ev::LANDING_PAIN_FIRST as i32 + 29;
    LANDING_PAIN_SLUSH = ev::LANDING_PAIN_FIRST as i32 + 30;
    // A vehicle gunner's gun fired (VEH_FireGunnerWeapon on the vehicle):
    // gunners 1-4 from their first flash tag, then 1A-4A from their second.
    FIRE_GUNNER_1 = ev::FIRE_GUNNER_1 as i32;
    FIRE_GUNNER_2 = ev::FIRE_GUNNER_2 as i32;
    FIRE_GUNNER_3 = ev::FIRE_GUNNER_3 as i32;
    FIRE_GUNNER_4 = ev::FIRE_GUNNER_4 as i32;
    FIRE_GUNNER_1A = ev::FIRE_GUNNER_1A as i32;
    FIRE_GUNNER_2A = ev::FIRE_GUNNER_2A as i32;
    FIRE_GUNNER_3A = ev::FIRE_GUNNER_3A as i32;
    FIRE_GUNNER_4A = ev::FIRE_GUNNER_4A as i32;
    // The slide's ground start and stop, sent by the server only; the parm
    // is the ground's slide sound class.
    SLIDE_START = ev::SLIDE_START as i32;
    SLIDE_STOP = ev::SLIDE_STOP as i32;
    // A crossbow bolt or ballistic knife blade landing.
    BOLT_IMPACT = ev::BOLT_IMPACT as i32;
    // bo2mp: the Sensor Grenade's laser on an enemy this frame. BO2 draws it
    // in CG_SensorGrenade; our server sends where it runs, on a number past
    // BO2's table (TODO: draw it from the grenade's entity state).
    SENSOR_LASER = 0x100;
}

impl EntityEventKind {
    pub const LANDING_FIRST: Self = Self::LANDING_DEFAULT;

    pub const LANDING_LAST: Self = Self::LANDING_SLUSH;

    pub const LANDING_PAIN_FIRST: Self = Self::LANDING_PAIN_DEFAULT;

    pub const LANDING_PAIN_LAST: Self = Self::LANDING_PAIN_SLUSH;

    pub const fn from_event_entity_type(e_type: i16) -> Option<Self> {
        match crate::entity_type::event_of_entity_type(e_type) {
            Some(ev) => Some(Self(ev as i32)),
            None => None,
        }
    }

    pub const fn event_entity_type(self) -> Option<i16> {
        if self.0 > Self::NONE.0 && self.0 < ev::MAX_EVENTS as i32 {
            Some(crate::entity_type::event_entity_type(self.0 as u8))
        } else {
            None
        }
    }

    pub const fn landing_surface_index(self) -> Option<i32> {
        if self.0 >= Self::LANDING_FIRST.0 && self.0 <= Self::LANDING_LAST.0 {
            Some(self.0 - Self::LANDING_FIRST.0)
        } else if self.0 >= Self::LANDING_PAIN_FIRST.0 && self.0 <= Self::LANDING_PAIN_LAST.0 {
            Some(self.0 - Self::LANDING_PAIN_FIRST.0)
        } else {
            None
        }
    }

    pub fn taxonomy_name(self) -> Option<&'static str> {
        Self::TAXONOMY
            .iter()
            .find(|fact| fact.event.0 == self.0)
            .map(|fact| fact.name)
    }

    /// An event the wire carries: BO2's table (`entity_event_t`, past
    /// `EV_NONE`), and our Sensor Grenade laser.
    pub const fn on_the_wire(self) -> bool {
        (self.0 > Self::NONE.0 && self.0 < ev::MAX_EVENTS as i32) || self.0 == Self::SENSOR_LASER.0
    }

    pub const fn is_landing_pain(self) -> bool {
        self.0 >= Self::LANDING_PAIN_FIRST.0 && self.0 <= Self::LANDING_PAIN_LAST.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityEventAction {
    None,
    Sound,
    WeaponFire,
    EjectBrass,
    BulletHit,
    GrenadeContact,
    // bo2mp: a crossbow bolt or a thrown blade striking (BO2's
    // EV_BOLT_IMPACT).
    BoltImpact,
    // bo2mp: the Hellstorm's boost (BO2's EV_MISSILE_REMOTE_BOOST).
    RemoteMissileBoost,
    // bo2mp: a script's earthquake (BO2's EV_EARTHQUAKE).
    Earthquake,
    // bo2mp: a script's rumble played or stopped (BO2's EV_PLAY_RUMBLE_*,
    // EV_STOP_RUMBLE, EV_STOP_ALL_RUMBLES): the pad shakes.
    Rumble,
    // bo2mp: a missile striking a riot shield, or a blast it partly stops
    // (EV_EXPLOSIVE_IMPACT_ON_SHIELD / _SPLASH_ON_SHIELD): its bearer's
    // view shakes. No bullet strikes, so no impact effect or sound.
    ShieldBlast,
    // bo2mp: a melee swing (BO2's EV_FIRE_MELEE): loose glass shards in
    // front of the swinger break.
    FireMelee,
    Explosion,
    PlayFx,
    Obituary,
    MovementSound,

    ResetAds,

    MeleeBlood,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnsupportedEntityEvent(pub EntityEventKind);

pub fn entity_event_action(
    event: EntityEventKind,
) -> Result<EntityEventAction, UnsupportedEntityEvent> {
    Ok(match event {
        EntityEventKind::NONE => EntityEventAction::None,
        EntityEventKind::RESET_ADS => EntityEventAction::ResetAds,
        EntityEventKind::FOLIAGE_SOUND
        | EntityEventKind::STOP_WEAPON_SOUND
        | EntityEventKind::SOUND_ALIAS
        | EntityEventKind::STOPSOUNDS
        | EntityEventKind::ITEM_PICKUP
        | EntityEventKind::AMMO_PICKUP
        | EntityEventKind::NOAMMO
        | EntityEventKind::RELOAD
        | EntityEventKind::RELOAD_FROM_EMPTY
        | EntityEventKind::RELOAD_START
        | EntityEventKind::RELOAD_END
        | EntityEventKind::RAISE_WEAPON
        | EntityEventKind::FIRST_RAISE_WEAPON
        | EntityEventKind::PUTAWAY_WEAPON
        | EntityEventKind::WEAPON_ALT
        | EntityEventKind::PULLBACK_WEAPON
        | EntityEventKind::RECHAMBER_WEAPON
        | EntityEventKind::PREP_OFFHAND
        | EntityEventKind::USE_OFFHAND
        | EntityEventKind::DETONATE
        | EntityEventKind::MELEE_SWIPE
        | EntityEventKind::MELEE_HIT
        | EntityEventKind::MELEE_MISS => EntityEventAction::Sound,
        EntityEventKind::MELEE_BLOOD => EntityEventAction::MeleeBlood,
        EntityEventKind::FIRE_MELEE => EntityEventAction::FireMelee,
        EntityEventKind::FIRE_WEAPON
        | EntityEventKind::FIRE_WEAPON_LASTSHOT
        | EntityEventKind::FIRE_WEAPON_LEFT
        | EntityEventKind::FIRE_WEAPON_LASTSHOT_LEFT
        | EntityEventKind::FIRE_GUNNER_1
        | EntityEventKind::FIRE_GUNNER_2
        | EntityEventKind::FIRE_GUNNER_3
        | EntityEventKind::FIRE_GUNNER_4
        | EntityEventKind::FIRE_GUNNER_1A
        | EntityEventKind::FIRE_GUNNER_2A
        | EntityEventKind::FIRE_GUNNER_3A
        | EntityEventKind::FIRE_GUNNER_4A => EntityEventAction::WeaponFire,
        EntityEventKind::EJECT_BRASS => EntityEventAction::EjectBrass,
        EntityEventKind::BULLET_HIT
        | EntityEventKind::BULLET_HIT_SHIELD
        | EntityEventKind::BULLET_HIT_CLIENT_SMALL
        | EntityEventKind::BULLET_HIT_CLIENT_LARGE
        | EntityEventKind::BULLET_HIT_CLIENT_SHIELD => EntityEventAction::BulletHit,
        EntityEventKind::EXPLOSIVE_IMPACT_ON_SHIELD
        | EntityEventKind::EXPLOSIVE_SPLASH_ON_SHIELD => EntityEventAction::ShieldBlast,
        EntityEventKind::GRENADE_BOUNCE => EntityEventAction::GrenadeContact,
        EntityEventKind::BOLT_IMPACT => EntityEventAction::BoltImpact,
        EntityEventKind::MISSILE_REMOTE_BOOST => EntityEventAction::RemoteMissileBoost,
        EntityEventKind::EARTHQUAKE => EntityEventAction::Earthquake,
        EntityEventKind::PLAY_RUMBLE_ON_ENT
        | EntityEventKind::PLAY_RUMBLE_ON_POS
        | EntityEventKind::PLAY_RUMBLELOOP_ON_ENT
        | EntityEventKind::PLAY_RUMBLELOOP_ON_POS
        | EntityEventKind::STOP_RUMBLE
        | EntityEventKind::STOP_ALL_RUMBLES => EntityEventAction::Rumble,
        EntityEventKind::GRENADE_EXPLODE
        | EntityEventKind::ROCKET_EXPLODE
        | EntityEventKind::ROCKET_EXPLODE_NOMARKS
        | EntityEventKind::FLASHBANG_EXPLODE => EntityEventAction::Explosion,
        EntityEventKind::PLAY_FX | EntityEventKind::SENSOR_LASER => EntityEventAction::PlayFx,
        EntityEventKind::OBITUARY => EntityEventAction::Obituary,
        EntityEventKind::FOOTSTEP_SPRINT
        | EntityEventKind::FOOTSTEP_RUN
        | EntityEventKind::FOOTSTEP_WALK
        | EntityEventKind::FOOTSTEP_PRONE
        | EntityEventKind::FOOTSTEP_CROUCHRUN
        | EntityEventKind::FOOTSTEP_CROUCHWALK
        | EntityEventKind::LADDER_HAND_GRAB
        | EntityEventKind::JUMP
        | EntityEventKind::MANTLE
        | EntityEventKind::SLIDE_START
        | EntityEventKind::SLIDE_STOP => EntityEventAction::MovementSound,
        other => {
            if other.landing_surface_index().is_some() {
                EntityEventAction::MovementSound
            } else {
                return Err(UnsupportedEntityEvent(other));
            }
        }
    })
}

#[must_use]
pub const fn is_left_hand_fire_event(event: EntityEventKind) -> bool {
    event.0 >= 0 && event.0 <= u8::MAX as i32 && ev::is_left_gun_fire(event.0 as u8)
}

#[must_use]
pub const fn is_weapon_fire_last_shot_event(event: EntityEventKind) -> bool {
    event.0 >= 0 && event.0 <= u8::MAX as i32 && ev::is_last_shot_fire(event.0 as u8)
}

/// The event a vehicle sends when its gunner `gunner` (0-3) fires from his
/// gun's first flash tag (`EV_FIRE_GUNNER_1` plus the gunner), or from its
/// `second` (`EV_FIRE_GUNNER_1A` plus the gunner).
#[must_use]
pub const fn gunner_fire_event(gunner: usize, second: bool) -> EntityEventKind {
    EntityEventKind(ev::gunner_fire_event(gunner, second) as i32)
}

/// One of a vehicle gunner's fire events, from either flash tag.
#[must_use]
pub const fn is_gunner_fire_event(event: EntityEventKind) -> bool {
    event.0 >= 0
        && event.0 <= u8::MAX as i32
        && ev::gunner_of_fire_event(event.0 as u8).is_some()
}

#[must_use]
pub const fn predicted_weapon_fire_event(hand: i32, last_shot: bool) -> EntityEventKind {
    match (hand != 0, last_shot) {
        (false, false) => EntityEventKind::FIRE_WEAPON,
        (false, true) => EntityEventKind::FIRE_WEAPON_LASTSHOT,
        (true, false) => EntityEventKind::FIRE_WEAPON_LEFT,
        (true, true) => EntityEventKind::FIRE_WEAPON_LASTSHOT_LEFT,
    }
}

pub const fn bullet_hit_event(impact_type: i32, local_client: bool) -> Option<EntityEventKind> {
    if local_client {
        match impact_type {
            1 | 9 => Some(EntityEventKind::BULLET_HIT_CLIENT_SMALL),
            2 => Some(EntityEventKind::BULLET_HIT_CLIENT_LARGE),
            3 => Some(EntityEventKind::BULLET_HIT_CLIENT_SHIELD),
            _ => None,
        }
    } else {
        match impact_type {
            1 | 2 | 9 => Some(EntityEventKind::BULLET_HIT),
            3 => Some(EntityEventKind::BULLET_HIT_SHIELD),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SequencedEntityEvent {
    pub sequence: i32,
    pub event: EntityEventKind,
    pub event_parm: i32,
}

pub fn packet_entity_uses_event_ring(e_type: i16) -> bool {
    e_type != crate::et::FX && e_type != crate::et::LOOP_FX && e_type < ET_EVENTS
}

pub fn consume_entity_events(
    state: &EntityState,
    consumed_sequence: &mut i32,
    mut dispatch: impl FnMut(SequencedEntityEvent),
) {
    let next = i32::from(state.event_sequence);
    if *consumed_sequence == next {
        return;
    }
    if next == 0 {
        *consumed_sequence = 0;
        return;
    }

    let mut consumed = *consumed_sequence;
    if next + EVENT_SEQUENCE_WRAP_WINDOW < consumed {
        consumed -= EVENT_SEQUENCE_MASK + 1;
    }
    if next - consumed > EVENT_RING_LEN {
        consumed = next - EVENT_RING_LEN;
    }

    while consumed < next {
        let slot = (consumed as u32 & 3) as usize;
        dispatch(SequencedEntityEvent {
            sequence: consumed & EVENT_SEQUENCE_MASK,
            event: EntityEventKind(i32::from(state.events[slot])),
            event_parm: state.event_parms[slot] as i32,
        });
        consumed += 1;
    }
    *consumed_sequence = next;
}

#[cfg(test)]
mod gunner_fire_tests {
    use super::*;

    /// A gunner's shot is a gun firing, on BO2's numbers: gunner 0 fires
    /// 202, gunner 3 205, and his second flash tag's 206-209 are the same
    /// to the client.
    #[test]
    fn a_vehicle_gunners_shot_is_a_weapon_fire() {
        assert_eq!(gunner_fire_event(0, false), EntityEventKind::FIRE_GUNNER_1);
        assert_eq!(gunner_fire_event(3, false), EntityEventKind::FIRE_GUNNER_4);
        assert_eq!(gunner_fire_event(0, true), EntityEventKind::FIRE_GUNNER_1A);
        assert_eq!(gunner_fire_event(3, true), EntityEventKind::FIRE_GUNNER_4A);
        for n in 202..=209 {
            let event = EntityEventKind(n);
            assert!(is_gunner_fire_event(event) && event.on_the_wire(), "{n}");
            assert_eq!(entity_event_action(event), Ok(EntityEventAction::WeaponFire));
            assert!(!is_left_hand_fire_event(event) && !is_weapon_fire_last_shot_event(event));
        }
        assert!(!is_gunner_fire_event(EntityEventKind::FIRE_WEAPON));
        assert!(!is_gunner_fire_event(EntityEventKind(210)));
        assert!(!is_gunner_fire_event(EntityEventKind(201)));
        assert_eq!(
            EntityEventKind::FIRE_GUNNER_4A.taxonomy_name(),
            Some("EV_FIRE_GUNNER_4A")
        );
    }
}
