//! The knife lunge: the client's target pick and the server's movement while lunging.
//!
//! Movement runs this, not the weapon tick: the weapon side is only the lunge flag set by
//! [`super::charge_start`] and the lunge timers/anim picked in [`super::melee_init`].

use super::{MELEE_RANGE, MeleeFacts};

/// Lunge slow-down, units/s² (`player_meleeChargeFriction`).
pub const CHARGE_FRICTION: f32 = 1200.0;
/// Catch-up speed cap (`player_meleeChargeMaxSpeed`): multiplayer, Zombies.
pub const CHARGE_MAX_SPEED_MP: f32 = 800.0;
pub const CHARGE_MAX_SPEED_ZM: f32 = 600.0;
/// The lunge stops this far short of the target after the hit.
pub const CHARGE_STOP_SHORT: f32 = 15.0;
/// Ending a lunge never leaves more upward speed than this.
pub const CHARGE_END_MAX_UP: f32 = 300.0;
/// Multiplayer: an airborne lunge rising faster than this ends.
pub const CHARGE_AIR_MAX_UP: f32 = 240.0;
/// Auto-melee target reach: multiplayer, Zombies, bayonet (`aim_automelee_range`,
/// `aim_autobayonet_range`).
pub const AUTO_MELEE_RANGE_MP: f32 = 100.0;
pub const AUTO_MELEE_RANGE_ZM: f32 = 120.0;
pub const AUTO_BAYONET_RANGE: f32 = 200.0;
/// Lunge move limit: always fine this close; else at least this share of base speed, at
/// most this far off the target (cosine).
pub const MOVE_LIMIT_RANGE: f32 = 20.0;
pub const MOVE_LIMIT_SPEED_FRAC: f32 = 0.1;
pub const MOVE_LIMIT_MIN_COS: f32 = 0.1;
/// The distance sent with a lunge is this much shorter than the real one.
pub const CHARGE_DIST_SHORTEN: f32 = 30.0;
/// View turn toward the lunge target (`aim_automelee_lerp`).
pub const AUTO_MELEE_LERP: f32 = 40.0;

/// Auto-melee reach for picking a lunge target. `is_melee_weapon`: weapon type MELEE or
/// use-as-melee.
pub fn auto_melee_range(m: &MeleeFacts, is_melee_weapon: bool, zombies: bool) -> f32 {
    if m.charge_range != 0.0 && is_melee_weapon {
        m.charge_range
    } else if m.has_bayonet {
        AUTO_BAYONET_RANGE
    } else if zombies {
        AUTO_MELEE_RANGE_ZM
    } else {
        AUTO_MELEE_RANGE_MP
    }
}

/// The lunge is not taken when moving away from the target: inside 20 units it always is;
/// further, horizontal speed must be 10% of base speed and pointed within ~84° of the target.
pub fn move_limit_ok(to_target: [f32; 3], velocity: [f32; 3], base_speed: f32) -> bool {
    let dist = len3(to_target);
    if dist <= MOVE_LIMIT_RANGE {
        return true;
    }
    let speed = len2(velocity);
    if speed < base_speed * MOVE_LIMIT_SPEED_FRAC {
        return false;
    }
    let dir = [to_target[0] / dist, to_target[1] / dist];
    let v = [velocity[0] / speed, velocity[1] / speed];
    dir[0] * v[0] + dir[1] * v[1] >= MOVE_LIMIT_MIN_COS
}

/// The distance sent with a lunge press (a byte on the wire).
pub fn charge_cmd_dist(dist: f32) -> u8 {
    (dist - CHARGE_DIST_SHORTEN).clamp(0.0, 255.0) as u8
}

/// What the lunge needs of the player this frame.
#[derive(Clone, Debug, Default)]
pub struct Body {
    pub origin: [f32; 3],
    pub velocity: [f32; 3],
    /// Normal player type, no special move flags, not prone-locked.
    pub can_lunge: bool,
    pub on_ground: bool,
    pub zombies: bool,
    /// Base move speed (`ps->speed`).
    pub base_speed: f32,
}

/// The lunge target this frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct Target {
    pub origin: [f32; 3],
    pub velocity: [f32; 3],
}

/// One player's lunge (`pm_flags` charge bit, `meleeChargeTime`, `meleeChargeDist`).
#[derive(Clone, Debug, Default)]
pub struct Lunge {
    pub active: bool,
    /// ms left; 0 right after the start = launch on the next move.
    pub time_ms: f32,
    pub dist: f32,
    pub has_target: bool,
}

impl Lunge {
    /// The press: start a lunge, or clear one that cannot. Returns whether it started.
    pub fn start(&mut self, body: &mut Body, cmd_has_target: bool, cmd_dist: f32) -> bool {
        if self.active || !cmd_has_target || !body.can_lunge || !body.on_ground {
            self.clear(body);
            return false;
        }
        self.active = true;
        self.time_ms = 0.0;
        self.dist = cmd_dist;
        self.has_target = true;
        true
    }

    /// End the lunge; it never leaves the player rising fast.
    pub fn clear(&mut self, body: &mut Body) {
        if self.active {
            body.velocity[2] = body.velocity[2].min(CHARGE_END_MAX_UP);
        }
        *self = Lunge::default();
    }

    fn still_valid(&self, body: &mut Body) -> bool {
        if !self.active || !body.can_lunge {
            return false;
        }
        if !body.zombies && !body.on_ground && body.velocity[2] > CHARGE_AIR_MAX_UP {
            let s = len2(body.velocity);
            if s > body.base_speed && s > 0.0 {
                let k = body.base_speed / s;
                body.velocity[0] *= k;
                body.velocity[1] *= k;
            }
            return false;
        }
        true
    }

    /// Launch speed for a lunge of `dist`: stops exactly `dist` on at 1200 u/s².
    pub fn launch_speed(dist: f32) -> f32 {
        (2.0 * dist * CHARGE_FRICTION).sqrt()
    }

    /// Early in the move: launch on the first frame, then catch up while the hit is still
    /// coming, and count the lunge time down. `hit_delay_ms` is the swing's hit timer (0 once
    /// the hit landed).
    pub fn update(&mut self, body: &mut Body, target: Option<Target>, hit_delay_ms: i32, msec: i32) {
        if !self.still_valid(body) {
            self.clear(body);
            return;
        }
        let frametime = msec as f32 / 1000.0;
        if self.time_ms == 0.0 {
            let v0 = Self::launch_speed(self.dist);
            if let Some(t) = target {
                let yaw = (t.origin[1] - body.origin[1]).atan2(t.origin[0] - body.origin[0]);
                body.velocity[0] = yaw.cos() * v0;
                body.velocity[1] = yaw.sin() * v0;
            }
            self.time_ms = v0 / CHARGE_FRICTION * 1000.0;
        }
        if let Some(t) = target {
            self.catch_up(body, t, hit_delay_ms, frametime);
        }
        self.time_ms -= msec as f32;
        if self.time_ms < 1.0 {
            self.clear(body);
        }
    }

    fn catch_up(&self, body: &mut Body, t: Target, hit_delay_ms: i32, frametime: f32) {
        let secs = hit_delay_ms as f32 / 1000.0;
        if secs <= 0.0 {
            return;
        }
        let to = sub3(t.origin, body.origin);
        let d = len3(to);
        if d <= MELEE_RANGE {
            return;
        }
        let k = (d - MELEE_RANGE) / d;
        let boost = 1.0 + frametime * CHARGE_FRICTION;
        let mut v = [
            (t.velocity[0] * secs + to[0] * k) / secs * boost,
            (t.velocity[1] * secs + to[1] * k) / secs * boost,
        ];
        let cap = if body.zombies { CHARGE_MAX_SPEED_ZM } else { CHARGE_MAX_SPEED_MP };
        let s = (v[0] * v[0] + v[1] * v[1]).sqrt();
        if s > cap {
            v = [v[0] * cap / s, v[1] * cap / s];
        }
        body.velocity[0] = v[0];
        body.velocity[1] = v[1];
    }

    /// Ground friction while lunging (replaces the normal friction; water drag is added by
    /// the caller). Returns the speed to take off this frame.
    pub fn friction_drop(&self, body: &Body, target: Option<Target>, hit_delay_ms: i32, frametime: f32) -> f32 {
        if let Some(t) = target.filter(|_| hit_delay_ms < 1 && self.time_ms > 0.0) {
            let dist = len2(sub3(t.origin, body.origin)) - CHARGE_STOP_SHORT;
            let speed = len2(body.velocity);
            let secs = self.time_ms / 1000.0;
            if dist < speed * secs {
                return speed - dist / secs;
            }
        }
        CHARGE_FRICTION * frametime
    }
}

fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn len3(a: [f32; 3]) -> f32 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

fn len2(a: [f32; 3]) -> f32 {
    (a[0] * a[0] + a[1] * a[1]).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lunge_launches_to_stop_at_the_sent_distance() {
        let mut body = Body { can_lunge: true, on_ground: true, base_speed: 190.0, ..Body::default() };
        let mut l = Lunge::default();
        assert!(l.start(&mut body, true, 100.0));
        let target = Target { origin: [200.0, 0.0, 0.0], ..Target::default() };
        // Hit already landed (delay 0): no catch-up, pure launch.
        l.update(&mut body, Some(target), 0, 50);
        let v0 = (2.0f32 * 100.0 * 1200.0).sqrt();
        assert!((body.velocity[0] - v0).abs() < 0.01 && body.velocity[1].abs() < 0.01);
        assert!((l.time_ms - (v0 / 1200.0 * 1000.0 - 50.0)).abs() < 0.01);
        // Not in the air.
        let mut air = Body { on_ground: false, ..body.clone() };
        assert!(!Lunge::default().start(&mut air, true, 100.0));
    }

    #[test]
    fn catch_up_aims_for_reach_and_caps() {
        let mut body = Body { can_lunge: true, on_ground: true, ..Body::default() };
        let mut l = Lunge::default();
        l.start(&mut body, true, 100.0);
        let target = Target { origin: [155.0, 0.0, 0.0], ..Target::default() };
        // 250 ms to the hit, 100 units to close: 400 u/s, x (1 + 0.05 x 1200) = 61x, capped at 800.
        l.update(&mut body, Some(target), 250, 50);
        assert!((body.velocity[0] - 800.0).abs() < 0.01);
        let mut zb = Body { can_lunge: true, on_ground: true, zombies: true, ..Body::default() };
        let mut l = Lunge::default();
        l.start(&mut zb, true, 100.0);
        l.update(&mut zb, Some(target), 250, 50);
        assert!((zb.velocity[0] - 600.0).abs() < 0.01);
    }

    #[test]
    fn after_the_hit_friction_stops_15_short() {
        let body = Body { velocity: [600.0, 0.0, 0.0], ..Body::default() };
        let l = Lunge { active: true, time_ms: 200.0, dist: 100.0, has_target: true };
        let t = Target { origin: [65.0, 0.0, 0.0], ..Target::default() };
        // 50 units to go in 0.2 s: speed drops to 250.
        assert!((l.friction_drop(&body, Some(t), 0, 0.05) - 350.0).abs() < 0.01);
        // Before the hit: plain 1200/s.
        assert!((l.friction_drop(&body, Some(t), 100, 0.05) - 60.0).abs() < 0.01);
    }

    #[test]
    fn ending_a_lunge_caps_upward_speed() {
        let mut body = Body { velocity: [0.0, 0.0, 500.0], can_lunge: true, on_ground: true, ..Body::default() };
        let mut l = Lunge::default();
        l.start(&mut body, true, 50.0);
        l.clear(&mut body);
        assert_eq!(body.velocity[2], 300.0);
    }

    #[test]
    fn target_pick_numbers() {
        let mut m = MeleeFacts::default();
        assert_eq!(auto_melee_range(&m, false, false), 100.0);
        assert_eq!(auto_melee_range(&m, false, true), 120.0);
        m.has_bayonet = true;
        assert_eq!(auto_melee_range(&m, false, false), 200.0);
        m.charge_range = 150.0;
        assert_eq!(auto_melee_range(&m, true, false), 150.0);
        assert_eq!(charge_cmd_dist(110.0), 80);
        assert_eq!(charge_cmd_dist(10.0), 0);
        assert_eq!(charge_cmd_dist(400.0), 255);
        // Backing away from a target 50 units off: no lunge; standing close: fine.
        assert!(!move_limit_ok([50.0, 0.0, 0.0], [-190.0, 0.0, 0.0], 190.0));
        assert!(move_limit_ok([50.0, 0.0, 0.0], [190.0, 0.0, 0.0], 190.0));
        assert!(move_limit_ok([15.0, 0.0, 0.0], [0.0, 0.0, 0.0], 190.0));
        assert!(!move_limit_ok([50.0, 0.0, 0.0], [0.0, 0.0, 0.0], 190.0));
    }
}
