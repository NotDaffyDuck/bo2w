//! BO2's `entityType_t` and its entity-number limits.

/// BO2's `entityType_t`, as `entityState_s::eType` holds it.
pub mod et {
    pub const GENERAL: i16 = 0;
    pub const PLAYER: i16 = 1;
    pub const PLAYER_CORPSE: i16 = 2;
    pub const ITEM: i16 = 3;
    pub const MISSILE: i16 = 4;
    pub const INVISIBLE: i16 = 5;
    pub const SCRIPTMOVER: i16 = 6;
    pub const SOUND_BLEND: i16 = 7;
    pub const FX: i16 = 8;
    pub const LOOP_FX: i16 = 9;
    pub const PRIMARY_LIGHT: i16 = 10;
    pub const TURRET: i16 = 11;
    pub const HELICOPTER: i16 = 12;
    pub const PLANE: i16 = 13;
    pub const VEHICLE: i16 = 14;
    pub const VEHICLE_CORPSE: i16 = 15;
    pub const ACTOR: i16 = 16;
    pub const ACTOR_SPAWNER: i16 = 17;
    pub const ACTOR_CORPSE: i16 = 18;
    pub const STREAMER_HINT: i16 = 19;
    pub const ZBARRIER: i16 = 20;
    /// A temporary event entity: `eType` is `EVENTS` plus the event number.
    pub const EVENTS: i16 = 21;
}

/// The `eType` of a temporary entity that carries event `ev`.
pub const fn event_entity_type(ev: u8) -> i16 {
    et::EVENTS + ev as i16
}

/// The event an `eType` carries, if it is a temporary event entity.
pub const fn event_of_entity_type(e_type: i16) -> Option<u8> {
    if e_type >= et::EVENTS && e_type - et::EVENTS < crate::event::MAX_EVENTS as i16 {
        Some((e_type - et::EVENTS) as u8)
    } else {
        None
    }
}

/// BO2's entity slots (`MAX_GENTITIES`).
pub const BO2_MAX_GENTITIES: i32 = 1024;
/// The game's entity slots. Every entity limit and `ENTITYNUM_*` number in
/// the game comes from this one number. It is still MW2's 2048; moving the
/// game to BO2's 1024 ([`BO2_MAX_GENTITIES`]) is this one line.
pub const MAX_GENTITIES: i32 = 2048;
/// `ENTITYNUM_NONE`: no entity (`Dtp_Start` clears `groundEntityNum` to it).
pub const ENTITYNUM_NONE: i32 = MAX_GENTITIES - 1;
/// `ENTITYNUM_WORLD`: the world, as ground and as a trace hit.
pub const ENTITYNUM_WORLD: i32 = MAX_GENTITIES - 2;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_bo2s() {
        assert_eq!(BO2_MAX_GENTITIES - 1, 0x3ff);
        assert_eq!(ENTITYNUM_NONE, MAX_GENTITIES - 1);
        assert_eq!(ENTITYNUM_WORLD, MAX_GENTITIES - 2);
        assert_eq!(et::SCRIPTMOVER, 6);
        assert_eq!(et::EVENTS, 21);
    }

    #[test]
    fn event_entities_round_trip() {
        let e = event_entity_type(crate::event::MAX_EVENTS - 1);
        assert_eq!(event_of_entity_type(e), Some(crate::event::MAX_EVENTS - 1));
        assert_eq!(event_of_entity_type(et::ZBARRIER), None);
        assert_eq!(event_of_entity_type(et::EVENTS + 239), None);
    }
}

#[cfg(test)]
mod playerstate_bridge_tests {
    #[test]
    fn bridge_entitynum_none_matches() {
        assert_eq!(playerstate_t6::game::ENTITYNUM_NONE, super::ENTITYNUM_NONE);
    }
}
