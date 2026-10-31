//! The server's (and a predicting client's) record of one client's match state.

use super::{PlayerSessionStage, KitPlan, LocationSelection, MenuOrder, RadarMode, SteeredMissile, T6ViewInputs, ViewEffects};
use crate::match_ids::LifeCount;

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
    pub(crate) fn record(&mut self, moving: bool, from: [f32; 3], till: [f32; 3]) {
        self.applied_cmds = self.applied_cmds.wrapping_add(1);
        if moving {
            self.moving_cmds = self.moving_cmds.wrapping_add(1);
        }
        let delta = [till[0] - from[0], till[1] - from[1], till[2] - from[2]];
        let length = delta.iter().map(|axis| axis * axis).sum::<f32>().sqrt();
        if length.is_finite() {
            self.path_units += length;
        }
    }
}

/// What the scripts have switched off for a client, and the weapon they force.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScriptControls {
    pub stuck: bool,
    pub weapons_disabled: bool,
    /// The view rides another entity (the Lodestar): the gun does not fire, but the player still aims.
    pub fire_disabled: bool,
    pub offhands_disabled: bool,
    pub switch_disabled: bool,
    pub jump_disabled: bool,
    pub usability_disabled: bool,
    pub paired: bool,
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
    /// bo2zm: `forceslick` (Die Rise's Sliquifier goo): the ground is ice under him.
    pub slick: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerMatchRecord {
    // The T6 brain and body.
    /// The riot shield they bear: `Some(true)` in hand, `Some(false)` stowed on the back.
    pub(crate) riot_shield: Option<bool>,
    /// Their hit boxes as the BO2 body stands this tick (own space: feet at the origin, facing +x).
    pub(crate) t6_hit_bones: Option<std::sync::Arc<Vec<xmodel_runtime::HitShapeBone>>>,
    /// The tag the riot shield hangs on this tick, in the same space (the tag's place is the shield's centre).
    pub(crate) t6_shield_tag: Option<xmodel_runtime::HitShapeBone>,
    /// The weapon brain's state between ticks; `None` until the next tick starts it from the held weapons. The
    /// server's own: never sent.
    pub(crate) t6_gun: Option<Box<crate::gun_brain::T6Gun>>,
    /// Client prediction keeps the brain as each predicted move left it (command time, state), so a snapshot can rewind
    /// it to the move the server last ran.
    pub(crate) t6_gun_history: Vec<(i32, Box<crate::gun_brain::T6Gun>)>,
    /// Perks as BO2 perk numbers (bit n is perk n), for the weapon brain.
    pub(crate) t6_perks: u64,
    /// The server's viewmodel values from the last snapshot (client side).
    pub(crate) t6_view: Option<T6ViewInputs>,

    // Lifecycle and loadout.
    pub weapon_lock: crate::WeaponLock,
    pub lifecycle: PlayerSessionStage,
    pub input_receipt: InputReceipt,
    pub kit: Option<KitPlan>,
    pub life_sequence: LifeCount,
    pub(crate) died_at_step: Option<u32>,
    pub(crate) forced_spawn: Option<crate::SpawnPick>,
    pub(crate) killer_turn: i32,

    // Using an item on the ground.
    pub item_ready_ms: i32,
    pub use_target: Option<crate::EntHandle>,
    /// When the use press on that item began, and whether it was the use/reload button alone (that one must be held to
    /// take the item; let go sooner, it reloads). The server's own: never sent.
    pub(crate) item_use_hold: Option<(i32, bool)>,
    /// A use/reload tap let go before it took anything: the next command reloads.
    pub(crate) item_use_reload: bool,

    // Ammunition.
    pub(crate) mag_rounds: i32,
    pub(crate) spare_rounds: i32,
    pub(crate) rounds_per_gun: Vec<(u32, i32, i32)>,
    pub(crate) taped_mag_spent: Vec<u32>,
    pub(crate) shots_per_gun: u8,
    pub(crate) burst_held: bool,
    pub(crate) burst_latch_secondary: bool,
    pub(crate) bolt_due: bool,
    pub(crate) rechamber_pending_secondary: bool,

    // Identity and score.
    pub(crate) name: [u8; 16],
    pub frags: i32,
    pub died: i32,
    pub score: i32,
    pub kill_streak: i32,
    pub radar: RadarMode,
    pub remote_missile: Option<SteeredMissile>,
    pub(crate) solo_side: Option<u8>,
    pub(crate) seat_side: i32,
    pub(crate) grade: i32,
    pub(crate) star: i32,
    pub(crate) card_icon: u32,
    pub(crate) card_title: u32,
    pub(crate) card_plate: u32,

    // What the scripts do to this client.
    pub(crate) client_dvars: Vec<(String, String)>,
    pub(crate) shellshock: Option<hud_t6::flashbang::ShellshockParms>,
    pub(crate) view_effects: ViewEffects,
    pub(crate) menu_commands: Vec<MenuOrder>,
    pub(crate) location_selection: Option<LocationSelection>,
    /// Riding a scorestreak vehicle: how far they may look up and down (its own limits instead of the usual 85).
    pub(crate) view_pitch_clamp: Option<[f32; 2]>,
    pub(crate) controls: ScriptControls,
    pub(crate) hit_point_ceiling: i32,

    /// This frame's hits so far (BO2 sends one flinch a frame for all of them): the frame, their damage, and the share
    /// of health already sent (`None` while nothing has gone out this frame).
    pub(crate) hit_frame: Option<u32>,
    pub(crate) hit_blood: i32,
    pub(crate) hit_sent: Option<i32>,
}

impl PlayerMatchRecord {
    /// The `(clip, stock)` this player holds for `weapon`; zeros for a gun he does not carry.
    pub(crate) fn ammo_counts_of(&self, gun_no: u32) -> (i32, i32) {
        let row = self.rounds_per_gun.iter().find(|&&(gun, _, _)| gun == gun_no);
        row.map_or((0, 0), |&(_, clip, stock)| (clip, stock))
    }

    /// Records `(clip, stock)` for `weapon`, overwriting its row or adding one at the end. Weapon 0 (empty hands)
    /// is never stored.
    pub(crate) fn store_ammo_counts(&mut self, gun: u32, clip: i32, reserve: i32) {
        if gun == 0 {
            return;
        }
        let entry = (gun, clip, reserve);
        if let Some(slot) = self.rounds_per_gun.iter_mut().find(|slot| slot.0 == gun) {
            *slot = entry;
        } else {
            self.rounds_per_gun.push(entry);
        }
    }

    /// True until the taped spare magazine on `weapon` has been used for a quick reload.
    pub(crate) fn quick_reload_ready(&self, gun: u32) -> bool {
        !self.taped_mag_spent.contains(&gun)
    }

    /// Marks the taped magazine on `weapon` ready again or used up. Each gun is listed at most once; weapon 0
    /// is never listed.
    pub(crate) fn set_quick_reload_ready(&mut self, gun_no: u32, primed: bool) {
        let spent = &mut self.taped_mag_spent;
        spent.retain(|&gun| gun != gun_no);
        if !primed && gun_no != 0 {
            spent.push(gun_no);
        }
    }

    /// Forgets every gun's ammo and every used taped magazine, and empties the shown clip and stock.
    pub(crate) fn wipe_ammo_stock(&mut self) {
        let Self { rounds_per_gun: ammo_by_weapon, taped_mag_spent, mag_rounds: ammo_clip, spare_rounds: ammo_stock, .. } = self;
        ammo_by_weapon.clear();
        taped_mag_spent.clear();
        (*ammo_clip, *ammo_stock) = (0, 0);
    }

    /// Shows `weapon`'s stored counts as the player's current clip and stock.
    pub(crate) fn copy_carried_ammo(&mut self, gun: u32) {
        (self.mag_rounds, self.spare_rounds) = self.ammo_counts_of(gun);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ammo_rows_update_in_place_and_ignore_weapon_zero() {
        let mut state = PlayerMatchRecord::default();
        state.store_ammo_counts(0, 9, 9);
        assert!(state.rounds_per_gun.is_empty());
        state.store_ammo_counts(7, 30, 90);
        state.store_ammo_counts(8, 8, 16);
        state.store_ammo_counts(7, 12, 60);
        assert_eq!(state.rounds_per_gun, [(7, 12, 60), (8, 8, 16)]);
        assert_eq!(state.ammo_counts_of(7), (12, 60));
        assert_eq!(state.ammo_counts_of(99), (0, 0));
        state.copy_carried_ammo(8);
        assert_eq!((state.mag_rounds, state.spare_rounds), (8, 16));
        state.wipe_ammo_stock();
        assert_eq!((state.mag_rounds, state.spare_rounds, state.rounds_per_gun.len()), (0, 0, 0));
    }

    #[test]
    fn the_taped_magazine_is_spent_once_per_gun() {
        let mut state = PlayerMatchRecord::default();
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
