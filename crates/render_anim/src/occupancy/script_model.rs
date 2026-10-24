//! Script models: what poses and draws them each frame, and the vehicle
//! parts (wheels, rotors, turret guns) that move by themselves.

use bevy::prelude::*;

mod anim_clock;
mod focus;
mod lod;
mod mover;
mod plan;
mod pose;
mod pose_dobj;
mod presented;
mod product;
mod scene_ents;
mod schedule;
mod skels;
mod spawn;
mod state;

pub use focus::RenderFocus;
pub use lod::DObjLodView;
pub use pose_dobj::{pose_script_dobj_with_controller, pose_script_dobj_with_materials};
pub use schedule::{ScriptModelDrawSet, ScriptModelSkinSet, register_script_model_systems};
pub use skels::collect_presented_models;
pub(crate) use anim_clock::{advance_script_anim, anim_ahead_secs};
pub(crate) use mover::script_mover_pose_now;
pub(crate) use presented::presented_dobj_states;

/// bo2mp: a ground vehicle's wheels (the RC-XD), as BO2 poses them
/// (CG_Vehicle_PreControllers + DoControllersGround): the spin
/// (`steerPitch += frametime * wheel_rot_rate * forward speed`, degrees) on
/// every wheel, the steer (`steerYaw = steer fraction * 45`) on the front two.
/// BO2 networks the steer; here it is read back off the car's own turn
/// (a car turning at yaw rate w at speed v steers atan(w * wheelbase / v)).
pub(crate) struct VehicleWheels {
    last: Option<(std::time::Instant, Vec3, f32)>,
    spin_deg: f32,
    steer_deg: f32,
    /// Its body on its springs over the wheels (BO2's nitrous car rides
    /// its physics body on a spring per wheel): how far it has sunk or
    /// risen (units), dipped its nose and leant (degrees), and how fast.
    vel: Vec3,
    acc: Vec3,
    body: [f32; 3],
    body_v: [f32; 3],
}

/// Its springs: about 3 bounces a second, settling in about two.
const SPRING_K: f32 = 355.0;
const SPRING_C: f32 = 13.0;
/// How far its body gives for a push (units or degrees per unit/s^2),
/// and how far it can go before it bottoms out.
const SPRING_GIVE: f32 = 0.004;
const SINK_MAX: f32 = 2.5;
const RISE_MAX: f32 = 1.5;
const LEAN_MAX: f32 = 5.0;
const BODY_TAG: &str = "tag_body";
const WHEEL_TAGS: [&str; 4] = [
    "tag_wheel_front_left",
    "tag_wheel_front_right",
    "tag_wheel_back_left",
    "tag_wheel_back_right",
];

/// rc_car_medium_mp: wheel_rot_rate 4, steer_angle_max 0.7 rad; the
/// wheelbase from veh_t6_drone_rcxd's wheel tags (front x 12.45, back -15).
const WHEEL_ROT_RATE: f32 = 4.0;
const STEER_ANGLE_MAX: f32 = 0.7;
const WHEELBASE: f32 = 27.45;
const FRONT_WHEELS: [&str; 2] = ["tag_wheel_front_left", "tag_wheel_front_right"];
const SPIN_BONES: [&str; 4] = [
    "left_wheel_01_jnt",
    "right_wheel_01_jnt",
    "left_wheel_02_jnt",
    "right_wheel_02_jnt",
];

impl VehicleWheels {
    pub(crate) fn new() -> Self {
        Self {
            last: None,
            spin_deg: 0.0,
            steer_deg: 0.0,
            vel: Vec3::ZERO,
            acc: Vec3::ZERO,
            body: [0.0; 3],
            body_v: [0.0; 3],
        }
    }

    /// Its body's springs, pushed by how hard the car speeds up, brakes,
    /// turns, lands or bumps: it squats when it pulls away, dips its nose
    /// when it stops, leans out of a turn, sinks on a landing and hangs
    /// on its wheels in the air.
    fn bounce(&mut self, dt: f32, moved: Vec3, rotation: Quat) {
        let vel = moved / dt;
        let was = self.vel;
        self.vel += (vel - self.vel) * (dt / 0.08).min(1.0);
        let a = (self.vel - was) / dt;
        let local = Vec3::new(
            a.dot(rotation * Vec3::X),
            a.dot(rotation * Vec3::Y),
            a.dot(rotation * Vec3::Z),
        );
        self.acc += (local - self.acc) * (dt / 0.06).min(1.0);
        let target = [
            (-self.acc.z * SPRING_GIVE).clamp(-SINK_MAX, RISE_MAX),
            (-self.acc.x * SPRING_GIVE).clamp(-LEAN_MAX, LEAN_MAX),
            (self.acc.y * SPRING_GIVE).clamp(-LEAN_MAX, LEAN_MAX),
        ];
        let limits = [(-SINK_MAX, RISE_MAX), (-LEAN_MAX, LEAN_MAX), (-LEAN_MAX, LEAN_MAX)];
        let steps = (dt / 0.01).ceil().max(1.0);
        let h = dt / steps;
        for _ in 0..steps as u32 {
            for i in 0..3 {
                let pull = -SPRING_K * (self.body[i] - target[i]) - SPRING_C * self.body_v[i];
                self.body_v[i] += pull * h;
                self.body[i] += self.body_v[i] * h;
                let (lo, hi) = limits[i];
                if self.body[i] < lo || self.body[i] > hi {
                    // Its bump stop.
                    self.body[i] = self.body[i].clamp(lo, hi);
                    self.body_v[i] = 0.0;
                }
            }
        }
    }

    /// `steer`: the steer its server sends (degrees), else read from how
    /// fast it turns.
    pub(crate) fn advance(&mut self, at: Vec3, rotation: Quat, steer: Option<f32>) {
        let now = std::time::Instant::now();
        let fwd = rotation * Vec3::X;
        let yaw = fwd.y.atan2(fwd.x);
        if let Some((then, was, was_yaw)) = self.last {
            let dt = now.duration_since(then).as_secs_f32();
            if dt > 0.0 && dt < 0.5 {
                self.bounce(dt, at - was, rotation);
                let flat = Vec3::new(fwd.x, fwd.y, 0.0).normalize_or_zero();
                let speed = (at - was).dot(flat) / dt;
                self.spin_deg = (self.spin_deg + dt * WHEEL_ROT_RATE * speed).rem_euclid(360.0);
                let mut turn = yaw - was_yaw;
                if turn > std::f32::consts::PI {
                    turn -= std::f32::consts::TAU;
                } else if turn < -std::f32::consts::PI {
                    turn += std::f32::consts::TAU;
                }
                let target = if let Some(sent) = steer {
                    sent
                } else if speed.abs() > 10.0 {
                    let steer = ((turn / dt) * WHEELBASE / speed)
                        .atan()
                        .clamp(-STEER_ANGLE_MAX, STEER_ANGLE_MAX);
                    steer / STEER_ANGLE_MAX * 45.0
                } else {
                    self.steer_deg
                };
                // BO2 eases the networked steer in (steer_speed 5).
                self.steer_deg += (target - self.steer_deg) * (dt * 10.0).min(1.0);
            }
        }
        self.last = Some((now, at, yaw));
    }

    /// A pose key: one step per degree of spin and of steer, per quarter
    /// unit of sink and per half degree of lean.
    pub(crate) fn key(&self) -> u32 {
        let spin = self.spin_deg.round() as u32 % 360;
        let steer = (self.steer_deg.round() as i32 + 64).clamp(0, 127) as u32;
        let step = |v: f32, per: f32| ((v / per).round() as i32 + 15).clamp(0, 31) as u32;
        let sink = step(self.body[0], 0.25);
        let pitch = step(self.body[1], 0.5);
        let roll = step(self.body[2], 0.5);
        0x8000_0000 | (roll << 26) | (pitch << 21) | (sink << 16) | (steer << 9) | spin
    }

    /// Its body moved on its springs while its wheels stay on the ground:
    /// the body tag takes the sink and lean, and each wheel tag the undo
    /// of it, so the gap between body and wheels opens and closes.
    fn apply_springs(&self, dobj: &xmodel_runtime::DObj, locals: &mut [dobj_t6::PartPose]) {
        let [sink, pitch, roll] = self.body;
        if sink.abs() < 1e-3 && pitch.abs() < 1e-3 && roll.abs() < 1e-3 {
            return;
        }
        let Some(b) = dobj.find(BODY_TAG) else {
            return;
        };
        let lean = Quat::from_rotation_y(pitch.to_radians()) * Quat::from_rotation_x(roll.to_radians());
        let r = Quat::from_array(locals[b].rotation);
        locals[b].rotation = (r * lean).normalize().to_array();
        locals[b].translation[2] += sink;
        let body_bind = dobj.bones[b].bind_world;
        let moved = Mat4::from_rotation_translation(lean, Vec3::new(0.0, 0.0, sink));
        let undo = body_bind * moved.inverse() * body_bind.inverse();
        for name in WHEEL_TAGS {
            let Some(w) = dobj.find(name) else {
                continue;
            };
            let Some(parent) = dobj.bones[w].parent else {
                continue;
            };
            let parent_bind = dobj.bones[parent].bind_world;
            let back = parent_bind.inverse() * undo * parent_bind;
            let bind_t = dobj.bones[w].bind_translation;
            let t = Vec3::from_array(locals[w].translation);
            let local = Mat4::from_rotation_translation(
                Quat::from_array(locals[w].rotation),
                Vec3::new(bind_t.x, bind_t.y, bind_t.z) + t,
            );
            let (_, rot, trans) = (back * local).to_scale_rotation_translation();
            locals[w].rotation = rot.normalize().to_array();
            locals[w].translation = (trans - Vec3::new(bind_t.x, bind_t.y, bind_t.z)).to_array();
        }
    }

    pub(crate) fn apply(&self, dobj: &xmodel_runtime::DObj, locals: &mut [dobj_t6::PartPose]) {
        self.apply_springs(dobj, locals);
        for name in FRONT_WHEELS {
            xmodel_runtime::yaw_bone(dobj, locals, name, self.steer_deg.to_radians());
        }
        for name in SPIN_BONES {
            spin_bone(dobj, locals, name, self.spin_deg.to_radians());
        }
    }
}

/// A wheel's roll about the car's side axis: the top of the wheel goes
/// forward as the angle grows.
fn spin_bone(dobj: &xmodel_runtime::DObj, locals: &mut [dobj_t6::PartPose], name: &str, angle: f32) {
    let Some(i) = dobj.find(name) else {
        return;
    };
    let (_, bind_rot, _) = dobj.bones[i].bind_world.to_scale_rotation_translation();
    let axis = (bind_rot.inverse() * Vec3::Y).normalize_or_zero();
    if axis == Vec3::ZERO {
        return;
    }
    let q = Quat::from_axis_angle(axis, angle);
    let r = Quat::from_array(locals[i].rotation);
    let out = (r * q).normalize();
    locals[i].rotation = out.to_array();
}

pub(crate) fn has_wheels(dobj: &xmodel_runtime::DObj) -> bool {
    FRONT_WHEELS.iter().all(|n| dobj.find(n).is_some())
        && SPIN_BONES.iter().any(|n| dobj.find(n).is_some())
}

/// bo2mp: a helicopter's rotors, as BO2 turns them
/// (CG_Vehicle_PreControllersAir + DoControllersAir): the main rotor about
/// the body's up axis, the tail rotor about its own shaft, both at
/// rotation_max_speed (4000 degrees a second) from the moment it appears;
/// above 0.55 of that the still blades are hidden and the blur disc shows.
pub(crate) struct HeliRotors {
    last: Option<std::time::Instant>,
    main_deg: f32,
    tail_deg: f32,
}

const ROTOR_MAX_SPEED: f32 = 4000.0;
const MAIN_ROTOR: &str = "tag_main_rotor";
const TAIL_ROTOR: &str = "tag_tail_rotor";
const ROTOR_STATIC: [&str; 2] = ["tag_main_rotor_static", "tag_tail_rotor_static"];

impl HeliRotors {
    pub(crate) fn new() -> Self {
        Self {
            last: None,
            main_deg: 0.0,
            tail_deg: 0.0,
        }
    }

    pub(crate) fn advance(&mut self) {
        let now = std::time::Instant::now();
        if let Some(then) = self.last {
            let dt = now.duration_since(then).as_secs_f32().min(0.5);
            self.main_deg = (self.main_deg + dt * ROTOR_MAX_SPEED).rem_euclid(360.0);
            self.tail_deg = (self.tail_deg + dt * ROTOR_MAX_SPEED).rem_euclid(360.0);
        }
        self.last = Some(now);
    }

    /// A pose key: one step per degree of each rotor.
    pub(crate) fn key(&self) -> u32 {
        0x4000_0000 | ((self.tail_deg as u32 % 360) << 9) | (self.main_deg as u32 % 360)
    }

    /// The still blades go while it spins at full speed.
    pub(crate) fn hide(&self, dobj: &xmodel_runtime::DObj, hide: &mut xmodel_runtime::HidePartBits) {
        for name in ROTOR_STATIC {
            if let Some(i) = dobj.find(name) {
                hide.set(i);
            }
        }
    }

    pub(crate) fn apply(&self, dobj: &xmodel_runtime::DObj, locals: &mut [dobj_t6::PartPose]) {
        // Main: BO2 sets the tag to (pitch -90, yaw angle): its rest pose
        // turned about the body's up axis.
        if let Some(i) = dobj.find(MAIN_ROTOR) {
            let r = Quat::from_array(locals[i].rotation);
            let out = (Quat::from_rotation_z(self.main_deg.to_radians()) * r).normalize();
            locals[i].rotation = out.to_array();
        }
        // Tail: (0, yaw 90, roll angle): turned about its own shaft.
        if let Some(i) = dobj.find(TAIL_ROTOR) {
            let r = Quat::from_array(locals[i].rotation);
            let out = (r * Quat::from_rotation_x(self.tail_deg.to_radians())).normalize();
            locals[i].rotation = out.to_array();
        }
    }
}

pub(crate) fn has_rotors(dobj: &xmodel_runtime::DObj) -> bool {
    dobj.find(MAIN_ROTOR).is_some()
}

/// bo2mp: a turret's gun, as BO2 poses it from its networked gun angles:
/// its gun turns on `tag_aim` and tilts on `tag_aim_pitch` (on `tag_aim`
/// itself when it has none) while its legs stay on the facing it was set
/// down with. The server turns the whole turret to its gun's yaw and sends
/// its gun's pitch and that facing, so everything else on its root is
/// turned back to the facing; its tilt follows the sent pitch smoothly.
pub(crate) struct TurretGun {
    last: Option<std::time::Instant>,
    pitch: Option<f32>,
    /// How far its gun is turned off its legs' facing (degrees, left as
    /// plus).
    turn: f32,
}

const AIM_TAG: &str = "tag_aim";
const AIM_ANIMATED_TAG: &str = "tag_aim_animated";
const AIM_PITCH_TAG: &str = "tag_aim_pitch";
/// How fast its drawn tilt catches up with the sent one (per second).
const TURRET_TILT_EASE: f32 = 20.0;

impl TurretGun {
    pub(crate) fn new() -> Self {
        Self {
            last: None,
            pitch: None,
            turn: 0.0,
        }
    }

    /// `rotation`: the turret as drawn; `aim`: its gun's pitch (down as
    /// plus) and its legs' facing (yaw), as sent.
    pub(crate) fn advance(&mut self, rotation: Quat, aim: [f32; 2]) {
        let now = std::time::Instant::now();
        let dt = self
            .last
            .map_or(0.0, |then| now.duration_since(then).as_secs_f32().min(0.5));
        self.last = Some(now);
        let fwd = rotation * Vec3::X;
        let yaw = fwd.y.atan2(fwd.x).to_degrees();
        self.turn = (yaw - aim[1] + 180.0).rem_euclid(360.0) - 180.0;
        self.pitch = Some(match self.pitch {
            Some(p) => p + (aim[0] - p) * (dt * TURRET_TILT_EASE).min(1.0),
            None => aim[0],
        });
    }

    /// A pose key: one step per half degree of its turn and its tilt.
    pub(crate) fn key(&self) -> u32 {
        let half = |d: f32| ((d * 2.0).round() as i32 + 360).clamp(0, 720) as u32;
        0x2000_0000 | (half(self.turn) << 10) | half(self.pitch.unwrap_or(0.0))
    }

    pub(crate) fn apply(&self, dobj: &xmodel_runtime::DObj, locals: &mut [dobj_t6::PartPose]) {
        let Some(aim) = dobj.find(AIM_TAG) else {
            return;
        };
        let tilt = Quat::from_rotation_y(self.pitch.unwrap_or(0.0).to_radians());
        let tilted: &[&str] = if dobj.find(AIM_PITCH_TAG).is_some() {
            &[AIM_PITCH_TAG]
        } else {
            &[AIM_TAG, AIM_ANIMATED_TAG]
        };
        for name in tilted {
            if let Some(i) = dobj.find(name) {
                let r = Quat::from_array(locals[i].rotation);
                locals[i].rotation = (r * tilt).normalize().to_array();
            }
        }
        // Its legs: everything else on its root, turned back about the
        // root's up axis.
        let back = Quat::from_rotation_z(-self.turn.to_radians());
        let model = dobj.bones[aim].model;
        for (i, bone) in dobj.bones.iter().enumerate() {
            let on_root = bone.parent.is_some_and(|p| dobj.bones[p].parent.is_none());
            if !on_root || bone.model != model || bone.name == AIM_TAG || bone.name == AIM_ANIMATED_TAG
            {
                continue;
            }
            let scale = if bone.no_scale {
                1.0
            } else {
                dobj.models[bone.model].scale
            };
            if scale == 0.0 {
                continue;
            }
            let bind = Vec3::new(
                bone.bind_translation.x,
                bone.bind_translation.y,
                bone.bind_translation.z,
            );
            let at = bind + Vec3::from_array(locals[i].translation) * scale;
            locals[i].translation = ((back * at - bind) / scale).to_array();
            let r = Quat::from_array(locals[i].rotation);
            locals[i].rotation = (back * r).normalize().to_array();
        }
    }
}

pub(crate) fn has_turret_gun(dobj: &xmodel_runtime::DObj) -> bool {
    dobj.find(AIM_TAG).is_some()
}


#[cfg(test)]
mod turret_gun_tests {
    use super::*;

    /// Its legs turn back by however far the drawn turret is turned off
    /// their facing; its tilt starts where it was sent.
    #[test]
    fn turn_is_the_drawn_yaw_off_its_facing() {
        let mut gun = TurretGun::new();
        gun.advance(Quat::from_rotation_z(30f32.to_radians()), [12.0, 10.0]);
        assert!((gun.turn - 20.0).abs() < 1e-3, "{}", gun.turn);
        assert_eq!(gun.pitch, Some(12.0));
        // Across the wrap: drawn at -170, set down facing 170.
        gun.advance(Quat::from_rotation_z((-170f32).to_radians()), [12.0, 170.0]);
        assert!((gun.turn - 20.0).abs() < 1e-3, "{}", gun.turn);
    }

    /// Its pose key changes with its turn and its tilt, and stays apart from
    /// the rotors' and wheels' keys.
    #[test]
    fn key_follows_turn_and_tilt() {
        let mut gun = TurretGun::new();
        gun.advance(Quat::IDENTITY, [0.0, 0.0]);
        let level = gun.key();
        assert_eq!(level & 0xe000_0000, 0x2000_0000);
        let mut tilted = TurretGun::new();
        tilted.advance(Quat::IDENTITY, [10.0, 0.0]);
        assert_ne!(tilted.key(), level);
        let mut turned = TurretGun::new();
        turned.advance(Quat::IDENTITY, [0.0, -45.0]);
        assert_ne!(turned.key(), level);
        assert_ne!(turned.key(), tilted.key());
    }
}
