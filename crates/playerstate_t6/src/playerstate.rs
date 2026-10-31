//! BO2's `playerState_s`, field for field in BO2's order.
//!
//! Left out: the Wii U controller fields and BO2's unused padding word,
//! which the PC game never reads.

use crate::consts::{
    MAX_HELD_WEAPONS, MAX_HUDELEMS_ARCHIVAL, MAX_HUDELEMS_CURRENT, MAX_OBJECTIVES,
    MAX_PREDICTABLE_EVENTS, MAX_STATS, action_slot,
};
use crate::hud::HudElement;
use crate::weapon::{RenderOptions, Weapon};

/// BO2's `PERK_COUNT`.
pub const PERK_COUNT: u32 = 63;

/// BO2's `SprintState`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SprintState {
    pub sprint_needs_release: i32,
    pub sprint_wait: i32,
    pub sprint_begin_ms: i32,
    pub sprint_stop_ms: i32,
    pub sprint_launch_cap: i32,
    pub sprint_duration: i32,
    pub sprint_cooldown: i32,
}

impl SprintState {
    /// `PM_IsSprinting`: a sprint has started and not ended since.
    #[must_use]
    pub const fn is_sprinting(&self) -> bool {
        self.sprint_begin_ms != 0 && self.sprint_stop_ms < self.sprint_begin_ms
    }
}

/// BO2's `MantleState`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MantleState {
    pub yaw: f32,
    pub timer: i32,
    pub trans_index: i32,
    pub flags: i32,
}

/// BO2's `PlayerVehicleState`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerVehicleState {
    pub origin: [f32; 3],
    pub angles: [f32; 3],
    pub velocity: [f32; 3],
    pub ang_velocity: [f32; 3],
    pub tilt: [f32; 2],
    pub tilt_velocity: [f32; 2],
    pub target_height_delta: f32,
    pub last_ground_height: f32,
    pub entity: i32,
    pub flags: i32,
    pub full_physics: bool,
}

/// BO2's `PlayerHeldWeapon`: one weapon the player carries.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerHeldWeapon {
    pub weapon: Weapon,
    pub options: RenderOptions,
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

/// BO2's `ActionSlotParam`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ActionSlotParam {
    pub weapon: Weapon,
}

/// BO2's `objective_t`: one minimap/compass objective.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MissionMarker {
    pub state: i32,
    pub origin: [f32; 3],
    pub ent_num: i16,
    pub size: [f32; 2],
    pub icon: i32,
    pub owner_num: i16,
    pub name: u16,
    pub team_mask: i16,
    pub progress: u8,
    pub client_use_mask: [i32; 1],
    pub gamemode_flags: u8,
    pub flags: u8,
    pub team_num: u8,
}

/// The two HUD element lists in BO2's `playerState_s`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerHud {
    pub current: [HudElement; MAX_HUDELEMS_CURRENT],
    pub archival: [HudElement; MAX_HUDELEMS_ARCHIVAL],
}

/// BO2's `playerState_s`. `Default` is BO2's cleared (all-zero) state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AvatarState {
    pub last_cmd_ms: i32,
    pub movement_mode: i32,
    pub sway_phase: i32,
    pub movement_state_bits: i32,
    /// 64 bits in BO2 (`weapFlags` is a `long64`).
    pub weapon_state_bits: u64,
    pub secondary_state_bits: i32,
    pub move_timer: i32,
    pub loop_sound_id: u32,
    pub loop_sound_fade: i32,
    pub origin: [f32; 3],
    pub velocity: [f32; 3],
    pub remote_eyes_ent: i32,
    pub remote_eyes_tagname: i32,
    pub remote_control_ent: i32,
    pub weapon_timer: i32,
    pub weapon_ready_wait: i32,
    pub weapon_time_left: i32,
    pub weapon_delay_left: i32,
    pub weapon_idle_time: i32,
    pub cook_remaining: i32,
    pub returned_grenade_owner: i32,
    pub throw_back_grenade_time_left: i32,
    pub gun_lockout_kick_stamp: i32,
    pub mount_available: bool,
    pub b_run_left_gun: bool,
    pub b_carrying_turret: bool,
    pub mount_pos: [f32; 3],
    pub mount_dir: f32,
    pub b_third_person: bool,
    pub foliage_sound_time: i32,
    pub gravity: i32,
    pub tilt_sideways: f32,
    pub speed: i32,
    pub view_offset_angles: [f32; 3],
    pub standing_on: i32,
    pub mover_entity_num: i32,
    pub mover_timestamp: i32,
    pub ground_type: i32,
    pub ladder_normal_dir: [f32; 3],
    pub hop_start_ms: i32,
    pub takeoff_height: f32,
    pub slide_time: i32,
    pub move_type: i32,
    pub lower_body_clock: i32,
    pub upper_body_clock: i32,
    pub legs_anim: i16,
    pub torso_anim: i16,
    pub legs_anim_duration: i32,
    pub torso_anim_duration: i32,
    pub hurt_clock: i32,
    pub hurt_length: i32,
    pub dmg_direction: i32,
    pub dmg_type: i32,
    pub body_index: i32,
    pub move_heading: i32,
    pub e_flags: i32,
    pub e_flags2: i32,
    pub vehicle_state: PlayerVehicleState,
    pub predictable_event_sequence: i16,
    pub predictable_event_sequence_old: i16,
    pub predictable_events: [i32; MAX_PREDICTABLE_EVENTS],
    pub predictable_event_parms: [u32; MAX_PREDICTABLE_EVENTS],
    pub unpredictable_event_sequence: i16,
    pub unpredictable_event_sequence_old: i16,
    pub unpredictable_events: [i32; MAX_PREDICTABLE_EVENTS],
    pub unpredictable_event_parms: [u32; MAX_PREDICTABLE_EVENTS],
    pub slot: i32,
    pub off_hand_weapon: Weapon,
    pub secondary_gear_alt: i32,
    pub secondary_gear_main: i32,
    pub render_options: RenderOptions,
    pub momentum: i32,
    pub weapon: Weapon,
    pub last_stand_prev_weapon: Weapon,
    pub last_weapon_alt_mode_switch: Weapon,
    pub stowed_weapon: Weapon,
    pub melee_weapon: Weapon,
    pub weapon_phase: i32,
    pub weaponstate_left: i32,
    pub weapon_shot_count: u32,
    pub weapon_shot_count_left: u32,
    pub weapon_raise_fraction: f32,
    pub scope_in_wait: i32,
    pub spread_forced_value: i32,
    pub spread_forced_phase: i32,
    pub weapon_spin_lerp: f32,
    pub viewmodel_index: i32,
    pub look_angles: [f32; 3],
    pub eye_height_goal: i32,
    pub eye_height_now: f32,
    pub eye_blend_clock: i32,
    pub eye_blend_goal: i32,
    pub eye_lower_blend: i32,
    pub view_angle_clamp_base: [f32; 2],
    pub view_angle_clamp_range: [f32; 2],
    pub damage_event: i32,
    pub damage_yaw: i32,
    pub damage_pitch: i32,
    pub hurt_amount: i32,
    pub stats: [i32; MAX_STATS],
    pub held_weapons: [PlayerHeldWeapon; MAX_HELD_WEAPONS],
    /// BO2's `AmmoPool`: spare rounds, one count per held-weapon slot.
    pub ammo_not_in_clip: [i32; MAX_HELD_WEAPONS],
    /// BO2's `AmmoClip`: rounds in the clip, one count per held-weapon slot.
    pub ammo_in_clip: [i32; MAX_HELD_WEAPONS],
    pub prone_direction: f32,
    pub prone_direction_pitch: f32,
    pub prone_torso_pitch: f32,
    pub viewlocked: i32,
    pub viewlocked_ent_num: i16,
    pub vehicle_pos: i32,
    pub vehicle_type: i32,
    pub vehicle_anim_bone_index: i32,
    pub attach_bits: i32,
    pub link_angles: [f32; 3],
    pub hint_kind: i32,
    pub hint_text: i32,
    pub hint_entity: i32,
    pub cursor_hint_weapon: Weapon,
    pub i_compass_player_info: i32,
    pub spyplane_type_enabled: i32,
    pub satellite_type_enabled: i32,
    pub location_selection_info: i32,
    pub location_selection_type: i32,
    pub sprint_state: SprintState,
    pub last_dtp_end: i32,
    pub f_torso_pitch: f32,
    pub f_waist_pitch: f32,
    pub steady_aim_factor: f32,
    pub hold_breath_timer: i32,
    pub charge_shot_timer: i32,
    pub charge_shot_level: u32,
    pub shots_fired_from_chamber: u32,
    pub quick_scope_scale: f32,
    pub quick_scope_timer: i32,
    pub client_fields: u32,
    pub client_fields2: u32,
    pub client_fields3: u32,
    pub entity_state_client_fields: u32,
    pub run_speed_modifier: f32,
    pub mantle_state: MantleState,
    pub vehicle_anim_stage: i32,
    pub vehicle_entry_point: i32,
    pub scripted_anim: u32,
    pub scripted_anim_time: i32,
    pub melee_charge_ent: i32,
    pub lunge_reach: i32,
    pub lunge_clock: i32,
    pub weap_lock_flags: i32,
    pub weap_locked_entnum: i32,
    pub airburst_mark_distance: u32,
    pub perks: [u32; 2],
    pub dpad_kind: [i32; action_slot::COUNT],
    pub action_slot_param: [ActionSlotParam; action_slot::COUNT],
    pub inventory_weapon: Weapon,
    pub vehicle_def_index: i32,
    pub entity_event_sequence: i32,
    pub weapon_anim_id: i32,
    pub weap_anim_left: i32,
    pub sight_spread_factor: f32,
    pub stun_effect_id: i32,
    pub stun_start: i32,
    pub stun_length: i32,
    pub dof_near_start: f32,
    pub dof_near_end: f32,
    pub dof_far_start: f32,
    pub dof_far_end: f32,
    pub dof_near_blur: f32,
    pub dof_far_blur: f32,
    pub dof_viewmodel_start: f32,
    pub dof_viewmodel_end: f32,
    pub waterlevel: i32,
    pub smoke_color_index: i32,
    pub hud_elem_last_assigned_sound_id: i32,
    pub ads_zoom_select: i32,
    pub ads_zoom_latch_time: i32,
    pub ads_zoom_latch_state: bool,
    pub ads_prev_zoom_select: i32,
    pub ads_prev_zoom_select_time: i32,
    pub artillery_inbound_icon_location: i32,
    pub vision_set_lerp_ratio: f32,
    pub poisoned: i32,
    pub binoculars: i32,
    pub script_cursor_hint_string: i32,
    pub objective: [MissionMarker; MAX_OBJECTIVES],
    pub time_slice: i32,
    pub death_replay_target: i32,
    pub kill_cam_target_entity: i32,
    pub intro_shots_fired: i32,
    pub hud: Box<PlayerHud>,
}

impl AvatarState {
    /// `PM_IsSprinting`.
    #[must_use]
    pub const fn is_sprinting(&self) -> bool {
        self.sprint_state.is_sprinting()
    }

    /// `PM_GetEffectiveStance`: the stance the view height says, with the
    /// 22-unit height read through the water level.
    #[must_use]
    pub const fn effective_stance(&self) -> i32 {
        use crate::flags::*;
        match self.eye_height_goal {
            eye_height::LOW if self.waterlevel > 2 => stance::UPRIGHT,
            eye_height::LOW if self.waterlevel != 0 => stance::DUCK_BUTTON_BIT,
            eye_height::LOW => stance::LYING_FLAT,
            eye_height::DUCK_BUTTON_BIT => stance::DUCK_BUTTON_BIT,
            eye_height::LYING_FLAT => stance::LYING_FLAT,
            _ => stance::UPRIGHT,
        }
    }

    /// `BG_IsSliding`: BO2 marks a slide with move type 26.
    #[must_use]
    pub const fn is_sliding(&self) -> bool {
        self.move_type == crate::flags::MOVE_TYPE_SLIDE
    }

    /// The player's own perk bit (`BG_HasPerk`'s first test: bit
    /// `perk % 32` of word `perk / 32`). BO2's `BG_HasPerk` also counts the
    /// perks the viewmodel weapon grants; that needs the weapon table.
    #[must_use]
    pub const fn has_perk_bit(&self, perk: u32) -> bool {
        perk < PERK_COUNT && self.perks[(perk >> 5) as usize] & 1 << (perk & 31) != 0
    }

    /// Give or take a perk.
    pub fn set_perk(&mut self, perk: u32, on: bool) {
        if perk >= PERK_COUNT {
            return;
        }
        let word = &mut self.perks[(perk >> 5) as usize];
        if on {
            *word |= 1 << (perk & 31);
        } else {
            *word &= !(1 << (perk & 31));
        }
    }

    /// The first held-weapon slot carrying exactly this weapon and
    /// attachments (`BG_GetHeldWeaponSlot`; BO2 returns -1 for none).
    #[must_use]
    pub fn held_weapon_slot(&self, weapon: Weapon) -> Option<usize> {
        self.held_weapons.iter().position(|h| h.weapon == weapon)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sprinting_like_bo2() {
        let mut s = SprintState::default();
        assert!(!s.is_sprinting());
        s.sprint_begin_ms = 500;
        assert!(s.is_sprinting());
        s.sprint_stop_ms = 800;
        assert!(!s.is_sprinting());
    }

    #[test]
    fn perks_are_two_words() {
        let mut ps = AvatarState::default();
        ps.set_perk(3, true);
        ps.set_perk(40, true);
        assert_eq!(ps.perks, [1 << 3, 1 << 8]);
        assert!(ps.has_perk_bit(40) && !ps.has_perk_bit(41) && !ps.has_perk_bit(63));
        ps.set_perk(3, false);
        assert!(!ps.has_perk_bit(3));
    }

    #[test]
    fn effective_stance_like_bo2() {
        use crate::flags::stance;
        let mut ps = AvatarState::default();
        for (h, w, want) in [
            (60, 0, stance::UPRIGHT),
            (0x28, 0, stance::DUCK_BUTTON_BIT),
            (0xb, 3, stance::LYING_FLAT),
            (0x16, 0, stance::LYING_FLAT),
            (0x16, 2, stance::DUCK_BUTTON_BIT),
            (0x16, 3, stance::UPRIGHT),
        ] {
            ps.eye_height_goal = h;
            ps.waterlevel = w;
            assert_eq!(ps.effective_stance(), want, "{h} {w}");
        }
    }

    #[test]
    fn held_slot_matches_attachments_too() {
        let mut ps = AvatarState::default();
        let gun = Weapon::new(12, 1, 0, 0);
        ps.held_weapons[4].weapon = gun;
        assert_eq!(ps.held_weapon_slot(gun), Some(4));
        assert_eq!(ps.held_weapon_slot(Weapon::new(12, 0, 0, 0)), None);
        assert_eq!(ps.held_weapon_slot(Weapon::new(13, 0, 0, 0)), None);
    }
}
