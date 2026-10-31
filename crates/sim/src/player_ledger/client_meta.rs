//! The part of a client's match state that rides in every snapshot.

use hud_t6::flashbang::ShellshockParms;
use hud_t6::overlay_piece::OverlayPiece;

use super::menu_cmds::{LocationSelection, MenuOrder};
use super::radar::RadarMode;
use super::remote::SteeredMissile;
use super::screen_fx::ViewEffects;
use super::ScriptControls;

/// What one client's snapshot row says about them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SeatTickNotes {
    pub controls: ScriptControls,
    pub weapon_lock: crate::WeaponLock,
    pub killcam_hud: Option<super::seat_script::DeathReelBoard>,
    pub lifecycle: super::PlayerSessionStage,
    pub loadout: Option<super::loadout::KitPlan>,
    pub life_sequence: crate::match_ids::LifeCount,

    // Using an item on the ground.
    pub item_ready_ms: i32,
    pub item_use_entity: Option<crate::EntHandle>,

    // Ammunition and the gun's own bookkeeping.
    pub mag_rounds: i32,
    pub ammo_stock: i32,
    pub rounds_per_gun: Vec<(u32, i32, i32)>,
    pub taped_mag_spent: Vec<u32>,
    pub weapon_shot_count: u8,
    pub burst_latch: bool,
    pub burst_latch_secondary: bool,
    pub rechamber_pending: bool,
    pub rechamber_pending_secondary: bool,

    // The scoreboard row.
    pub score: i32,
    pub kills: i32,
    pub deaths: i32,
    pub kill_streak: i32,

    pub radar: RadarMode,
    pub remote_missile: Option<SteeredMissile>,
    pub died_at_step: Option<u32>,
    pub look_at_killer_yaw: i32,

    // Who they are.
    pub name: [u8; 16],
    pub ffa_team: Option<u8>,
    pub client_state_team: i32,
    pub rank: i32,
    pub prestige: i32,
    pub card_icon: u32,
    pub card_title: u32,
    pub card_plate: u32,

    // What the scripts put on their screen.
    pub hud_archival: Vec<OverlayPiece>,
    pub hud_current: Vec<OverlayPiece>,
    pub client_dvars: Vec<(String, String)>,
    pub shellshock: Option<ShellshockParms>,
    pub view_effects: ViewEffects,
    pub menu_commands: Vec<MenuOrder>,
    pub location_selection: Option<LocationSelection>,

    /// The T6 brain's values that the client's BO2 viewmodel player reads.
    pub t6_view: Option<T6ViewInputs>,
    /// Their perks as BO2 perk numbers (bit n is perk n), so the client's own prediction runs the T6 brain with the
    /// perks the server's scripts set.
    pub t6_perks: u64,
}
/// Older spelling ClientSnapshotMeta still calls; the rename sweep moves it over.
pub use self::SeatTickNotes as ClientSnapshotMeta;

/// The local player's T6 weapon values the viewmodel needs, as raw T6 numbers (the older mapping loses some: sprint
/// and low-ready share a slot, the dive animations merge, and a left-hand shot reads as a plain one).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct T6ViewInputs {
    pub weap_anim: i32,
    pub weap_anim_left: i32,
    pub weaponstate: i32,
    pub weaponstate_left: i32,
    pub weapon_time: i32,
    pub weapon_time_left: i32,
    pub intro_shots_fired: i32,
    /// Bit i: the gun in slot i of the older player state's weapon list has been raised before (a first raise plays
    /// differently).
    pub ever_had: u16,
    /// Bit i: that slot's gun still has to work its bolt. BO2 sends the held weapons' flags whole, so the client never
    /// bolts a gun the server has not, nor skips one.
    pub needs_rechamber: u16,
}
