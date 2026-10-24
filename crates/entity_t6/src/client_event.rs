//! Entity events as the client sees them. BO2 numbers its events in `crate::event`; here they get one named type,
//! the effect each one drives on the client, the event a shot or a bullet strike sends, and the reader for the
//! four-slot event ring a packet entity carries.

use crate::{EntityState, et, event as bo2};

/// One entity event: BO2's event number, or our own number past BO2's table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct EntityEventKind(pub i32);

/// Gives each listed BO2 event its name on [`EntityEventKind`], with the same number as in `crate::event`.
macro_rules! bo2_events {
    ($($name:ident)*) => {
        impl EntityEventKind {
            $(pub const $name: Self = Self(bo2::$name as i32);)*
        }
    };
}

bo2_events! {
    NONE
    // Sounds and the weapon's own cycle.
    FOLIAGE_SOUND STOP_WEAPON_SOUND SOUND_ALIAS STOPSOUNDS ITEM_PICKUP AMMO_PICKUP NOAMMO RESET_ADS
    RELOAD RELOAD_FROM_EMPTY RELOAD_START RELOAD_END RELOAD_ADDAMMO
    RAISE_WEAPON FIRST_RAISE_WEAPON PUTAWAY_WEAPON WEAPON_ALT PULLBACK_WEAPON RECHAMBER_WEAPON EJECT_BRASS
    // Shots, both hands, and the last round of a clip.
    FIRE_WEAPON FIRE_WEAPON_LASTSHOT FIRE_WEAPON_LEFT FIRE_WEAPON_LASTSHOT_LEFT
    // Knife, grenades and equipment in hand.
    MELEE_SWIPE FIRE_MELEE MELEE_HIT MELEE_MISS MELEE_BLOOD PREP_OFFHAND USE_OFFHAND DETONATE
    // Where bullets and blasts land.
    BULLET_HIT BULLET_HIT_SHIELD BULLET_HIT_CLIENT_SMALL BULLET_HIT_CLIENT_LARGE BULLET_HIT_CLIENT_SHIELD
    EXPLOSIVE_IMPACT_ON_SHIELD EXPLOSIVE_SPLASH_ON_SHIELD
    GRENADE_BOUNCE GRENADE_EXPLODE ROCKET_EXPLODE ROCKET_EXPLODE_NOMARKS FLASHBANG_EXPLODE
    // Script effects: particle effects, earthquakes, the Hellstorm's boost, pad rumble.
    PLAY_FX EARTHQUAKE MISSILE_REMOTE_BOOST
    PLAY_RUMBLE_ON_ENT PLAY_RUMBLE_ON_POS PLAY_RUMBLELOOP_ON_ENT PLAY_RUMBLELOOP_ON_POS STOP_RUMBLE STOP_ALL_RUMBLES
    OBITUARY
    // The body moving: steps by gait, ladder, mantle, jump.
    FOOTSTEP_SPRINT FOOTSTEP_RUN FOOTSTEP_WALK FOOTSTEP_CROUCHRUN FOOTSTEP_CROUCHWALK FOOTSTEP_PRONE
    LADDER_HAND_GRAB MANTLE JUMP
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
        impl EntityEventKind {
            pub const $soft: Self = Self(bo2::LANDING_FIRST as i32 + $index);
            pub const $hurt: Self = Self(bo2::LANDING_PAIN_FIRST as i32 + $index);
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

impl EntityEventKind {
    /// Ours, not BO2's: the Sensor Grenade's laser on an enemy this frame. BO2 draws it in `CG_SensorGrenade`; our
    /// server sends it from where the grenade runs, on a number past BO2's table. TODO: draw it from the grenade's
    /// entity state instead.
    pub const SENSOR_LASER: Self = Self(0x100);

    /// The surface of a landing, soft or hurt, counted from the first surface.
    pub const fn landing_surface_index(self) -> Option<i32> {
        match landing_block(self.0) {
            Some((surface, _)) => Some(surface),
            None => None,
        }
    }

    /// A landing hard enough to hurt.
    pub const fn is_landing_pain(self) -> bool {
        matches!(landing_block(self.0), Some((_, true)))
    }

    /// The wire carries every number of BO2's table after "none", plus our Sensor Grenade laser.
    pub const fn on_the_wire(self) -> bool {
        self.0 == Self::SENSOR_LASER.0 || (self.0 > Self::NONE.0 && self.0 < bo2::MAX_EVENTS as i32)
    }
}

/// `(surface, hurt)` for a landing number from either block.
const fn landing_block(number: i32) -> Option<(i32, bool)> {
    let soft = bo2::LANDING_FIRST as i32;
    let hurt = bo2::LANDING_PAIN_FIRST as i32;
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
pub enum EntityEventAction {
    /// Nothing at all.
    None,
    /// Play the event's sound.
    Sound,
    /// A step, landing, jump, mantle, ladder grab or slide: the movement sound for the surface.
    MovementSound,
    /// Leave aim down the sights.
    ResetAds,
    /// Any gun firing, a vehicle gunner's included: flash, sound, tracer.
    WeaponFire,
    /// A spent case flies from the gun.
    EjectBrass,
    /// A melee swing: loose glass shards in front of the swinger break (BO2's `EV_FIRE_MELEE`).
    FireMelee,
    /// Blood from a knife hit.
    MeleeBlood,
    /// A bullet strikes a surface, a body or a shield: impact effect and sound.
    BulletHit,
    /// A missile hits a riot shield, or a blast is partly stopped by one: its bearer's view shakes. No impact effect.
    ShieldBlast,
    /// A grenade bounces.
    GrenadeContact,
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
pub struct UnsupportedEntityEvent(pub EntityEventKind);

/// The client's effect for `event`. Numbers with no effect, `RELOAD_ADDAMMO` among them, are an error for the
/// caller to report.
pub fn entity_event_action(event: EntityEventKind) -> Result<EntityEventAction, UnsupportedEntityEvent> {
    use EntityEventAction as Then;
    use EntityEventKind as Ev;
    let action = match event {
        Ev::NONE => Then::None,
        Ev::FIRE_WEAPON
        | Ev::FIRE_WEAPON_LASTSHOT
        | Ev::FIRE_WEAPON_LEFT
        | Ev::FIRE_WEAPON_LASTSHOT_LEFT
        | Ev::FIRE_GUNNER_1
        | Ev::FIRE_GUNNER_2
        | Ev::FIRE_GUNNER_3
        | Ev::FIRE_GUNNER_4
        | Ev::FIRE_GUNNER_1A
        | Ev::FIRE_GUNNER_2A
        | Ev::FIRE_GUNNER_3A
        | Ev::FIRE_GUNNER_4A => Then::WeaponFire,
        Ev::BULLET_HIT
        | Ev::BULLET_HIT_SHIELD
        | Ev::BULLET_HIT_CLIENT_SMALL
        | Ev::BULLET_HIT_CLIENT_LARGE
        | Ev::BULLET_HIT_CLIENT_SHIELD => Then::BulletHit,
        Ev::EXPLOSIVE_IMPACT_ON_SHIELD | Ev::EXPLOSIVE_SPLASH_ON_SHIELD => Then::ShieldBlast,
        Ev::GRENADE_EXPLODE | Ev::ROCKET_EXPLODE | Ev::ROCKET_EXPLODE_NOMARKS | Ev::FLASHBANG_EXPLODE => {
            Then::Explosion
        }
        Ev::PLAY_RUMBLE_ON_ENT
        | Ev::PLAY_RUMBLE_ON_POS
        | Ev::PLAY_RUMBLELOOP_ON_ENT
        | Ev::PLAY_RUMBLELOOP_ON_POS
        | Ev::STOP_RUMBLE
        | Ev::STOP_ALL_RUMBLES => Then::Rumble,
        Ev::FOOTSTEP_SPRINT
        | Ev::FOOTSTEP_RUN
        | Ev::FOOTSTEP_WALK
        | Ev::FOOTSTEP_CROUCHRUN
        | Ev::FOOTSTEP_CROUCHWALK
        | Ev::FOOTSTEP_PRONE
        | Ev::LADDER_HAND_GRAB
        | Ev::MANTLE
        | Ev::JUMP
        | Ev::SLIDE_START
        | Ev::SLIDE_STOP => Then::MovementSound,
        Ev::FOLIAGE_SOUND
        | Ev::STOP_WEAPON_SOUND
        | Ev::SOUND_ALIAS
        | Ev::STOPSOUNDS
        | Ev::ITEM_PICKUP
        | Ev::AMMO_PICKUP
        | Ev::NOAMMO
        | Ev::RELOAD
        | Ev::RELOAD_FROM_EMPTY
        | Ev::RELOAD_START
        | Ev::RELOAD_END
        | Ev::RAISE_WEAPON
        | Ev::FIRST_RAISE_WEAPON
        | Ev::PUTAWAY_WEAPON
        | Ev::WEAPON_ALT
        | Ev::PULLBACK_WEAPON
        | Ev::RECHAMBER_WEAPON
        | Ev::MELEE_SWIPE
        | Ev::MELEE_HIT
        | Ev::MELEE_MISS
        | Ev::PREP_OFFHAND
        | Ev::USE_OFFHAND
        | Ev::DETONATE => Then::Sound,
        Ev::PLAY_FX | Ev::SENSOR_LASER => Then::PlayFx,
        Ev::RESET_ADS => Then::ResetAds,
        Ev::EJECT_BRASS => Then::EjectBrass,
        Ev::FIRE_MELEE => Then::FireMelee,
        Ev::MELEE_BLOOD => Then::MeleeBlood,
        Ev::GRENADE_BOUNCE => Then::GrenadeContact,
        Ev::BOLT_IMPACT => Then::BoltImpact,
        Ev::MISSILE_REMOTE_BOOST => Then::RemoteMissileBoost,
        Ev::EARTHQUAKE => Then::Earthquake,
        Ev::OBITUARY => Then::Obituary,
        landing if landing.landing_surface_index().is_some() => Then::MovementSound,
        unknown => return Err(UnsupportedEntityEvent(unknown)),
    };
    Ok(action)
}

/// The shot event a client predicts for itself: `hand` 0 is the right gun, anything else the left, and the clip's
/// last round has its own event.
#[must_use]
pub const fn predicted_weapon_fire_event(hand: i32, last_shot: bool) -> EntityEventKind {
    let left = hand != 0;
    match (left, last_shot) {
        (true, true) => EntityEventKind::FIRE_WEAPON_LASTSHOT_LEFT,
        (true, false) => EntityEventKind::FIRE_WEAPON_LEFT,
        (false, true) => EntityEventKind::FIRE_WEAPON_LASTSHOT,
        (false, false) => EntityEventKind::FIRE_WEAPON,
    }
}

/// The event a vehicle sends when gunner `gunner` (0 to 3) fires, from the gun's first flash tag or its `second`.
#[must_use]
pub const fn gunner_fire_event(gunner: usize, second: bool) -> EntityEventKind {
    EntityEventKind(bo2::gunner_fire_event(gunner, second) as i32)
}

/// The strike event for a bullet's impact type. The client being hit hears its own versions: 1 and 9 small, 2
/// large, 3 its shield. Everyone else gets the plain strike for 1, 2 and 9, and the shield strike for 3. Other
/// types send nothing.
pub const fn bullet_hit_event(impact_type: i32, local_client: bool) -> Option<EntityEventKind> {
    let event = match (impact_type, local_client) {
        (1 | 9, true) => EntityEventKind::BULLET_HIT_CLIENT_SMALL,
        (2, true) => EntityEventKind::BULLET_HIT_CLIENT_LARGE,
        (3, true) => EntityEventKind::BULLET_HIT_CLIENT_SHIELD,
        (1 | 2 | 9, false) => EntityEventKind::BULLET_HIT,
        (3, false) => EntityEventKind::BULLET_HIT_SHIELD,
        _ => return None,
    };
    Some(event)
}

/// The entity number that means "the listener's own sound": no entity.
pub const LOCAL_SOUND_ENTITY: i32 = crate::ENTITYNUM_NONE;

/// The ring as the client reads it. TODO(fact): this wrap window and counter mask are MW2's; BO2's own ring is in
/// `crate::event_ring` (wrap 0x100, window 0x40), and this reader does not use it yet.
const RING_SLOTS: i32 = 4;
const RING_SEQUENCE_MASK: i32 = 0x7ff;
const RING_WRAP_WINDOW: i32 = 0x200;

/// One event read from an entity's ring, with its place in the entity's count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SequencedEntityEvent {
    pub sequence: i32,
    pub event: EntityEventKind,
    pub event_parm: i32,
}

/// Whether a packet entity of this type carries an event ring. Effect entities and the temporary event entities
/// (types from `et::EVENTS` up) do not.
pub fn packet_entity_uses_event_ring(e_type: i16) -> bool {
    e_type < et::EVENTS && !matches!(e_type, et::FX | et::LOOP_FX)
}

/// Plays every event the entity added since `seen`, the last count this client handled, oldest first, and moves
/// `seen` up to the entity's count. A count of zero is a fresh entity: nothing plays and `seen` starts over. Only
/// the four newest events are still in the ring, so a client further behind than that skips the older ones.
pub fn consume_entity_events(state: &EntityState, seen: &mut i32, mut play: impl FnMut(SequencedEntityEvent)) {
    let latest = i32::from(state.event_sequence);
    if latest == *seen {
        return;
    }
    if latest == 0 {
        *seen = 0;
        return;
    }
    let mut from = *seen;
    // Far above the entity's count means our count wrapped first: take it back one turn.
    if from > latest + RING_WRAP_WINDOW {
        from -= RING_SEQUENCE_MASK + 1;
    }
    if latest - from > RING_SLOTS {
        from = latest - RING_SLOTS;
    }
    for count in from..latest {
        let slot = (count & (RING_SLOTS - 1)) as usize;
        play(SequencedEntityEvent {
            sequence: count & RING_SEQUENCE_MASK,
            event: EntityEventKind(state.events[slot].into()),
            event_parm: state.event_parms[slot] as i32,
        });
    }
    *seen = latest;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// BO2's gunner shots are 202 to 209; to the client every one is a gun firing.
    #[test]
    fn every_gunner_shot_is_a_gun_firing() {
        assert_eq!(gunner_fire_event(0, false), EntityEventKind::FIRE_GUNNER_1);
        assert_eq!(gunner_fire_event(3, true), EntityEventKind::FIRE_GUNNER_4A);
        for number in 202..=209 {
            let event = EntityEventKind(number);
            assert!(event.on_the_wire());
            assert_eq!(entity_event_action(event), Ok(EntityEventAction::WeaponFire));
        }
    }

    #[test]
    fn landings_cover_31_surfaces_in_each_block() {
        let soft = bo2::LANDING_FIRST as i32;
        let hurt = bo2::LANDING_PAIN_FIRST as i32;
        assert_eq!(EntityEventKind(soft + 30).landing_surface_index(), Some(30));
        assert_eq!(EntityEventKind(soft + 31).landing_surface_index(), None);
        assert!(EntityEventKind(hurt + 4).is_landing_pain() && !EntityEventKind(soft + 4).is_landing_pain());
        assert!(entity_event_action(EntityEventKind(hurt + 31)).is_err());
    }

    #[test]
    fn a_reader_far_behind_gets_only_the_last_four() {
        let mut state = EntityState::default();
        state.event_sequence = 10;
        state.events = [1, 2, 3, 4];
        let mut seen = 3;
        let mut got = [0; 8];
        let mut n = 0;
        consume_entity_events(&state, &mut seen, |e| {
            got[n] = e.sequence;
            n += 1;
        });
        assert_eq!((&got[..n], seen), (&[6, 7, 8, 9][..], 10));
    }
}
