//! recoil_trace FACTS_DUMP CASE [WEAPON [SEED [FIRE_MS]]]: run one recoil/spread case frame by frame (16 ms frames)
//! with a weapon's real file numbers and print a TSV trace for the checker.
//!
//! Cases:
//! - `spray-hip` / `spray-ads`: hold fire FIRE_MS (default 2000) from a standing still player, then let go
//!   for 1 s. SEED (default 12345) is the recoil seed the caller hands in.
//! - `hip`: stand, walk, stand, jump, stand, turn, fire 5 shots, stand.
//! - `land`: the landing dip after falls of 20, 60 and 140 units (camera and gun).
//! - `quickscope`: a sniper reaching full aim with the anti-quickscope timer at its start.
//!
//! The first line is `#facts` (the numbers used, key=value), then a `#cols` header, then frames.

use std::process::ExitCode;

use fastfile_t6::layout::WeaponDef;
use weapon_t6::facts::{T6WeaponFacts, unhex};
use weapon_t6::pm::spread::SpreadFacts;
use weapon_t6::recoil::*;
use weapon_t6::state;

const FRAME_MS: i32 = 16;

fn facts_line(name: &str, s: &SpreadFacts, fire_time: i32, seed: u32, fire_ms: i32) -> String {
    let k = |r: &weapon_t6::pm::spread::KickRange| {
        format!("{},{},{},{}", r.pitch_lower_bound, r.pitch_upper_bound, r.turn_floor, r.turn_ceiling)
    };
    let sp = |g: &weapon_t6::pm::spread::GunSpring| {
        format!("{},{},{},{}", g.speed_up_rate, g.velocity_cap, g.velocity_falloff, g.resting_falloff)
    };
    format!(
        "#facts\tweapon={name}\tseed={seed}\tfire_ms={fire_ms}\tfire_time={fire_time}\tstand={},{}\tducked={},{}\tprone={},{}\tdecay={}\tfire_add={}\tturn_add={}\tmove_add={}\tads_spread={}\tads_kick={}\tads_min_mag={}\thip_kick={}\thip_min_mag={}\tads_gun={}\thip_gun={}\tred={},{},{}\tcenter={},{}\tads_spring={}\thip_spring={}\tgun_max={},{}\taim_down_sight={}\toverlay={}\taqs={},{},{},{}",
        s.upright_narrowest, s.upright_widest, s.crouch_narrowest, s.crouch_widest, s.lying_narrowest, s.lying_widest,
        s.settle_rate, s.per_shot_bloom, s.turn_gain, s.motion_bloom, s.aimed_cone,
        k(&s.ads_view_kick), s.ads_view_kick_min_magnitude, k(&s.hip_view_kick), s.hip_view_kick_min_magnitude,
        k(&s.ads_gun_kick), k(&s.hip_gun_kick),
        s.ads_recoil_reduction_rate, s.ads_recoil_reduction_limit, s.ads_recoil_return_rate,
        s.ads_center_speed, s.hip_center_speed, sp(&s.ads_spring), sp(&s.hip_spring),
        s.weapon_pitch_ceiling, s.weapon_yaw_ceiling, s.scoped_aiming as u8, s.overlay as u8,
        s.aqs_time, s.aqs_scale, s.aqs_spread_multiplier, s.aqs_spread_max,
    )
}

fn still(frac: f32) -> AimSpreadInput {
    let mut input = AimSpreadInput::default();
    input.step_millis_elapsed = FRAME_MS as f32 * 0.001;
    input.frac = frac;
    input.speed = 190;
    input
}

struct Gun<'a> {
    s: &'a SpreadFacts,
    d: RecoilDvars,
    k: KickState,
    scale: f32,
    ov: SpreadOverride,
    seed: u32,
    shots: i32,
}

impl Gun<'_> {
    fn shoot(&mut self, now: i32, frac: f32) {
        self.scale = widen_cone_on_shot(self.scale, self.s, frac);
        let i = FireRecoilInput {
            facts: self.s,
            dual: None,
            frac,
            restrict_kick_time: 0,
            held: self.shots > 0,
            now,
            charge_level: None,
            gas: false,
            gun_phase: state::FIRING,
            weaponstate_left: 0,
        };
        fire_recoil(&mut self.k, &i, &mut self.seed, &self.d);
        self.shots += 1;
    }

    fn settle(&mut self, frac: f32) {
        let c = view_kick_center_speed(
            &CenterInput {
                facts: Some(self.s),
                dual: None,
                frac,
                charge_level: None,
                gun_phase: state::FIRING,
                weaponstate_left: 0,
                stance: 0,
            },
            &self.d,
        );
        settle_view_kick(&mut self.k, FRAME_MS, c);
        gun_spring(&mut self.k, self.s, frac, FRAME_MS as f32 * 0.001);
    }

    fn cone(&self, frac: f32) -> (f32, f32, f32) {
        let (min, max) = spread_for_weapon(
            self.s,
            &{
                let mut input = SpreadInput::default();
                input.view_height = 60.0;
                input.frac = frac;
                input
            },
            &self.d,
        );
        (min, max, bullet_cone(self.s, self.scale, min, max, frac))
    }

    fn line_at(&self, t: i32, stage: &str, fired: i32, amount: f32) -> String {
        let (min, max, cone) = self.cone(amount);
        let k = &self.k;
        format!(
            "{t}\t{stage}\t{fired}\t{:.4}\t{min:.4}\t{max:.4}\t{cone:.4}\t{:.5}\t{:.5}\t{:.5}\t{:.4}\t{:.4}\t{:.4}\t{:.5}\t{:.5}\t{:.4}\t{:.4}\t{:.4}\t{}",
            self.scale,
            k.look_rotation[0], k.look_rotation[1], k.look_rotation[2],
            k.view_avel[0], k.view_avel[1], k.view_avel[2],
            k.weapon_aim_angles[0], k.weapon_aim_angles[1], k.gun_swing_rate[0], k.gun_swing_rate[1],
            k.ads_recoil_reduction, self.seed,
        )
    }
}

const COLS: &str = "#cols\tt\tphase\tfired\tscale\tmin\tmax\tcone\tkick_p\tkick_y\tkick_r\tavel_p\tavel_y\tavel_r\tgun_p\tgun_y\tgun_vp\tgun_vy\tred\tseed";

fn spray(s: &SpreadFacts, fire_time: i32, frac: f32, seed: u32, fire_ms: i32) -> Vec<String> {
    let mut g = Gun { s, d: RecoilDvars::default(), k: KickState::default(), scale: 0.0, ov: SpreadOverride::default(), seed, shots: 0 };
    let mut out = vec![COLS.to_owned()];
    let mut next = 0;
    let mut t = 0;
    while t <= fire_ms + 1000 {
        adjust_aim_spread_scale(&mut g.scale, &mut g.ov, s, &still(frac), &g.d);
        let mut fired = 0;
        let firing = t < fire_ms;
        if firing {
            while next <= t {
                g.shoot(t, frac);
                next += fire_time.max(1);
                fired += 1;
            }
        }
        g.settle(frac);
        out.push(g.line_at(t, if firing { "fire" } else { "release" }, fired, frac));
        t += FRAME_MS;
    }
    out
}

fn hip(s: &SpreadFacts, fire_time: i32) -> Vec<String> {
    let mut g = Gun { s, d: RecoilDvars::default(), k: KickState::default(), scale: 0.0, ov: SpreadOverride::default(), seed: 12345, shots: 0 };
    let mut out = vec![COLS.to_owned()];
    let mut yaw: i32 = 0;
    let mut next = 3500;
    let mut t = 0;
    while t <= 5000 {
        let mut i = still(0.0);
        let stage_name = match t {
            500..1500 => "walk",
            2000..2500 => "jump",
            3000..3500 => "turn",
            3500..4000 => "fire",
            _ => "still",
        };
        i.old_cmd_angles = [0, yaw];
        match stage_name {
            "walk" => {
                i.moving = true;
                i.vel_flat = [190.0, 0.0];
            }
            "jump" => i.in_air = true,
            // 90 degrees per second, in command units.
            "turn" => yaw = yaw.wrapping_add((90.0 * FRAME_MS as f32 * 0.001 / WORD16_TO_DEGREES) as i32) & 0xffff,
            _ => {}
        }
        i.input_angles = [0, yaw];
        adjust_aim_spread_scale(&mut g.scale, &mut g.ov, s, &i, &g.d);
        let mut fired = 0;
        if stage_name == "fire" && g.shots < 5 {
            while next <= t && g.shots < 5 {
                g.shoot(t, 0.0);
                next += fire_time.max(1);
                fired += 1;
            }
        }
        g.settle(0.0);
        out.push(g.line_at(t, stage_name, fired, 0.0));
        t += FRAME_MS;
    }
    out
}

/// The hard-fall landing dip from a fall height (units): 1..24.
fn hard_fall_dip(fall: f32) -> f32 {
    -((((fall - 12.0) / 26.0 + 1.0) * 4.0) as i32).clamp(1, 24) as f32
}

fn land(s: &SpreadFacts) -> Vec<String> {
    let mut out = vec!["#cols\tfall\tdip\tms\tcamera\tgun".to_owned()];
    for fall in [20.0, 60.0, 140.0] {
        let dip = hard_fall_dip(fall);
        let gs = gun_landing_scale(s, 0.0);
        let mut ms = 0;
        while ms <= 496 {
            out.push(format!(
                "{fall}\t{dip}\t{ms}\t{:.4}\t{:.4}",
                touchdown_shift(ms, dip, 1.0),
                touchdown_shift(ms, dip, gs)
            ));
            ms += FRAME_MS;
        }
    }
    out
}

fn quickscope(s: &SpreadFacts) -> Vec<String> {
    let d = RecoilDvars::default();
    let mut out = vec!["#cols\tt\ttimer\tscale\tsway_p\tsway_y".to_owned()];
    let mut scale = 0.0;
    let mut ov = SpreadOverride::default();
    let mut timer = (s.aqs_time * 1000.0) as i32;
    let mut sw = SwayState::default();
    let mut t = 0;
    while t <= 800 {
        let mut i = still(1.0);
        i.quick_scope_scale = s.aqs_scale;
        i.quick_scope_timer = timer;
        adjust_aim_spread_scale(&mut scale, &mut ov, s, &i, &d);
        // Scoped and aimed: the sway must not move while the view turns.
        sway(&mut sw, s, &{
            let mut input = SwayInput::default();
            input.frac = 1.0;
            input.look_rotation = [0.0, t as f32 * 0.1, 0.0];
            input.dt = 0.016;
            input
        });
        out.push(format!("{t}\t{timer}\t{scale:.4}\t{:.4}\t{:.4}", sw.angles[0], sw.angles[1]));
        // Test input: the timer counts down by the frame time.
        timer = (timer - FRAME_MS).max(0);
        t += FRAME_MS;
    }
    out
}

fn main() -> ExitCode {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let (Some(dump), Some(case)) = (args.first(), args.get(1)) else {
        eprintln!("usage: recoil_trace FACTS_DUMP spray-hip|spray-ads|hip|land|quickscope [WEAPON [SEED [FIRE_MS]]]");
        return ExitCode::from(2);
    };
    let text = match std::fs::read_to_string(dump) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{dump}: {e}");
            return ExitCode::from(2);
        }
    };
    let want = args.get(2).cloned().unwrap_or_else(|| {
        match case.as_str() {
            "quickscope" => "dsr50_mp",
            _ => "mp7_mp",
        }
        .to_owned()
    });
    let Some((v, df)) = text.lines().find_map(|l| {
        let mut p = l.split('\t');
        (p.next()? == want).then(|| Some((unhex(p.next()?)?, unhex(p.next()?)?)))?
    }) else {
        eprintln!("{want}: not in {dump}");
        return ExitCode::from(2);
    };
    let seed: u32 = args.get(3).and_then(|a| a.parse().ok()).unwrap_or(12345);
    let fire_ms: i32 = args.get(4).and_then(|a| a.parse().ok()).unwrap_or(2000);
    let s = T6WeaponFacts::read(&v, &df).spread;
    let fire_time = df
        .get(WeaponDef::iFireTime..WeaponDef::iFireTime + 4)
        .map_or(0, |b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let rows = match case.as_str() {
        "spray-hip" => spray(&s, fire_time, 0.0, seed, fire_ms),
        "spray-ads" => spray(&s, fire_time, 1.0, seed, fire_ms),
        "hip" => hip(&s, fire_time),
        "land" => land(&s),
        "quickscope" => quickscope(&s),
        _ => {
            eprintln!("unknown case {case}");
            return ExitCode::from(2);
        }
    };
    println!("{line}", line = facts_line(&want, &s, fire_time, seed, fire_ms));
    for r in rows {
        println!("{r}");
    }
    ExitCode::SUCCESS
}
