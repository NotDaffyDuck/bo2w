//! bo2mp: Black Ops II's own gun pose (BG_CalculateWeaponMovement_Base): where the gun sits for
//! the stance and the walk, plus the sprint, dive, slide and climb offsets, eased by the gun's
//! own rates. It replaces the MW2 stance pose for every BO2 gun; `recoil::weapon_movement_angles`
//! then scales what it leaves by the aim.

use crate::pm::spread::{rd_f32, rd_i32};
use crate::state;
use fastfile_t6::layout::WeaponDef;

/// The pose numbers from the weapon file (the stand and strafe turn include the attachments'
/// overrides, folded in by `SpreadFacts::fold_in_addons`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PoseFacts {
    pub stand_move: [f32; 3],
    pub stand_rot: [f32; 3],
    pub strafe_move: [f32; 3],
    pub strafe_rot: [f32; 3],
    pub ducked_move: [f32; 3],
    pub ducked_rot: [f32; 3],
    pub prone_move: [f32; 3],
    pub prone_rot: [f32; 3],
    pub ducked_ofs: [f32; 3],
    pub prone_ofs: [f32; 3],
    pub aimed_pitch: f32,
    pub stand_min_speed: f32,
    pub ducked_min_speed: f32,
    pub prone_min_speed: f32,
    pub pos_move_rate: f32,
    pub pos_prone_move_rate: f32,
    pub pos_rot_rate: f32,
    pub pos_prone_rot_rate: f32,
    pub sprint_ofs: [f32; 3],
    pub sprint_rot: [f32; 3],
    pub dtp_ofs: [f32; 3],
    pub dtp_rot: [f32; 3],
    pub slide_ofs: [f32; 3],
    pub slide_rot: [f32; 3],
    pub mantle_ofs: [f32; 3],
    pub mantle_rot: [f32; 3],
    pub sprint_in_ms: i32,
    pub sprint_out_ms: i32,
    pub dtp_in_ms: i32,
    pub dtp_out_ms: i32,
    pub slide_in_ms: i32,
}

fn vec3(def: &[u8], at: usize) -> [f32; 3] {
    [rd_f32(def, at), rd_f32(def, at + 4), rd_f32(def, at + 8)]
}

impl PoseFacts {
    pub fn read(def: &[u8]) -> Self {
        Self {
            stand_move: vec3(def, WeaponDef::vStandMove),
            stand_rot: vec3(def, WeaponDef::vStandRot),
            strafe_move: vec3(def, WeaponDef::vStrafeMove),
            strafe_rot: vec3(def, WeaponDef::vStrafeRot),
            ducked_move: vec3(def, WeaponDef::vDuckedMove),
            ducked_rot: vec3(def, WeaponDef::vDuckedRot),
            prone_move: vec3(def, WeaponDef::vProneMove),
            prone_rot: vec3(def, WeaponDef::vProneRot),
            ducked_ofs: vec3(def, WeaponDef::vDuckedOfs),
            prone_ofs: vec3(def, WeaponDef::vProneOfs),
            aimed_pitch: rd_f32(def, WeaponDef::fAdsAimPitch),
            stand_min_speed: rd_f32(def, WeaponDef::fStandMoveMinSpeed),
            ducked_min_speed: rd_f32(def, WeaponDef::fDuckedMoveMinSpeed),
            prone_min_speed: rd_f32(def, WeaponDef::fProneMoveMinSpeed),
            pos_move_rate: rd_f32(def, WeaponDef::fPosMoveRate),
            pos_prone_move_rate: rd_f32(def, WeaponDef::fPosProneMoveRate),
            pos_rot_rate: rd_f32(def, WeaponDef::fPosRotRate),
            pos_prone_rot_rate: rd_f32(def, WeaponDef::fPosProneRotRate),
            sprint_ofs: vec3(def, WeaponDef::vSprintOfs),
            sprint_rot: vec3(def, WeaponDef::vSprintRot),
            dtp_ofs: vec3(def, WeaponDef::vDtpOfs),
            dtp_rot: vec3(def, WeaponDef::vDtpRot),
            slide_ofs: vec3(def, WeaponDef::vSlideOfs),
            slide_rot: vec3(def, WeaponDef::vSlideRot),
            mantle_ofs: vec3(def, WeaponDef::vMantleOfs),
            mantle_rot: vec3(def, WeaponDef::vMantleRot),
            sprint_in_ms: rd_i32(def, WeaponDef::sprintInTime),
            sprint_out_ms: rd_i32(def, WeaponDef::sprintOutTime),
            dtp_in_ms: rd_i32(def, WeaponDef::dtpInTime),
            dtp_out_ms: rd_i32(def, WeaponDef::dtpOutTime),
            slide_in_ms: rd_i32(def, WeaponDef::slideInTime),
        }
    }
}

/// What the pose reads from the player this frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct PoseInput {
    /// eFlags crouched (0x4) and prone (0x8).
    pub ducked: bool,
    pub prone: bool,
    /// A dive to prone is under way (in the air or sliding): a prone diver's gun walks like a
    /// standing one.
    pub diving: bool,
    pub on_ladder: bool,
    /// The right hand's and the left hand's weapon state, and the right hand's timer (ms).
    pub gun_phase: i32,
    pub weaponstate_left: i32,
    pub gun_timer_ms: i32,
    /// How far the aim is in (`fWeaponPosFrac`).
    pub frac: f32,
    pub ground_velocity: f32,
    /// The player's top speed (`ps.speed`).
    pub speed: f32,
    pub velocity: [f32; 3],
    pub look_angles: [f32; 3],
    /// Seconds this frame.
    pub step_millis_elapsed: f32,
    /// The player or the gun has the fast sprint recovery perk (57).
    pub fast_sprint_recovery: bool,
    /// `perk_sprintRecoveryMultiplier` (0.3).
    pub sprint_recovery_multiplier: f32,
    /// How far through the climb, when the gun stays up for it (`Mantle_GetMantleFrac`).
    pub mantle_frac: Option<f32>,
}

/// The eased pose the gun carries from frame to frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BasePose {
    pub angles: [f32; 3],
    pub origin: [f32; 3],
}

impl BasePose {
    /// One frame: build this frame's target, then ease toward it.
    pub fn step(&mut self, f: &PoseFacts, scoped_aiming: bool, i: &PoseInput) {
        let (shift, turn) = target(f, scoped_aiming, i);
        let (move_rate, rot_rate) = if i.prone {
            (f.pos_prone_move_rate, f.pos_prone_rot_rate)
        } else {
            (f.pos_move_rate, f.pos_rot_rate)
        };
        let o = (move_rate * i.step_millis_elapsed).min(1.0);
        let a = (rot_rate * i.step_millis_elapsed).min(1.0);
        for k in 0..3 {
            self.origin[k] += (shift[k] - self.origin[k]) * o;
            self.angles[k] += (turn[k] - self.angles[k]) * a;
        }
    }
}

fn add(v: &mut [f32; 3], d: [f32; 3], k: f32) {
    for (a, b) in v.iter_mut().zip(d) {
        *a += b * k;
    }
}

/// This frame's pose before easing (origin, angles).
pub fn target(f: &PoseFacts, scoped_aiming: bool, i: &PoseInput) -> ([f32; 3], [f32; 3]) {
    let mut ofs = [0.0; 3];
    let mut rot = [0.0; 3];
    let slowest_pace = if i.prone {
        f.prone_min_speed
    } else if i.ducked {
        f.ducked_min_speed
    } else {
        f.stand_min_speed
    };
    let moving = i.ground_velocity > slowest_pace && i.speed > slowest_pace;
    let reloading = i.gun_phase == state::RELOADING || i.weaponstate_left == state::RELOADING;
    if i.prone || i.ducked {
        stance(f, scoped_aiming, i, &mut ofs, &mut rot);
    }
    if moving && !i.on_ladder && !reloading {
        movement(f, i, slowest_pace, &mut ofs, &mut rot);
    }
    weapon_state(f, i, &mut ofs, &mut rot);
    (ofs, rot)
}

/// The crouched or prone offset (none mid-melee) and, for a gun with sights, its aim pitch.
fn stance(
    f: &PoseFacts,
    scoped_aiming: bool,
    i: &PoseInput,
    shift: &mut [f32; 3],
    turn: &mut [f32; 3],
) {
    let meleeing = matches!(
        i.gun_phase,
        state::MELEE_INIT | state::MELEE_FIRE | state::MELEE_END
    );
    if i.ducked && !meleeing {
        add(shift, f.ducked_ofs, 1.0);
    } else if i.prone && !meleeing {
        add(shift, f.prone_ofs, 1.0);
    }
    if scoped_aiming {
        turn[0] += f.aimed_pitch * i.frac;
    }
}

/// The walk: the stance's move and turn by speed, then the lean into a sideways step.
fn movement(
    f: &PoseFacts,
    i: &PoseInput,
    slowest_pace: f32,
    shift: &mut [f32; 3],
    turn: &mut [f32; 3],
) {
    let (mv, turn_matrix) = if i.prone && !i.diving {
        (f.prone_move, f.prone_rot)
    } else if i.ducked {
        (f.ducked_move, f.ducked_rot)
    } else {
        (f.stand_move, f.stand_rot)
    };
    let pace_frac = ((i.ground_velocity - slowest_pace) / (i.speed - slowest_pace)).min(1.0);
    let angle_weight = 1.0 - i.frac;
    add(shift, mv, pace_frac);
    add(turn, turn_matrix, angle_weight * pace_frac);
    let right = eye_right(i.look_angles);
    let strafe = (i.velocity[0] * right[0] + i.velocity[1] * right[1]) * pace_frac / i.ground_velocity;
    add(shift, f.strafe_move, strafe);
    add(turn, f.strafe_rot, strafe * angle_weight);
}

/// AngleVectors' right vector.
fn eye_right(a: [f32; 3]) -> [f32; 3] {
    let (sp, codepoint) = a[0].to_radians().sin_cos();
    let (sy, cy) = a[1].to_radians().sin_cos();
    let (sr, cr) = a[2].to_radians().sin_cos();
    [-sr * sp * cy + cr * sy, -(sr * sp * sy + cr * cy), -sr * codepoint]
}

/// The sprint, dive and slide offsets by the weapon state, else the climb's. (BO2 first gives a
/// rider with the gun out the ride offset; an MP ride here hides the gun, so that branch is not
/// needed.)
fn weapon_state(f: &PoseFacts, i: &PoseInput, shift: &mut [f32; 3], turn: &mut [f32; 3]) {
    let (k, ofs, turn_matrix) = match i.gun_phase {
        state::SPRINT_RAISE..=state::SPRINT_DROP => (sprint_lerp(f, i), f.sprint_ofs, f.sprint_rot),
        state::DTP_IN..=state::DTP_OUT => (dtp_lerp(f, i), f.dtp_ofs, f.dtp_rot),
        state::SLIDE_IN => {
            let k = if f.slide_in_ms != 0 {
                1.0 - i.gun_timer_ms as f32 / f.slide_in_ms as f32
            } else {
                1.0
            };
            (k, f.slide_ofs, f.slide_rot)
        }
        _ => match i.mantle_frac {
            Some(k) => (k, f.mantle_ofs, f.mantle_rot),
            None => return,
        },
    };
    if k <= 0.0 {
        return;
    }
    add(shift, ofs, k);
    add(turn, turn_matrix, k);
}

/// GetSprintLerp: in from the start of the raise, out over the drop; the fast recovery perk
/// cuts the drop's share.
fn sprint_lerp(f: &PoseFacts, i: &PoseInput) -> f32 {
    let t = i.gun_timer_ms as f32;
    let k = if i.gun_phase == state::SPRINT_DROP && f.sprint_out_ms != 0 {
        t / f.sprint_out_ms as f32
    } else if i.gun_phase == state::SPRINT_RAISE && f.sprint_in_ms != 0 {
        1.0 - t / f.sprint_in_ms as f32
    } else {
        1.0
    };
    if i.fast_sprint_recovery
        && i.gun_phase == state::SPRINT_DROP
        && i.sprint_recovery_multiplier != 0.0
    {
        k * i.sprint_recovery_multiplier
    } else {
        k
    }
}

/// GetDTPLerp: in over the dive's start, out over its end.
fn dtp_lerp(f: &PoseFacts, i: &PoseInput) -> f32 {
    let t = i.gun_timer_ms as f32;
    if i.gun_phase == state::DTP_OUT && f.dtp_out_ms != 0 {
        t / f.dtp_out_ms as f32
    } else if i.gun_phase == state::DTP_IN && f.dtp_in_ms != 0 {
        1.0 - t / f.dtp_in_ms as f32
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> PoseFacts {
        PoseFacts {
            stand_move: [1.0, 0.0, 0.0],
            stand_rot: [0.0, 2.0, 0.0],
            strafe_move: [0.0, 1.0, 0.0],
            strafe_rot: [0.0, 0.0, 4.0],
            ducked_move: [2.0, 0.0, 0.0],
            prone_move: [3.0, 0.0, 0.0],
            ducked_ofs: [0.0, 0.0, -1.0],
            prone_ofs: [0.0, 0.0, -2.0],
            aimed_pitch: 5.0,
            stand_min_speed: 10.0,
            ducked_min_speed: 10.0,
            prone_min_speed: 10.0,
            pos_move_rate: 10.0,
            pos_prone_move_rate: 5.0,
            pos_rot_rate: 10.0,
            pos_prone_rot_rate: 5.0,
            sprint_ofs: [-1.0, 0.0, -3.0],
            sprint_rot: [10.0, 20.0, 0.0],
            dtp_ofs: [0.0, 0.0, -4.0],
            slide_ofs: [0.0, 2.0, 0.0],
            mantle_ofs: [0.0, -5.0, 0.0],
            sprint_in_ms: 200,
            sprint_out_ms: 400,
            dtp_in_ms: 100,
            dtp_out_ms: 300,
            slide_in_ms: 250,
            ..PoseFacts::default()
        }
    }

    fn still() -> PoseInput {
        PoseInput {
            speed: 190.0,
            sprint_recovery_multiplier: 0.3,
            step_millis_elapsed: 0.05,
            ..PoseInput::default()
        }
    }

    #[test]
    fn the_sprint_offset_eases_in_over_the_raise_and_is_full_in_the_loop() {
        let f = facts();
        let half = PoseInput {
            gun_phase: state::SPRINT_RAISE,
            gun_timer_ms: 100,
            ..still()
        };
        assert_eq!(target(&f, true, &half).0, [-0.5, 0.0, -1.5]);
        let full = PoseInput {
            gun_phase: state::SPRINT_LOOP,
            gun_timer_ms: Default::default(),
            ..still()
        };
        assert_eq!(target(&f, true, &full).1, [10.0, 20.0, 0.0]);
    }

    #[test]
    fn the_sprint_drop_runs_out_with_the_timer_and_the_recovery_perk_cuts_it() {
        let f = facts();
        let drop = PoseInput {
            gun_phase: state::SPRINT_DROP,
            gun_timer_ms: 100,
            ..still()
        };
        assert_eq!(target(&f, true, &drop).0, [-0.25, 0.0, -0.75]);
        let perk = PoseInput {
            fast_sprint_recovery: true,
            ..drop
        };
        let o = target(&f, true, &perk).0;
        assert!((o[2] + 0.225).abs() < 1e-6, "{o:?}");
    }

    #[test]
    fn a_dive_and_a_slide_carry_their_own_offsets() {
        let f = facts();
        let dive = PoseInput {
            gun_phase: state::DTP_IN,
            gun_timer_ms: 50,
            ..still()
        };
        assert_eq!(target(&f, true, &dive).0, [0.0, 0.0, -2.0]);
        let dive_out = PoseInput {
            gun_phase: state::DTP_OUT,
            gun_timer_ms: 150,
            ..still()
        };
        assert_eq!(target(&f, true, &dive_out).0, [0.0, 0.0, -2.0]);
        let slide = PoseInput {
            gun_phase: state::SLIDE_IN,
            gun_timer_ms: 125,
            ..still()
        };
        assert_eq!(target(&f, true, &slide).0, [0.0, 1.0, 0.0]);
    }

    #[test]
    fn the_climb_offset_follows_the_climb_only_while_the_gun_stays_up() {
        let f = facts();
        let up = PoseInput {
            mantle_frac: Some(0.4),
            ..still()
        };
        assert_eq!(target(&f, true, &up).0, [0.0, -2.0, 0.0]);
        assert_eq!(target(&f, true, &still()).0, [0.0; 3]);
    }

    #[test]
    fn crouched_and_prone_offsets_skip_a_melee_and_the_aim_pitch_is_only_low() {
        let f = facts();
        let crouch = PoseInput {
            ducked: true,
            frac: 1.0,
            ..still()
        };
        assert_eq!(
            target(&f, true, &crouch),
            ([0.0, 0.0, -1.0], [5.0, 0.0, 0.0])
        );
        let melee = PoseInput {
            gun_phase: state::MELEE_FIRE,
            ..crouch
        };
        assert_eq!(target(&f, true, &melee), ([0.0; 3], [5.0, 0.0, 0.0]));
        let prone = PoseInput {
            prone: true,
            ..still()
        };
        assert_eq!(target(&f, false, &prone).0, [0.0, 0.0, -2.0]);
        let standing = PoseInput {
            frac: 1.0,
            ..still()
        };
        assert_eq!(target(&f, true, &standing).1, [0.0; 3]);
    }

    #[test]
    fn walking_moves_the_gun_by_speed_and_a_reload_or_ladder_stops_it() {
        let f = facts();
        // Straight ahead at yaw 0 (right is -y): half speed, no sideways lean.
        let walk = PoseInput {
            ground_velocity: 100.0,
            velocity: [100.0, 0.0, 0.0],
            ..still()
        };
        let (o, a) = target(&f, true, &walk);
        assert!((o[0] - 0.5).abs() < 1e-6 && o[1].abs() < 1e-6, "{o:?}");
        assert!((a[1] - 1.0).abs() < 1e-6, "{a:?}");
        // Stepping right at full speed leans the gun all the way.
        let right = PoseInput {
            ground_velocity: 190.0,
            velocity: [0f32, -190.0, 0.0],
            ..still()
        };
        let (o, a) = target(&f, true, &right);
        assert!(
            (o[1] - 1.0).abs() < 1e-6 && (a[2] - 4.0).abs() < 1e-6,
            "{o:?} {a:?}"
        );
        let reload = PoseInput {
            weaponstate_left: state::RELOADING,
            ..walk
        };
        assert_eq!(target(&f, true, &reload).0, [0.0; 3]);
        let ladder = PoseInput {
            on_ladder: true,
            ..walk
        };
        assert_eq!(target(&f, true, &ladder).0, [0.0; 3]);
        // Aimed in, the walk still moves the gun but turns it no more.
        let aimed = PoseInput { frac: 1.0, ..walk };
        assert_eq!(target(&f, true, &aimed).1, [0.0; 3]);
    }

    #[test]
    fn a_prone_diver_walks_the_gun_like_a_standing_player() {
        let f = facts();
        let crawl = PoseInput {
            prone: true,
            ground_velocity: 190.0,
            velocity: [190.0, 0.0, 0.0],
            ..still()
        };
        assert!((target(&f, true, &crawl).0[0] - 3.0).abs() < 1e-6);
        let dive = PoseInput {
            diving: true,
            ..crawl
        };
        assert!((target(&f, true, &dive).0[0] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn the_pose_eases_by_the_guns_rates_and_prone_has_its_own() {
        let f = facts();
        let loop_ = PoseInput {
            gun_phase: state::SPRINT_LOOP,
            ..still()
        };
        let mut p = BasePose::default();
        p.step(&f, true, &loop_);
        // 10 per second for 0.05 s: half way.
        assert_eq!(p.origin, [-0.5, 0.0, -1.5]);
        let mut p = BasePose::default();
        p.step(
            &f,
            true,
            &PoseInput {
                prone: true,
                ..loop_
            },
        );
        // Prone: the prone offset plus the sprint's, at 5 per second (a quarter).
        assert_eq!(p.origin, [-0.25, 0.0, (-2.0 - 3.0) * 0.25]);
    }
}
