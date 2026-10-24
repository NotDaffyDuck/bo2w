//! bo2mp: Black Ops II's helicopter flight (BG_VehicleHelicopterMove and the
//! steps it runs), the way a driven Dragonfire flies: its stick squared to a
//! circle with a dead zone, speeds eased toward what the stick asks (faster
//! when slowing), a turn toward the view no faster than `rotRate`, a body
//! that leans from what it does, its height held over the ground and under
//! a ceiling when no up or down key is held, and its collision brush slid
//! through the world without gravity.
//!
//! Not here: `vehHelicopterAccelTwardsView` (forward climbing with the
//! view's pitch; 0 for the Dragonfire), the locked height of vehicle flag 2,
//! and the quad rotor's arm angles (they only draw).

use core::f32::consts::PI;

use crate::bridge::GroundTraceInput;

use crate::gjk::{Bo2Collision, MoverBrush, vehicle_slide_move};

/// What a helicopter's slide and its height traces meet (BO2's clip mask
/// for a vehicle).
pub const HELI_CLIP_MASK: u32 = 0x0280_0491;

/// Its height hold: how far off its height it asks for full speed up or
/// down, and how close counts as there.
const INPUT_FALLOFF: f32 = 15.0;
const STOP_INPUT: f32 = 1.0;
/// Climbing or falling faster than this the wrong way, the hold pushes back
/// at full strength.
const DRIFT_SPEED: f32 = 4.0;

/// How a helicopter flies: its VehicleDef's values.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HeliDef {
    /// Top speed and acceleration, flat and up or down.
    pub max_speed: f32,
    pub accel: f32,
    pub max_speed_vertical: f32,
    pub accel_vertical: f32,
    /// `rotRate`, `rotAccel`: the most degrees a second it turns, and how
    /// fast it gets to that.
    pub rot_rate: f32,
    pub rot_accel: f32,
    /// `maxBodyPitch`, `maxBodyRoll`: how far it leans at full tilt.
    pub max_body_pitch: f32,
    pub max_body_roll: f32,
    /// Its tilt from speeding up, slowing down and its speed, and how fast
    /// the tilt follows: forward, then sideways.
    pub tilt_from_accel: [f32; 2],
    pub tilt_from_decel: [f32; 2],
    pub tilt_from_vel: [f32; 2],
    pub tilt_speed: [f32; 2],
    /// How fast it stops: forward, sideways, up.
    pub decel: [f32; 3],
    /// Tilt from the stick, from turning at speed (all of it from
    /// `vel_at_max_tilt` of its top speed), and how slowly the tilt answers.
    pub tilt_from_axes: f32,
    pub tilt_from_fwd_yaw: f32,
    pub vel_at_max_tilt: f32,
    pub tilt_momentum: f32,
    /// Holding its height over the ground: on, the most of a step in the
    /// ground it follows at once, the least height, how far down it looks
    /// for the ground, and the room it keeps under a ceiling.
    pub maintain_height: bool,
    pub height_limit: f32,
    pub height_min: f32,
    pub height_max: f32,
    pub ceiling_min: f32,
}

/// Where a helicopter is and what it is doing.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HeliState {
    pub origin: [f32; 3],
    /// Pitch (its lean forward), yaw, roll (its lean sideways).
    pub angles: [f32; 3],
    pub velocity: [f32; 3],
    /// How fast it turns (degrees a second).
    pub yaw_speed: f32,
    /// Its lean (-1..1, forward then sideways) and how fast that changes.
    pub tilt: [f32; 2],
    pub tilt_vel: [f32; 2],
    /// How far it is off the height it holds (`targetHeightDelta`: below it
    /// is negative), and the ground's height under it last frame.
    pub height_delta: f32,
    pub last_ground: f32,
}

/// What its driver does this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HeliControls {
    /// His move keys as the command carries them (-127..127).
    pub forward: i8,
    pub right: i8,
    /// The vehicle's up and down buttons.
    pub up: bool,
    pub down: bool,
    /// Where he looks (yaw, degrees): it turns to face it.
    pub view_yaw: f32,
}

/// What a helicopter flies through.
pub trait HeliWorld {
    /// Where a point traced from `start` to `end` stops when it meets
    /// something; None when it gets all the way.
    fn point_trace(&self, start: [f32; 3], end: [f32; 3]) -> Option<[f32; 3]>;
    /// Its slide from `origin` along `velocity` for `dt`: where it ends up
    /// and its velocity after.
    fn slide(&self, origin: [f32; 3], velocity: [f32; 3], dt: f32) -> ([f32; 3], [f32; 3]);
}

/// A helicopter in BO2's collision: its brush slid through `collision`
/// ([`vehicle_slide_move`], no gravity) when `gjk`; otherwise (a map without
/// its brushes' corners) its brush's box traced, stopping where it meets
/// something.
pub struct HeliCollision<'a, C: Bo2Collision> {
    pub collision: &'a C,
    pub brush: MoverBrush,
    pub gjk: bool,
}

impl<C: Bo2Collision> HeliWorld for HeliCollision<'_, C> {
    fn point_trace(&self, start: [f32; 3], end: [f32; 3]) -> Option<[f32; 3]> {
        let t = self.collision.trace(GroundTraceInput {
            start,
            end,
            mins: [0.0; 3],
            maxs: [0.0; 3],
            tracemask: HELI_CLIP_MASK,
        });
        (t.fraction < 1.0).then(|| core::array::from_fn(|i| start[i] + (end[i] - start[i]) * t.fraction))
    }

    fn slide(&self, origin: [f32; 3], velocity: [f32; 3], dt: f32) -> ([f32; 3], [f32; 3]) {
        if self.gjk {
            let (pos, vel, _) = vehicle_slide_move(self.collision, &self.brush, origin, velocity, HELI_CLIP_MASK, dt, None);
            return (pos, vel);
        }
        let (mins, maxs) = self.brush.bounds();
        let end = core::array::from_fn(|i| origin[i] + velocity[i] * dt);
        let t = self.collision.trace(GroundTraceInput { start: origin, end, mins, maxs, tracemask: HELI_CLIP_MASK });
        if t.startsolid {
            return (origin, [0.0; 3]);
        }
        let f = clampf(t.fraction, 0.0, 1.0);
        let pos = core::array::from_fn(|i| origin[i] + (end[i] - origin[i]) * f);
        (pos, if f < 1.0 { [0.0; 3] } else { velocity })
    }
}

/// An angle wrapped to -180..180.
pub fn angle_wrap180(a: f32) -> f32 {
    let t = a * (1.0 / 360.0);
    (t - libm::floorf(t + 0.5)) * 360.0
}

/// BO2's DiffTrack: `cur` eased toward `tgt` by `(tgt - cur) * rate * dt`,
/// snapping to it when within 0.001 or when the step would pass it.
pub fn diff_track(tgt: f32, cur: f32, rate: f32, dt: f32) -> f32 {
    let gap = tgt - cur;
    let step = gap * rate * dt;
    if gap.abs() > 0.001 && step.abs() <= gap.abs() { cur + step } else { tgt }
}

/// [`diff_track`] for angles: the target brought within 180 of `cur`
/// first; the result 0..360.
pub fn diff_track_angle(mut tgt: f32, cur: f32, rate: f32, dt: f32) -> f32 {
    while tgt - cur > 180.0 {
        tgt -= 360.0;
    }
    while tgt - cur < -180.0 {
        tgt += 360.0;
    }
    let t = diff_track(tgt, cur, rate, dt) * (1.0 / 360.0);
    (t - libm::floorf(t)) * 360.0
}

/// C's clamp: never panics, whatever the bounds.
fn clampf(v: f32, lo: f32, hi: f32) -> f32 {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

/// Its flat forward and right for a yaw (degrees).
fn flat_axes(yaw: f32) -> ([f32; 3], [f32; 3]) {
    let r = yaw * (PI / 180.0);
    let (s, c) = (libm::sinf(r), libm::cosf(r));
    ([c, s, 0.0], [s, -c, 0.0])
}

/// BO2's stick for a helicopter (`clamp_diag` on for its flight): each of
/// forward, right, up and spare over 127; the flat pair squared out to a
/// circle, the sideways part dropped under 0.3, and the smaller of the two
/// pulled in by how much smaller it is; up and spare dropped under 0.3.
pub fn cmd_scale(clamp_diag: bool, m: [i8; 4]) -> [f32; 4] {
    let mut out = m.map(|c| f32::from(c) * 0.007_874_016);
    if m[0] != 0 || m[1] != 0 {
        let most = i32::from(m[0]).abs().max(i32::from(m[1]).abs());
        if most != 0 {
            let (a, b) = (f32::from(m[0]), f32::from(m[1]));
            let f = most as f32 / libm::sqrtf(a * a + b * b);
            out[0] *= f;
            out[1] *= f;
        }
        if out[1].abs() < 0.3 {
            out[1] = 0.0;
        }
        if clamp_diag {
            let (a0, a1) = (out[0].abs(), out[1].abs());
            if a0 <= a1 {
                out[0] *= 1.0 - (a1 - a0);
            } else {
                out[1] *= 1.0 - (a0 - a1);
            }
        }
    }
    for c in &mut out[2..] {
        if c.abs() < 0.3 {
            *c = 0.0;
        }
    }
    out
}

/// BO2's height hold (MaintainHeight_CalcAccel): -1..1, down to up. It
/// follows the ground's steps under it (at most `height_limit` at once),
/// climbs when lower than `height_min` over the ground, keeps
/// `ceiling_min` under a ceiling (between the two when both press), and
/// pushes back hard against drifting the wrong way.
pub fn maintain_height_accel<W: HeliWorld>(def: &HeliDef, st: &mut HeliState, world: &W) -> f32 {
    let o = st.origin;
    let mut too_low = false;
    let down = [o[0], o[1], o[2] - def.height_max];
    let ground = world.point_trace(o, down);
    let gz = ground.map_or(down[2], |p| p[2]);
    if def.maintain_height && ground.is_some() {
        if st.last_ground == 0.0 && st.height_delta == 0.0 {
            st.last_ground = gz;
        }
        let step = gz - st.last_ground;
        let lim = def.height_limit;
        let t = if lim == 0.0 {
            st.height_delta - step
        } else {
            clampf(st.height_delta - clampf(step, -lim, lim), -lim, lim)
        };
        st.height_delta = t;
        let min = def.height_min;
        if 0.001 < min && t < min && o[2] - gz < min {
            too_low = true;
        }
    }
    let ceiling = if def.ceiling_min > 0.001 {
        world.point_trace([o[0], o[1], o[2] + def.ceiling_min], o).map(|p| p[2])
    } else {
        None
    };
    let x = match (too_low, ceiling) {
        (true, None) => (o[2] - gz) - def.height_min,
        (true, Some(cz)) => o[2] - (cz + gz + def.height_min - def.ceiling_min) * 0.5,
        (false, Some(cz)) => def.ceiling_min - (cz - o[2]),
        (false, None) => st.height_delta,
    };
    let a = x.abs();
    let mut s = 1.0;
    if a < INPUT_FALLOFF {
        s = if a >= STOP_INPUT { a / INPUT_FALLOFF } else { 0.0 };
    }
    if x >= 0.0 {
        if st.velocity[2] > DRIFT_SPEED {
            s = 1.0;
        }
        s = -s;
    } else if st.velocity[2] < -DRIFT_SPEED {
        s = 1.0;
    }
    st.last_ground = gz;
    clampf(s, -1.0, 1.0)
}

/// Its up-down stick (BO2's VerticalMove): the up and down buttons, or the
/// height hold when neither is held, as a command's -127..127.
pub fn vertical_move<W: HeliWorld>(def: &HeliDef, st: &mut HeliState, c: &HeliControls, world: &W) -> i8 {
    let mut v = f32::from(u8::from(c.up)) - f32::from(u8::from(c.down));
    if v == 0.0 {
        v = maintain_height_accel(def, st, world);
    }
    libm::rintf(v * 127.0 + 9.313_226e-10) as i8
}

/// After its slide, with no up or down key held: the height it holds takes
/// in how far it actually went up or down (BO2's UpdateHeight).
fn update_height(st: &mut HeliState, c: &HeliControls, prev_z: f32) {
    if c.up || c.down || st.height_delta.abs() <= 1.0 {
        return;
    }
    let d = (st.origin[2] - prev_z) + st.height_delta;
    st.height_delta = if st.height_delta < 0.0 { d.min(0.0) } else { d.max(0.0) };
}

/// BO2's helicopter acceleration (HeliCalcAccel) for its speed along its
/// own forward, right and up (`body_vel`) and the scaled stick: its
/// acceleration along those, and [its pitch, how fast its turn speeds up,
/// its roll]. Its lean (`st.tilt`, `st.tilt_vel`) moves on.
pub fn calc_accel(
    def: &HeliDef,
    st: &mut HeliState,
    body_vel: [f32; 3],
    cmd: [f32; 4],
    view_yaw: f32,
    prev_yaw: f32,
    dt: f32,
) -> ([f32; 3], [f32; 3]) {
    let acc = [def.accel, def.accel, def.accel_vertical];
    let max_s = [def.max_speed, def.max_speed, def.max_speed_vertical];
    let k_flat = if def.max_speed == 0.0 { 1.0 } else { def.accel / def.max_speed };
    let k = [
        k_flat,
        k_flat,
        if def.max_speed_vertical == 0.0 { 1.0 } else { def.accel_vertical / def.max_speed_vertical },
    ];
    let desired = [max_s[0] * cmd[0], max_s[1] * cmd[1], cmd[2] * max_s[2]];
    // Its speed along where the stick points, and across it.
    let h2 = desired[0] * desired[0] + desired[1] * desired[1];
    let len = if h2 > 0.0 { libm::sqrtf(h2) } else { 1.0 };
    let dir = [desired[0] / len, desired[1] / len];
    let mut total = libm::sqrtf(desired[2] * desired[2] + h2);
    let (along, along_vec, perp) = if total >= 1.0 {
        let along = dir[0] * body_vel[0] + dir[1] * body_vel[1];
        let along_vec = [dir[0] * along, dir[1] * along];
        (along, along_vec, [body_vel[0] - along_vec[0], body_vel[1] - along_vec[1]])
    } else {
        total = 0.0;
        (0.0, [0.0; 2], [body_vel[0], body_vel[1]])
    };
    let mut body_accel = [0.0; 3];
    let mut dec_acc = [0.0; 2];
    let mut acc_acc = [0.0; 2];
    for i in 0..2 {
        let rate = def.decel[i] * k[i];
        dec_acc[i] = (diff_track(0.0, perp[i], rate, dt) - perp[i]) / dt;
        if total <= along {
            dec_acc[i] += (diff_track(desired[i], along_vec[i], rate, dt) - along_vec[i]) / dt;
        } else {
            acc_acc[i] = (diff_track(desired[i], along_vec[i], k[i], dt) - along_vec[i]) / dt;
        }
        body_accel[i] = clampf(acc_acc[i] + dec_acc[i], -acc[i], acc[i]);
        dec_acc[i] = clampf(dec_acc[i], -acc[i], acc[i]);
        acc_acc[i] = clampf(acc_acc[i], -acc[i], acc[i]);
    }
    // Up and down: slowing (toward zero) at its up deceleration.
    let vz = body_vel[2];
    let mut rate_v = k[2];
    if (desired[2] < vz && vz > 0.0) || (vz < desired[2] && vz < 0.0) {
        rate_v = def.decel[2] * k[2];
    }
    body_accel[2] = clampf((diff_track(desired[2], vz, rate_v, dt) - vz) / dt, -acc[2], acc[2]);
    // Its turn toward his view.
    let kr = def.rot_accel / def.rot_rate;
    let toward = diff_track_angle(view_yaw, prev_yaw, kr, dt);
    let mut want = clampf(angle_wrap180(toward - prev_yaw) / dt, -def.rot_rate, def.rot_rate);
    if st.yaw_speed.abs() < want.abs() {
        want = diff_track(want, st.yaw_speed, kr, dt);
    }
    let yaw_accel = clampf((want - st.yaw_speed) / dt, -def.rot_accel, def.rot_accel);
    // Its lean: from slowing, speeding up, its speed and the stick; sideways
    // also from turning at speed.
    let inv = 1.0 / max_s[0];
    let tx = clampf(
        dec_acc[0] / acc[0] * def.tilt_from_decel[0]
            + acc_acc[0] / acc[0] * def.tilt_from_accel[0]
            + inv * body_vel[0] * def.tilt_from_vel[0]
            + desired[0] * inv * def.tilt_from_axes,
        -1.0,
        1.0,
    );
    let mut ty = clampf(
        dec_acc[1] / acc[1] * def.tilt_from_decel[1]
            + acc_acc[1] / acc[1] * def.tilt_from_accel[1]
            + desired[1] / max_s[1] * def.tilt_from_axes
            + body_vel[1] / max_s[1] * def.tilt_from_vel[1],
        -1.0,
        1.0,
    );
    if body_vel[0] > 0.0 && st.yaw_speed.abs() > 0.0 {
        let f = body_vel[0] * inv;
        let f = if f >= def.vel_at_max_tilt { 1.0 } else { f / def.vel_at_max_tilt };
        ty += (-1.0 / def.rot_rate) * st.yaw_speed * def.tilt_from_fwd_yaw * f;
    }
    let ty = clampf(ty, -1.0, 1.0);
    // The lean follows on a spring.
    let m = 1.0 / def.tilt_momentum;
    let target = [tx, ty];
    for i in 0..2 {
        let tv = st.tilt_vel[i];
        let nv = (-tv * m + def.tilt_speed[i] * m * (target[i] - st.tilt[i])) * dt + tv;
        st.tilt[i] += (tv + nv) * 0.5 * dt;
        st.tilt_vel[i] = nv;
        if st.tilt[i] > 1.0 {
            st.tilt[i] = 1.0;
            if nv > 0.0 {
                st.tilt_vel[i] = 0.0;
            }
        }
        if st.tilt[i] < -1.0 {
            st.tilt[i] = -1.0;
            if nv < 0.0 {
                st.tilt_vel[i] = 0.0;
            }
        }
    }
    (body_accel, [def.max_body_pitch * st.tilt[0], yaw_accel, def.max_body_roll * st.tilt[1]])
}

/// One frame of BO2's helicopter flight (BG_VehicleHelicopterMove) of `dt`
/// seconds. A def with no top speed or acceleration (none loaded) does not
/// move.
pub fn heli_move<W: HeliWorld>(def: &HeliDef, st: &mut HeliState, c: &HeliControls, dt: f32, world: &W) {
    if dt <= 0.0 || def.max_speed <= 0.0 || def.accel <= 0.0 || def.rot_rate <= 0.0 || def.tilt_momentum <= 0.0 {
        return;
    }
    let prev_z = st.origin[2];
    let prev_yaw = st.angles[1];
    let vertical = vertical_move(def, st, c, world);
    let (fwd, right) = flat_axes(prev_yaw);
    let v = st.velocity;
    let body_vel = [
        v[0] * fwd[0] + v[1] * fwd[1],
        v[0] * right[0] + v[1] * right[1],
        v[2],
    ];
    let cmd = cmd_scale(true, [c.forward, c.right, vertical, 0]);
    let (accel, rot) = calc_accel(def, st, body_vel, cmd, c.view_yaw, prev_yaw, dt);
    st.yaw_speed += dt * rot[1];
    let yaw = angle_wrap180(prev_yaw + dt * st.yaw_speed);
    st.angles = [rot[0], yaw, rot[2]];
    let (fwd, right) = flat_axes(yaw);
    for i in 0..3 {
        let up = if i == 2 { accel[2] } else { 0.0 };
        st.velocity[i] += (fwd[i] * accel[0] + right[i] * accel[1] + up) * dt;
    }
    if st.velocity != [0.0; 3] {
        let (pos, vel) = world.slide(st.origin, st.velocity, dt);
        st.origin = pos;
        st.velocity = vel;
        update_height(st, c, prev_z);
    }
}

#[cfg(test)]
mod tests {
    use core::cell::Cell;

    use super::*;

    const DT: f32 = 0.05;

    /// The Dragonfire's own values (qrdrone_mp).
    fn dragonfire() -> HeliDef {
        HeliDef {
            max_speed: 352.0,
            accel: 440.0,
            max_speed_vertical: 193.6,
            accel_vertical: 369.6,
            rot_rate: 200.0,
            rot_accel: 1500.0,
            max_body_pitch: 20.0,
            max_body_roll: 25.0,
            tilt_from_accel: [0.01, 0.01],
            tilt_from_decel: [0.05, 0.2],
            tilt_from_vel: [0.0, 0.0],
            tilt_speed: [5.0, 5.0],
            decel: [2.0, 2.0, 2.5],
            tilt_from_axes: 0.4,
            tilt_from_fwd_yaw: 0.5,
            vel_at_max_tilt: 0.75,
            tilt_momentum: 0.1,
            maintain_height: true,
            height_limit: 15.0,
            height_min: 20.0,
            height_max: 200.0,
            ceiling_min: 45.0,
        }
    }

    /// A floor (solid below it), maybe a thin ceiling, maybe a wall across
    /// x; the drone is a 16 x 16 brush, 15.5 under its origin.
    struct Room {
        floor: Cell<f32>,
        ceiling: Option<f32>,
        wall_x: Option<f32>,
    }

    impl Room {
        fn open(floor: f32) -> Self {
            Room { floor: Cell::new(floor), ceiling: None, wall_x: None }
        }
    }

    impl HeliWorld for Room {
        fn point_trace(&self, s: [f32; 3], e: [f32; 3]) -> Option<[f32; 3]> {
            let mut best: Option<f32> = None;
            for z in [Some(self.floor.get()), self.ceiling].into_iter().flatten() {
                let crosses = (s[2] > z && e[2] <= z) || (s[2] < z && e[2] >= z);
                if crosses {
                    let t = (s[2] - z) / (s[2] - e[2]);
                    best = Some(best.map_or(t, |b: f32| b.min(t)));
                }
            }
            best.map(|t| core::array::from_fn(|i| s[i] + (e[i] - s[i]) * t))
        }

        fn slide(&self, o: [f32; 3], v: [f32; 3], dt: f32) -> ([f32; 3], [f32; 3]) {
            let mut p: [f32; 3] = core::array::from_fn(|i| o[i] + v[i] * dt);
            let mut v = v;
            let low = self.floor.get() + 15.5;
            if p[2] < low {
                p[2] = low;
                v[2] = 0.0;
            }
            if let Some(w) = self.wall_x {
                if p[0] > w - 16.0 {
                    p[0] = w - 16.0;
                    v[0] = 0.0;
                }
            }
            (p, v)
        }
    }

    fn at(z: f32) -> HeliState {
        HeliState { origin: [0.0, 0.0, z], ..HeliState::default() }
    }

    fn fly(st: &mut HeliState, room: &Room, c: HeliControls, frames: usize) {
        fly_as(&dragonfire(), st, room, c, frames);
    }

    fn fly_as(def: &HeliDef, st: &mut HeliState, room: &Room, c: HeliControls, frames: usize) {
        for _ in 0..frames {
            heli_move(def, st, &c, DT, room);
            assert!(st.tilt.iter().all(|t| (-1.0..=1.0).contains(t)), "{st:?}");
            assert!(st.angles[0].abs() <= 20.0 + 1e-4 && st.angles[2].abs() <= 25.0 + 1e-4, "{st:?}");
            assert!(st.yaw_speed.abs() <= 200.0 + 1e-3, "{st:?}");
        }
    }

    fn near(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol
    }

    fn flat_speed(st: &HeliState) -> f32 {
        libm::sqrtf(st.velocity[0] * st.velocity[0] + st.velocity[1] * st.velocity[1])
    }

    #[test]
    fn the_stick_is_squared_to_a_circle_with_a_dead_zone() {
        assert_eq!(cmd_scale(true, [127, 0, 0, 0]), [1.0, 0.0, 0.0, 0.0]);
        assert_eq!(cmd_scale(true, [0, -127, 0, 0]), [0.0, -1.0, 0.0, 0.0]);
        // A little sideways with full forward is dropped (after the pair is
        // squared to the circle).
        let d = cmd_scale(true, [127, 30, 0, 0]);
        assert!(near(d[0], 127.0 / libm::sqrtf(127.0 * 127.0 + 30.0 * 30.0), 1e-6) && d[1] == 0.0, "{d:?}");
        // A full diagonal is a circle's, not a square's.
        let d = cmd_scale(true, [127, 127, 0, 0]);
        assert!(near(d[0], 0.70710677, 1e-5) && near(d[1], 0.70710677, 1e-5), "{d:?}");
        // Off the diagonal the smaller part is pulled in.
        let d = cmd_scale(true, [127, 60, 0, 0]);
        let f = 127.0 / libm::sqrtf(127.0 * 127.0 + 60.0 * 60.0);
        let (a0, a1) = (f, 60.0 / 127.0 * f);
        assert!(near(d[0], a0, 1e-5) && near(d[1], a1 * (1.0 - (a0 - a1)), 1e-5), "{d:?}");
        // Up and down under 0.3 are dropped.
        assert_eq!(cmd_scale(true, [0, 0, 30, 0])[2], 0.0);
        assert_eq!(cmd_scale(true, [0, 0, -127, 0])[2], -1.0);
    }

    #[test]
    fn it_hovers_where_it_is() {
        let room = Room::open(0.0);
        let mut st = at(100.0);
        fly(&mut st, &room, HeliControls::default(), 100);
        assert!(near(st.origin[2], 100.0, 1e-3), "{st:?}");
        assert_eq!(st.velocity, [0.0; 3]);
        assert_eq!(st.angles, [0.0; 3]);
    }

    #[test]
    fn full_forward_speeds_it_up_nose_down() {
        let room = Room::open(0.0);
        let mut st = at(100.0);
        let c = HeliControls { forward: 127, ..HeliControls::default() };
        fly(&mut st, &room, c, 1);
        // The first frame: 1/16 of the way to 352 (its acceleration over
        // its top speed, 1.25 a second).
        assert!(near(st.velocity[0], 22.0, 1e-3), "{st:?}");
        fly(&mut st, &room, c, 59);
        let speed = flat_speed(&st);
        assert!(speed > 340.0 && speed <= 352.0, "{st:?}");
        assert!(near(st.velocity[1], 0.0, 1e-3), "{st:?}");
        // Leaning forward (the stick's 0.4 of full tilt) at its height.
        assert!(st.angles[0] > 6.0 && st.angles[0] < 10.0, "{st:?}");
        assert!(near(st.origin[2], 100.0, 1e-3), "{st:?}");
        // Let go: it slows faster than it sped up, nose up once its lean
        // has swung back.
        fly(&mut st, &room, HeliControls::default(), 4);
        assert!(flat_speed(&st) < 260.0 && st.angles[0] < 4.0, "{st:?}");
        fly(&mut st, &room, HeliControls::default(), 6);
        assert!(st.angles[0] < 0.0, "{st:?}");
        fly(&mut st, &room, HeliControls::default(), 60);
        assert!(flat_speed(&st) < 3.0, "{st:?}");
    }

    #[test]
    fn it_turns_to_his_view_no_faster_than_its_turn_rate() {
        let room = Room::open(0.0);
        let mut st = at(100.0);
        let c = HeliControls { view_yaw: 90.0, ..HeliControls::default() };
        let mut last = 0.0;
        for _ in 0..60 {
            fly(&mut st, &room, c, 1);
            assert!(st.angles[1] - last <= 200.0 * DT + 1e-3, "{st:?}");
            last = st.angles[1];
        }
        assert!(near(st.angles[1], 90.0, 0.5), "{st:?}");
        // Over the 180 line it goes the short way.
        let mut st = HeliState { angles: [0.0, 170.0, 0.0], ..at(100.0) };
        let c = HeliControls { view_yaw: -170.0, ..HeliControls::default() };
        fly(&mut st, &room, c, 60);
        assert!(near(angle_wrap180(st.angles[1] + 170.0), 0.0, 0.5), "{st:?}");
    }

    #[test]
    fn turning_at_speed_banks_into_the_turn() {
        let room = Room::open(0.0);
        let mut st = at(100.0);
        fly(&mut st, &room, HeliControls { forward: 127, ..HeliControls::default() }, 40);
        let c = HeliControls { forward: 127, view_yaw: 90.0, ..HeliControls::default() };
        fly(&mut st, &room, c, 5);
        // Turning left (yaw going up) leans it left.
        assert!(st.yaw_speed > 0.0 && st.angles[2] < -1.0, "{st:?}");
    }

    #[test]
    fn up_climbs_and_letting_go_holds_the_new_height() {
        let room = Room::open(0.0);
        let mut st = at(100.0);
        fly(&mut st, &room, HeliControls { up: true, ..HeliControls::default() }, 20);
        assert!(st.velocity[2] > 150.0, "{st:?}");
        let top = st.origin[2];
        fly(&mut st, &room, HeliControls::default(), 60);
        assert!(near(st.velocity[2], 0.0, 0.5), "{st:?}");
        // It brakes, a little past where it was let go, and stays.
        assert!(st.origin[2] > top && st.origin[2] < top + 60.0, "{st:?} {top}");
        let z = st.origin[2];
        fly(&mut st, &room, HeliControls::default(), 40);
        assert!(near(st.origin[2], z, 1.0), "{st:?} {z}");
    }

    #[test]
    fn it_follows_a_step_up_in_the_ground() {
        // (A ground at exactly 0 reads as no ground seen yet, as in BO2.)
        let room = Room::open(50.0);
        let mut st = at(150.0);
        fly(&mut st, &room, HeliControls::default(), 10);
        assert!(near(st.origin[2], 150.0, 1e-3), "{st:?}");
        room.floor.set(60.0);
        fly(&mut st, &room, HeliControls::default(), 20);
        assert!(st.velocity[2] > 0.0 || st.origin[2] > 154.0, "{st:?}");
        fly(&mut st, &room, HeliControls::default(), 100);
        assert!(st.origin[2] > 154.0 && st.origin[2] < 168.0, "{st:?}");
        assert!(near(st.velocity[2], 0.0, 0.5), "{st:?}");
    }

    #[test]
    fn too_low_it_climbs_to_its_least_height() {
        // A ground that jumps up more than it follows at once leaves it
        // under its least height (here 40): it climbs at full speed.
        let def = HeliDef { height_min: 40.0, ..dragonfire() };
        let room = Room::open(50.0);
        let mut st = at(120.0);
        fly_as(&def, &mut st, &room, HeliControls::default(), 10);
        room.floor.set(100.0);
        // Pushed up to the new floor by its brush, then climbing.
        fly_as(&def, &mut st, &room, HeliControls::default(), 6);
        assert!(st.origin[2] > 125.0 && st.velocity[2] > 50.0, "{st:?}");
        fly_as(&def, &mut st, &room, HeliControls::default(), 100);
        assert!(st.origin[2] > 135.0 && st.origin[2] < 150.0, "{st:?}");
        assert!(near(st.velocity[2], 0.0, 0.5), "{st:?}");
    }

    #[test]
    fn it_keeps_room_under_a_ceiling() {
        let room = Room { ceiling: Some(130.0), ..Room::open(0.0) };
        let mut st = at(100.0);
        fly(&mut st, &room, HeliControls::default(), 80);
        assert!(st.origin[2] < 90.0 && st.origin[2] > 70.0, "{st:?}");
    }

    #[test]
    fn a_wall_stops_it() {
        let room = Room { wall_x: Some(200.0), ..Room::open(0.0) };
        let mut st = at(100.0);
        fly(&mut st, &room, HeliControls { forward: 127, ..HeliControls::default() }, 60);
        assert!(near(st.origin[0], 184.0, 1e-3), "{st:?}");
    }
}
