//! The per-player record the match carries from tick to tick, and the entity numbers it points at.

use crate::consts::MAX_PREDICTABLE_EVENTS;
use crate::{MantleState, SprintState};

use super::flag_bits::weap_flags;

/// The entity number meaning "nobody" (the last slot of the 2048-entry entity table).
pub const ENTITYNUM_NONE: i32 = 2047;

// Movement types the camera and the view model care about (`PlayerState::pm_type`).
pub const PM_TYPE_NORMAL_LINKED: i32 = 1;
pub const PM_TYPE_SPECTATOR: i32 = 5;
pub const PM_TYPE_INTERMISSION: i32 = 6;
pub const PM_TYPE_LAST_STAND: i32 = 7;
/// This type and every one above it is a dead player.
pub const PM_TYPE_DEAD: i32 = 10;
pub const PM_TYPE_DEAD_LINKED: i32 = 11;

/// Bodies left behind by players take the entity slots right after the client slots...
pub const PLAYER_CORPSE_ENTITY_BASE: i32 = 18;
/// ...one slot per body that can lie in the world at once...
pub const MAX_CLIENT_CORPSES: i32 = 8;
/// ...and entities spawned by the level and the scripts start after them.
pub const GENTITY_SPAWN_BASE: i32 = MAX_CLIENT_CORPSES + PLAYER_CORPSE_ENTITY_BASE;

const STANDING_STILL: SprintState = SprintState {
    sprint_button_up_required: 0,
    sprint_delay: 0,
    last_sprint_start: 0,
    last_sprint_end: 0,
    sprint_start_max_length: 0,
    sprint_duration: 0,
    sprint_cooldown: 0,
};

const NOT_CLIMBING: MantleState = MantleState { yaw: 0.0, timer: 0, trans_index: 0, flags: 0 };

/// Writes the record and its blank value from one list, so a field cannot be added to one and forgotten in the other.
macro_rules! live_record {
    ($($(#[$meta:meta])* $field:ident: $ty:ty = $blank:expr,)*) => {
        /// Everything the match keeps about one player between ticks: movement, view, body animation, damage, guns,
        /// loadout, prompts and the kill-cam seat. Snapshots carry it to the owning client, and the kill cam replays
        /// archived copies of it.
        #[derive(Clone, Copy, Debug, PartialEq)]
        pub struct PlayerState {
            $($(#[$meta])* pub $field: $ty,)*
        }

        impl PlayerState {
            /// A player who has not spawned: everything cleared, standing on nothing, no gun state yet.
            pub const ZERO: Self = Self { $($field: $blank,)* };
        }
    };
}

live_record! {
    // How the movement code runs this player.
    /// Server time of the last user command played through movement.
    command_time: i32 = 0,
    pm_type: i32 = 0,
    /// Time left on the current movement lock (landing, knockback, and so on).
    pm_time: i32 = 0,
    pm_flags: u32 = 0,
    other_flags: u32 = 0,
    link_flags: u32 = 0,
    bob_cycle: i32 = 0,

    // Where the player is and how they move.
    origin: [f32; 3] = [0.0; 3],
    velocity: [f32; 3] = [0.0; 3],
    delta_angles: [f32; 3] = [0.0; 3],
    gravity: i32 = 0,
    speed: i32 = 0,
    leanf: f32 = 0.0,
    ground_entity_num: i32 = 0,
    /// Facing direction of the ladder being climbed.
    v_ladder_vec: [f32; 3] = [0.0; 3],
    jump_time: i32 = 0,
    jump_origin_z: f32 = 0.0,
    dive_end_time: i32 = 0,
    foliage_sound_time: i32 = 0,
    slide_time: i32 = 0,
    /// The moving platform the player rides; nobody until they step onto one.
    mover_entity_num: i32 = ENTITYNUM_NONE,
    mover_timestamp: i32 = 0,
    waterlevel: i32 = 0,
    sprint_state: SprintState = STANDING_STILL,
    move_speed_scale_multiplier: f32 = 0.0,
    mantle_state: MantleState = NOT_CLIMBING,

    // Lying down.
    prone_direction: f32 = 0.0,
    prone_direction_pitch: f32 = 0.0,
    prone_torso_pitch: f32 = 0.0,

    // Eyes and view height.
    viewangles: [f32; 3] = [0.0; 3],
    view_height_target: i32 = 0,
    view_height_current: f32 = 0.0,
    view_height_lerp_time: i32 = 0,
    view_height_lerp_target: i32 = 0,
    view_height_lerp_down: i32 = 0,
    link_weapon_angles: [f32; 3] = [0.0; 3],

    // Body animation as other players see it.
    f_torso_pitch: f32 = 0.0,
    f_waist_pitch: f32 = 0.0,
    legs_timer: i32 = 0,
    legs_anim: i32 = 0,
    torso_timer: i32 = 0,
    torso_anim: i32 = 0,
    flinch_yaw_anim: i32 = 0,
    movement_dir: i32 = 0,
    corpse_index: i32 = 0,
    e_flags: u32 = 0,

    // Events the owning client predicts for itself.
    predictable_event_sequence: i16 = 0,
    predictable_event_sequence_old: i16 = 0,
    predictable_events: [i32; MAX_PREDICTABLE_EVENTS] = [0; MAX_PREDICTABLE_EVENTS],
    predictable_event_parms: [u32; MAX_PREDICTABLE_EVENTS] = [0; MAX_PREDICTABLE_EVENTS],

    // Getting hurt.
    health: i32 = 0,
    max_health: i32 = 0,
    damage_timer: i32 = 0,
    damage_duration: i32 = 0,
    damage_event: u32 = 0,
    damage_yaw: u32 = 0,
    damage_pitch: u32 = 0,
    damage_count: i32 = 0,
    shellshock_index: i32 = 0,
    shellshock_time: i32 = 0,
    shellshock_duration: i32 = 0,

    // Grenades in hand.
    grenade_time_left: i32 = 0,
    throw_back_grenade_owner: i32 = 0,

    // The BO2 gun brain's own state per hand; -1 until a gun is raised.
    weapon_state_t6: i32 = -1,
    weapon_state_left_t6: i32 = -1,
    weap_anim_t6: i32 = 0,
    weap_anim_left_t6: i32 = 0,

    // Right hand (and the only hand without dual wield).
    weap_anim: i32 = 0,
    weapon_time: i32 = 0,
    weapon_delay: i32 = 0,
    weapon_restrict_kick_time: i32 = 0,
    weaponstate_primary: i32 = 0,
    weap_hand_flags: i32 = 0,
    weapon_shot_count: i32 = 0,

    // Left hand.
    weap_anim_secondary: i32 = 0,
    weapon_time_secondary: i32 = 0,
    weapon_delay_secondary: i32 = 0,
    weapon_restrict_kick_time_secondary: i32 = 0,
    weaponstate_secondary: i32 = 0,
    weap_hand_flags_secondary: i32 = 0,
    weapon_shot_count_secondary: i32 = 0,

    // Guns carried, their ammo, and the one up.
    weapons: [i32; 15] = [0; 15],
    weapon_data: [u8; 76] = [0; 76],
    weapon: u32 = 0,
    weapon_primary: u32 = 0,
    weap_flags: u32 = 0,
    last_weapon_hand: i32 = 0,
    inventory_weapon: i32 = 0,
    ammo: [u8; 120] = [0; 120],
    ammoclip: [u8; 180] = [0; 180],

    // Lethal and tactical slots.
    off_hand_index: i32 = 0,
    offhand_primary: i32 = 0,
    offhand_secondary: i32 = 0,

    // Aiming and spread.
    f_weapon_pos_frac: f32 = 0.0,
    aim_spread_scale: f32 = 0.0,
    ads_delay_time: i32 = 0,
    spread_override: i32 = 0,
    spread_override_state: i32 = 0,
    recoil_scale: i32 = 0,

    // Knife lunge.
    melee_charge_yaw: f32 = 0.0,
    melee_charge_dist: i32 = 0,
    melee_charge_time: i32 = 0,

    // Perks and the d-pad slots.
    perks: [u32; 2] = [0; 2],
    perk_slots: [u32; 8] = [0; 8],
    action_slot_type: [i32; 4] = [0; 4],
    action_slot_param: [i32; 4] = [0; 4],

    // Use prompts and the objective list on the HUD.
    cursor_hint: i32 = 0,
    cursor_hint_string: i32 = 0,
    cursor_hint_ent_index: i32 = 0,
    cursor_hint_dual_wield: i32 = 0,
    objectives: [u8; 896] = [0; 896],

    // Kill-cam replay.
    /// How far an archived copy's clocks are behind "now"; zero on a live tick.
    delta_time: i32 = 0,
    kill_cam_entity: i32 = 0,
    kill_cam_look_at_entity: i32 = 0,
    kill_cam_client_num: i32 = 0,
}

impl PlayerState {
    /// True for the record of a tick being played now, false for an archived copy shown in a kill cam.
    pub const fn is_live_frame(&self) -> bool {
        self.delta_time == 0
    }

    /// The legs and torso animations, together.
    pub const fn anim(&self) -> AnimPair {
        AnimPair { legs_anim: self.legs_anim, torso_anim: self.torso_anim }
    }
}

/// The two halves of a body animation: legs and torso play separately.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AnimPair {
    pub legs_anim: i32,
    pub torso_anim: i32,
}

/// The gun the first-person view model shows: the grenade or equipment while it is out, otherwise the gun in hand.
/// A grenade slot that is somehow negative shows nothing (gun 0).
#[must_use]
pub fn get_viewmodel_weapon_index(ps: &PlayerState) -> u32 {
    match ps.weap_flags & weap_flags::OFFHAND_VIEW {
        0 => ps.weapon,
        _ => ps.off_hand_index.try_into().unwrap_or(0),
    }
}
