#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerState {
    pub command_time: i32,
    pub pm_type: i32,
    pub pm_time: i32,
    pub pm_flags: u32,
    pub other_flags: u32,
    pub link_flags: u32,
    pub bob_cycle: i32,
    pub origin: [f32; 3],
    pub velocity: [f32; 3],
    pub grenade_time_left: i32,
    pub throw_back_grenade_owner: i32,
    pub gravity: i32,
    pub leanf: f32,
    pub speed: i32,
    pub delta_angles: [f32; 3],
    pub ground_entity_num: i32,
    pub v_ladder_vec: [f32; 3],
    pub jump_time: i32,
    pub jump_origin_z: f32,
    /// bo2zm: when the last dive's slide ended (0: no dive yet).
    pub dive_end_time: i32,
    /// bo2mp: when the last foliage rustle sounded (0: none lately).
    pub foliage_sound_time: i32,
    /// bo2mp: Black Ops II's `slideTime`, the server time the player first
    /// stood on slide ground (a surface with the slick flag), 0 off it.
    pub slide_time: i32,
    /// bo2zm: the moving entity (a platform, a lift, a ladder on a mover)
    /// the player rides, `ENTITYNUM_NONE` when none. Each move starts by
    /// carrying the player and the view by how it moved since the last one.
    pub mover_entity_num: i32,
    /// bo2zm: the server time the mover was last stood on, climbed or
    /// mantled on; a player who left it is still carried for 2 s after.
    pub mover_timestamp: i32,
    /// bo2zm: the BO2 gun brain's own weapon state (0..64), which the legacy
    /// `weaponstate` above cannot say (the riot shield's states read as
    /// ready there). -1: no BO2 gun brain this frame, movement reads the
    /// legacy state.
    pub weapon_state_t6: i32,
    /// bo2mp: the left gun's BO2 weapon state (playerstate_t6 `weaponstate_left`),
    /// -1 with no BO2 gun brain. The legacy `weaponstate_secondary` is the bridge copy.
    pub weapon_state_left_t6: i32,
    /// bo2mp: BO2's weapon anim numbers (playerstate_t6 `weap_anim`,
    /// `weap_anim_left`), toggle bit included. The legacy `weap_anim` and
    /// `weap_anim_secondary` are the bridge copies.
    pub weap_anim_t6: i32,
    pub weap_anim_left_t6: i32,
    /// bo2mp: how deep the player stands in water, Black Ops II's water
    /// level 0..6 (worked out each move: 2 waist deep, 4 swimming, 6 the
    /// head under).
    pub waterlevel: i32,
    /// bo2zm: the way a prone body lies (yaw), its slope pitch and the
    /// torso's slope pitch, as Black Ops II keeps them for its crawl.
    pub prone_direction: f32,
    pub prone_direction_pitch: f32,
    pub prone_torso_pitch: f32,
    /// bo2zm: the last prone check's torso and waist pitch.
    pub f_torso_pitch: f32,
    pub f_waist_pitch: f32,
    pub legs_timer: i32,
    pub legs_anim: i32,
    pub torso_timer: i32,
    pub torso_anim: i32,
    pub damage_timer: i32,
    pub damage_duration: i32,
    pub flinch_yaw_anim: i32,
    pub corpse_index: i32,
    pub movement_dir: i32,
    pub e_flags: u32,
    pub predictable_event_sequence: i16,
    pub predictable_event_sequence_old: i16,
    pub predictable_events: [i32; crate::consts::MAX_PREDICTABLE_EVENTS],
    pub predictable_event_parms: [u32; crate::consts::MAX_PREDICTABLE_EVENTS],
    pub viewangles: [f32; 3],
    pub view_height_target: i32,
    pub view_height_current: f32,
    pub view_height_lerp_time: i32,
    pub view_height_lerp_target: i32,
    pub view_height_lerp_down: i32,
    pub damage_event: u32,
    pub damage_yaw: u32,
    pub damage_pitch: u32,
    pub damage_count: i32,
    pub health: i32,
    pub max_health: i32,
    pub link_weapon_angles: [f32; 3],
    pub cursor_hint: i32,
    pub cursor_hint_string: i32,
    pub cursor_hint_ent_index: i32,
    pub cursor_hint_dual_wield: i32,
    /// BO2's `sprintState`.
    pub sprint_state: crate::SprintState,
    pub move_speed_scale_multiplier: f32,
    /// BO2's `mantleState`.
    pub mantle_state: crate::MantleState,
    pub weap_anim: i32,
    pub weapon_time: i32,
    pub weapon_delay: i32,
    pub weapon_restrict_kick_time: i32,
    pub weaponstate_primary: i32,
    pub weap_hand_flags: i32,
    pub weapon_shot_count: i32,
    pub weap_anim_secondary: i32,
    pub weapon_time_secondary: i32,
    pub weapon_delay_secondary: i32,
    pub weapon_restrict_kick_time_secondary: i32,
    pub weaponstate_secondary: i32,
    pub weap_hand_flags_secondary: i32,
    pub weapon_shot_count_secondary: i32,
    pub weapons: [i32; 15],
    pub weapon_data: [u8; 76],
    pub off_hand_index: i32,
    pub offhand_primary: i32,
    pub offhand_secondary: i32,
    pub weapon: u32,
    pub weapon_primary: u32,
    pub weap_flags: u32,
    pub f_weapon_pos_frac: f32,
    pub aim_spread_scale: f32,
    pub ads_delay_time: i32,
    pub spread_override: i32,
    pub spread_override_state: i32,
    pub last_weapon_hand: i32,
    pub ammo: [u8; 0x78],
    pub ammoclip: [u8; 0xb4],
    pub melee_charge_yaw: f32,
    pub melee_charge_dist: i32,
    pub melee_charge_time: i32,
    pub perks: [u32; 2],
    pub perk_slots: [u32; 8],
    pub action_slot_type: [i32; 4],
    pub action_slot_param: [i32; 4],
    /// Black Ops II's `ps.inventoryWeapon` (`setinventoryweapon`); 0 = none.
    pub inventory_weapon: i32,
    pub shellshock_index: i32,
    pub shellshock_time: i32,
    pub shellshock_duration: i32,
    pub objectives: [u8; 0x380],
    pub delta_time: i32,
    pub kill_cam_entity: i32,
    pub kill_cam_look_at_entity: i32,
    pub kill_cam_client_num: i32,
    pub recoil_scale: i32,
}

pub mod eflags {
    pub const TELEPORT: u32 = 0x2;

    pub const DUCK: u32 = 0x4;

    pub const PRONE: u32 = 0x8;
    /// the legacy EF_NODRAW: nobody else draws this player's body (bo2mp mod
    /// menu "Invisible").
    pub const HIDDEN: u32 = 0x20;

    pub const KILLCAM_PRESERVED: u32 = 0x80;

    /// Black Ops II's dead bit (BG_PlayerStateToEntityState sets it from pm_type).
    pub const DEAD: u32 = 0x4_0000;

    pub const RADAR_JAM: u32 = 0x200000;
}

pub mod other_flags {
    pub const DEAD_KILLCAM_TPV: u32 = 0x800;

    /// Black Ops II's playing bit (ClientEndFrame; PM_IsAdsAllowed needs it).
    pub const PLAYER: u32 = 0x4;

    /// Black Ops II's SetEMPJammed bit.
    pub const EMP_JAMMED: u32 = 0x40;

    /// Black Ops II's SetInfraredVision bit.
    pub const INFRARED: u32 = 0x1000;
}

pub mod pm_flags {
    pub const PRONE: u32 = 0x1;

    pub const CROUCH: u32 = 0x2;

    pub const MANTLE: u32 = 0x4;

    pub const LADDER: u32 = 0x8;

    pub const ADS_INTENT: u32 = 0x10;

    pub const BACKWARDS_RUN: u32 = 0x20;

    pub const WALKING: u32 = 0x40;

    pub const TIME_HARDLANDING: u32 = 0x80;

    pub const PRONEMOVE_OVERRIDDEN: u32 = 0x200;

    pub const LADDER_FALL: u32 = 0x2000;

    pub const JUMPING: u32 = 0x4000;

    pub const SPRINTING: u32 = 0x8000;

    pub const SHELLSHOCKED: u32 = 0x0001_0000;

    pub const MELEE_CHARGE: u32 = 0x0002_0000;

    pub const SPRINT_BLOCKED: u32 = 0x0004_0000;
}

pub mod weap_flags {
    pub const NIGHT_VISION: u32 = 0x40;

    pub const OFFHAND_VIEW: u32 = 0x2;

    pub const NO_ADS: u32 = 0x20;

    pub const DOUBLEBARREL_RECOIL: u32 = 0x200;

    pub const RECOIL_SCALE: u32 = 0x400;
}

#[must_use]
pub fn get_viewmodel_weapon_index(ps: &PlayerState) -> u32 {
    if (ps.weap_flags & weap_flags::OFFHAND_VIEW) != 0 {
        u32::try_from(ps.off_hand_index).unwrap_or(0)
    } else {
        ps.weapon
    }
}

pub mod mantle_flags {
    pub const OVER: i32 = 1 << 0;

    pub const ACTIVE: i32 = 1 << 3;

    pub const QUICK: i32 = 1 << 4;
    pub const FAST_MANTLE: i32 = 1 << 6;

    /// bo2zm: a Black Ops II climb: the gun follows its rules below.
    pub const BO2: i32 = 1 << 10;
    /// bo2zm: a low Black Ops II climb keeps the gun in hand.
    pub const BO2_WEAPON_UP: i32 = 1 << 8;
    /// bo2zm: ... and plays the gun's climb camera animation over it.
    pub const BO2_WEAPON_ANIM: i32 = 1 << 9;
    /// bo2zm: a Black Ops II climb's whole length in ms sits from this bit
    /// up, so the client can play its gun animation without the climb tree.
    pub const BO2_DURATION_SHIFT: i32 = 16;

    #[must_use]
    pub const fn bo2_duration_ms(flags: i32) -> i32 {
        (flags as u32 >> BO2_DURATION_SHIFT) as i32
    }

    /// bo2zm: how far through a Black Ops II climb the player is (0..=1),
    /// or `None` outside one.
    #[must_use]
    pub fn bo2_progress(ps: &super::PlayerState) -> Option<f32> {
        if ps.pm_flags & super::pm_flags::MANTLE == 0 || ps.mantle_state.flags & BO2 == 0 {
            return None;
        }
        let duration = bo2_duration_ms(ps.mantle_state.flags);
        if duration <= 0 {
            return Some(1.0);
        }
        Some((ps.mantle_state.timer as f32 / duration as f32).clamp(0.0, 1.0))
    }

    /// bo2zm: the gun stays in hand through this Black Ops II climb.
    #[must_use]
    pub fn bo2_weapon_up(ps: &super::PlayerState) -> bool {
        bo2_progress(ps).is_some() && ps.mantle_state.flags & BO2_WEAPON_UP != 0
    }
}

/// entity_t6's `ENTITYNUM_NONE` (entity_t6 depends on this crate; its test
/// keeps the two equal).
pub const ENTITYNUM_NONE: i32 = 2047;

impl PlayerState {
    pub const ZERO: Self = Self {
        command_time: 0,
        pm_type: 0,
        pm_time: 0,
        pm_flags: 0,
        other_flags: 0,
        link_flags: 0,
        link_weapon_angles: [0.0; 3],
        bob_cycle: 0,
        origin: [0.0; 3],
        velocity: [0.0; 3],
        grenade_time_left: 0,
        throw_back_grenade_owner: 0,
        gravity: 0,
        leanf: 0.0,
        speed: 0,
        delta_angles: [0.0; 3],
        ground_entity_num: 0,
        v_ladder_vec: [0.0; 3],
        jump_time: 0,
        jump_origin_z: 0.0,
        dive_end_time: 0,
        foliage_sound_time: 0,
        slide_time: 0,
        mover_entity_num: ENTITYNUM_NONE,
        mover_timestamp: 0,
        weapon_state_t6: -1,
        weapon_state_left_t6: -1,
        weap_anim_t6: 0,
        weap_anim_left_t6: 0,
        waterlevel: 0,
        prone_direction: 0.0,
        prone_direction_pitch: 0.0,
        prone_torso_pitch: 0.0,
        f_torso_pitch: 0.0,
        f_waist_pitch: 0.0,
        legs_timer: 0,
        legs_anim: 0,
        torso_timer: 0,
        torso_anim: 0,
        damage_timer: 0,
        damage_duration: 0,
        flinch_yaw_anim: 0,
        corpse_index: 0,
        movement_dir: 0,
        e_flags: 0,
        predictable_event_sequence: 0,
        predictable_event_sequence_old: 0,
        predictable_events: [0; crate::consts::MAX_PREDICTABLE_EVENTS],
        predictable_event_parms: [0; crate::consts::MAX_PREDICTABLE_EVENTS],
        viewangles: [0.0; 3],
        view_height_target: 0,
        view_height_current: 0.0,
        view_height_lerp_time: 0,
        view_height_lerp_target: 0,
        view_height_lerp_down: 0,
        damage_event: 0,
        damage_yaw: 0,
        damage_pitch: 0,
        damage_count: 0,
        health: 0,
        max_health: 0,
        cursor_hint: 0,
        cursor_hint_string: 0,
        cursor_hint_ent_index: 0,
        cursor_hint_dual_wield: 0,
        sprint_state: crate::SprintState {
            sprint_button_up_required: 0,
            sprint_delay: 0,
            last_sprint_start: 0,
            last_sprint_end: 0,
            sprint_start_max_length: 0,
            sprint_duration: 0,
            sprint_cooldown: 0,
        },
        move_speed_scale_multiplier: 0.0,
        mantle_state: crate::MantleState {
            yaw: 0.0,
            timer: 0,
            trans_index: 0,
            flags: 0,
        },
        weap_anim: 0,
        weapon_time: 0,
        weapon_delay: 0,
        weapon_restrict_kick_time: 0,
        weaponstate_primary: 0,
        weap_hand_flags: 0,
        weapon_shot_count: 0,
        weap_anim_secondary: 0,
        weapon_time_secondary: 0,
        weapon_delay_secondary: 0,
        weapon_restrict_kick_time_secondary: 0,
        weaponstate_secondary: 0,
        weap_hand_flags_secondary: 0,
        weapon_shot_count_secondary: 0,
        weapons: [0; 15],
        weapon_data: [0; 76],
        off_hand_index: 0,
        offhand_primary: 0,
        offhand_secondary: 0,
        weapon: 0,
        weapon_primary: 0,
        weap_flags: 0,
        f_weapon_pos_frac: 0.0,
        aim_spread_scale: 0.0,
        ads_delay_time: 0,
        spread_override: 0,
        spread_override_state: 0,
        last_weapon_hand: 0,
        ammo: [0; 0x78],
        ammoclip: [0; 0xb4],
        melee_charge_yaw: 0.0,
        melee_charge_dist: 0,
        melee_charge_time: 0,
        perks: [0; 2],
        perk_slots: [0; 8],
        action_slot_type: [0; 4],
        action_slot_param: [0; 4],
        inventory_weapon: 0,
        shellshock_index: 0,
        shellshock_time: 0,
        shellshock_duration: 0,
        objectives: [0; 0x380],
        delta_time: 0,
        kill_cam_entity: 0,
        kill_cam_look_at_entity: 0,
        kill_cam_client_num: 0,
        recoil_scale: 0,
    };

    pub fn anim(&self) -> AnimPair {
        AnimPair {
            legs_anim: self.legs_anim,
            torso_anim: self.torso_anim,
        }
    }
}

pub const PERK_PISTOLDEATH: u32 = 1 << 7;

pub const PERK_QUIETER: u32 = 1 << 8;

pub const PERK_SCAVENGER: u32 = 1 << 22;

pub const PERK_COLDBLOODED: u32 = 1 << 27;

pub const PERK_HEARTBREAKER: u32 = 1 << 28;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AnimPair {
    pub legs_anim: i32,

    pub torso_anim: i32,
}
