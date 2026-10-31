//! Entity events as the client sees them. BO2 numbers its events in `crate::event`; here they get one named type,
//! the effect each one drives on the client, the event a shot or a bullet strike sends, and the reader for the
//! four-slot event ring a packet entity carries.

use crate::event_ring::{EVENT_SLOTS, EVENT_SEQUENCE_WRAP, unwrap_previous};
use crate::event;
use crate::{et, EntitySnapshot};

/// One entity event: BO2's event number, or our own number past BO2's table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct EventCode(pub i32);

/// Gives each listed BO2 event its name on [`EntityEventKind`], with the same number as in `crate::event`.
macro_rules! bo2_events {
    ($($name:ident)*) => {
        impl EventCode {
            $(pub const $name: Self = Self(event::$name as i32);)*
        }
    };
}

bo2_events! {
    NONE
    // Sounds and the weapon's own cycle.
    FOLIAGE_SOUND GUN_SOUND_OFF SOUND_ALIAS ALL_SOUNDS_OFF ITEM_PICKUP AMMO_PICKUP NOAMMO RESET_ADS
    RELOAD RELOAD_FROM_EMPTY RELOAD_START RELOAD_END RELOAD_ADDAMMO
    RAISE_WEAPON FIRST_RAISE_WEAPON PUTAWAY_WEAPON WEAPON_ALT PULLBACK_WEAPON RECHAMBER_WEAPON EJECT_BRASS
    // Shots, both hands, and the last round of a clip.
    FIRE_WEAPON GUN_LAST_ROUND GUN_LEFT_HAND GUN_LAST_ROUND_LEFT
    // Knife, grenades and equipment in hand.
    MELEE_SWIPE FIRE_MELEE MELEE_HIT MELEE_MISS MELEE_BLOOD PREP_OFFHAND USE_OFFHAND DETONATE
    // Where bullets and blasts land.
    ROUND_HIT ROUND_HIT_SHIELD ROUND_HIT_BODY_SMALL ROUND_HIT_BODY_LARGE BULLET_HIT_CLIENT_SHIELD
    EXPLOSIVE_IMPACT_ON_SHIELD EXPLOSIVE_SPLASH_ON_SHIELD
    BOMB_HOP BOMB_BURST ROCKET_EXPLODE ROCKET_EXPLODE_NOMARKS FLASHBANG_EXPLODE
    // Script effects: particle effects, earthquakes, the Hellstorm's boost, pad rumble.
    PLAY_FX EARTHQUAKE MISSILE_REMOTE_BOOST
    PLAY_RUMBLE_ON_ENT PLAY_RUMBLE_ON_POS PLAY_RUMBLELOOP_ON_ENT PLAY_RUMBLELOOP_ON_POS STOP_RUMBLE STOP_ALL_RUMBLES
    OBITUARY
    // The body moving: steps by gait, ladder, mantle, jump.
    FOOTSTEP_SPRINT FOOTSTEP_RUN FOOTSTEP_WALK FOOTSTEP_CROUCHRUN FOOTSTEP_CROUCHWALK FOOTSTEP_PRONE
    LADDER_HAND_GRAB MANTLE LEAP
    // A vehicle gunner firing (`VEH_FireGunnerWeapon`): gunners 1 to 4 from their gun's first flash tag, then the
    // same four from the second.
    FIRE_GUNNER_1 FIRE_GUNNER_2 FIRE_GUNNER_3 FIRE_GUNNER_4 FIRE_GUNNER_1A FIRE_GUNNER_2A FIRE_GUNNER_3A FIRE_GUNNER_4A
    // Sliding on the ground starts and stops (server only; the parm is the ground's slide sound class).
    SLIDE_START SLIDE_STOP
    // A crossbow bolt or a ballistic knife blade sticking in.
    BOLT_IMPACT
}

/// Names the landing events. Each pair is a surface's soft and hurt landing; a pair's place in the list is added to the
/// first number of the soft block and of the hurt block.
macro_rules! landing_events {
    ($($soft:ident $hurt:ident)*) => { landing_events!(@from 0i32; $($soft $hurt)*); };
    (@from $index:expr; $soft:ident $hurt:ident $($rest:ident)*) => {
        impl EventCode {
            pub const $soft: Self = Self(event::LANDING_FIRST as i32 + $index);
            pub const $hurt: Self = Self(event::LANDING_PAIN_FIRST as i32 + $index);
        }
        landing_events!(@from $index + 1; $($rest)*);
    };
    (@from $index:expr;) => {};
}

landing_events! {
    LANDING_DEFAULT LANDING_PAIN_DEFAULT  LANDING_BARK LANDING_PAIN_BARK  LANDING_BRICK LANDING_PAIN_BRICK
    LANDING_CARPET LANDING_PAIN_CARPET  LANDING_CLOTH LANDING_PAIN_CLOTH  LANDING_CONCRETE LANDING_PAIN_CONCRETE
    LANDING_DIRT LANDING_PAIN_DIRT  LANDING_FLESH LANDING_PAIN_FLESH  LANDING_FOLIAGE LANDING_PAIN_FOLIAGE
    LANDING_GLASS LANDING_PAIN_GLASS  LANDING_GRASS LANDING_PAIN_GRASS  LANDING_GRAVEL LANDING_PAIN_GRAVEL
    LANDING_ICE LANDING_PAIN_ICE  LANDING_METAL LANDING_PAIN_METAL  LANDING_MUD LANDING_PAIN_MUD
    LANDING_PAPER LANDING_PAIN_PAPER  LANDING_PLASTER LANDING_PAIN_PLASTER  LANDING_ROCK LANDING_PAIN_ROCK
    LANDING_SAND LANDING_PAIN_SAND  LANDING_SNOW LANDING_PAIN_SNOW  LANDING_WATER LANDING_PAIN_WATER
    LANDING_WOOD LANDING_PAIN_WOOD  LANDING_ASPHALT LANDING_PAIN_ASPHALT  LANDING_CERAMIC LANDING_PAIN_CERAMIC
    LANDING_PLASTIC LANDING_PAIN_PLASTIC  LANDING_RUBBER LANDING_PAIN_RUBBER  LANDING_CUSHION LANDING_PAIN_CUSHION
    LANDING_FRUIT LANDING_PAIN_FRUIT  LANDING_PAINTEDMETAL LANDING_PAIN_PAINTEDMETAL
    LANDING_RIOTSHIELD LANDING_PAIN_RIOTSHIELD  LANDING_SLUSH LANDING_PAIN_SLUSH
}

/// How many surfaces the landing events cover: ours, in our surface order, one short of BO2's 32 until footsteps use
/// BO2's surface table. The last landing number of each block is therefore not one of ours.
const LANDING_SURFACES: i32 = 31;

impl EventCode {
    /// Ours, not BO2's: the Sensor Grenade's laser on an enemy this frame. BO2 draws it in `CG_SensorGrenade`; our
    /// server sends it from where the grenade runs, on a number past BO2's table. TODO: draw it from the grenade's
    /// entity state instead.
    pub const SENSOR_LASER: Self = Self(0x100);

    /// The surface of a landing, soft or hurt, counted from the first surface.
    pub const fn touchdown_material_index(self) -> Option<i32> {
        match landing_block(self.0) {
            Some((surface, _)) => Some(surface),
            None => None,
        }
    }

    /// A landing hard enough to hurt.
    pub const fn hurts_on_landing(self) -> bool {
        matches!(landing_block(self.0), Some((_, true)))
    }

    /// The wire carries every number of BO2's table after "none", plus our Sensor Grenade laser.
    pub const fn on_the_wire(self) -> bool {
        self.0 == Self::SENSOR_LASER.0 || (self.0 > Self::NONE.0 && self.0 < event::MAX_EVENTS as i32)
    }
}

/// `(surface, hurt)` for a landing number from either block.
const fn landing_block(number: i32) -> Option<(i32, bool)> {
    let soft = event::LANDING_FIRST as i32;
    let hurt = event::LANDING_PAIN_FIRST as i32;
    if number >= soft && number < soft + LANDING_SURFACES {
        Some((number - soft, false))
    } else if number >= hurt && number < hurt + LANDING_SURFACES {
        Some((number - hurt, true))
    } else {
        None
    }
}

/// What the client does when an entity event arrives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityEventResponse {
    /// Nothing at all.
    None,
    /// Play the event's sound.
    Sound,
    /// A step, landing, jump, mantle, ladder grab or slide: the movement sound for the surface.
    FootstepNoise,
    /// Leave aim down the sights.
    ResetAds,
    /// Any gun firing, a vehicle gunner's included: flash, sound, tracer.
    GunShot,
    /// A spent case flies from the gun.
    ShellEject,
    /// A melee swing: loose glass shards in front of the swinger break (BO2's `EV_FIRE_MELEE`).
    FireMelee,
    /// Blood from a knife hit.
    MeleeGore,
    /// A bullet strikes a surface, a body or a shield: impact effect and sound.
    BulletHit,
    /// A missile hits a riot shield, or a blast is partly stopped by one: its bearer's view shakes. No impact effect.
    ShieldBlast,
    /// A grenade bounces.
    GrenadeBounceImpact,
    /// A crossbow bolt or a thrown blade strikes (BO2's `EV_BOLT_IMPACT`).
    BoltImpact,
    /// A grenade, rocket or flashbang goes off.
    Explosion,
    /// The Hellstorm missile's boost kicks in (BO2's `EV_MISSILE_REMOTE_BOOST`).
    RemoteMissileBoost,
    /// A script's earthquake (BO2's `EV_EARTHQUAKE`).
    Earthquake,
    /// A script starts or stops a rumble: the pad shakes.
    Rumble,
    /// Play a particle effect.
    PlayFx,
    /// A kill for the kill feed.
    Obituary,
}

/// An event number the client has no effect for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntityEventNotHandled(pub EventCode);

/// The client's effect for `event`. Numbers with no effect, `RELOAD_ADDAMMO` among them, are an error for the
/// caller to report.
pub fn entity_event_action(event: EventCode) -> Result<EntityEventResponse, EntityEventNotHandled> {
    let action = match event {
        EventCode::NONE => EntityEventResponse::None,
        EventCode::FIRE_WEAPON
        | EventCode::GUN_LAST_ROUND
        | EventCode::GUN_LEFT_HAND
        | EventCode::GUN_LAST_ROUND_LEFT
        | EventCode::FIRE_GUNNER_1
        | EventCode::FIRE_GUNNER_2
        | EventCode::FIRE_GUNNER_3
        | EventCode::FIRE_GUNNER_4
        | EventCode::FIRE_GUNNER_1A
        | EventCode::FIRE_GUNNER_2A
        | EventCode::FIRE_GUNNER_3A
        | EventCode::FIRE_GUNNER_4A => EntityEventResponse::GunShot,
        EventCode::ROUND_HIT
        | EventCode::ROUND_HIT_SHIELD
        | EventCode::ROUND_HIT_BODY_SMALL
        | EventCode::ROUND_HIT_BODY_LARGE
        | EventCode::BULLET_HIT_CLIENT_SHIELD => EntityEventResponse::BulletHit,
        EventCode::EXPLOSIVE_IMPACT_ON_SHIELD | EventCode::EXPLOSIVE_SPLASH_ON_SHIELD => EntityEventResponse::ShieldBlast,
        EventCode::BOMB_BURST | EventCode::ROCKET_EXPLODE | EventCode::ROCKET_EXPLODE_NOMARKS | EventCode::FLASHBANG_EXPLODE => {
            EntityEventResponse::Explosion
        }
        EventCode::PLAY_RUMBLE_ON_ENT
        | EventCode::PLAY_RUMBLE_ON_POS
        | EventCode::PLAY_RUMBLELOOP_ON_ENT
        | EventCode::PLAY_RUMBLELOOP_ON_POS
        | EventCode::STOP_RUMBLE
        | EventCode::STOP_ALL_RUMBLES => EntityEventResponse::Rumble,
        EventCode::FOOTSTEP_SPRINT
        | EventCode::FOOTSTEP_RUN
        | EventCode::FOOTSTEP_WALK
        | EventCode::FOOTSTEP_CROUCHRUN
        | EventCode::FOOTSTEP_CROUCHWALK
        | EventCode::FOOTSTEP_PRONE
        | EventCode::LADDER_HAND_GRAB
        | EventCode::MANTLE
        | EventCode::LEAP
        | EventCode::SLIDE_START
        | EventCode::SLIDE_STOP => EntityEventResponse::FootstepNoise,
        EventCode::FOLIAGE_SOUND
        | EventCode::GUN_SOUND_OFF
        | EventCode::SOUND_ALIAS
        | EventCode::ALL_SOUNDS_OFF
        | EventCode::ITEM_PICKUP
        | EventCode::AMMO_PICKUP
        | EventCode::NOAMMO
        | EventCode::RELOAD
        | EventCode::RELOAD_FROM_EMPTY
        | EventCode::RELOAD_START
        | EventCode::RELOAD_END
        | EventCode::RAISE_WEAPON
        | EventCode::FIRST_RAISE_WEAPON
        | EventCode::PUTAWAY_WEAPON
        | EventCode::WEAPON_ALT
        | EventCode::PULLBACK_WEAPON
        | EventCode::RECHAMBER_WEAPON
        | EventCode::MELEE_SWIPE
        | EventCode::MELEE_HIT
        | EventCode::MELEE_MISS
        | EventCode::PREP_OFFHAND
        | EventCode::USE_OFFHAND
        | EventCode::DETONATE => EntityEventResponse::Sound,
        EventCode::PLAY_FX | EventCode::SENSOR_LASER => EntityEventResponse::PlayFx,
        EventCode::RESET_ADS => EntityEventResponse::ResetAds,
        EventCode::EJECT_BRASS => EntityEventResponse::ShellEject,
        EventCode::FIRE_MELEE => EntityEventResponse::FireMelee,
        EventCode::MELEE_BLOOD => EntityEventResponse::MeleeGore,
        EventCode::BOMB_HOP => EntityEventResponse::GrenadeBounceImpact,
        EventCode::BOLT_IMPACT => EntityEventResponse::BoltImpact,
        EventCode::MISSILE_REMOTE_BOOST => EntityEventResponse::RemoteMissileBoost,
        EventCode::EARTHQUAKE => EntityEventResponse::Earthquake,
        EventCode::OBITUARY => EntityEventResponse::Obituary,
        landing if landing.touchdown_material_index().is_some() => EntityEventResponse::FootstepNoise,
        unknown => return Err(EntityEventNotHandled(unknown)),
    };
    Ok(action)
}

/// The shot event a client predicts for itself: `hand` 0 is the right gun, anything else the left, and the clip's
/// last round has its own event.
#[must_use]
pub const fn predicted_weapon_fire_event(hand: i32, prior_shot: bool) -> EventCode {
    // Indexed by [left hand][last round of the clip].
    const BY_HAND: [[EventCode; 2]; 2] = [
        [EventCode::FIRE_WEAPON, EventCode::GUN_LAST_ROUND],
        [EventCode::GUN_LEFT_HAND, EventCode::GUN_LAST_ROUND_LEFT],
    ];
    BY_HAND[(hand != 0) as usize][prior_shot as usize]
}

/// The event a vehicle sends when gunner `gunner` (0 to 3) fires, from the gun's first flash tag or its `second`.
#[must_use]
pub const fn gunner_fire_event(gunner: usize, second: bool) -> EventCode {
    EventCode(event::gunner_fire_event(gunner, second) as i32)
}

/// The strike event for a bullet's impact type. The client being hit hears its own versions: 1 and 9 small, 2
/// large, 3 its shield. Everyone else gets the plain strike for 1, 2 and 9, and the shield strike for 3. Other
/// types send nothing.
pub const fn bullet_hit_event(impact_kind_id: i32, local_client: bool) -> Option<EventCode> {
    let event = match (impact_kind_id, local_client) {
        (1 | 9, true) => EventCode::ROUND_HIT_BODY_SMALL,
        (2, true) => EventCode::ROUND_HIT_BODY_LARGE,
        (3, true) => EventCode::BULLET_HIT_CLIENT_SHIELD,
        (1 | 2 | 9, false) => EventCode::ROUND_HIT,
        (3, false) => EventCode::ROUND_HIT_SHIELD,
        _ => return None,
    };
    Some(event)
}

/// The entity number that means "the listener's own sound": no entity.
pub const LOCAL_SOUND_ENTITY: i32 = crate::NO_ENTITY_SLOT;

/// The ring as the client reads it: four slots, the sequence wrapping and unwrapping as `crate::event_ring` has it.
const RING_SLOTS: i32 = EVENT_SLOTS as i32;
const RING_SEQUENCE_MASK: i32 = EVENT_SEQUENCE_WRAP as i32 - 1;

/// One event read from an entity's ring, with its place in the entity's count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OrderedEntityEvent {
    pub ring_seq: i32,
    pub code: EventCode,
    pub parm_value: i32,
}

/// Whether a packet entity of this type carries an event ring. Effect entities and the temporary event entities
/// (types from `crate::et::EVENTS` up) do not.
pub fn packet_entity_uses_event_ring(e_type: i16) -> bool {
    e_type < et::EVENTS && !matches!(e_type, et::FX | et::LOOP_FX)
}

/// Plays every event the entity added since `seen`, the last count this client handled, oldest first, and moves
/// `seen` up to the entity's count. A count of zero is a fresh entity: nothing plays and `seen` starts over. Only
/// the four newest events are still in the ring, so a client further behind than that skips the older ones.
pub fn drain_entity_events(state: &EntitySnapshot, seen: &mut i32, mut play: impl FnMut(OrderedEntityEvent)) {
    let latest = i32::from(state.event_sequence);
    if latest == *seen {
        return;
    }
    if latest == 0 {
        *seen = 0;
        return;
    }
    // Far above the entity's count means our count wrapped first: take it back one turn.
    let mut from = i32::from(unwrap_previous(latest as i16, *seen as i16));
    if latest - from > RING_SLOTS {
        from = latest - RING_SLOTS;
    }
    for count in from..latest {
        let slot = (count & (RING_SLOTS - 1)) as usize;
        play(OrderedEntityEvent {
            ring_seq: count & RING_SEQUENCE_MASK,
            code: EventCode(state.events[slot].into()),
            parm_value: state.event_parms[slot] as i32,
        });
    }
    *seen = latest;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EntitySnapshot;

    /// BO2's gunner shots are 202 to 209; to the client every one is a gun firing.
    #[test]
    fn every_gunner_shot_is_a_gun_firing() {
        assert_eq!(gunner_fire_event(0, false), EventCode::FIRE_GUNNER_1);
        assert_eq!(gunner_fire_event(3, true), EventCode::FIRE_GUNNER_4A);
        for number in 202..=209 {
            let event = EventCode(number);
            assert!(event.on_the_wire());
            assert_eq!(entity_event_action(event), Ok(EntityEventResponse::GunShot));
        }
    }

    #[test]
    fn landings_cover_31_surfaces_in_each_block() {
        let soft = event::LANDING_FIRST as i32;
        let hurt = event::LANDING_PAIN_FIRST as i32;
        assert_eq!(EventCode(soft + 30).touchdown_material_index(), Some(30));
        assert_eq!(EventCode(soft + 31).touchdown_material_index(), None);
        assert!(EventCode(hurt + 4).hurts_on_landing() && !EventCode(soft + 4).hurts_on_landing());
        assert!(entity_event_action(EventCode(hurt + 31)).is_err());
    }

    #[test]
    fn a_reader_far_behind_gets_only_the_last_four() {
        let mut state = EntitySnapshot::default();
        state.event_sequence = 10;
        state.events = [1, 2, 3, 4];
        let mut seen = 3;
        let mut got = [0; 8];
        let mut n = 0;
        drain_entity_events(&state, &mut seen, |e| {
            got[n] = e.ring_seq;
            n += 1;
        });
        assert_eq!((&got[..n], seen), (&[6, 7, 8, 9][..], 10));
    }

    #[test]
    fn a_reader_across_the_256_wrap_keeps_the_events_before_it() {
        let mut state = EntitySnapshot::default();
        state.event_sequence = 2;
        let mut seen = 0xfe;
        let mut got = [0; 8];
        let mut n = 0;
        drain_entity_events(&state, &mut seen, |e| {
            got[n] = e.ring_seq;
            n += 1;
        });
        assert_eq!((&got[..n], seen), (&[0xfe, 0xff, 0, 1][..], 2));
    }
}
