//! bo2mp: Black Ops II's own gun pose (BG_CalculateWeaponMovement_Base): where the gun sits for
//! the stance and the walk, plus the sprint, dive, slide and climb offsets, eased by the gun's
//! own rates. It replaces the MW2 stance pose for every BO2 gun; `recoil::weapon_movement_angles`
//! then scales what it leaves by the aim.

use crate::pm::spread::{rd_f32, rd_i32};
use crate::state;
use fastfile_t6::layout::WeaponDef as D;

/// The pose numbers from the weapon file (the stand and strafe turn include the attachments'
/// overrides, folded in by `SpreadFacts::apply_attachments`).
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
    pub ads_aim_pitch: f32,
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
            stand_move: vec3(def, D::vStandMove),
            stand_rot: vec3(def, D::vStandRot),
            strafe_move: vec3(def, D::vStrafeMove),
            strafe_rot: vec3(def, D::vStrafeRot),
            ducked_move: vec3(def, D::vDuckedMove),
            ducked_rot: vec3(def, D::vDuckedRot),
            prone_move: vec3(def, D::vProneMove),
            prone_rot: vec3(def, D::vProneRot),
            ducked_ofs: vec3(def, D::vDuckedOfs),
            prone_ofs: vec3(def, D::vProneOfs),
            ads_aim_pitch: rd_f32(def, D::fAdsAimPitch),
            stand_min_speed: rd_f32(def, D::fStandMoveMinSpeed),
            ducked_min_speed: rd_f32(def, D::fDuckedMoveMinSpeed),
            prone_min_speed: rd_f32(def, D::fProneMoveMinSpeed),
            pos_move_rate: rd_f32(def, D::fPosMoveRate),
            pos_prone_move_rate: rd_f32(def, D::fPosProneMoveRate),
            pos_rot_rate: rd_f32(def, D::fPosRotRate),
            pos_prone_rot_rate: rd_f32(def, D::fPosProneRotRate),
            sprint_ofs: vec3(def, D::vSprintOfs),
            sprint_rot: vec3(def, D::vSprintRot),
            dtp_ofs: vec3(def, D::vDtpOfs),
            dtp_rot: vec3(def, D::vDtpRot),
            slide_ofs: vec3(def, D::vSlideOfs),
            slide_rot: vec3(def, D::vSlideRot),
            mantle_ofs: vec3(def, D::vMantleOfs),
            mantle_rot: vec3(def, D::vMantleRot),
            sprint_in_ms: rd_i32(def, D::sprintInTime),
            sprint_out_ms: rd_i32(def, D::sprintOutTime),
            dtp_in_ms: rd_i32(def, D::dtpInTime),
            dtp_out_ms: rd_i32(def, D::dtpOutTime),
            slide_in_ms: rd_i32(def, D::slideInTime),
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
    pub weaponstate: i32,
    pub weaponstate_left: i32,
    pub weapon_time: i32,
    /// How far the aim is in (`fWeaponPosFrac`).
    pub frac: f32,
    pub xyspeed: f32,
    /// The player's top speed (`ps.speed`).
    pub speed: f32,
    pub velocity: [f32; 3],
    pub viewangles: [f32; 3],
    /// Seconds this frame.
    pub frametime: f32,
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
    pub fn step(&mut self, f: &PoseFacts, aim_down_sight: bool, i: &PoseInput) {
        let (origin, angles) = target(f, aim_down_sight, i);
        let (move_rate, rot_rate) = if i.prone {
            (f.pos_prone_move_rate, f.pos_prone_rot_rate)
        } else {
            (f.pos_move_rate, f.pos_rot_rate)
        };
        let o = (move_rate * i.frametime).min(1.0);
        let a = (rot_rate * i.frametime).min(1.0);
        for k in 0..3 {
            self.origin[k] += (origin[k] - self.origin[k]) * o;
            self.angles[k] += (angles[k] - self.angles[k]) * a;
        }
    }
}

fn add(v: &mut [f32; 3], d: [f32; 3], k: f32) {
    for (a, b) in v.iter_mut().zip(d) {
        *a += b * k;
    }
}

/// This frame's pose before easing (origin, angles).
pub fn target(f: &PoseFacts, aim_down_sight: bool, i: &PoseInput) -> ([f32; 3], [f32; 3]) {
    let mut origin = [0.0; 3];
    let mut angles = [0.0; 3];
    let min_speed = if i.prone {
        f.prone_min_speed
    } else if i.ducked {
        f.ducked_min_speed
    } else {
        f.stand_min_speed
    };
    let moving = i.xyspeed > min_speed && i.speed > min_speed;
    let reloading = i.weaponstate == state::RELOADING || i.weaponstate_left == state::RELOADING;
    if i.prone || i.ducked {
        stance(f, aim_down_sight, i, &mut origin, &mut angles);
    }
    if moving && !i.on_ladder && !reloading {
        movement(f, i, min_speed, &mut origin, &mut angles);
    }
    weapon_state(f, i, &mut origin, &mut angles);
    (origin, angles)
}

/// The crouched or prone offset (none mid-melee) and, for a gun with sights, its aim pitch.
fn stance(
    f: &PoseFacts,
    aim_down_sight: bool,
    i: &PoseInput,
    origin: &mut [f32; 3],
    angles: &mut [f32; 3],
) {
    let meleeing = matches!(
        i.weaponstate,
        state::MELEE_INIT | state::MELEE_FIRE | state::MELEE_END
    );
    if i.ducked && !meleeing {
        add(origin, f.ducked_ofs, 1.0);
    } else if i.prone && !meleeing {
        add(origin, f.prone_ofs, 1.0);
    }
    if aim_down_sight {
        angles[0] += f.ads_aim_pitch * i.frac;
    }
}

/// The walk: the stance's move and turn by speed, then the lean into a sideways step.
fn movement(
    f: &PoseFacts,
    i: &PoseInput,
    min_speed: f32,
    origin: &mut [f32; 3],
    angles: &mut [f32; 3],
) {
    let (mv, rot) = if i.prone && !i.diving {
        (f.prone_move, f.prone_rot)
    } else if i.ducked {
        (f.ducked_move, f.ducked_rot)
    } else {
        (f.stand_move, f.stand_rot)
    };
    let speed_frac = ((i.xyspeed - min_speed) / (i.speed - min_speed)).min(1.0);
    let angle_weight = 1.0 - i.frac;
    add(origin, mv, speed_frac);
    add(angles, rot, angle_weight * speed_frac);
    let right = view_right(i.viewangles);
    let strafe = (i.velocity[0] * right[0] + i.velocity[1] * right[1]) * speed_frac / i.xyspeed;
    add(origin, f.strafe_move, strafe);
    add(angles, f.strafe_rot, strafe * angle_weight);
}

/// AngleVectors' right vector.
fn view_right(a: [f32; 3]) -> [f32; 3] {
    let (sp, cp) = a[0].to_radians().sin_cos();
    let (sy, cy) = a[1].to_radians().sin_cos();
    let (sr, cr) = a[2].to_radians().sin_cos();
    [-sr * sp * cy + cr * sy, -sr * sp * sy - cr * cy, -sr * cp]
}

/// The sprint, dive and slide offsets by the weapon state, else the climb's. (BO2 first gives a
/// rider with the gun out the ride offset; an MP ride here hides the gun, so that branch is not
/// needed.)
fn weapon_state(f: &PoseFacts, i: &PoseInput, origin: &mut [f32; 3], angles: &mut [f32; 3]) {
    let (k, ofs, rot) = match i.weaponstate {
        state::SPRINT_RAISE..=state::SPRINT_DROP => (sprint_lerp(f, i), f.sprint_ofs, f.sprint_rot),
        state::DTP_IN..=state::DTP_OUT => (dtp_lerp(f, i), f.dtp_ofs, f.dtp_rot),
        state::SLIDE_IN => {
            let k = if f.slide_in_ms != 0 {
                1.0 - i.weapon_time as f32 / f.slide_in_ms as f32
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
    add(origin, ofs, k);
    add(angles, rot, k);
}

/// GetSprintLerp: in from the start of the raise, out over the drop; the fast recovery perk
/// cuts the drop's share.
fn sprint_lerp(f: &PoseFacts, i: &PoseInput) -> f32 {
    let t = i.weapon_time as f32;
    let k = if i.weaponstate == state::SPRINT_DROP && f.sprint_out_ms != 0 {
        t / f.sprint_out_ms as f32
    } else if i.weaponstate == state::SPRINT_RAISE && f.sprint_in_ms != 0 {
        1.0 - t / f.sprint_in_ms as f32
    } else {
        1.0
    };
    if i.fast_sprint_recovery
        && i.weaponstate == state::SPRINT_DROP
        && i.sprint_recovery_multiplier != 0.0
    {
        k * i.sprint_recovery_multiplier
    } else {
        k
    }
}

/// GetDTPLerp: in over the dive's start, out over its end.
fn dtp_lerp(f: &PoseFacts, i: &PoseInput) -> f32 {
    let t = i.weapon_time as f32;
    if i.weaponstate == state::DTP_OUT && f.dtp_out_ms != 0 {
        t / f.dtp_out_ms as f32
    } else if i.weaponstate == state::DTP_IN && f.dtp_in_ms != 0 {
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
            ads_aim_pitch: 5.0,
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
            frametime: 0.05,
            ..PoseInput::default()
        }
    }

    #[test]
    fn the_sprint_offset_eases_in_over_the_raise_and_is_full_in_the_loop() {
        let f = facts();
        let half = PoseInput {
            weaponstate: state::SPRINT_RAISE,
            weapon_time: 100,
            ..still()
        };
        assert_eq!(target(&f, true, &half).0, [-0.5, 0.0, -1.5]);
        let full = PoseInput {
            weaponstate: state::SPRINT_LOOP,
            weapon_time: 0,
            ..still()
        };
        assert_eq!(target(&f, true, &full).1, [10.0, 20.0, 0.0]);
    }

    #[test]
    fn the_sprint_drop_runs_out_with_the_timer_and_the_recovery_perk_cuts_it() {
        let f = facts();
        let drop = PoseInput {
            weaponstate: state::SPRINT_DROP,
            weapon_time: 100,
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
            weaponstate: state::DTP_IN,
            weapon_time: 50,
            ..still()
        };
        assert_eq!(target(&f, true, &dive).0, [0.0, 0.0, -2.0]);
        let dive_out = PoseInput {
            weaponstate: state::DTP_OUT,
            weapon_time: 150,
            ..still()
        };
        assert_eq!(target(&f, true, &dive_out).0, [0.0, 0.0, -2.0]);
        let slide = PoseInput {
            weaponstate: state::SLIDE_IN,
            weapon_time: 125,
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
            weaponstate: state::MELEE_FIRE,
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
            xyspeed: 100.0,
            velocity: [100.0, 0.0, 0.0],
            ..still()
        };
        let (o, a) = target(&f, true, &walk);
        assert!((o[0] - 0.5).abs() < 1e-6 && o[1].abs() < 1e-6, "{o:?}");
        assert!((a[1] - 1.0).abs() < 1e-6, "{a:?}");
        // Stepping right at full speed leans the gun all the way.
        let right = PoseInput {
            xyspeed: 190.0,
            velocity: [0.0, -190.0, 0.0],
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
            xyspeed: 190.0,
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
            weaponstate: state::SPRINT_LOOP,
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
