//! Pins the live player state (`playerstate_t6::game`) to its answers from before the rewrite of 10-23: every flag
//! and number it publishes, the blank state's fields and their types, the kill-cam seat, the chase camera on seeded
//! inputs, the climb progress and the gun the view model shows. All of it folds into one FNV-1a hash, and the
//! exhaustive destructures below stop the build if a field is added, dropped or renamed.

use playerstate_t6::game::{self, *};
use playerstate_t6::game::buttons as button_bit;

const EXPECTED: u64 = 0x557c_100a_f5c1_e8a4;

struct Fold(u64);

impl Fold {
    fn eat(&mut self, word: u32) {
        for byte in word.to_le_bytes() {
            self.0 = (self.0 ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    fn eat_i(&mut self, n: i32) {
        self.eat(n as u32);
    }
    fn eat_f(&mut self, x: f32) {
        self.eat(x.to_bits());
    }
    fn eat_v(&mut self, v: [f32; 3]) {
        v.into_iter().for_each(|x| self.eat_f(x));
    }
    fn eat_text(&mut self, words: &str) {
        words.bytes().for_each(|b| self.eat(b.into()));
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

const KILLCAM_MODES: [DeathReplayStyle; 9] = [
    DeathReplayStyle::Mode0,
    DeathReplayStyle::HelicopterView,
    DeathReplayStyle::AirstrikeView,
    DeathReplayStyle::GuidedMissileView,
    DeathReplayStyle::AltMissileView,
    DeathReplayStyle::RocketView,
    DeathReplayStyle::TurretView,
    DeathReplayStyle::AntiTankView,
    DeathReplayStyle::RemoteDeviceView,
];

fn numbers(out: &mut Fold) {
    let words = [
        movement_state_bits::LYING_FLAT,
        movement_state_bits::DUCK_BUTTON_BIT,
        movement_state_bits::MANTLE,
        movement_state_bits::LADDER,
        movement_state_bits::ADS_INTENT,
        movement_state_bits::BACKWARDS_RUN,
        movement_state_bits::WALKING,
        movement_state_bits::TIME_HARDLANDING,
        movement_state_bits::PRONEMOVE_OVERRIDDEN,
        movement_state_bits::LADDER_FALL,
        movement_state_bits::JUMPING,
        movement_state_bits::SPRINTING,
        movement_state_bits::SHELLSHOCKED,
        movement_state_bits::MELEE_CHARGE,
        movement_state_bits::SPRINT_BLOCKED,
        entity_flag_bits::TELEPORT,
        entity_flag_bits::CROUCH_BIT,
        entity_flag_bits::LYING_FLAT,
        entity_flag_bits::HIDDEN,
        entity_flag_bits::KILLCAM_PRESERVED,
        entity_flag_bits::DEAD,
        entity_flag_bits::RADAR_JAM,
        secondary_state_bits::DEAD_KILLCAM_TPV,
        secondary_state_bits::PLAYER,
        secondary_state_bits::EMP_JAMMED,
        secondary_state_bits::INFRARED,
        weapon_state_bits::NIGHT_VISION,
        weapon_state_bits::OFFHAND_VIEW,
        weapon_state_bits::AIM_DISABLED_BIT,
        weapon_state_bits::DOUBLEBARREL_RECOIL,
        weapon_state_bits::RECOIL_SCALE,
    ]
    .into_iter()
    .chain([CHASE_CAM_RETREAT_MASK, LINK_FLAGS_FORCE_THIRD_PERSON])
    .chain([
        button_bit::ATTACK,
        button_bit::SPRINT,
        button_bit::MELEE_CHARGE,
        button_bit::INTERACT_KEY,
        button_bit::RELOAD,
        button_bit::INTERACT_RELOAD_KEY,
        button_bit::LYING_FLAT,
        button_bit::DUCK_BUTTON_BIT,
        button_bit::LEAP,
        button_bit::ADS,
        button_bit::STANCE_LOCK_BIT,
        button_bit::STEADY_AIM_HOLD_BIT,
        button_bit::SAMPLE_FRAG_ASSET,
        button_bit::OBSCURANT,
        button_bit::MAP_TARGET_PICK,
        button_bit::LOCATION_CANCEL,
        button_bit::THROW,
        button_bit::REMOTE_CONTROL,
        button_bit::OFFHAND_HOLD_CANCEL,
        button_bit::CHANGE_SEAT,
        button_bit::INVENTORY,
        button_bit::JAM,
        button_bit::DOUBLE_TAP,
    ]);
    words.for_each(|w| out.eat(w));
    let ints = [
        ledge_climb_bits::OVER,
        ledge_climb_bits::ACTIVE,
        ledge_climb_bits::QUICK,
        ledge_climb_bits::FAST_MANTLE,
        ledge_climb_bits::BO2,
        ledge_climb_bits::BO2_WEAPON_UP,
        ledge_climb_bits::BO2_WEAPON_ANIM,
        ledge_climb_bits::BO2_DURATION_SHIFT,
        game::NO_ENTITY_SLOT,
        game::HITSCAN_KILL_CAM_ENTITY,
        game::MOVE_MODE_OBSERVER,
        game::MOVE_MODE_INTERLUDE,
        game::MOVE_MODE_DOWNED,
        game::MOVE_KIND_DEAD,
        game::MOVE_KIND_DEAD_TETHERED,
        game::MOVE_MODE_WALK_ATTACHED,
        game::CORPSE_ENTITY_COUNT,
        game::BODY_ENTITY_FIRST_SLOT,
        game::GENTITY_SPAWN_BASE,
    ];
    ints.into_iter().for_each(|i| out.eat_i(i));
    out.eat_f(CHASE_CAM_DISTANCE_FALLBACK);
    out.eat_f(CHASE_CAM_PROBE_HALF_EXTENT);
    out.eat(last_input::GAMEPAD.into());
    out.eat(last_input::MOUSECLICK.into());
    for mode in KILLCAM_MODES {
        out.eat(mode as u32);
        out.eat_text(&format!("{mode:?}"));
    }
}

fn blank_state(out: &mut Fold) {
    macro_rules! eat_every_field {
        ($sample:expr => $ty:ident: $($field:ident)*) => {
            let $ty { $($field),* } = $sample;
            $( out.eat_field(stringify!($field), &$field); )*
        };
    }
    eat_every_field!(AvatarState::ZERO => AvatarState:
        last_cmd_ms movement_mode move_timer movement_state_bits secondary_state_bits attach_bits sway_phase origin velocity cook_remaining
        returned_grenade_owner gravity tilt_sideways speed view_offset_angles standing_on ladder_normal_dir hop_start_ms takeoff_height
        dive_end_time foliage_sound_time slide_time mover_entity_num mover_timestamp weapon_state_t6 weapon_state_left_t6
        weap_anim_t6 weap_anim_left_t6 waterlevel prone_direction prone_direction_pitch prone_torso_pitch f_torso_pitch
        f_waist_pitch lower_body_clock legs_anim upper_body_clock torso_anim hurt_clock hurt_length flinch_yaw_anim
        body_index move_heading e_flags predictable_event_sequence predictable_event_sequence_old predictable_events
        predictable_event_parms look_angles eye_height_goal eye_height_now eye_blend_clock
        eye_blend_goal eye_lower_blend damage_event damage_yaw damage_pitch hurt_amount health
        hit_point_ceiling link_weapon_angles hint_kind hint_text hint_entity cursor_hint_dual_wield
        sprint_state run_speed_modifier mantle_state weapon_anim_id weapon_timer weapon_ready_wait
        gun_lockout_kick_stamp weaponstate_primary weap_hand_flags shots_fired weap_anim_secondary
        weapon_time_secondary weapon_delay_secondary weapon_restrict_kick_time_secondary weaponstate_secondary
        weap_hand_flags_secondary weapon_shot_count_secondary weapons weapon_data secondary_hand_slot secondary_gear_main
        secondary_gear_alt weapon weapon_primary weapon_state_bits weapon_raise_fraction sight_spread_factor scope_in_wait
        spread_forced_value spread_forced_phase previous_gun_hand ammo magazine_rows lunge_heading lunge_reach
        lunge_clock perks loadout_perks dpad_kind action_slot_param inventory_weapon stun_effect_id
        stun_start stun_length objectives time_slice death_replay_target death_replay_focus
        death_replay_player recoil_scale
    );
    eat_every_field!(InputCommand::default() => InputCommand:
        authority_clock_ms buttons angles weapon weapon_slot_code secondary_hand_slot thrust_ahead thrust_lateral lunge_heading
        lunge_reach melee_charge_ent chosen_map_spot remote_operated last_input
    );
    eat_every_field!(BodyAnims::default() => BodyAnims: legs torso);
    let pair = AvatarState { legs_anim: 41, torso_anim: -7, ..AvatarState::ZERO }.anim();
    out.eat_i(pair.legs);
    out.eat_i(pair.torso);
    for time_slice in [0, 1, -1, 50] {
        out.eat(AvatarState { time_slice, ..AvatarState::ZERO }.is_live_frame().into());
    }
}

fn random_state(dice: &mut Dice) -> AvatarState {
    let mut ps = AvatarState::ZERO;
    ps.last_cmd_ms = dice.int(-5, 900_000);
    ps.move_timer = if dice.chance(3) { 0 } else { dice.int(-40, 4_000) };
    ps.hop_start_ms = if dice.chance(3) { 0 } else { dice.word() as i32 };
    ps.eye_blend_clock = if dice.chance(2) { 0 } else { dice.int(1, 900_000) };
    ps.stun_start = if dice.chance(2) { 0 } else { dice.int(-1, 900_000) };
    ps.time_slice = if dice.chance(4) { 0 } else { dice.word() as i32 };
    ps.e_flags = dice.flags(&[entity_flag_bits::TELEPORT, entity_flag_bits::CROUCH_BIT, entity_flag_bits::LYING_FLAT, entity_flag_bits::KILLCAM_PRESERVED, entity_flag_bits::DEAD]);
    ps.secondary_state_bits = dice.flags(&[secondary_state_bits::PLAYER, secondary_state_bits::DEAD_KILLCAM_TPV, secondary_state_bits::EMP_JAMMED, 2]);
    ps.movement_state_bits = dice.flags(&[movement_state_bits::MANTLE, movement_state_bits::LYING_FLAT, movement_state_bits::SPRINTING]);
    ps.weapon_state_bits = dice.flags(&[weapon_state_bits::OFFHAND_VIEW, weapon_state_bits::AIM_DISABLED_BIT]);
    ps.secondary_hand_slot = dice.int(-3, 90);
    ps.weapon = dice.word() % 300;
    ps.death_replay_target = dice.int(-1, 2_047);
    ps.death_replay_focus = dice.int(-1, 2_047);
    ps.death_replay_player = dice.int(-1, 17);
    ps.origin = dice.point(4_000.0);
    ps.health = dice.int(0, 100);
    ps.mantle_state.flags = if dice.chance(5) {
        dice.word() as i32
    } else {
        let bits = dice.flags(&[ledge_climb_bits::BO2 as u32, ledge_climb_bits::BO2_WEAPON_UP as u32]) as i32 & 0xffff;
        bits | (dice.int(-2, 1_400) << ledge_climb_bits::BO2_DURATION_SHIFT)
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
        for t in [ps.last_cmd_ms, ps.move_timer, ps.hop_start_ms, ps.eye_blend_clock, ps.stun_start] {
            out.eat_i(t);
        }
        out.eat_i(ps.time_slice);
        out.eat(AvatarState { last_cmd_ms: before.last_cmd_ms, ..ps }.health as u32);

        let viewer = random_state(dice);
        let archived = random_state(dice);
        let focus = if dice.chance(2) {
            SpectatorTarget::hitscan(dice.int(-1, 17))
        } else {
            SpectatorTarget {
                death_replay_target: dice.int(-1, 2_047),
                death_replay_focus: dice.int(-1, 2_047),
                death_replay_player: dice.int(-1, 17),
            }
        };
        out.eat_text(&format!("{focus:?}"));
        let seat = apply_killcam_seat(&viewer, &archived, focus);
        out.eat(seat.e_flags);
        out.eat(seat.secondary_state_bits);
        out.eat_i(seat.death_replay_target);
        out.eat_i(seat.death_replay_focus);
        out.eat_i(seat.death_replay_player);
        let rest_kept = seat
            == AvatarState {
                e_flags: seat.e_flags,
                secondary_state_bits: seat.secondary_state_bits,
                death_replay_target: seat.death_replay_target,
                death_replay_focus: seat.death_replay_focus,
                death_replay_player: seat.death_replay_player,
                ..archived
            };
        out.eat(rest_kept.into());

        out.eat(get_viewmodel_weapon_index(&viewer));
        out.eat_i(ledge_climb_bits::bo2_duration_ms(viewer.mantle_state.flags));
        out.eat_f(ledge_climb_bits::bo2_progress(&viewer).unwrap_or(-9.0));
        out.eat(ledge_climb_bits::bo2_weapon_up(&viewer).into());
    }
    for clock_millis in [0, 1, 7_000, i32::MAX - 100, i32::MAX, -5] {
        for mode in KILLCAM_MODES {
            for (entering, changed) in [(false, false), (false, true), (true, false), (true, true)] {
                out.eat_i(killcam_blend_until_ms(entering, changed, mode, clock_millis));
            }
        }
    }
}

fn chase_camera(out: &mut Fold, dice: &mut Dice) {
    for movement_mode in -2..=14 {
        out.eat(viewweapon_frame_runs(movement_mode).into());
    }
    for _ in 0..6_000 {
        let view = ThirdPersonViewInputs {
            movement_mode: dice.int(-1, 12),
            secondary_state_bits: dice.flags(&[2, secondary_state_bits::DEAD_KILLCAM_TPV, secondary_state_bits::PLAYER]),
            attach_bits: dice.flags(&[LINK_FLAGS_FORCE_THIRD_PERSON, 1]),
            chase_cam_enabled: dice.chance(2),
            replaying_final_death: dice.chance(2),
            killcam_mode: KILLCAM_MODES[dice.int(0, 8) as usize],
        };
        out.eat(is_third_person_view(view).into());

        let me = dice.point(3_000.0);
        let killer = (!dice.chance(4)).then(|| if dice.chance(9) { me } else { dice.point(3_000.0) });
        out.eat_i(look_at_killer_yaw(killer, me, dice.real(-400.0, 400.0)));
    }
    for _ in 0..6_000 {
        let secondary_state_bits = dice.flags(&[secondary_state_bits::DEAD_KILLCAM_TPV, secondary_state_bits::PLAYER]);
        let time_slice = if secondary_state_bits & secondary_state_bits::DEAD_KILLCAM_TPV != 0 { dice.int(1, 9_000) } else { 0 };
        let inputs = ChaseCamPlacementInputs {
            origin: dice.point(3_000.0),
            eye_height_now: dice.real(10.0, 70.0),
            look_angles: [dice.real(-89.0, 89.0), dice.real(-180.0, 360.0), dice.real(-5.0, 5.0)],
            move_mode: dice.int(0, 12),
            killer_facing_yaw: dice.real(-180.0, 180.0),
            body_root_joint_pos: dice.chance(2).then(|| dice.point(3_000.0)),
            secondary_state_bits,
            time_slice,
            chase_cam_yaw_offset: if dice.chance(3) { 0.0 } else { dice.real(-90.0, 90.0) },
            chase_cam_back_distance: if dice.chance(3) { CHASE_CAM_DISTANCE_FALLBACK } else { dice.real(0.0, 300.0) },
        };
        let wall = dice.int(0, 3);
        let first_hit = dice.real(0.0, 1.0);
        let second_hit = dice.real(0.0, 1.0);
        let mut traces = 0;
        let mut calls = Vec::new();
        let seen = place_chase_camera(inputs, |from, to| {
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
    let default_range = CHASE_CAM_DISTANCE_FALLBACK;
    let still = [0.0; 3];
    let inputs = ChaseCamPlacementInputs {
        origin: still,
        move_mode: MOVE_KIND_DEAD,
        secondary_state_bits: secondary_state_bits::DEAD_KILLCAM_TPV,
        chase_cam_yaw_offset: 0.0,
        eye_height_now: 60.0,
        killer_facing_yaw: 0.0,
        look_angles: still,
        time_slice: i32::default(),
        body_root_joint_pos: Option::default(),
        chase_cam_back_distance: default_range,
    };
    place_chase_camera(inputs, |_, _| 1.0);
}
