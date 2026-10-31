//! BO2 entity event numbers the weapon brain raises (BO2's `entity_event_t` values).
//!
//! Only the weapon-related part of the list; add more here (through window 1) when a
//! mechanic needs one.

pub const EV_NONE: u16 = 0;
pub const EV_FOLIAGE_SOUND: u16 = 1;
pub const EV_STOP_WEAPON_SOUND: u16 = 2;
pub const EV_STOP_SOUND_ALIAS: u16 = 3;
pub const EV_SOUND_ALIAS: u16 = 4;
pub const EV_SOUND_ALIAS_NOTIFY: u16 = 5;
pub const EV_SOUND_BATTLECHAT_ALIAS: u16 = 6;
pub const EV_STOPSOUNDS: u16 = 7;
pub const EV_STANCE_FORCE_STAND: u16 = 8;
pub const EV_STANCE_FORCE_CROUCH: u16 = 9;
pub const EV_STANCE_FORCE_PRONE: u16 = 10;
pub const EV_ITEM_PICKUP: u16 = 11;
pub const EV_AMMO_PICKUP: u16 = 12;
pub const EV_NOAMMO: u16 = 13;
pub const EV_NOAMMO_LEFT: u16 = 14;
pub const EV_EMPTYCLIP: u16 = 15;
pub const EV_EMPTY_OFFHAND: u16 = 16;
pub const SIGNAL_SCOPE_RESTART: u16 = 17;
pub const EV_RELOAD: u16 = 18;
pub const EV_RELOAD_FROM_EMPTY: u16 = 19;
pub const EV_RELOAD_START: u16 = 20;
pub const EV_RELOAD_END: u16 = 21;
pub const EV_RELOAD_START_NOTIFY: u16 = 22;
pub const EV_RELOAD_ADDAMMO: u16 = 23;
pub const EV_RAISE_WEAPON: u16 = 24;
pub const EV_FIRST_RAISE_WEAPON: u16 = 25;
pub const EV_RAISE_PICKUP_WEAPON: u16 = 26;
pub const EV_PUTAWAY_WEAPON: u16 = 27;
pub const EV_WEAPON_ALT: u16 = 28;
pub const EV_WEAPON_SWITCH_STARTED: u16 = 29;
pub const EV_PULLBACK_WEAPON: u16 = 30;
pub const EV_FIRE_WEAPON_SCRIPTED: u16 = 31;
pub const EV_FIRE_WEAPON: u16 = 32;
pub const EV_FIRE_WEAPON_LASTSHOT: u16 = 33;
pub const EV_FIRE_WEAPON_LEFT: u16 = 34;
pub const EV_FIRE_WEAPON_LASTSHOT_LEFT: u16 = 35;
pub const EV_FIRE_RICOCHET: u16 = 36;
pub const EV_RECHAMBER_WEAPON: u16 = 37;
pub const EV_EJECT_BRASS: u16 = 38;
pub const EV_MELEE_SWIPE: u16 = 39;
pub const EV_FIRE_MELEE: u16 = 40;
pub const EV_WEAPON_DEPLOYING: u16 = 41;
pub const EV_WEAPON_FINISH_DEPLOYING: u16 = 42;
pub const EV_WEAPON_BREAKING_DOWN: u16 = 43;
pub const EV_WEAPON_FINISH_BREAKING_DOWN: u16 = 44;
pub const EV_INC_CHARGESHOT_LEVEL: u16 = 45;
pub const EV_CHARGESHOT_DISCHARGEWEAPON: u16 = 46;
pub const EV_PREP_OFFHAND: u16 = 47;
pub const EV_USE_OFFHAND: u16 = 48;
pub const EV_SWITCH_OFFHAND: u16 = 49;
pub const EV_PREP_RIOTSHIELD: u16 = 50;
pub const EV_DEPLOY_RIOTSHIELD: u16 = 51;
pub const EV_LOWER_RIOTSHIELD: u16 = 52;
pub const EV_MELEE_HIT: u16 = 53;
pub const EV_MELEE_MISS: u16 = 54;
pub const EV_MELEE_BLOOD: u16 = 55;
pub const EV_FIRE_WEAPON_MOUNTED_TURRET: u16 = 56;
pub const EV_FIRE_WEAPON_MOUNTED_TURRETA: u16 = 57;
pub const EV_FIRE_BARREL_1: u16 = 58;
pub const EV_FIRE_BARREL_2: u16 = 59;
pub const EV_FIRE_BARREL_3: u16 = 60;
pub const EV_FIRE_BARREL_4: u16 = 61;
pub const EV_EARTHQUAKE: u16 = 105;
pub const EV_GRENADE_DROP: u16 = 106;
pub const EV_GRENADE_SUICIDE: u16 = 107;
pub const EV_DETONATE: u16 = 108;
pub const EV_NIGHTVISION_WEAR: u16 = 109;
pub const EV_NIGHTVISION_REMOVE: u16 = 110;
pub const EV_NO_FRAG_GRENADE_HINT: u16 = 123;
pub const EV_NO_SPECIAL_GRENADE_HINT: u16 = 124;
pub const EV_TARGET_TOO_CLOSE_HINT: u16 = 125;
pub const EV_TARGET_NOT_ENOUGH_CLEARANCE: u16 = 126;
pub const EV_LOCKON_REQUIRED_HINT: u16 = 127;
pub const EV_CANNOTPLANT: u16 = 213;
pub const EV_DTP_LAUNCH: u16 = 215;
pub const EV_DTP_LAND: u16 = 216;
pub const EV_SLIDE_START: u16 = 217;
pub const EV_SLIDE_STOP: u16 = 218;
pub const EV_SCOPE_ZOOM: u16 = 219;
pub const EV_JAM_WEAPON: u16 = 220;
pub const EV_STACKFIRE: u16 = 221;

/// One event raised by a weapon tick, in the order BO2 raised it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    /// BO2 event number (`EV_*`).
    pub id: u16,
    /// Event parameter.
    pub parm: u32,
    /// `true` = `BG_AddPredictableEventToPlayerstate` (carries a parm); `false` = `PM_AddEvent`.
    pub predictable: bool,
}

/// Third-person anim script events the weapon tick fires (BO2's `scriptAnimEventTypes_t` names).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnimScript {
    DropWeapon,
    RaiseWeapon,
    FirstRaiseWeapon,
    /// Offhand prepare for a grenade throw.
    PrimeGrenade,
    /// Offhand prepare for gear that is not thrown (tactical / equipment).
    PrimeGear,
    /// ANIM_ET_RELOAD: third-person reload.
    Reload,
    /// Gun fire start (third person StartFiring).
    FireWeapon,
    /// Riot shield fire start.
    RiotshieldStart,
    /// Offhand throw start.
    OffHandStart,
    /// Dive to prone: leaving the ground / landing.
    DtpTakeoff,
    DtpLand,
    /// Moving stopped (third person run-to-idle).
    RunToIdle,
    MeleeAttack,
    KnifeMelee,
    MeleeLeft,
    KnifeMeleeCharge,
}

/// Name of an event number, for traces.
pub fn name(id: u16) -> &'static str {
    match id {
        EV_NONE => "NONE",
        EV_FOLIAGE_SOUND => "FOLIAGE_SOUND",
        EV_STOP_WEAPON_SOUND => "STOP_WEAPON_SOUND",
        EV_STOP_SOUND_ALIAS => "STOP_SOUND_ALIAS",
        EV_SOUND_ALIAS => "SOUND_ALIAS",
        EV_SOUND_ALIAS_NOTIFY => "SOUND_ALIAS_NOTIFY",
        EV_SOUND_BATTLECHAT_ALIAS => "SOUND_BATTLECHAT_ALIAS",
        EV_STOPSOUNDS => "STOPSOUNDS",
        EV_STANCE_FORCE_STAND => "STANCE_FORCE_STAND",
        EV_STANCE_FORCE_CROUCH => "STANCE_FORCE_CROUCH",
        EV_STANCE_FORCE_PRONE => "STANCE_FORCE_PRONE",
        EV_ITEM_PICKUP => "ITEM_PICKUP",
        EV_AMMO_PICKUP => "AMMO_PICKUP",
        EV_NOAMMO => "NOAMMO",
        EV_NOAMMO_LEFT => "NOAMMO_LEFT",
        EV_EMPTYCLIP => "EMPTYCLIP",
        EV_EMPTY_OFFHAND => "EMPTY_OFFHAND",
        SIGNAL_SCOPE_RESTART => "RESET_ADS",
        EV_RELOAD => "RELOAD",
        EV_RELOAD_FROM_EMPTY => "RELOAD_FROM_EMPTY",
        EV_RELOAD_START => "RELOAD_START",
        EV_RELOAD_END => "RELOAD_END",
        EV_RELOAD_START_NOTIFY => "RELOAD_START_NOTIFY",
        EV_RELOAD_ADDAMMO => "RELOAD_ADDAMMO",
        EV_RAISE_WEAPON => "RAISE_WEAPON",
        EV_FIRST_RAISE_WEAPON => "FIRST_RAISE_WEAPON",
        EV_RAISE_PICKUP_WEAPON => "RAISE_PICKUP_WEAPON",
        EV_PUTAWAY_WEAPON => "PUTAWAY_WEAPON",
        EV_WEAPON_ALT => "WEAPON_ALT",
        EV_WEAPON_SWITCH_STARTED => "WEAPON_SWITCH_STARTED",
        EV_PULLBACK_WEAPON => "PULLBACK_WEAPON",
        EV_FIRE_WEAPON_SCRIPTED => "FIRE_WEAPON_SCRIPTED",
        EV_FIRE_WEAPON => "FIRE_WEAPON",
        EV_FIRE_WEAPON_LASTSHOT => "FIRE_WEAPON_LASTSHOT",
        EV_FIRE_WEAPON_LEFT => "FIRE_WEAPON_LEFT",
        EV_FIRE_WEAPON_LASTSHOT_LEFT => "FIRE_WEAPON_LASTSHOT_LEFT",
        EV_FIRE_RICOCHET => "FIRE_RICOCHET",
        EV_RECHAMBER_WEAPON => "RECHAMBER_WEAPON",
        EV_EJECT_BRASS => "EJECT_BRASS",
        EV_MELEE_SWIPE => "MELEE_SWIPE",
        EV_FIRE_MELEE => "FIRE_MELEE",
        EV_WEAPON_DEPLOYING => "WEAPON_DEPLOYING",
        EV_WEAPON_FINISH_DEPLOYING => "WEAPON_FINISH_DEPLOYING",
        EV_WEAPON_BREAKING_DOWN => "WEAPON_BREAKING_DOWN",
        EV_WEAPON_FINISH_BREAKING_DOWN => "WEAPON_FINISH_BREAKING_DOWN",
        EV_INC_CHARGESHOT_LEVEL => "INC_CHARGESHOT_LEVEL",
        EV_CHARGESHOT_DISCHARGEWEAPON => "CHARGESHOT_DISCHARGEWEAPON",
        EV_PREP_OFFHAND => "PREP_OFFHAND",
        EV_USE_OFFHAND => "USE_OFFHAND",
        EV_SWITCH_OFFHAND => "SWITCH_OFFHAND",
        EV_PREP_RIOTSHIELD => "PREP_RIOTSHIELD",
        EV_DEPLOY_RIOTSHIELD => "DEPLOY_RIOTSHIELD",
        EV_LOWER_RIOTSHIELD => "LOWER_RIOTSHIELD",
        EV_MELEE_HIT => "MELEE_HIT",
        EV_MELEE_MISS => "MELEE_MISS",
        EV_MELEE_BLOOD => "MELEE_BLOOD",
        EV_FIRE_WEAPON_MOUNTED_TURRET => "FIRE_WEAPON_MOUNTED_TURRET",
        EV_FIRE_WEAPON_MOUNTED_TURRETA => "FIRE_WEAPON_MOUNTED_TURRETA",
        EV_FIRE_BARREL_1 => "FIRE_BARREL_1",
        EV_FIRE_BARREL_2 => "FIRE_BARREL_2",
        EV_FIRE_BARREL_3 => "FIRE_BARREL_3",
        EV_FIRE_BARREL_4 => "FIRE_BARREL_4",
        EV_EARTHQUAKE => "EARTHQUAKE",
        EV_GRENADE_DROP => "GRENADE_DROP",
        EV_GRENADE_SUICIDE => "GRENADE_SUICIDE",
        EV_DETONATE => "DETONATE",
        EV_NIGHTVISION_WEAR => "NIGHTVISION_WEAR",
        EV_NIGHTVISION_REMOVE => "NIGHTVISION_REMOVE",
        EV_NO_FRAG_GRENADE_HINT => "NO_FRAG_GRENADE_HINT",
        EV_NO_SPECIAL_GRENADE_HINT => "NO_SPECIAL_GRENADE_HINT",
        EV_TARGET_TOO_CLOSE_HINT => "TARGET_TOO_CLOSE_HINT",
        EV_TARGET_NOT_ENOUGH_CLEARANCE => "TARGET_NOT_ENOUGH_CLEARANCE",
        EV_LOCKON_REQUIRED_HINT => "LOCKON_REQUIRED_HINT",
        EV_CANNOTPLANT => "CANNOTPLANT",
        EV_DTP_LAUNCH => "DTP_LAUNCH",
        EV_DTP_LAND => "DTP_LAND",
        EV_SLIDE_START => "SLIDE_START",
        EV_SLIDE_STOP => "SLIDE_STOP",
        EV_SCOPE_ZOOM => "SCOPE_ZOOM",
        EV_JAM_WEAPON => "JAM_WEAPON",
        EV_STACKFIRE => "STACKFIRE",
        _ => "?",
    }
}
