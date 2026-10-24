//! Pins the live player state (`playerstate_t6::game`) to its answers from before the rewrite of 10-23: every flag
//! and number it publishes, the blank state's fields and their types, the kill-cam seat, the chase camera on seeded
//! inputs, the climb progress and the gun the view model shows. All of it folds into one FNV-1a hash, and the
//! exhaustive destructures below stop the build if a field is added, dropped or renamed.

use playerstate_t6::game::{
    AnimPair, CG_CAMERA_PULLBACK_BOX_HALF, CG_CAMERA_PULLBACK_CLIPMASK, CG_THIRD_PERSON_RANGE_DEFAULT, ENTITYNUM_NONE,
    GENTITY_SPAWN_BASE, HITSCAN_KILL_CAM_ENTITY, KillCamMode, LINK_FLAGS_FORCE_THIRD_PERSON, MAX_CLIENT_CORPSES,
    OffsetThirdPersonViewInputs, PERK_COLDBLOODED, PERK_HEARTBREAKER, PERK_PISTOLDEATH, PERK_QUIETER, PERK_SCAVENGER,
    PLAYER_CORPSE_ENTITY_BASE, PM_TYPE_DEAD, PM_TYPE_DEAD_LINKED, PM_TYPE_INTERMISSION, PM_TYPE_LAST_STAND,
    PM_TYPE_NORMAL_LINKED, PM_TYPE_SPECTATOR, PlayerState, SeatFocus, ThirdPersonViewInputs, UserCmd, apply_killcam_seat,
    buttons, eflags, get_viewmodel_weapon_index, is_third_person_view, killcam_lerp_deadline_ms, last_input,
    look_at_killer_yaw, mantle_flags, offset_third_person_view, other_flags, pm_flags, rebase_archived_timers,
    viewweapon_frame_runs, weap_flags,
};

const EXPECTED: u64 = 0x385a_bc2c_fcaa_6b4d;

struct Fold(u64);

impl Fold {
    fn eat(&mut self, word: u32) {
        for byte in word.to_le_bytes() {
            self.0 = (self.0 ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    fn eat_i(&mut self, value: i32) {
        self.eat(value as u32);
    }
    fn eat_f(&mut self, value: f32) {
        self.eat(value.to_bits());
    }
    fn eat_v(&mut self, v: [f32; 3]) {
        v.into_iter().for_each(|x| self.eat_f(x));
    }
    fn eat_text(&mut self, text: &str) {
        text.bytes().for_each(|b| self.eat(b.into()));
    }
    /// A field: its name, its exact type and how it prints.
    fn eat_field<T: std::fmt::Debug>(&mut self, name: &str, value: &T) {
        self.eat_text(name);
        self.eat_text(std::any::type_name::<T>());
        self.eat_text(&format!("{value:?}"));
    }
}

/// xorshift64*.
struct Dice(u64);

impl Dice {
    fn roll(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn word(&mut self) -> u32 {
        (self.roll() >> 32) as u32
    }
    fn int(&mut self, lo: i32, hi: i32) -> i32 {
        lo + ((self.roll() >> 33) % (hi - lo + 1) as u64) as i32
    }
    fn chance(&mut self, one_in: u64) -> bool {
        (self.roll() >> 40) % one_in == 0
    }
    fn real(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * ((self.roll() >> 40) as f32 / (1u64 << 24) as f32)
    }
    fn point(&mut self, reach: f32) -> [f32; 3] {
        [self.real(-reach, reach), self.real(-reach, reach), self.real(-reach, reach)]
    }
    /// Mostly the given bits, now and then a random word.
    fn flags(&mut self, bits: &[u32]) -> u32 {
        if self.chance(5) {
            return self.word();
        }
        bits.iter().filter(|_| self.chance(2)).fold(0, |all, b| all | b)
    }
}

const KILLCAM_MODES: [KillCamMode; 9] = [
    KillCamMode::Mode0,
    KillCamMode::Mode1Heli,
    KillCamMode::Mode2Airstrike,
    KillCamMode::Mode3Missile,
    KillCamMode::Mode4MissileAlt,
    KillCamMode::Mode5Rocket,
    KillCamMode::Mode6Turret,
    KillCamMode::Mode7Javelin,
    KillCamMode::Mode8Remote,
];

fn numbers(out: &mut Fold) {
    let words = [
        pm_flags::PRONE,
        pm_flags::CROUCH,
        pm_flags::MANTLE,
        pm_flags::LADDER,
        pm_flags::ADS_INTENT,
        pm_flags::BACKWARDS_RUN,
        pm_flags::WALKING,
        pm_flags::TIME_HARDLANDING,
        pm_flags::PRONEMOVE_OVERRIDDEN,
        pm_flags::LADDER_FALL,
        pm_flags::JUMPING,
        pm_flags::SPRINTING,
        pm_flags::SHELLSHOCKED,
        pm_flags::MELEE_CHARGE,
        pm_flags::SPRINT_BLOCKED,
        eflags::TELEPORT,
        eflags::DUCK,
        eflags::PRONE,
        eflags::HIDDEN,
        eflags::KILLCAM_PRESERVED,
        eflags::DEAD,
        eflags::RADAR_JAM,
        other_flags::DEAD_KILLCAM_TPV,
        other_flags::PLAYER,
        other_flags::EMP_JAMMED,
        other_flags::INFRARED,
        weap_flags::NIGHT_VISION,
        weap_flags::OFFHAND_VIEW,
        weap_flags::NO_ADS,
        weap_flags::DOUBLEBARREL_RECOIL,
        weap_flags::RECOIL_SCALE,
        PERK_PISTOLDEATH,
        PERK_QUIETER,
        PERK_SCAVENGER,
        PERK_COLDBLOODED,
        PERK_HEARTBREAKER,
        CG_CAMERA_PULLBACK_CLIPMASK,
        LINK_FLAGS_FORCE_THIRD_PERSON,
        buttons::ATTACK,
        buttons::SPRINT,
        buttons::MELEE_CHARGE,
        buttons::USE,
        buttons::RELOAD,
        buttons::USE_RELOAD,
        buttons::PRONE,
        buttons::CROUCH,
        buttons::JUMP,
        buttons::ADS,
        buttons::STANCE_HELD,
        buttons::BREATH,
        buttons::FRAG,
        buttons::SMOKE,
        buttons::LOCATION_SELECT,
        buttons::LOCATION_CANCEL,
        buttons::THROW,
        buttons::REMOTE_CONTROL,
        buttons::OFFHAND_HOLD_CANCEL,
        buttons::CHANGE_SEAT,
        buttons::INVENTORY,
        buttons::JAM,
        buttons::DOUBLE_TAP,
    ];
    words.into_iter().for_each(|w| out.eat(w));
    let ints = [
        mantle_flags::OVER,
        mantle_flags::ACTIVE,
        mantle_flags::QUICK,
        mantle_flags::FAST_MANTLE,
        mantle_flags::BO2,
        mantle_flags::BO2_WEAPON_UP,
        mantle_flags::BO2_WEAPON_ANIM,
        mantle_flags::BO2_DURATION_SHIFT,
        ENTITYNUM_NONE,
        HITSCAN_KILL_CAM_ENTITY,
        PM_TYPE_SPECTATOR,
        PM_TYPE_INTERMISSION,
        PM_TYPE_LAST_STAND,
        PM_TYPE_DEAD,
        PM_TYPE_DEAD_LINKED,
        PM_TYPE_NORMAL_LINKED,
        MAX_CLIENT_CORPSES,
        PLAYER_CORPSE_ENTITY_BASE,
        GENTITY_SPAWN_BASE,
    ];
    ints.into_iter().for_each(|i| out.eat_i(i));
    out.eat_f(CG_THIRD_PERSON_RANGE_DEFAULT);
    out.eat_f(CG_CAMERA_PULLBACK_BOX_HALF);
    out.eat(last_input::GAMEPAD.into());
    out.eat(last_input::MOUSECLICK.into());
    for mode in KILLCAM_MODES {
        out.eat(mode as u32);
        out.eat_text(&format!("{mode:?}"));
    }
}

fn blank_state(out: &mut Fold) {
    let PlayerState {
        command_time,
        pm_type,
        pm_time,
        pm_flags,
        other_flags,
        link_flags,
        bob_cycle,
        origin,
        velocity,
        grenade_time_left,
        throw_back_grenade_owner,
        gravity,
        leanf,
        speed,
        delta_angles,
        ground_entity_num,
        v_ladder_vec,
        jump_time,
        jump_origin_z,
        dive_end_time,
        foliage_sound_time,
        slide_time,
        mover_entity_num,
        mover_timestamp,
        weapon_state_t6,
        weapon_state_left_t6,
        weap_anim_t6,
        weap_anim_left_t6,
        waterlevel,
        prone_direction,
        prone_direction_pitch,
        prone_torso_pitch,
        f_torso_pitch,
        f_waist_pitch,
        legs_timer,
        legs_anim,
        torso_timer,
        torso_anim,
        damage_timer,
        damage_duration,
        flinch_yaw_anim,
        corpse_index,
        movement_dir,
        e_flags,
        predictable_event_sequence,
        predictable_event_sequence_old,
        predictable_events,
        predictable_event_parms,
        viewangles,
        view_height_target,
        view_height_current,
        view_height_lerp_time,
        view_height_lerp_target,
        view_height_lerp_down,
        damage_event,
        damage_yaw,
        damage_pitch,
        damage_count,
        health,
        max_health,
        link_weapon_angles,
        cursor_hint,
        cursor_hint_string,
        cursor_hint_ent_index,
        cursor_hint_dual_wield,
        sprint_state,
        move_speed_scale_multiplier,
        mantle_state,
        weap_anim,
        weapon_time,
        weapon_delay,
        weapon_restrict_kick_time,
        weaponstate_primary,
        weap_hand_flags,
        weapon_shot_count,
        weap_anim_secondary,
        weapon_time_secondary,
        weapon_delay_secondary,
        weapon_restrict_kick_time_secondary,
        weaponstate_secondary,
        weap_hand_flags_secondary,
        weapon_shot_count_secondary,
        weapons,
        weapon_data,
        off_hand_index,
        offhand_primary,
        offhand_secondary,
        weapon,
        weapon_primary,
        weap_flags,
        f_weapon_pos_frac,
        aim_spread_scale,
        ads_delay_time,
        spread_override,
        spread_override_state,
        last_weapon_hand,
        ammo,
        ammoclip,
        melee_charge_yaw,
        melee_charge_dist,
        melee_charge_time,
        perks,
        perk_slots,
        action_slot_type,
        action_slot_param,
        inventory_weapon,
        shellshock_index,
        shellshock_time,
        shellshock_duration,
        objectives,
        delta_time,
        kill_cam_entity,
        kill_cam_look_at_entity,
        kill_cam_client_num,
        recoil_scale,
    } = PlayerState::ZERO;
    macro_rules! eat_all {
        ($($field:ident)*) => { $( out.eat_field(stringify!($field), &$field); )* };
    }
    eat_all!(
        command_time pm_type pm_time pm_flags other_flags link_flags bob_cycle origin velocity grenade_time_left
        throw_back_grenade_owner gravity leanf speed delta_angles ground_entity_num v_ladder_vec jump_time jump_origin_z
        dive_end_time foliage_sound_time slide_time mover_entity_num mover_timestamp weapon_state_t6 weapon_state_left_t6
        weap_anim_t6 weap_anim_left_t6 waterlevel prone_direction prone_direction_pitch prone_torso_pitch f_torso_pitch
        f_waist_pitch legs_timer legs_anim torso_timer torso_anim damage_timer damage_duration flinch_yaw_anim
        corpse_index movement_dir e_flags predictable_event_sequence predictable_event_sequence_old predictable_events
        predictable_event_parms viewangles view_height_target view_height_current view_height_lerp_time
        view_height_lerp_target view_height_lerp_down damage_event damage_yaw damage_pitch damage_count health
        max_health link_weapon_angles cursor_hint cursor_hint_string cursor_hint_ent_index cursor_hint_dual_wield
        sprint_state move_speed_scale_multiplier mantle_state weap_anim weapon_time weapon_delay
        weapon_restrict_kick_time weaponstate_primary weap_hand_flags weapon_shot_count weap_anim_secondary
        weapon_time_secondary weapon_delay_secondary weapon_restrict_kick_time_secondary weaponstate_secondary
        weap_hand_flags_secondary weapon_shot_count_secondary weapons weapon_data off_hand_index offhand_primary
        offhand_secondary weapon weapon_primary weap_flags f_weapon_pos_frac aim_spread_scale ads_delay_time
        spread_override spread_override_state last_weapon_hand ammo ammoclip melee_charge_yaw melee_charge_dist
        melee_charge_time perks perk_slots action_slot_type action_slot_param inventory_weapon shellshock_index
        shellshock_time shellshock_duration objectives delta_time kill_cam_entity kill_cam_look_at_entity
        kill_cam_client_num recoil_scale
    );
    let UserCmd {
        server_time,
        buttons,
        angles,
        weapon,
        weapon_mapped,
        off_hand_index,
        forwardmove,
        rightmove,
        melee_charge_yaw,
        melee_charge_dist,
        melee_charge_ent,
        selected_location,
        remote_control,
        last_input,
    } = UserCmd::default();
    eat_all!(
        server_time buttons angles weapon weapon_mapped off_hand_index forwardmove rightmove melee_charge_yaw
        melee_charge_dist melee_charge_ent selected_location remote_control last_input
    );
    let AnimPair { legs_anim, torso_anim } = AnimPair::default();
    eat_all!(legs_anim torso_anim);
    let pair = PlayerState { legs_anim: 41, torso_anim: -7, ..PlayerState::ZERO }.anim();
    out.eat_i(pair.legs_anim);
    out.eat_i(pair.torso_anim);
    for delta_time in [0, 1, -1, 50] {
        out.eat(PlayerState { delta_time, ..PlayerState::ZERO }.is_live_frame().into());
    }
}

fn random_state(dice: &mut Dice) -> PlayerState {
    let mut ps = PlayerState::ZERO;
    ps.command_time = dice.int(-5, 900_000);
    ps.pm_time = if dice.chance(3) { 0 } else { dice.int(-40, 4_000) };
    ps.jump_time = if dice.chance(3) { 0 } else { dice.word() as i32 };
    ps.view_height_lerp_time = if dice.chance(2) { 0 } else { dice.int(1, 900_000) };
    ps.shellshock_time = if dice.chance(2) { 0 } else { dice.int(-1, 900_000) };
    ps.delta_time = if dice.chance(4) { 0 } else { dice.word() as i32 };
    ps.e_flags = dice.flags(&[eflags::TELEPORT, eflags::DUCK, eflags::PRONE, eflags::KILLCAM_PRESERVED, eflags::DEAD]);
    ps.other_flags = dice.flags(&[other_flags::PLAYER, other_flags::DEAD_KILLCAM_TPV, other_flags::EMP_JAMMED, 2]);
    ps.pm_flags = dice.flags(&[pm_flags::MANTLE, pm_flags::PRONE, pm_flags::SPRINTING]);
    ps.weap_flags = dice.flags(&[weap_flags::OFFHAND_VIEW, weap_flags::NO_ADS]);
    ps.off_hand_index = dice.int(-3, 90);
    ps.weapon = dice.word() % 300;
    ps.kill_cam_entity = dice.int(-1, 2_047);
    ps.kill_cam_look_at_entity = dice.int(-1, 2_047);
    ps.kill_cam_client_num = dice.int(-1, 17);
    ps.origin = dice.point(4_000.0);
    ps.health = dice.int(0, 100);
    ps.mantle_state.flags = if dice.chance(5) {
        dice.word() as i32
    } else {
        let bits = dice.flags(&[mantle_flags::BO2 as u32, mantle_flags::BO2_WEAPON_UP as u32]) as i32 & 0xffff;
        bits | (dice.int(-2, 1_400) << mantle_flags::BO2_DURATION_SHIFT)
    };
    ps.mantle_state.timer = dice.int(-50, 1_600);
    ps
}

fn seats(out: &mut Fold, dice: &mut Dice) {
    for _ in 0..4_000 {
        let mut ps = random_state(dice);
        let before = ps;
        let delta = if dice.chance(6) { dice.word() as i32 } else { dice.int(-60_000, 60_000) };
        rebase_archived_timers(&mut ps, delta);
        for t in [ps.command_time, ps.pm_time, ps.jump_time, ps.view_height_lerp_time, ps.shellshock_time] {
            out.eat_i(t);
        }
        out.eat_i(ps.delta_time);
        out.eat(PlayerState { command_time: before.command_time, ..ps }.health as u32);

        let viewer = random_state(dice);
        let archived = random_state(dice);
        let focus = if dice.chance(2) {
            SeatFocus::hitscan(dice.int(-1, 17))
        } else {
            SeatFocus {
                kill_cam_entity: dice.int(-1, 2_047),
                kill_cam_look_at_entity: dice.int(-1, 2_047),
                kill_cam_client_num: dice.int(-1, 17),
            }
        };
        out.eat_text(&format!("{focus:?}"));
        let seat = apply_killcam_seat(&viewer, &archived, focus);
        out.eat(seat.e_flags);
        out.eat(seat.other_flags);
        out.eat_i(seat.kill_cam_entity);
        out.eat_i(seat.kill_cam_look_at_entity);
        out.eat_i(seat.kill_cam_client_num);
        let rest_kept = seat
            == PlayerState {
                e_flags: seat.e_flags,
                other_flags: seat.other_flags,
                kill_cam_entity: seat.kill_cam_entity,
                kill_cam_look_at_entity: seat.kill_cam_look_at_entity,
                kill_cam_client_num: seat.kill_cam_client_num,
                ..archived
            };
        out.eat(rest_kept.into());

        out.eat(get_viewmodel_weapon_index(&viewer));
        out.eat_i(mantle_flags::bo2_duration_ms(viewer.mantle_state.flags));
        out.eat_f(mantle_flags::bo2_progress(&viewer).unwrap_or(-9.0));
        out.eat(mantle_flags::bo2_weapon_up(&viewer).into());
    }
    for now_ms in [0, 1, 7_000, i32::MAX - 100, i32::MAX, -5] {
        for mode in KILLCAM_MODES {
            for (entering, changed) in [(false, false), (false, true), (true, false), (true, true)] {
                out.eat_i(killcam_lerp_deadline_ms(entering, changed, mode, now_ms));
            }
        }
    }
}

fn chase_camera(out: &mut Fold, dice: &mut Dice) {
    for pm_type in -2..=14 {
        out.eat(viewweapon_frame_runs(pm_type).into());
    }
    for _ in 0..6_000 {
        let view = ThirdPersonViewInputs {
            pm_type: dice.int(-1, 12),
            other_flags: dice.flags(&[2, other_flags::DEAD_KILLCAM_TPV, other_flags::PLAYER]),
            link_flags: dice.flags(&[LINK_FLAGS_FORCE_THIRD_PERSON, 1]),
            cg_third_person: dice.chance(2),
            in_killcam: dice.chance(2),
            killcam_mode: KILLCAM_MODES[dice.int(0, 8) as usize],
        };
        out.eat(is_third_person_view(view).into());

        let me = dice.point(3_000.0);
        let killer = (!dice.chance(4)).then(|| if dice.chance(9) { me } else { dice.point(3_000.0) });
        out.eat_i(look_at_killer_yaw(killer, me, dice.real(-400.0, 400.0)));
    }
    for _ in 0..6_000 {
        let other_flags = dice.flags(&[other_flags::DEAD_KILLCAM_TPV, other_flags::PLAYER]);
        let delta_time = if other_flags & other_flags::DEAD_KILLCAM_TPV != 0 { dice.int(1, 9_000) } else { 0 };
        let inputs = OffsetThirdPersonViewInputs {
            origin: dice.point(3_000.0),
            view_height_current: dice.real(10.0, 70.0),
            viewangles: [dice.real(-89.0, 89.0), dice.real(-180.0, 360.0), dice.real(-5.0, 5.0)],
            pm_type: dice.int(0, 12),
            look_at_killer_yaw: dice.real(-180.0, 180.0),
            corpse_j_mainroot: dice.chance(2).then(|| dice.point(3_000.0)),
            other_flags,
            delta_time,
            cg_third_person_angle: if dice.chance(3) { 0.0 } else { dice.real(-90.0, 90.0) },
            cg_third_person_range: if dice.chance(3) { CG_THIRD_PERSON_RANGE_DEFAULT } else { dice.real(0.0, 300.0) },
        };
        let wall = dice.int(0, 3);
        let first_hit = dice.real(0.0, 1.0);
        let second_hit = dice.real(0.0, 1.0);
        let mut traces = 0;
        let mut calls = Vec::new();
        let seen = offset_third_person_view(inputs, |from, to| {
            calls.push((from, to));
            traces += 1;
            match (wall, traces) {
                (0, _) => 1.0,
                (1, 1) => first_hit,
                (2, 1) => 0.0,
                (3, 1) => first_hit,
                (3, _) => second_hit,
                _ => 1.0,
            }
        });
        for (from, to) in calls {
            out.eat_v(from);
            out.eat_v(to);
        }
        out.eat_v(seen.origin);
        out.eat_v(seen.angles);
    }
}

#[test]
fn live_state_answers_match_the_recorded_bits() {
    let mut out = Fold(0xcbf2_9ce4_8422_2325);
    let mut dice = Dice(0x51ed_2701_d00d_f00d);
    numbers(&mut out);
    blank_state(&mut out);
    seats(&mut out, &mut dice);
    chase_camera(&mut out, &mut dice);
    let got = out.0;
    assert_eq!(got, EXPECTED, "live state fingerprint is now {got:#018x}");
}

#[test]
#[should_panic]
fn a_death_cam_seat_on_a_live_frame_is_refused() {
    let inputs = OffsetThirdPersonViewInputs {
        origin: [0.0; 3],
        view_height_current: 60.0,
        viewangles: [0.0; 3],
        pm_type: PM_TYPE_DEAD,
        look_at_killer_yaw: 0.0,
        corpse_j_mainroot: None,
        other_flags: other_flags::DEAD_KILLCAM_TPV,
        delta_time: 0,
        cg_third_person_angle: 0.0,
        cg_third_person_range: CG_THIRD_PERSON_RANGE_DEFAULT,
    };
    offset_third_person_view(inputs, |_, _| 1.0);
}
