//! BO2's `entity_event_t` (`EV_*`), with BO2's own numbers. An event is
//! one byte in `entityState_s::events` and in the player's predictable
//! event ring.

pub const NONE: u8 = 0;
pub const FOLIAGE_SOUND: u8 = 1;
pub const STOP_WEAPON_SOUND: u8 = 2;
pub const STOP_SOUND_ALIAS: u8 = 3;
pub const SOUND_ALIAS: u8 = 4;
pub const SOUND_ALIAS_NOTIFY: u8 = 5;
pub const SOUND_BATTLECHAT_ALIAS: u8 = 6;
pub const STOPSOUNDS: u8 = 7;
pub const STANCE_FORCE_STAND: u8 = 8;
pub const STANCE_FORCE_CROUCH: u8 = 9;
pub const STANCE_FORCE_PRONE: u8 = 10;
pub const ITEM_PICKUP: u8 = 11;
pub const AMMO_PICKUP: u8 = 12;
pub const NOAMMO: u8 = 13;
pub const NOAMMO_LEFT: u8 = 14;
pub const EMPTYCLIP: u8 = 15;
pub const EMPTY_OFFHAND: u8 = 16;
pub const RESET_ADS: u8 = 17;
pub const RELOAD: u8 = 18;
pub const RELOAD_FROM_EMPTY: u8 = 19;
pub const RELOAD_START: u8 = 20;
pub const RELOAD_END: u8 = 21;
pub const RELOAD_START_NOTIFY: u8 = 22;
pub const RELOAD_ADDAMMO: u8 = 23;
pub const RAISE_WEAPON: u8 = 24;
pub const FIRST_RAISE_WEAPON: u8 = 25;
pub const RAISE_PICKUP_WEAPON: u8 = 26;
pub const PUTAWAY_WEAPON: u8 = 27;
pub const WEAPON_ALT: u8 = 28;
pub const WEAPON_SWITCH_STARTED: u8 = 29;
pub const PULLBACK_WEAPON: u8 = 30;
pub const FIRE_WEAPON_SCRIPTED: u8 = 31;
pub const FIRE_WEAPON: u8 = 32;
pub const FIRE_WEAPON_LASTSHOT: u8 = 33;
pub const FIRE_WEAPON_LEFT: u8 = 34;
pub const FIRE_WEAPON_LASTSHOT_LEFT: u8 = 35;
pub const FIRE_RICOCHET: u8 = 36;
pub const RECHAMBER_WEAPON: u8 = 37;
pub const EJECT_BRASS: u8 = 38;
pub const MELEE_SWIPE: u8 = 39;
pub const FIRE_MELEE: u8 = 40;
pub const WEAPON_DEPLOYING: u8 = 41;
pub const WEAPON_FINISH_DEPLOYING: u8 = 42;
pub const WEAPON_BREAKING_DOWN: u8 = 43;
pub const WEAPON_FINISH_BREAKING_DOWN: u8 = 44;
pub const INC_CHARGESHOT_LEVEL: u8 = 45;
pub const CHARGESHOT_DISCHARGEWEAPON: u8 = 46;
pub const PREP_OFFHAND: u8 = 47;
pub const USE_OFFHAND: u8 = 48;
pub const SWITCH_OFFHAND: u8 = 49;
pub const PREP_RIOTSHIELD: u8 = 50;
pub const DEPLOY_RIOTSHIELD: u8 = 51;
pub const LOWER_RIOTSHIELD: u8 = 52;
pub const MELEE_HIT: u8 = 53;
pub const MELEE_MISS: u8 = 54;
pub const MELEE_BLOOD: u8 = 55;
pub const FIRE_WEAPON_MOUNTED_TURRET: u8 = 56;
pub const FIRE_WEAPON_MOUNTED_TURRETA: u8 = 57;
pub const FIRE_BARREL_1: u8 = 58;
pub const FIRE_BARREL_2: u8 = 59;
pub const FIRE_BARREL_3: u8 = 60;
pub const FIRE_BARREL_4: u8 = 61;
pub const BULLET_HIT: u8 = 62;
pub const BULLET_HIT_SHIELD: u8 = 63;
pub const BULLET_HIT_CLIENT_SHIELD: u8 = 64;
pub const EXPLOSIVE_IMPACT_ON_SHIELD: u8 = 65;
pub const EXPLOSIVE_SPLASH_ON_SHIELD: u8 = 66;
pub const BULLET_HIT_CLIENT_SMALL: u8 = 67;
pub const BULLET_HIT_CLIENT_LARGE: u8 = 68;
pub const DESTRUCTIBLE_BULLET_HIT: u8 = 69;
pub const DESTRUCTIBLE_EXPLOSION_HIT: u8 = 70;
pub const GRENADE_BOUNCE: u8 = 71;
pub const GRENADE_EXPLODE: u8 = 72;
pub const ROCKET_EXPLODE: u8 = 73;
pub const ROCKET_EXPLODE_NOMARKS: u8 = 74;
pub const FLASHBANG_EXPLODE: u8 = 75;
pub const CUSTOM_EXPLODE: u8 = 76;
pub const CUSTOM_EXPLODE_NOMARKS: u8 = 77;
pub const SENSOR_GRENADE_EXPLODE: u8 = 78;
pub const CHANGE_TO_DUD: u8 = 79;
pub const DUD_EXPLODE: u8 = 80;
pub const DUD_IMPACT: u8 = 81;
pub const FIRE_EXPLODE: u8 = 82;
pub const TIMED_FX: u8 = 83;
pub const MOLOTOV_FLOAT: u8 = 84;
pub const FAKE_FIRE: u8 = 85;
pub const PLAY_FX: u8 = 86;
pub const PLAY_FX_ON_TAG: u8 = 87;
pub const PHYS_EXPLOSION_SPHERE: u8 = 88;
pub const PHYS_EXPLOSION_CYLINDER: u8 = 89;
pub const PHYS_EXPLOSION_JOLT: u8 = 90;
pub const PHYS_JET_THRUST: u8 = 91;
pub const PHYS_LAUNCH: u8 = 92;
pub const CREATE_DYNENT: u8 = 93;
pub const CREATE_ROPE: u8 = 94;
pub const BLOOD_IMPACTS: u8 = 95;
pub const DETACH_ENTITY: u8 = 96;
pub const DELETE_ROPE: u8 = 97;
pub const ROPE_COLLIDE: u8 = 98;
pub const ROPE_SETFLAG: u8 = 99;
pub const ROPE_SETPARAM: u8 = 100;
pub const ROPE_ADDANCHOR: u8 = 101;
pub const ROPE_REMOVEANCHOR: u8 = 102;
pub const ROPE_MOVEANCHOR: u8 = 103;
pub const SETWETNESS: u8 = 104;
pub const EARTHQUAKE: u8 = 105;
pub const GRENADE_DROP: u8 = 106;
pub const GRENADE_SUICIDE: u8 = 107;
pub const DETONATE: u8 = 108;
pub const NIGHTVISION_WEAR: u8 = 109;
pub const NIGHTVISION_REMOVE: u8 = 110;
pub const MISSILE_REMOTE_BOOST: u8 = 111;
pub const PLAY_RUMBLE_ON_ENT: u8 = 112;
pub const PLAY_RUMBLE_ON_POS: u8 = 113;
pub const PLAY_RUMBLELOOP_ON_ENT: u8 = 114;
pub const PLAY_RUMBLELOOP_ON_POS: u8 = 115;
pub const STOP_RUMBLE: u8 = 116;
pub const STOP_ALL_RUMBLES: u8 = 117;
pub const OBITUARY: u8 = 118;
pub const REVIVE_OBITUARY: u8 = 119;
pub const DIRECTIONAL_HIT_INDICATOR: u8 = 120;
pub const ANIMATE_UI: u8 = 121;
pub const KILLSTREAK_DAMAGE: u8 = 122;
pub const NO_FRAG_GRENADE_HINT: u8 = 123;
pub const NO_SPECIAL_GRENADE_HINT: u8 = 124;
pub const TARGET_TOO_CLOSE_HINT: u8 = 125;
pub const TARGET_NOT_ENOUGH_CLEARANCE: u8 = 126;
pub const LOCKON_REQUIRED_HINT: u8 = 127;
pub const FOOTSTEP_SPRINT: u8 = 128;
pub const FOOTSTEP_RUN: u8 = 129;
pub const FOOTSTEP_WALK: u8 = 130;
pub const FOOTSTEP_CROUCHRUN: u8 = 131;
pub const FOOTSTEP_CROUCHWALK: u8 = 132;
pub const FOOTSTEP_PRONE: u8 = 133;
pub const LADDER_HAND_GRAB: u8 = 134;
pub const MANTLE: u8 = 135;
pub const JUMP: u8 = 136;
pub const LANDING_FIRST: u8 = 137;
pub const LANDING_LAST: u8 = 168;
pub const LANDING_PAIN_FIRST: u8 = 169;
pub const LANDING_PAIN_LAST: u8 = 200;
pub const FIRE_VEHICLE_TURRET: u8 = 201;
pub const FIRE_GUNNER_1: u8 = 202;
pub const FIRE_GUNNER_2: u8 = 203;
pub const FIRE_GUNNER_3: u8 = 204;
pub const FIRE_GUNNER_4: u8 = 205;
pub const FIRE_GUNNER_1A: u8 = 206;
pub const FIRE_GUNNER_2A: u8 = 207;
pub const FIRE_GUNNER_3A: u8 = 208;
pub const FIRE_GUNNER_4A: u8 = 209;
pub const START_CAMERA_TWEEN: u8 = 210;
pub const DESTRUCTIBLE_DISABLE_PIECES: u8 = 211;
pub const FOOTPRINT: u8 = 212;
pub const CANNOTPLANT: u8 = 213;
pub const CANT_EQUIP_PRONE: u8 = 214;
pub const DTP_LAUNCH: u8 = 215;
pub const DTP_LAND: u8 = 216;
pub const SLIDE_START: u8 = 217;
pub const SLIDE_STOP: u8 = 218;
pub const SCOPE_ZOOM: u8 = 219;
pub const JAM_WEAPON: u8 = 220;
pub const STACKFIRE: u8 = 221;
pub const BOLT_IMPACT: u8 = 222;
pub const BOLT_IMPALE: u8 = 223;
pub const PLAY_WEAPON_DEATH_EFFECTS: u8 = 224;
pub const PLAY_WEAPON_DAMAGE_EFFECTS: u8 = 225;
pub const FACE_EVENT: u8 = 226;
pub const SETLOCALWIND: u8 = 227;
pub const FLOAT_LONGER: u8 = 228;
pub const FORCE_BUOYANCY: u8 = 229;
pub const DISABLE_DEPTH_BUOYANCY_ADJUSTMENTS: u8 = 230;
pub const SCALE_BUOYANCY: u8 = 231;
pub const ALLOWPITCH: u8 = 232;
pub const GIB: u8 = 233;
pub const STANCE_INVALID: u8 = 234;
pub const ENTER_VEHICLE: u8 = 235;
pub const EXIT_VEHICLE: u8 = 236;
pub const IMMEDIATE_RAGDOLL: u8 = 237;
pub const ENT_SPAWNED_SENTINEL: u8 = 238;
pub const MAX_EVENTS: u8 = 239;

/// Events our server sends that BO2 has no number for. They sit past BO2's
/// one-byte table, so they never collide with a BO2 event.
pub mod ours {
    /// The Sensor Grenade's laser on an enemy this frame. BO2 draws it in
    /// `CG_SensorGrenade`; our server sends it where the grenade runs
    /// (TODO: draw it from the grenade's entity state).
    pub const SENSOR_LASER: i32 = 0x100;
}

/// What `CG_EntityEvent` does with an event, grouped the way its switch
/// groups them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventAction {
    /// In the switch, but BO2's client does nothing with it.
    Ignored,
    /// Plays or stops a sound on the entity, and nothing more.
    Sound,
    /// `EV_FOLIAGE_SOUND`: tells the client script.
    Foliage,
    /// `EV_STANCE_FORCE_*`: the local player's stance is set.
    Stance,
    /// `EV_ITEM_PICKUP`, `EV_AMMO_PICKUP`.
    Pickup,
    /// `EV_EMPTY_OFFHAND`: the "no offhand" notify, then the offhand cycles.
    EmptyOffhand,
    ResetAds,
    /// `EV_RAISE_PICKUP_WEAPON`: the airburst range resets.
    RaisePickup,
    /// A gun's shot: the player's (31-35), a mounted turret's (56-57), a
    /// barrel's (58-61) or a vehicle turret's (201).
    WeaponFire,
    /// A vehicle gunner's shot (202-209); `gunner_of_fire_event` says whose.
    GunnerFire,
    /// `EV_FIRE_RICOCHET`, `EV_RECHAMBER_WEAPON`, `EV_EJECT_BRASS`.
    EjectBrass,
    /// `EV_FIRE_MELEE`: dyn ents and glass near the swing react.
    FireMelee,
    PrepOffhand,
    UseOffhand,
    SwitchOffhand,
    DeployRiotshield,
    MeleeHit,
    MeleeBlood,
    /// `EV_BULLET_HIT`, `EV_BULLET_HIT_SHIELD`: impact effect and sound.
    BulletHit,
    /// `EV_BULLET_HIT_CLIENT_SHIELD`: the shield bearer's own hit.
    BulletHitClientShield,
    /// `EV_EXPLOSIVE_IMPACT_ON_SHIELD` / `_SPLASH_ON_SHIELD`: the bearer's
    /// view reacts.
    ShieldBlast,
    /// `EV_BULLET_HIT_CLIENT_SMALL` / `_LARGE`: a player was shot; the
    /// flesh sound for everyone but the victim.
    BulletHitClient,
    DestructibleHit,
    GrenadeBounce,
    /// Grenade, rocket, flashbang, custom, dud and fire explosions.
    Explosion,
    SensorGrenade,
    /// An effect at the entity: dud change and impact, timed fx, molotov
    /// float, `EV_PLAY_FX`, `EV_PLAY_FX_ON_TAG`.
    PlayFx,
    FakeFire,
    /// `EV_PHYS_*`: ragdolls and physics objects pushed.
    Physics,
    CreateDynent,
    /// `EV_BLOOD_IMPACTS`: turns a body's blood impacts on or off.
    BloodImpacts,
    Wetness,
    Earthquake,
    RemoteMissileBoost,
    Rumble,
    Obituary,
    DirectionalHitIndicator,
    /// A "can't do that" hint: no grenade, too close, no clearance,
    /// lock-on needed, can't plant, can't go prone, invalid stance.
    Hint,
    /// `EV_FOOTSTEP_*` and `EV_LADDER_HAND_GRAB` (128-134).
    Footstep,
    Mantle,
    Jump,
    /// 137-200: a landing on `surface`, painful or not.
    Landing { surface: u8, pain: bool },
    CameraTween,
    DestructiblePieces,
    /// `EV_DTP_LAUNCH`, `EV_DTP_LAND` (dive to prone).
    DiveToProne,
    /// `EV_SLIDE_START` spawns the slide's loop sound; `EV_SLIDE_STOP`
    /// frees it.
    Slide,
    BoltImpact,
    FaceEvent,
    Buoyancy,
    Gib,
    VehicleNotify,
    Ragdoll,
}

/// The `CG_EntityEvent` case an event takes. `None` for `EV_NONE`, from
/// `EV_ENT_SPAWNED_SENTINEL` on, and for the events BO2's client has no
/// case for (ropes, `EV_DETACH_ENTITY`, `EV_ANIMATE_UI`, `EV_FOOTPRINT`,
/// the weapon death and damage effects); BO2 errors on those with
/// "Unknown event".
#[must_use]
pub const fn event_action(ev: u8) -> Option<EventAction> {
    use EventAction as A;
    if let Some((surface, pain)) = landing_of_event(ev) {
        return Some(A::Landing { surface, pain });
    }
    Some(match ev {
        FOLIAGE_SOUND => A::Foliage,
        2..=7 | 13 | 14 | 18..=21 | 24 | 25 | 27 | 28 | 30 | 39 | 41..=46 | 54 | 108..=110
        | SCOPE_ZOOM | STACKFIRE => A::Sound,
        8..=10 => A::Stance,
        ITEM_PICKUP | AMMO_PICKUP => A::Pickup,
        EMPTYCLIP | RELOAD_START_NOTIFY | RELOAD_ADDAMMO | WEAPON_SWITCH_STARTED
        | PREP_RIOTSHIELD | LOWER_RIOTSHIELD | GRENADE_DROP | GRENADE_SUICIDE
        | KILLSTREAK_DAMAGE | JAM_WEAPON | SETLOCALWIND | ALLOWPITCH => A::Ignored,
        EMPTY_OFFHAND => A::EmptyOffhand,
        RESET_ADS => A::ResetAds,
        RAISE_PICKUP_WEAPON => A::RaisePickup,
        31..=35 | 56..=61 | FIRE_VEHICLE_TURRET => A::WeaponFire,
        202..=209 => A::GunnerFire,
        FIRE_RICOCHET | RECHAMBER_WEAPON | EJECT_BRASS => A::EjectBrass,
        FIRE_MELEE => A::FireMelee,
        PREP_OFFHAND => A::PrepOffhand,
        USE_OFFHAND => A::UseOffhand,
        SWITCH_OFFHAND => A::SwitchOffhand,
        DEPLOY_RIOTSHIELD => A::DeployRiotshield,
        MELEE_HIT => A::MeleeHit,
        MELEE_BLOOD => A::MeleeBlood,
        BULLET_HIT | BULLET_HIT_SHIELD => A::BulletHit,
        BULLET_HIT_CLIENT_SHIELD => A::BulletHitClientShield,
        EXPLOSIVE_IMPACT_ON_SHIELD | EXPLOSIVE_SPLASH_ON_SHIELD => A::ShieldBlast,
        BULLET_HIT_CLIENT_SMALL | BULLET_HIT_CLIENT_LARGE => A::BulletHitClient,
        DESTRUCTIBLE_BULLET_HIT | DESTRUCTIBLE_EXPLOSION_HIT => A::DestructibleHit,
        GRENADE_BOUNCE => A::GrenadeBounce,
        72..=77 | DUD_EXPLODE | FIRE_EXPLODE => A::Explosion,
        SENSOR_GRENADE_EXPLODE => A::SensorGrenade,
        CHANGE_TO_DUD | DUD_IMPACT | TIMED_FX | MOLOTOV_FLOAT | PLAY_FX | PLAY_FX_ON_TAG => {
            A::PlayFx
        }
        FAKE_FIRE => A::FakeFire,
        88..=92 => A::Physics,
        CREATE_DYNENT => A::CreateDynent,
        BLOOD_IMPACTS => A::BloodImpacts,
        SETWETNESS => A::Wetness,
        EARTHQUAKE => A::Earthquake,
        MISSILE_REMOTE_BOOST => A::RemoteMissileBoost,
        112..=117 => A::Rumble,
        OBITUARY | REVIVE_OBITUARY => A::Obituary,
        DIRECTIONAL_HIT_INDICATOR => A::DirectionalHitIndicator,
        123..=127 | CANNOTPLANT | CANT_EQUIP_PRONE | STANCE_INVALID => A::Hint,
        128..=134 => A::Footstep,
        MANTLE => A::Mantle,
        JUMP => A::Jump,
        START_CAMERA_TWEEN => A::CameraTween,
        DESTRUCTIBLE_DISABLE_PIECES => A::DestructiblePieces,
        DTP_LAUNCH | DTP_LAND => A::DiveToProne,
        SLIDE_START | SLIDE_STOP => A::Slide,
        BOLT_IMPACT | BOLT_IMPALE => A::BoltImpact,
        FACE_EVENT => A::FaceEvent,
        228..=231 => A::Buoyancy,
        GIB => A::Gib,
        ENTER_VEHICLE | EXIT_VEHICLE => A::VehicleNotify,
        IMMEDIATE_RAGDOLL => A::Ragdoll,
        _ => return None,
    })
}

/// `PM_Weapon_FireWeapon`'s event: `EV_FIRE_WEAPON`, plus one when that
/// shot emptied the clip, plus two for the left gun.
#[must_use]
pub const fn fire_weapon_event(left_gun: bool, clip_empty: bool) -> u8 {
    FIRE_WEAPON + clip_empty as u8 + 2 * left_gun as u8
}

/// The left gun's shots, which `CG_EntityEvent` fires from the dual-wield
/// weapon.
#[must_use]
pub const fn is_left_gun_fire(ev: u8) -> bool {
    matches!(ev, FIRE_WEAPON_LEFT | FIRE_WEAPON_LASTSHOT_LEFT)
}

/// The shots that emptied the clip, as `CG_FireWeapon` tests them.
#[must_use]
pub const fn is_last_shot_fire(ev: u8) -> bool {
    matches!(ev, FIRE_WEAPON_LASTSHOT | FIRE_WEAPON_LASTSHOT_LEFT)
}

/// A vehicle gunner's shot: `EV_FIRE_GUNNER_1` plus the gunner (0-3), or
/// `EV_FIRE_GUNNER_1A` plus the gunner for the gun's second flash tag.
#[must_use]
pub const fn gunner_fire_event(gunner: usize, second: bool) -> u8 {
    FIRE_GUNNER_1 + (gunner & 3) as u8 + 4 * second as u8
}

/// The gunner (0-3) a gunner fire event belongs to, as `CG_EntityEvent`
/// works it out (`(event - EV_FIRE_GUNNER_1) & 3`).
#[must_use]
pub const fn gunner_of_fire_event(ev: u8) -> Option<usize> {
    if ev >= FIRE_GUNNER_1 && ev <= FIRE_GUNNER_4A {
        Some(((ev - FIRE_GUNNER_1) & 3) as usize)
    } else {
        None
    }
}

/// The event everyone but a shot player gets (`finishPlayerDamage`):
/// `EV_BULLET_HIT_CLIENT_LARGE` for a weapon with `bRifleBullet`, else
/// `_SMALL`. The victim gets `EV_BULLET_HIT` instead, and every other
/// bullet strike is `EV_BULLET_HIT` too.
#[must_use]
pub const fn client_bullet_hit_event(rifle_bullet: bool) -> u8 {
    BULLET_HIT_CLIENT_SMALL + rifle_bullet as u8
}

/// A landing on `surface` (0-31), painful or not.
#[must_use]
pub const fn landing_event(surface: u8, pain: bool) -> u8 {
    let first = if pain { LANDING_PAIN_FIRST } else { LANDING_FIRST };
    first + (surface & 31)
}

/// The surface of a landing event, and whether it hurt.
#[must_use]
pub const fn landing_of_event(ev: u8) -> Option<(u8, bool)> {
    if ev >= LANDING_FIRST && ev <= LANDING_LAST {
        Some((ev - LANDING_FIRST, false))
    } else if ev >= LANDING_PAIN_FIRST && ev <= LANDING_PAIN_LAST {
        Some((ev - LANDING_PAIN_FIRST, true))
    } else {
        None
    }
}

/// The step `CG_AddPlayerFootstepEvent` takes (0 sprint, 1 run, 2 walk,
/// 3 crouch run, 4 crouch walk, 5 prone, 6 ladder hand grab).
#[must_use]
pub const fn footstep_of_event(ev: u8) -> Option<u8> {
    if ev >= FOOTSTEP_SPRINT && ev <= LADDER_HAND_GRAB {
        Some(ev - FOOTSTEP_SPRINT)
    } else {
        None
    }
}

/// The view dip (`cg.landChange`) a landing event gives the local player
/// it belongs to; `landTime` is set with it. `parm` is the event parm;
/// `fall_min` and `fall_max` are `BG_GetFallDamageMinHeight` and
/// `BG_GetFallDamageMaxHeight`. A soft landing dips by `parm`. A painful
/// one turns `parm` (a percent of the fall damage range) back into a
/// height and dips 4 per 26 units past 12, plus 4, at most 24; from 12
/// or less it leaves the view alone.
#[must_use]
pub fn land_change(pain: bool, parm: i32, fall_min: f32, fall_max: f32) -> Option<f32> {
    if !pain {
        return Some(-(parm as f32));
    }
    let height = fall_min + (parm as f64 * 0.01 * (fall_max - fall_min) as f64) as f32;
    if height <= 12.0 {
        return None;
    }
    let dip = (((height - 12.0) * 0.038_461_54 + 1.0) * 4.0) as i32;
    if dip < 1 {
        return None;
    }
    Some(-(dip.min(24) as f32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_event_bo2_handles_has_a_case() {
        for e in 0..=255u8 {
            let unknown = matches!(e, 0 | 94 | 96..=103 | 121 | 212 | 224 | 225 | 238..=255);
            assert_eq!(event_action(e).is_none(), unknown, "event {e}");
        }
        assert_eq!(event_action(MANTLE), Some(EventAction::Mantle));
        assert_eq!(event_action(STACKFIRE), Some(EventAction::Sound));
        assert_eq!(event_action(EMPTYCLIP), Some(EventAction::Ignored));
    }

    #[test]
    fn fire_events() {
        assert_eq!(fire_weapon_event(false, false), FIRE_WEAPON);
        assert_eq!(fire_weapon_event(false, true), FIRE_WEAPON_LASTSHOT);
        assert_eq!(fire_weapon_event(true, false), FIRE_WEAPON_LEFT);
        assert_eq!(fire_weapon_event(true, true), FIRE_WEAPON_LASTSHOT_LEFT);
        assert!(is_left_gun_fire(35) && !is_left_gun_fire(33));
        assert!(is_last_shot_fire(33) && is_last_shot_fire(35) && !is_last_shot_fire(32));
        assert_eq!(gunner_fire_event(0, false), FIRE_GUNNER_1);
        assert_eq!(gunner_fire_event(3, true), FIRE_GUNNER_4A);
        assert_eq!(gunner_of_fire_event(FIRE_GUNNER_3A), Some(2));
        assert_eq!(gunner_of_fire_event(FIRE_VEHICLE_TURRET), None);
        assert_eq!(event_action(FIRE_GUNNER_2A), Some(EventAction::GunnerFire));
        assert_eq!(client_bullet_hit_event(true), BULLET_HIT_CLIENT_LARGE);
        assert_eq!(client_bullet_hit_event(false), BULLET_HIT_CLIENT_SMALL);
    }

    #[test]
    fn landings_and_steps() {
        assert_eq!(landing_event(0, false), LANDING_FIRST);
        assert_eq!(landing_event(31, true), LANDING_PAIN_LAST);
        assert_eq!(landing_of_event(LANDING_FIRST + 7), Some((7, false)));
        assert_eq!(landing_of_event(LANDING_PAIN_FIRST + 3), Some((3, true)));
        assert_eq!(landing_of_event(JUMP), None);
        assert_eq!(event_action(170), Some(EventAction::Landing { surface: 1, pain: true }));
        assert_eq!(footstep_of_event(LADDER_HAND_GRAB), Some(6));
        assert_eq!(footstep_of_event(MANTLE), None);
        assert_eq!(land_change(false, 9, 0.0, 0.0), Some(-9.0));
        // 0% of the range from a 12 unit minimum: no dip.
        assert_eq!(land_change(true, 0, 12.0, 128.0), None);
        // 25 units: 13 past 12, (13 / 26 + 1) * 4 = 6.
        assert_eq!(land_change(true, 50, 0.0, 50.0), Some(-6.0));
        // 200 units: 32, held to 24.
        assert_eq!(land_change(true, 100, 12.0, 200.0), Some(-24.0));
    }
}
