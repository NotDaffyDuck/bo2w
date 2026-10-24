//! The server's (and a predicting client's) record of one client's match state.

use super::{ClientLifecycle, LoadoutSpec, LocationSelection, MenuCommand, RadarMode, RemoteMissile, T6ViewInputs, ViewEffects};
use crate::identities::LifeSequence;

/// Tallies the moves the server has run for a client, so tests and the net layer can tell a stalled input stream
/// from a still player.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct InputReceipt {
    pub applied_cmds: u32,
    pub moving_cmds: u32,
    pub path_units: f32,
}

impl InputReceipt {
    /// Counts one applied command that took the player from `from` to `to`.
    pub(crate) fn record(&mut self, moving: bool, from: [f32; 3], to: [f32; 3]) {
        self.applied_cmds = self.applied_cmds.wrapping_add(1);
        if moving {
            self.moving_cmds = self.moving_cmds.wrapping_add(1);
        }
        let delta = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
        let length = delta.iter().map(|axis| axis * axis).sum::<f32>().sqrt();
        if length.is_finite() {
            self.path_units += length;
        }
    }
}

/// What the scripts have switched off for a client, and the weapon they force.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScriptControls {
    pub frozen: bool,
    pub weapons_disabled: bool,
    /// The view rides another entity (the Lodestar): the gun does not fire, but the player still aims.
    pub fire_disabled: bool,
    pub offhands_disabled: bool,
    pub switch_disabled: bool,
    pub jump_disabled: bool,
    pub usability_disabled: bool,
    pub linked: bool,
    pub stunned: bool,
    // BO2 `allowprone`, `allowcrouch`, `allowstand`, `allowsprint`, `allowmelee`, `allowads` set to false: the server
    // drops that request.
    pub prone_disabled: bool,
    pub crouch_disabled: bool,
    pub stand_disabled: bool,
    pub sprint_disabled: bool,
    pub melee_disabled: bool,
    pub ads_disabled: bool,
    pub switch_to: u32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClientMatchState {
    // The T6 brain and body.
    /// The riot shield they bear: `Some(true)` in hand, `Some(false)` stowed on the back.
    pub(crate) riot_shield: Option<bool>,
    /// Their hit boxes as the BO2 body stands this tick (own space: feet at the origin, facing +x).
    pub(crate) t6_hit_bones: Option<std::sync::Arc<Vec<xmodel_runtime::CollisionBone>>>,
    /// The tag the riot shield hangs on this tick, in the same space (the tag's place is the shield's centre).
    pub(crate) t6_shield_tag: Option<xmodel_runtime::CollisionBone>,
    /// The weapon brain's state between ticks; `None` until the next tick starts it from the held weapons. The
    /// server's own: never sent.
    pub(crate) t6_gun: Option<Box<crate::t6_gun::T6Gun>>,
    /// Client prediction keeps the brain as each predicted move left it (command time, state), so a snapshot can rewind
    /// it to the move the server last ran.
    pub(crate) t6_gun_history: Vec<(i32, Box<crate::t6_gun::T6Gun>)>,
    /// Perks as BO2 perk numbers (bit n is perk n), for the weapon brain.
    pub(crate) t6_perks: u64,
    /// The server's viewmodel values from the last snapshot (client side).
    pub(crate) t6_view: Option<T6ViewInputs>,

    // Lifecycle and loadout.
    pub weapon_lock: crate::WeaponLock,
    pub lifecycle: ClientLifecycle,
    pub input_receipt: InputReceipt,
    pub loadout: Option<LoadoutSpec>,
    pub life_sequence: LifeSequence,
    pub(crate) dead_since_tick: Option<u32>,
    pub(crate) forced_spawn: Option<crate::SpawnPick>,
    pub(crate) look_at_killer_yaw: i32,

    // Using an item on the ground.
    pub item_use_spawn_ms: i32,
    pub item_use_entity: Option<crate::EntityRef>,
    /// When the use press on that item began, and whether it was the use/reload button alone (that one must be held to
    /// take the item; let go sooner, it reloads). The server's own: never sent.
    pub(crate) item_use_hold: Option<(i32, bool)>,
    /// A use/reload tap let go before it took anything: the next command reloads.
    pub(crate) item_use_reload: bool,

    // Ammunition.
    pub(crate) ammo_clip: i32,
    pub(crate) ammo_stock: i32,
    pub(crate) ammo_by_weapon: Vec<(u32, i32, i32)>,
    pub(crate) taped_mag_spent: Vec<u32>,
    pub(crate) weapon_shot_count: u8,
    pub(crate) burst_latch: bool,
    pub(crate) burst_latch_secondary: bool,
    pub(crate) rechamber_pending: bool,
    pub(crate) rechamber_pending_secondary: bool,

    // Identity and score.
    pub(crate) name: [u8; 16],
    pub kills: i32,
    pub deaths: i32,
    pub score: i32,
    pub kill_streak: i32,
    pub radar: RadarMode,
    pub remote_missile: Option<RemoteMissile>,
    pub(crate) ffa_team: Option<u8>,
    pub(crate) client_state_team: i32,
    pub(crate) rank: i32,
    pub(crate) prestige: i32,
    pub(crate) player_card_icon: u32,
    pub(crate) player_card_title: u32,
    pub(crate) player_card_nameplate: u32,

    // What the scripts do to this client.
    pub(crate) client_dvars: Vec<(String, String)>,
    pub(crate) shellshock: Option<hud_t6::flashbang::ShellshockParms>,
    pub(crate) view_effects: ViewEffects,
    pub(crate) menu_commands: Vec<MenuCommand>,
    pub(crate) location_selection: Option<LocationSelection>,
    /// Riding a scorestreak vehicle: how far they may look up and down (its own limits instead of the usual 85).
    pub(crate) view_pitch_clamp: Option<[f32; 2]>,
    pub(crate) controls: ScriptControls,
    pub(crate) max_health: i32,

    /// This frame's hits so far (BO2 sends one flinch a frame for all of them): the frame, their damage, and the share
    /// of health already sent (`None` while nothing has gone out this frame).
    pub(crate) hit_frame: Option<u32>,
    pub(crate) hit_blood: i32,
    pub(crate) hit_sent: Option<i32>,
}

impl ClientMatchState {
    /// (clip, stock) held for `weapon`; (0, 0) for a gun not carried.
    pub(crate) fn ammo_for(&self, weapon: u32) -> (i32, i32) {
        match self.ammo_by_weapon.iter().find(|row| row.0 == weapon) {
            Some(&(_, clip, stock)) => (clip, stock),
            None => (0, 0),
        }
    }

    pub(crate) fn set_ammo(&mut self, weapon: u32, clip: i32, stock: i32) {
        if weapon == 0 {
            return;
        }
        match self.ammo_by_weapon.iter_mut().find(|row| row.0 == weapon) {
            Some(row) => *row = (weapon, clip, stock),
            None => self.ammo_by_weapon.push((weapon, clip, stock)),
        }
    }

    /// Whether the gun's taped second magazine is still ready for a quick reload.
    pub(crate) fn quick_reload_ready(&self, weapon: u32) -> bool {
        !self.taped_mag_spent.contains(&weapon)
    }

    pub(crate) fn set_quick_reload_ready(&mut self, weapon: u32, ready: bool) {
        self.taped_mag_spent.retain(|&spent| spent != weapon);
        if weapon != 0 && !ready {
            self.taped_mag_spent.push(weapon);
        }
    }

    pub(crate) fn clear_ammo_inventory(&mut self) {
        self.ammo_by_weapon.clear();
        self.taped_mag_spent.clear();
        self.ammo_clip = 0;
        self.ammo_stock = 0;
    }

    /// Copies the held gun's rows into the flat clip and stock the player state shows.
    pub(crate) fn mirror_held_ammo(&mut self, weapon: u32) {
        (self.ammo_clip, self.ammo_stock) = self.ammo_for(weapon);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ammo_rows_update_in_place_and_ignore_weapon_zero() {
        let mut state = ClientMatchState::default();
        state.set_ammo(0, 9, 9);
        assert!(state.ammo_by_weapon.is_empty());
        state.set_ammo(7, 30, 90);
        state.set_ammo(8, 8, 16);
        state.set_ammo(7, 12, 60);
        assert_eq!(state.ammo_by_weapon, [(7, 12, 60), (8, 8, 16)]);
        assert_eq!(state.ammo_for(7), (12, 60));
        assert_eq!(state.ammo_for(99), (0, 0));
        state.mirror_held_ammo(8);
        assert_eq!((state.ammo_clip, state.ammo_stock), (8, 16));
        state.clear_ammo_inventory();
        assert_eq!((state.ammo_clip, state.ammo_stock, state.ammo_by_weapon.len()), (0, 0, 0));
    }

    #[test]
    fn the_taped_magazine_is_spent_once_per_gun() {
        let mut state = ClientMatchState::default();
        assert!(state.quick_reload_ready(5));
        state.set_quick_reload_ready(5, false);
        state.set_quick_reload_ready(5, false);
        assert_eq!(state.taped_mag_spent, [5]);
        assert!(!state.quick_reload_ready(5));
        state.set_quick_reload_ready(5, true);
        assert!(state.quick_reload_ready(5));
        state.set_quick_reload_ready(0, false);
        assert!(state.taped_mag_spent.is_empty());
    }

    #[test]
    fn the_receipt_adds_up_the_path_and_skips_a_non_finite_step() {
        let mut receipt = InputReceipt::default();
        receipt.record(true, [0.0; 3], [3.0, 4.0, 0.0]);
        receipt.record(false, [3.0, 4.0, 0.0], [3.0, 4.0, 0.0]);
        receipt.record(true, [0.0; 3], [f32::NAN, 0.0, 0.0]);
        assert_eq!((receipt.applied_cmds, receipt.moving_cmds), (3, 2));
        assert_eq!(receipt.path_units, 5.0);
    }
}
