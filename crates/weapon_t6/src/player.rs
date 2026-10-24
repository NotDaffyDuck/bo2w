//! BO2's weapon part of the player state, with BO2's field names (snake case).
//!
//! Everything the weapon brain reads or writes between ticks lives here, so it can ride the
//! network as one blob and a trace can print it. Movement-owned values (pm_type, pm_flags,
//! water level, ...) come in per tick through [`crate::env::T6Env`] instead.

use crate::facts::FactsTable;

/// Bits of `weapFlags` the brain uses (BO2 values).
pub mod wf {
    /// Script asked for a reload (CheckForReload consumes it).
    pub const SCRIPT_RELOAD_REQUEST: u64 = 0x1;
    /// PWF_USING_OFFHAND: an offhand is in hand (asserted outside offhand/raise/drop states).
    pub const USING_OFFHAND: u64 = 0x2;
    /// Holding breath (scoped).
    pub const HOLDING_BREATH: u64 = 0x4;
    /// Script AllowADS(false).
    pub const NO_ADS: u64 = 0x20;
    /// Seen in BO2 only as a reader: with it set, running dry does not switch away and the
    /// alt-mode memory is kept (treated like `unlimitedAmmo`). Nothing in MP script sets it.
    pub const NO_AMMO_SWITCH: u64 = 0x200;
    /// Switching drops to no weapon (script took the weapons away). Must be clear to plant
    /// a riot shield.
    pub const DISABLE_SWITCH_TO_NONE: u64 = 0x80;
    /// Read by the riot shield plant check (must be clear); its setter is not identified yet.
    pub const GATE_100: u64 = 0x100;
    /// CheckForDeploy: the player asked to mount the weapon.
    pub const DEPLOY_REQUEST: u64 = 0x1000;
    /// The weapon is mounted (deployed). A switch reads it to break the mount down first;
    /// CheckForDeploy clears it before the breakdown.
    pub const MOUNTED: u64 = 0x2000;
    /// Old name of [`MOUNTED`], kept for switch.rs.
    pub const DEPLOY_SWITCH_PENDING: u64 = MOUNTED;
    /// Swimming put the weapon away.
    pub const SWIM_LOWERED: u64 = 0x4000;
    /// Offhand from a gun: set by OFFHAND_INIT when a gun is in hand (switching reads it to
    /// quick-drop); with it, switches drop to no weapon and finishing a drop goes to OFFHAND_INIT.
    pub const WEAPONS_DISABLED: u64 = 0x40000;
    /// A trigger pull is waiting (right / left hand).
    pub const PENDING_PULL_RIGHT: u64 = 0x400;
    pub const PENDING_PULL_LEFT: u64 = 0x800;
    /// A TV-guided shot went out (set by FireWeapon).
    pub const TV_GUIDED_FIRED: u64 = 0x1_0000;
    /// The left gun asked for a reload.
    pub const LEFT_RELOAD_REQUEST: u64 = 0x2_0000;
    /// A switch is forced quick.
    pub const FORCE_QUICK_SWITCH: u64 = 0x10_0000;
    /// Weapon change locked.
    pub const CHANGE_LOCKED: u64 = 0x20_0000;
    /// StartFiring sets this bit for a TV-guided gun (same bit as [`CHANGE_LOCKED`]).
    pub const TV_GUIDED_START: u64 = CHANGE_LOCKED;
    /// The weapon going up is a riot shield.
    pub const RIOTSHIELD_SWITCH: u64 = 0x40_0000;
    /// Riot shield HOLD: the aimed spot cannot take the shield (set while bad, cleared when good).
    pub const RIOTSHIELD_CANT_PLACE: u64 = 0x200_0000;
    /// Charge shot held.
    pub const CHARGE_SHOT: u64 = 0x400_0000;
}

/// Size of BO2's per-player weapon arrays.
pub const MAX_HELD: usize = 15;

/// One held weapon (BO2 `PlayerHeldWeapon`). Camo/render options live with the renderer.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HeldWeapon {
    pub weapon: u32,
    pub heat_percent: f32,
    pub fuel_tank_time: i32,
    pub ads_zoom_select: i32,
    pub over_heating: bool,
    pub needs_rechamber: bool,
    pub held_before: bool,
    pub quick_reload: bool,
    pub block_weapon_pickup: bool,
    pub model: u8,
}

/// Reserve ammo for one ammo type (BO2 `AmmoPool`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AmmoPool {
    pub weapon: u32,
    pub count: i32,
}

/// Rounds in one clip (BO2 `AmmoClip`), keyed by the weapon that owns the clip so a
/// dual-wield left hand has its own entry (an alt mode that shares its gun's magazine has none).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AmmoClip {
    pub weapon: u32,
    pub count: i32,
}

/// The weapon part of BO2's `playerState_s`, both hands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct T6WeaponPs {
    pub weapon: u32,
    pub weaponstate: i32,
    pub weaponstate_left: i32,
    pub weapon_time: i32,
    pub weapon_delay: i32,
    pub weapon_time_left: i32,
    pub weapon_delay_left: i32,
    pub weapon_idle_time: i32,
    pub weapon_restrict_kick_time: i32,
    pub weap_anim: i32,
    pub weap_anim_left: i32,
    pub weapon_shot_count: u32,
    pub weapon_shot_count_left: u32,
    pub b_run_left_gun: bool,
    pub weap_flags: u64,
    pub grenade_time_left: i32,
    pub off_hand_weapon: u32,
    pub offhand_primary: u32,
    pub offhand_secondary: u32,
    pub last_weapon_alt_mode_switch: u32,
    pub stowed_weapon: u32,
    pub melee_weapon: u32,
    pub last_stand_prev_weapon: u32,
    /// BO2 `inventoryWeapon`: never taken away when its clip runs dry.
    pub inventory_weapon: u32,
    pub ads_zoom_select: i32,
    pub ads_zoom_latch_time: i32,
    pub aim_spread_scale: f32,
    pub f_weapon_pos_frac: f32,
    pub ads_delay_time: i32,
    pub weapon_spin_lerp: f32,
    pub charge_shot_timer: i32,
    pub charge_shot_level: i32,
    pub shots_fired_from_chamber: i32,
    /// Shots since firing started (0..=250): StartFiring resets it, each shot adds one.
    pub intro_shots_fired: i32,
    /// Who threw back the grenade in hand (`None` = BO2's 0x3ff, nobody).
    pub throw_back_grenade_owner: Option<u16>,
    pub throw_back_grenade_time_left: i32,
    /// Entity of the use prompt under the crosshair (`None` = BO2's 0x3ff, no prompt).
    pub cursor_hint_ent_index: Option<u16>,
    /// Hold-breath sway scale (1.0 = no breath held).
    pub hold_breath_scale: f32,
    pub hold_breath_timer: i32,
    /// Quickscope scale (1.0 = settled).
    pub quick_scope_scale: f32,
    pub quick_scope_timer: i32,
    pub ads_zoom_latch_state: bool,
    pub ads_prev_zoom_select: i32,
    pub ads_prev_zoom_select_time: i32,
    /// Melee lunge target (`None` = BO2's 0x3ff).
    pub melee_charge_ent: Option<u16>,
    pub melee_charge_dist: u32,
    pub melee_charge_time: i32,
    pub held: [HeldWeapon; MAX_HELD],
    pub ammo_not_in_clip: [AmmoPool; MAX_HELD],
    pub ammo_in_clip: [AmmoClip; MAX_HELD],
}

impl Default for T6WeaponPs {
    /// A spawned player: everything zero except the breath and quickscope scales (1.0).
    fn default() -> Self {
        Self {
            weapon: Default::default(),
            weaponstate: Default::default(),
            weaponstate_left: Default::default(),
            weapon_time: Default::default(),
            weapon_delay: Default::default(),
            weapon_time_left: Default::default(),
            weapon_delay_left: Default::default(),
            weapon_idle_time: Default::default(),
            weapon_restrict_kick_time: Default::default(),
            weap_anim: Default::default(),
            weap_anim_left: Default::default(),
            weapon_shot_count: Default::default(),
            weapon_shot_count_left: Default::default(),
            b_run_left_gun: Default::default(),
            weap_flags: Default::default(),
            grenade_time_left: Default::default(),
            off_hand_weapon: Default::default(),
            offhand_primary: Default::default(),
            offhand_secondary: Default::default(),
            last_weapon_alt_mode_switch: Default::default(),
            stowed_weapon: Default::default(),
            melee_weapon: Default::default(),
            last_stand_prev_weapon: Default::default(),
            inventory_weapon: 0,
            ads_zoom_select: Default::default(),
            ads_zoom_latch_time: Default::default(),
            aim_spread_scale: Default::default(),
            f_weapon_pos_frac: Default::default(),
            ads_delay_time: Default::default(),
            weapon_spin_lerp: Default::default(),
            charge_shot_timer: Default::default(),
            charge_shot_level: Default::default(),
            shots_fired_from_chamber: Default::default(),
            intro_shots_fired: 0,
            throw_back_grenade_owner: Default::default(),
            throw_back_grenade_time_left: Default::default(),
            cursor_hint_ent_index: Default::default(),
            hold_breath_scale: 1.0,
            hold_breath_timer: 0,
            quick_scope_scale: 1.0,
            quick_scope_timer: 0,
            ads_zoom_latch_state: false,
            ads_prev_zoom_select: 0,
            ads_prev_zoom_select_time: 0,
            melee_charge_ent: None,
            melee_charge_dist: 0,
            melee_charge_time: 0,
            held: Default::default(),
            ammo_not_in_clip: Default::default(),
            ammo_in_clip: Default::default(),
        }
    }
}

/// One hand's copy of the per-hand fields, for code that runs either hand.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Hand {
    pub state: i32,
    pub time: i32,
    pub delay: i32,
    pub shot_count: u32,
}

impl T6WeaponPs {
    /// Slot of `weapon` in `held` (BG_GetHeldWeapon); weapon 0 is never held.
    pub fn held_slot(&self, weapon: u32) -> Option<usize> {
        if weapon == 0 {
            return None;
        }
        self.held.iter().position(|h| h.weapon == weapon)
    }

    pub fn held(&self, weapon: u32) -> Option<&HeldWeapon> {
        self.held_slot(weapon).map(|i| &self.held[i])
    }

    pub fn held_mut(&mut self, weapon: u32) -> Option<&mut HeldWeapon> {
        self.held_slot(weapon).map(|i| &mut self.held[i])
    }

    /// BG_PlayerHasWeapon.
    pub fn has_weapon(&self, weapon: u32) -> bool {
        self.held_slot(weapon).is_some()
    }

    /// BG_IsWeaponValid: a weapon in the table (of `num_weapons`) that he holds.
    pub fn is_weapon_valid(&self, weapon: u32, num_weapons: usize) -> bool {
        (weapon as usize) < num_weapons && self.has_weapon(weapon)
    }

    /// BG_PlayerEverHadWeapon: the held weapon has been raised before.
    pub fn ever_had_weapon(&self, weapon: u32) -> bool {
        self.held(weapon).is_some_and(|h| h.held_before)
    }

    /// BG_SetEverHadAllPlayerHeldWeapons.
    pub fn set_ever_had_all(&mut self, value: bool) {
        for h in &mut self.held {
            if h.weapon != 0 {
                h.held_before = value;
            }
        }
    }

    /// Set one held weapon's "raised before" flag (the script's initial-raise switch clears it).
    pub fn set_ever_had(&mut self, weapon: u32, value: bool) {
        if let Some(h) = self.held_mut(weapon) {
            h.held_before = value;
        }
    }

    /// Put `weapon` in a free held slot (no-op if already held). False when all slots are full.
    pub fn give(&mut self, weapon: u32) -> bool {
        if weapon == 0 || self.has_weapon(weapon) {
            return weapon != 0;
        }
        match self.held.iter_mut().find(|h| h.weapon == 0) {
            Some(h) => {
                *h = HeldWeapon {
                    weapon,
                    ..HeldWeapon::default()
                };
                true
            }
            None => false,
        }
    }

    /// Remove `weapon` from the held slots.
    pub fn take(&mut self, weapon: u32) {
        if let Some(h) = self.held_mut(weapon) {
            *h = HeldWeapon::default();
        }
    }

    /// BG_GetAmmoInClip: the rounds in the magazine `weapon` fires from
    /// ([`FactsTable::ammo_clip_weapon`]: Select Fire and the Hybrid's other optic share the
    /// gun's).
    pub fn ammo_in_clip(&self, facts: &FactsTable, weapon: u32) -> i32 {
        self.clip_slot(facts.ammo_clip_weapon(weapon))
    }

    /// BG_SetAmmoInClip (never below 0).
    pub fn set_ammo_in_clip(&mut self, facts: &FactsTable, weapon: u32, count: i32) {
        self.set_clip_slot(facts.ammo_clip_weapon(weapon), count.max(0));
    }

    /// BG_GetAmmoNotInClip: the reserve `weapon` draws from
    /// ([`FactsTable::ammo_pool_weapon`]).
    pub fn ammo_not_in_clip(&self, facts: &FactsTable, weapon: u32) -> i32 {
        self.pool_slot(facts.ammo_pool_weapon(weapon))
    }

    /// BG_SetAmmoInPool (never below 0).
    pub fn set_ammo_not_in_clip(&mut self, facts: &FactsTable, weapon: u32, count: i32) {
        self.set_pool_slot(facts.ammo_pool_weapon(weapon), count.max(0));
    }

    /// The clip stored under `weapon` itself, with no remap (the game's ammo sync; BO2 code
    /// goes through [`Self::ammo_in_clip`]).
    pub fn clip_slot(&self, weapon: u32) -> i32 {
        self.ammo_in_clip
            .iter()
            .find(|c| weapon != 0 && c.weapon == weapon)
            .map_or(0, |c| c.count)
    }

    /// Set the clip stored under `weapon`, taking a free slot if it has none yet.
    pub fn set_clip_slot(&mut self, weapon: u32, count: i32) {
        if weapon == 0 {
            return;
        }
        if let Some(c) = self.ammo_in_clip.iter_mut().find(|c| c.weapon == weapon) {
            c.count = count;
        } else if let Some(c) = self.ammo_in_clip.iter_mut().find(|c| c.weapon == 0) {
            *c = AmmoClip { weapon, count };
        }
    }

    /// The reserve stored under `weapon` itself, with no remap.
    pub fn pool_slot(&self, weapon: u32) -> i32 {
        self.ammo_not_in_clip
            .iter()
            .find(|p| weapon != 0 && p.weapon == weapon)
            .map_or(0, |p| p.count)
    }

    /// Set the reserve stored under `weapon`, taking a free slot if it has none yet.
    pub fn set_pool_slot(&mut self, weapon: u32, count: i32) {
        if weapon == 0 {
            return;
        }
        if let Some(p) = self
            .ammo_not_in_clip
            .iter_mut()
            .find(|p| p.weapon == weapon)
        {
            p.count = count;
        } else if let Some(p) = self.ammo_not_in_clip.iter_mut().find(|p| p.weapon == 0) {
            *p = AmmoPool { weapon, count };
        }
    }

    pub fn hand(&self, left: bool) -> Hand {
        if left {
            Hand {
                state: self.weaponstate_left,
                time: self.weapon_time_left,
                delay: self.weapon_delay_left,
                shot_count: self.weapon_shot_count_left,
            }
        } else {
            Hand {
                state: self.weaponstate,
                time: self.weapon_time,
                delay: self.weapon_delay,
                shot_count: self.weapon_shot_count,
            }
        }
    }

    pub fn set_hand(&mut self, left: bool, h: Hand) {
        if left {
            self.weaponstate_left = h.state;
            self.weapon_time_left = h.time;
            self.weapon_delay_left = h.delay;
            self.weapon_shot_count_left = h.shot_count;
        } else {
            self.weaponstate = h.state;
            self.weapon_time = h.time;
            self.weapon_delay = h.delay;
            self.weapon_shot_count = h.shot_count;
        }
    }

    /// Timers of the hand being run (BO2 uses the *Left fields when bRunLeftGun).
    pub fn hand_time_delay(&self) -> (i32, i32) {
        if self.b_run_left_gun {
            (self.weapon_time_left, self.weapon_delay_left)
        } else {
            (self.weapon_time, self.weapon_delay)
        }
    }

    /// Serialise for the network (little endian, fixed layout, version byte first).
    pub fn to_wire(&self) -> Vec<u8> {
        let mut w = Wire(Vec::with_capacity(WIRE_LEN));
        w.u8(WIRE_VERSION);
        w.u32(self.weapon);
        for v in [
            self.weaponstate,
            self.weaponstate_left,
            self.weapon_time,
            self.weapon_delay,
            self.weapon_time_left,
            self.weapon_delay_left,
            self.weapon_idle_time,
            self.weapon_restrict_kick_time,
            self.weap_anim,
            self.weap_anim_left,
        ] {
            w.i32(v);
        }
        w.u32(self.weapon_shot_count);
        w.u32(self.weapon_shot_count_left);
        w.u8(u8::from(self.b_run_left_gun));
        w.u64(self.weap_flags);
        w.i32(self.grenade_time_left);
        for v in [
            self.off_hand_weapon,
            self.offhand_primary,
            self.offhand_secondary,
            self.last_weapon_alt_mode_switch,
            self.stowed_weapon,
            self.melee_weapon,
            self.last_stand_prev_weapon,
            self.inventory_weapon,
        ] {
            w.u32(v);
        }
        w.i32(self.ads_zoom_select);
        w.i32(self.ads_zoom_latch_time);
        w.f32(self.aim_spread_scale);
        w.f32(self.f_weapon_pos_frac);
        w.i32(self.ads_delay_time);
        w.f32(self.weapon_spin_lerp);
        w.i32(self.charge_shot_timer);
        w.i32(self.charge_shot_level);
        w.i32(self.shots_fired_from_chamber);
        w.i32(self.intro_shots_fired);
        w.u16(ent_to_wire(self.throw_back_grenade_owner));
        w.i32(self.throw_back_grenade_time_left);
        w.u16(ent_to_wire(self.cursor_hint_ent_index));
        w.f32(self.hold_breath_scale);
        w.i32(self.hold_breath_timer);
        w.f32(self.quick_scope_scale);
        w.i32(self.quick_scope_timer);
        w.u8(u8::from(self.ads_zoom_latch_state));
        w.i32(self.ads_prev_zoom_select);
        w.i32(self.ads_prev_zoom_select_time);
        w.u16(ent_to_wire(self.melee_charge_ent));
        w.u32(self.melee_charge_dist);
        w.i32(self.melee_charge_time);
        for h in &self.held {
            w.u32(h.weapon);
            w.f32(h.heat_percent);
            w.i32(h.fuel_tank_time);
            w.i32(h.ads_zoom_select);
            let bits = u8::from(h.over_heating)
                | (u8::from(h.needs_rechamber) << 1)
                | (u8::from(h.held_before) << 2)
                | (u8::from(h.quick_reload) << 3)
                | (u8::from(h.block_weapon_pickup) << 4);
            w.u8(bits);
            w.u8(h.model);
        }
        for p in &self.ammo_not_in_clip {
            w.u32(p.weapon);
            w.i32(p.count);
        }
        for c in &self.ammo_in_clip {
            w.u32(c.weapon);
            w.i32(c.count);
        }
        debug_assert_eq!(w.0.len(), WIRE_LEN);
        w.0
    }

    /// Inverse of [`Self::to_wire`]; `None` on a wrong length or version.
    pub fn from_wire(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != WIRE_LEN || bytes[0] != WIRE_VERSION {
            return None;
        }
        let mut r = Read(&bytes[1..]);
        let mut ps = Self {
            weapon: r.u32(),
            weaponstate: r.i32(),
            weaponstate_left: r.i32(),
            weapon_time: r.i32(),
            weapon_delay: r.i32(),
            weapon_time_left: r.i32(),
            weapon_delay_left: r.i32(),
            weapon_idle_time: r.i32(),
            weapon_restrict_kick_time: r.i32(),
            weap_anim: r.i32(),
            weap_anim_left: r.i32(),
            weapon_shot_count: r.u32(),
            weapon_shot_count_left: r.u32(),
            b_run_left_gun: r.u8() != 0,
            weap_flags: r.u64(),
            grenade_time_left: r.i32(),
            off_hand_weapon: r.u32(),
            offhand_primary: r.u32(),
            offhand_secondary: r.u32(),
            last_weapon_alt_mode_switch: r.u32(),
            stowed_weapon: r.u32(),
            melee_weapon: r.u32(),
            last_stand_prev_weapon: r.u32(),
            inventory_weapon: r.u32(),
            ads_zoom_select: r.i32(),
            ads_zoom_latch_time: r.i32(),
            aim_spread_scale: r.f32(),
            f_weapon_pos_frac: r.f32(),
            ads_delay_time: r.i32(),
            weapon_spin_lerp: r.f32(),
            charge_shot_timer: r.i32(),
            charge_shot_level: r.i32(),
            shots_fired_from_chamber: r.i32(),
            intro_shots_fired: r.i32(),
            throw_back_grenade_owner: ent_from_wire(r.u16()),
            throw_back_grenade_time_left: r.i32(),
            cursor_hint_ent_index: ent_from_wire(r.u16()),
            hold_breath_scale: r.f32(),
            hold_breath_timer: r.i32(),
            quick_scope_scale: r.f32(),
            quick_scope_timer: r.i32(),
            ads_zoom_latch_state: r.u8() != 0,
            ads_prev_zoom_select: r.i32(),
            ads_prev_zoom_select_time: r.i32(),
            melee_charge_ent: ent_from_wire(r.u16()),
            melee_charge_dist: r.u32(),
            melee_charge_time: r.i32(),
            ..Self::default()
        };
        for h in &mut ps.held {
            h.weapon = r.u32();
            h.heat_percent = r.f32();
            h.fuel_tank_time = r.i32();
            h.ads_zoom_select = r.i32();
            let bits = r.u8();
            h.over_heating = bits & 1 != 0;
            h.needs_rechamber = bits & 2 != 0;
            h.held_before = bits & 4 != 0;
            h.quick_reload = bits & 8 != 0;
            h.block_weapon_pickup = bits & 16 != 0;
            h.model = r.u8();
        }
        for p in &mut ps.ammo_not_in_clip {
            p.weapon = r.u32();
            p.count = r.i32();
        }
        for c in &mut ps.ammo_in_clip {
            c.weapon = r.u32();
            c.count = r.i32();
        }
        Some(ps)
    }
}

const WIRE_VERSION: u8 = 2;
/// version, weapon, 10 x i32, 2 x u32, bool, u64, i32, 8 x u32, 9 x 4 bytes, throw-back +
/// inventory gun (4), intro shots (4), throw-back + hint (2 + 4 + 2), ADS (4 x 4 + 1 + 2 x 4), lunge (2 + 4 + 4), held, ammo.
pub const WIRE_LEN: usize =
    1 + 4 + 40 + 8 + 1 + 8 + 4 + 32 + 36 + 4 + 8 + 25 + 10 + MAX_HELD * 18 + MAX_HELD * 8 * 2;

/// BO2's "no entity" number in a 10-bit entity field.
pub const NO_ENTITY: u16 = 0x3ff;

fn ent_to_wire(e: Option<u16>) -> u16 {
    e.unwrap_or(NO_ENTITY)
}

fn ent_from_wire(v: u16) -> Option<u16> {
    (v != NO_ENTITY).then_some(v)
}

struct Wire(Vec<u8>);
impl Wire {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn i32(&mut self, v: i32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
}

struct Read<'a>(&'a [u8]);
impl Read<'_> {
    fn take<const N: usize>(&mut self) -> [u8; N] {
        let (a, b) = self.0.split_at(N);
        self.0 = b;
        a.try_into().unwrap_or([0; N])
    }
    fn u8(&mut self) -> u8 {
        self.take::<1>()[0]
    }
    fn u16(&mut self) -> u16 {
        u16::from_le_bytes(self.take())
    }
    fn u32(&mut self) -> u32 {
        u32::from_le_bytes(self.take())
    }
    fn i32(&mut self) -> i32 {
        i32::from_le_bytes(self.take())
    }
    fn u64(&mut self) -> u64 {
        u64::from_le_bytes(self.take())
    }
    fn f32(&mut self) -> f32 {
        f32::from_le_bytes(self.take())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_round_trip() {
        let mut ps = T6WeaponPs {
            weapon: 7,
            weaponstate: crate::state::RAISING,
            weap_anim: 0x40f,
            weap_anim_left: crate::anim::LEFT_NONE,
            weap_flags: wf::RIOTSHIELD_SWITCH | (1 << 40),
            aim_spread_scale: 255.0,
            b_run_left_gun: true,
            ..T6WeaponPs::default()
        };
        ps.held[0].weapon = 7;
        ps.held[0].held_before = true;
        ps.held[14].quick_reload = true;
        ps.set_clip_slot(7, 30);
        ps.set_pool_slot(7, 120);
        let bytes = ps.to_wire();
        assert_eq!(bytes.len(), WIRE_LEN);
        assert_eq!(T6WeaponPs::from_wire(&bytes), Some(ps));
        assert_eq!(T6WeaponPs::from_wire(&bytes[1..]), None);
    }

    #[test]
    fn a_valid_weapon_is_in_the_table_and_held() {
        let mut ps = T6WeaponPs::default();
        ps.held[0].weapon = 7;
        assert!(ps.is_weapon_valid(7, 8));
        assert!(!ps.is_weapon_valid(7, 7));
        assert!(!ps.is_weapon_valid(6, 8));
        assert!(!ps.is_weapon_valid(0, 8));
    }

    #[test]
    fn clips_are_per_weapon() {
        let mut ps = T6WeaponPs::default();
        ps.set_clip_slot(3, 12);
        ps.set_clip_slot(4, 11);
        assert_eq!(ps.clip_slot(3), 12);
        assert_eq!(ps.clip_slot(4), 11);
        assert_eq!(ps.clip_slot(0), 0);
    }
}
