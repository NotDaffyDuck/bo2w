//! The part of a client's match state that rides in every snapshot.

use hud_t6::flashbang::ShellshockParms;
use hud_t6::hudelem::HudElem;

use super::loadout::LoadoutSpec;
use super::menu::{LocationSelection, MenuCommand};
use super::radar::RadarMode;
use super::remote::RemoteMissile;
use super::seat::KillcamHud;
use super::view::ViewEffects;
use super::{ClientLifecycle, ScriptControls};
use crate::identities::LifeSequence;

/// What one client's snapshot row says about them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClientSnapshotMeta {
    pub controls: ScriptControls,
    pub weapon_lock: crate::WeaponLock,
    pub killcam_hud: Option<KillcamHud>,
    pub lifecycle: ClientLifecycle,
    pub loadout: Option<LoadoutSpec>,
    pub life_sequence: LifeSequence,

    // Using an item on the ground.
    pub item_use_spawn_ms: i32,
    pub item_use_entity: Option<crate::EntityRef>,

    // Ammunition and the gun's own bookkeeping.
    pub ammo_clip: i32,
    pub ammo_stock: i32,
    pub ammo_by_weapon: Vec<(u32, i32, i32)>,
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
    pub remote_missile: Option<RemoteMissile>,
    pub dead_since_tick: Option<u32>,
    pub look_at_killer_yaw: i32,

    // Who they are.
    pub name: [u8; 16],
    pub ffa_team: Option<u8>,
    pub client_state_team: i32,
    pub rank: i32,
    pub prestige: i32,
    pub player_card_icon: u32,
    pub player_card_title: u32,
    pub player_card_nameplate: u32,

    // What the scripts put on their screen.
    pub hud_archival: Vec<HudElem>,
    pub hud_current: Vec<HudElem>,
    pub client_dvars: Vec<(String, String)>,
    pub shellshock: Option<ShellshockParms>,
    pub view_effects: ViewEffects,
    pub menu_commands: Vec<MenuCommand>,
    pub location_selection: Option<LocationSelection>,

    /// The T6 brain's values that the client's BO2 viewmodel player reads.
    pub t6_view: Option<T6ViewInputs>,
    /// Their perks as BO2 perk numbers (bit n is perk n), so the client's own prediction runs the T6 brain with the
    /// perks the server's scripts set.
    pub t6_perks: u64,
}

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
