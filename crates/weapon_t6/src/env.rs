//! Per-tick values the weapon brain reads (and a few it writes) that belong to movement,
//! the entity or dvars. The game fills this from its own movement state each tick.
//!
//! All bit values are BO2's. Add a field here through window 1 when a mechanic needs one.

/// BO2 `pmtype_t` values the weapon code tests.
pub mod pm_type {
    pub const NORMAL: i32 = 0;
    pub const NORMAL_LINKED: i32 = 1;
    pub const NOCLIP: i32 = 2;
    pub const UFO: i32 = 3;
    pub const MPVIEWER: i32 = 4;
    pub const SPECTATOR: i32 = 5;
    pub const INTERMISSION: i32 = 6;
    pub const LASTSTAND: i32 = 7;
    pub const REVIVEE: i32 = 8;
    pub const LASTSTAND_TRANSITION: i32 = 9;
    pub const DEAD: i32 = 10;
    pub const DEAD_LINKED: i32 = 11;
}

/// BO2 `pm_flags` bits the weapon code tests.
pub mod pmf {
    pub const PRONE: u32 = 0x1;
    /// Mantling: ADS is refused.
    pub const MANTLE: u32 = 0x4;
    /// Mantling / ladder: the weapon must go away.
    pub const LADDER: u32 = 0x8;
    /// Aiming down sights.
    pub const ADS: u32 = 0x10;
    /// Set by PM_SetProneMovementOverride while prone; cleared by PM_Weapon_Idle. The ADS flag
    /// update also sets it with ADS (0x210) as "ADS granted while prone".
    pub const PRONE_MOVE_OVERRIDE: u32 = 0x200;
    /// Frozen: no weapon tick at all.
    pub const FROZEN: u32 = 0x400;
    /// No firing.
    pub const NO_FIRE: u32 = 0x800;
    /// Must be clear to start a melee (meaning not identified yet).
    pub const GATE_2000: u32 = 0x2000;
    /// Dive-to-prone still in the air.
    pub const DTP_AIR: u32 = 0x20_0000;
    /// Melee lunge running (set by the lunge start).
    pub const MELEE_CHARGE: u32 = 0x2_0000;
    /// Sprinting (BO2's footstep code names it). A riot shield cannot be planted while it is set.
    pub const SPRINT: u32 = 0x8000;
    /// Sliding on slide ground.
    pub const SLIDE: u32 = 0x40_0000;
}

/// BO2 `eFlags` bits the weapon code tests.
pub mod ef {
    /// Must be clear to plant a riot shield and to melee (meaning not identified yet).
    pub const GATE_4000: u32 = 0x4000;
}

/// BO2 `otherFlags` bits the weapon code tests.
pub mod of {
    /// A live player's weapon is usable (BO2's spectator code clears it). ADS needs it.
    pub const WEAPON_USABLE: u32 = 0x4;
}

/// BO2 `eFlags2` bits the weapon code tests.
pub mod ef2 {
    /// Weapon locked (vehicle / script).
    pub const WEAPON_LOCKED: u32 = 0x1000_0000;
}

/// BO2 perk numbers the weapon timer tests (`perksEnum`).
pub mod perk {
    /// FMJ grants these two (the attachment's perks): script hasperk sees them while it is held.
    pub const ARMORPIERCING: u32 = 1;
    /// PERK_EXTRABP. Bullet_FirePenetrate tests the firing weapon for it.
    pub const BULLETPENETRATION: u32 = 6;
    pub const FASTEQUIPMENTUSE: u32 = 16;
    pub const FASTMELEE: u32 = 19;
    pub const FASTRELOAD: u32 = 20;
    pub const FASTTOSS: u32 = 21;
    pub const FASTSWITCH: u32 = 22;
    pub const RATEOFFIRE: u32 = 52;
    pub const SPRINTRECOVERY: u32 = 57;
    pub const FASTADS: u32 = 15;

    /// A script perk name (`setperk`) -> its BO2 perk number, for the perks the weapon code tests.
    pub fn by_name(name: &str) -> Option<u32> {
        Some(match name {
            "specialty_armorpiercing" => ARMORPIERCING,
            "specialty_bulletpenetration" => BULLETPENETRATION,
            "specialty_fastads" => FASTADS,
            "specialty_fastequipmentuse" => FASTEQUIPMENTUSE,
            "specialty_fastmeleerecovery" => FASTMELEE,
            "specialty_fastreload" => FASTRELOAD,
            "specialty_fasttoss" => FASTTOSS,
            "specialty_fastweaponswitch" => FASTSWITCH,
            "specialty_rof" => RATEOFFIRE,
            "specialty_sprintrecovery" => SPRINTRECOVERY,
            _ => return None,
        })
    }
}

/// The dvars the weapon brain reads, with BO2's registered defaults.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct T6Dvars {
    pub player_disable_weapons_in_water: bool,
    pub tu_enforce_rechamber_on_switch: bool,
    pub perk_weap_switch_multiplier: f32,
    pub perk_weap_reload_multiplier: f32,
    pub perk_weap_rate_multiplier: f32,
    pub perk_weap_ads_multiplier: f32,
    pub perk_weap_melee_multiplier: f32,
    pub perk_weap_toss_multiplier: f32,
    pub perk_weap_equipment_use_multiplier: f32,
    pub perk_weap_spread_multiplier: f32,
    pub player_melee_range: f32,
    pub player_bayonet_range: f32,
    pub player_sustain_ammo: bool,
    pub bg_charge_shot_queue_time: i32,
    pub bg_charge_shot_max_bullets_in_queue: i32,
    pub bg_charge_shot_fire_when_queue_reaches_max: bool,
    pub bg_charge_shot_auto_fire_delay: i32,
    pub bg_charge_shot_max_bullet_queue_on_emp: i32,
    pub bg_charge_shot_discharge_when_queue_reaches_max: bool,
    pub bg_charge_shot_auto_discharge_delay: i32,
    pub bg_charge_shot_use_one_ammo_for_multiple_bullets: bool,
    pub tu5_uav_ads_fix: bool,
    /// `dtp`: dive to prone on.
    pub dtp: bool,
    /// `dtp_post_move_pause` (ms): no crawl straight after a dive ends.
    pub dtp_post_move_pause: f32,
    pub perk_sprint_recovery_multiplier: f32,
}

impl Default for T6Dvars {
    fn default() -> Self {
        Self {
            player_disable_weapons_in_water: false,
            tu_enforce_rechamber_on_switch: true,
            perk_weap_switch_multiplier: 0.5,
            perk_weap_reload_multiplier: 0.5,
            perk_weap_rate_multiplier: 0.75,
            perk_weap_ads_multiplier: 0.5,
            perk_weap_melee_multiplier: 0.5,
            perk_weap_toss_multiplier: 0.5,
            perk_weap_equipment_use_multiplier: 0.6,
            perk_weap_spread_multiplier: 0.65,
            player_melee_range: 55.0,
            player_bayonet_range: 85.0,
            player_sustain_ammo: false,
            bg_charge_shot_queue_time: 128,
            bg_charge_shot_max_bullets_in_queue: 5,
            bg_charge_shot_fire_when_queue_reaches_max: true,
            bg_charge_shot_auto_fire_delay: 0,
            bg_charge_shot_max_bullet_queue_on_emp: 1,
            bg_charge_shot_discharge_when_queue_reaches_max: false,
            bg_charge_shot_auto_discharge_delay: 0,
            bg_charge_shot_use_one_ammo_for_multiple_bullets: true,
            tu5_uav_ads_fix: true,
            dtp: true,
            dtp_post_move_pause: 100.0,
            perk_sprint_recovery_multiplier: 0.3,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct T6Env {
    pub pm_type: i32,
    pub pm_flags: u32,
    pub e_flags: u32,
    pub e_flags2: u32,
    pub link_flags: u32,
    pub other_flags: u32,
    pub waterlevel: i32,
    /// The player's own perks (`ps->perks`), one bit per perk. The perks a weapon grants come
    /// from the facts: [`crate::pm::Pm::has_perk`] (BG_HasPerk) and
    /// [`crate::pm::Pm::hand_has_perk`] (BG_GetPerks) add them.
    pub perks: u64,
    /// `mantleState.flags`.
    pub mantle_flags: u32,
    /// Mantle time still to run (ms); 0 when not mantling.
    pub mantle_time_left: i32,
    /// Mantle_IsWeaponInactive.
    pub mantle_weapon_inactive: bool,
    /// Mantle_IsWeaponEquipped.
    pub mantle_weapon_equipped: bool,
    /// `viewHeightCurrent == viewHeightTarget`.
    pub view_height_settled: bool,
    /// BG_IsShellshockAnim: shocked by a shock file whose `bg_shock_animation` is set
    /// (concussion, flashbang, tabun gas...). Its own anim plays, so the fire script is skipped.
    pub shellshock_anim: bool,
    /// `groundEntityNum != ENTITYNUM_NONE`.
    pub on_ground: bool,
    /// BO2's `proneChange`: he went prone or got up this tick (a reload then restarts in the body).
    pub prone_change: bool,
    /// Vertical speed (CheckForDeploy reads it).
    pub velocity_z: f32,
    /// The game's plant probe passed: the eye trace (1000 units) hit no no-plant entity and
    /// the spot is not in deep water.
    pub plant_spot_ok: bool,
    /// Tactical insertion may be planted here.
    pub tac_insert_allowed: bool,
    /// Riot shield HOLD: the game's placement test failed for the aimed spot. The game fills
    /// it before the tick while the state is RIOTSHIELD_HOLD (false = can place).
    pub riotshield_blocked: bool,
    /// Console rules (PC = false: there the sprint key ends ADS).
    pub is_console: bool,
    /// In a vehicle (BO2 vehicle entity != none): no melee. Not `using_vehicle_weapon`.
    pub in_vehicle: bool,
    /// BO2 `weapLockFlags` (lock-on state; 0x2 = locked, which lock-on guns need to fire).
    pub weap_lock_flags: u32,
    /// BG_UsingVehicleWeapon.
    pub using_vehicle_weapon: bool,
    /// G_IsServerGameSystem: prints only go out on the server.
    pub is_server: bool,
    /// Com_SessionMode_IsZombiesGame.
    pub zombies: bool,
    /// `jumpTime`: below 0 while a landed dive slides along the ground.
    pub jump_time: i32,
    /// `slideTime`: the server time the current slide started.
    pub slide_time: i32,
    /// When the last dive ended (server time; 0 = never).
    pub last_dtp_end: i32,
    /// bo2mp mod menu "Rapid Fire" (not BO2): a third of the fire time (30 ms floor), no fire
    /// delay, and single-shot and burst guns keep firing while the trigger is held.
    pub rapid_fire: bool,
    /// Movement's crawl put the gun to rest this frame (BO2's
    /// PM_InteruptWeaponWithProneMove calls PM_Weapon_Idle in PmoveSingle,
    /// before the aim flag): [`crate::tick::pre_weapon`] idles the gun first.
    pub crawl_idle: bool,
    pub dvars: T6Dvars,
}

impl T6Env {
    pub fn has_perk(&self, perk: u32) -> bool {
        perk < 64 && self.perks & (1u64 << perk) != 0
    }
}

#[cfg(test)]
mod perk_name_tests {
    use super::perk;

    #[test]
    fn script_perk_names_map_to_bo2_numbers() {
        assert_eq!(perk::by_name("specialty_fastweaponswitch"), Some(perk::FASTSWITCH));
        assert_eq!(perk::by_name("specialty_rof"), Some(perk::RATEOFFIRE));
        assert_eq!(perk::by_name("specialty_fastmeleerecovery"), Some(perk::FASTMELEE));
        assert_eq!(perk::by_name("specialty_coldblooded"), None);
    }
}
