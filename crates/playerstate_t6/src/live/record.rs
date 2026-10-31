//! The per-player record the match carries from tick to tick, and the entity numbers it points at.

use crate::consts::MAX_PREDICTABLE_EVENTS;
use crate::playerstate::{MantleState, SprintState};

use super::flag_bits::weapon_state_bits;

/// The entity number meaning "nobody" (the last slot of the 2048-entry entity table).
pub const NO_ENTITY_SLOT: i32 = 2047;

// Movement types the camera and the view model care about (`AvatarState::movement_mode`).
pub const MOVE_MODE_WALK_ATTACHED: i32 = 1;
pub const MOVE_MODE_OBSERVER: i32 = 5;
pub const MOVE_MODE_INTERLUDE: i32 = 6;
pub const MOVE_MODE_DOWNED: i32 = 7;
/// This type and every one above it is a dead player.
pub const MOVE_KIND_DEAD: i32 = 10;
pub const MOVE_KIND_DEAD_TETHERED: i32 = 11;

/// Bodies left behind by players take the entity slots right after the client slots...
pub const BODY_ENTITY_FIRST_SLOT: i32 = 18;
/// ...one slot per body that can lie in the world at once...
pub const CORPSE_ENTITY_COUNT: i32 = 8;
/// ...and entities spawned by the level and the scripts start after them.
pub const GENTITY_SPAWN_BASE: i32 = CORPSE_ENTITY_COUNT + BODY_ENTITY_FIRST_SLOT;

const STANDING_STILL: SprintState = SprintState {
    sprint_duration: 0, sprint_cooldown: 0, sprint_wait: 0,
    sprint_stop_ms: 0, sprint_begin_ms: 0,
    sprint_launch_cap: 0, sprint_needs_release: 0,
};

const NOT_CLIMBING: MantleState = MantleState { yaw: 0.0, timer: 0, trans_index: 0, flags: 0 };

/// Writes the record and its blank value from one list, so a field cannot be added to one and forgotten in the other.
macro_rules! live_record {
    ($record:ident; $($(#[$attr:meta])* $member:ident: $kind:ty = $empty:expr,)*) => {
        /// Everything the match keeps about one player between ticks: movement, view, body animation, damage, guns,
        /// loadout, prompts and the kill-cam seat. Snapshots carry it to the owning client, and the kill cam replays
        /// archived copies of it.
        #[derive(Clone, Copy, Debug, PartialEq)]
        pub struct $record {
            $($(#[$attr])* pub $member: $kind,)*
        }

        impl $record {
            /// A player who has not spawned: everything cleared, standing on nothing, no gun state yet.
            pub const ZERO: Self = $record { $($member: $empty,)* };
        }
    };
}

live_record! {
    AvatarState;
    // How the movement code runs this player.
    /// Server time of the last user command played through movement.
    last_cmd_ms: i32 = 0,
    movement_mode: i32 = 0,
    /// Time left on the current movement lock (landing, knockback, and so on).
    move_timer: i32 = 0,
    movement_state_bits: u32 = 0,
    secondary_state_bits: u32 = 0,
    attach_bits: u32 = 0,
    sway_phase: i32 = 0,

    // Where the player is and how they move.
    origin: [f32; 3] = [0.0; 3],
    velocity: [f32; 3] = [0.0; 3],
    view_offset_angles: [f32; 3] = [0.0; 3],
    gravity: i32 = 0,
    speed: i32 = 0,
    tilt_sideways: f32 = 0.0,
    standing_on: i32 = 0,
    /// Facing direction of the ladder being climbed.
    ladder_normal_dir: [f32; 3] = [0.0; 3],
    hop_start_ms: i32 = 0,
    takeoff_height: f32 = 0.0,
    dive_end_time: i32 = 0,
    foliage_sound_time: i32 = 0,
    slide_time: i32 = 0,
    /// The moving platform the player rides; nobody until they step onto one.
    mover_entity_num: i32 = NO_ENTITY_SLOT,
    mover_timestamp: i32 = 0,
    waterlevel: i32 = 0,
    sprint_state: SprintState = STANDING_STILL,
    run_speed_modifier: f32 = 0.0,
    mantle_state: MantleState = NOT_CLIMBING,

    // Lying down.
    prone_direction: f32 = 0.0,
    prone_direction_pitch: f32 = 0.0,
    prone_torso_pitch: f32 = 0.0,

    // Eyes and view height.
    look_angles: [f32; 3] = [0.0; 3],
    eye_height_goal: i32 = 0,
    eye_height_now: f32 = 0.0,
    eye_blend_clock: i32 = 0,
    eye_blend_goal: i32 = 0,
    eye_lower_blend: i32 = 0,
    link_weapon_angles: [f32; 3] = [0.0; 3],

    // Body animation as other players see it.
    f_torso_pitch: f32 = 0.0,
    f_waist_pitch: f32 = 0.0,
    lower_body_clock: i32 = 0,
    legs_anim: i32 = 0,
    upper_body_clock: i32 = 0,
    torso_anim: i32 = 0,
    flinch_yaw_anim: i32 = 0,
    move_heading: i32 = 0,
    body_index: i32 = 0,
    e_flags: u32 = 0,

    // Events the owning client predicts for itself.
    predictable_event_sequence: i16 = 0,
    predictable_event_sequence_old: i16 = 0,
    predictable_events: [i32; MAX_PREDICTABLE_EVENTS] = [0; MAX_PREDICTABLE_EVENTS],
    predictable_event_parms: [u32; MAX_PREDICTABLE_EVENTS] = [0; MAX_PREDICTABLE_EVENTS],

    // Getting hurt.
    health: i32 = 0,
    hit_point_ceiling: i32 = 0,
    hurt_clock: i32 = 0,
    hurt_length: i32 = 0,
    damage_event: u32 = 0,
    damage_yaw: u32 = 0,
    damage_pitch: u32 = 0,
    hurt_amount: i32 = 0,
    stun_effect_id: i32 = 0,
    stun_start: i32 = 0,
    stun_length: i32 = 0,

    // Grenades in hand.
    cook_remaining: i32 = 0,
    returned_grenade_owner: i32 = 0,

    // The BO2 gun brain's own state per hand; -1 until a gun is raised.
    weapon_state_t6: i32 = -1,
    weapon_state_left_t6: i32 = -1,
    weap_anim_t6: i32 = 0,
    weap_anim_left_t6: i32 = 0,

    // Right hand (and the only hand without dual wield).
    weapon_anim_id: i32 = 0,
    weapon_timer: i32 = 0,
    weapon_ready_wait: i32 = 0,
    gun_lockout_kick_stamp: i32 = 0,
    weaponstate_primary: i32 = 0,
    weap_hand_flags: i32 = 0,
    shots_fired: i32 = 0,

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
    weapon_state_bits: u32 = 0,
    previous_gun_hand: i32 = 0,
    inventory_weapon: i32 = 0,
    ammo: [u8; 120] = [0; 120],
    magazine_rows: [u8; 180] = [0; 180],

    // Lethal and tactical slots.
    secondary_hand_slot: i32 = 0,
    secondary_gear_main: i32 = 0,
    secondary_gear_alt: i32 = 0,

    // Aiming and spread.
    weapon_raise_fraction: f32 = 0.0,
    sight_spread_factor: f32 = 0.0,
    scope_in_wait: i32 = 0,
    spread_forced_value: i32 = 0,
    spread_forced_phase: i32 = 0,
    recoil_scale: i32 = 0,

    // Knife lunge.
    lunge_heading: f32 = 0.0,
    lunge_reach: i32 = 0,
    lunge_clock: i32 = 0,

    // Perks and the d-pad slots.
    perks: [u32; 2] = [0; 2],
    loadout_perks: [u32; 8] = [0; 8],
    dpad_kind: [i32; 4] = [0; 4],
    action_slot_param: [i32; 4] = [0; 4],

    // Use prompts and the objective list on the HUD.
    hint_kind: i32 = 0,
    hint_text: i32 = 0,
    hint_entity: i32 = 0,
    cursor_hint_dual_wield: i32 = 0,
    objectives: [u8; 896] = [0; 896],

    // Kill-cam replay.
    /// How far an archived copy's clocks are behind "now"; zero on a live tick.
    time_slice: i32 = 0,
    death_replay_target: i32 = 0,
    death_replay_focus: i32 = 0,
    death_replay_player: i32 = 0,
}

impl AvatarState {
    /// True for the record of a tick being played now, false for an archived copy shown in a kill cam.
    pub const fn is_live_frame(
        &self,
    ) -> bool {
        matches!(self.time_slice, 0)
    }

    /// The legs and torso animations, together.
    pub const fn anim(&self) -> BodyAnims {
        BodyAnims { legs: self.legs_anim, torso: self.torso_anim }
    }

    /// Whether the player holds this perk, by BO2's perksEnum number (`BG_HasPerk`'s first test: bit `perk % 32` of
    /// word `perk / 32`). Numbers from `PERK_COUNT` up (`PERK_UNKNOWN`) are never held.
    pub const fn has_perk_bit(&self, perk: u32) -> bool {
        perk < crate::PERK_COUNT && self.perks[(perk >> 5) as usize] & 1 << (perk & 31) != 0
    }

    /// Give or take a perk by its perksEnum number; numbers from `PERK_COUNT` up do nothing.
    pub fn set_perk(&mut self, perk: u32, on: bool) {
        if perk >= crate::PERK_COUNT {
            return;
        }
        let word = &mut self.perks[(perk >> 5) as usize];
        if on {
            *word |= 1 << (perk & 31);
        } else {
            *word &= !(1 << (perk & 31));
        }
    }
}

/// The two halves of a body animation: legs and torso play separately.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BodyAnims {
    pub legs: i32,
    pub torso: i32,
}

/// The gun the first-person view model shows: the grenade or equipment while it is out, otherwise the gun in hand.
/// A grenade slot that is somehow negative shows nothing (gun 0).
#[must_use]
pub fn get_viewmodel_weapon_index(ps: &AvatarState) -> u32 {
    match ps.weapon_state_bits & weapon_state_bits::OFFHAND_VIEW {
        0 => ps.weapon,
        _ => ps.secondary_hand_slot.try_into().unwrap_or(0),
    }
}
