//! The bench: Black Ops II's own frame (`movement_t6`) on scripted keys in a
//! small world of boxes. Run with
//! `cargo test -p movement_t6 --test compare -- --nocapture` for the table.
//! (It used to set the MW2 frame beside it; that frame is gone.)

use movement_t6::bridge::{
    ScopeBlendArgs, ScopeWishArgs, AirborneStepArgs, Bo2Feel, StrideScaleArgs,
    SolidProbe, FixedClimbClipDuration, SweepAsk, LeapKickoffArgs,
    LungeGunTimings, HullExtents, PRONE_LERP_MS, OneTickMotionArgs, DashArgs,
    LookRotationLimit, FootedStepArgs, StillClimbOffset,
};
use playerstate_t6::game::{AvatarState, InputCommand};
use trace_t6::Trace;

mod key {
    pub const SPRINT: u32 = 0x2;

    pub const LYING_FLAT: u32 = 0x100;
    pub const DUCK_BUTTON_BIT: u32 = 0x200;
    pub const LEAP: u32 = 0x400;
    pub const ADS: u32 = 0x0800;
}

/// How far short of a face the swept box stops (the engine's own gap).
const SKIN: f32 = 0.125;

// ---------------------------------------------------------------- the world

/// An axis-aligned block and the surface bits its faces report.
#[derive(Clone, Copy)]
struct Block {
    lo: [f32; 3],
    hi: [f32; 3],
    flags: u32,
}

/// What one block does to a sweep.
enum Touch {
    /// The sweep begins inside the block (and maybe ends inside it too).
    Inside { to_the_end: bool },
    /// The sweep runs into a face at `at` along its path.
    Face { at: f32, dim: usize, sign: f32 },
    Clear,
}

impl Block {
    fn touch(&self, src_pt: [f32; 3], dst_pt: [f32; 3], step: [f32; 3], lo_ext: [f32; 3], hi_ext: [f32; 3]) -> Touch {
        // Minkowski sum: the block grown by the player's box, the player a point.
        let lo: [f32; 3] = std::array::from_fn(|k| self.lo[k] - hi_ext[k]);
        let hi: [f32; 3] = std::array::from_fn(|k| self.hi[k] - lo_ext[k]);
        let holds = |p: [f32; 3]| p.iter().enumerate().all(|(k, &v)| lo[k] < v && v < hi[k]);
        if holds(src_pt) {
            return Touch::Inside { to_the_end: holds(dst_pt) };
        }
        let mut entry: Option<(f32, usize, f32)> = None;
        let mut latest = -1.0_f32;
        let mut exit = 1.0_f32;
        for k in 0..3 {
            let d = step[k];
            if d == 0.0 {
                let outside = src_pt[k] <= lo[k] || src_pt[k] >= hi[k];
                if outside {
                    return Touch::Clear;
                }
                continue;
            }
            let (near, far, sign) = match d > 0.0 {
                true => (lo[k], hi[k], -1.0),
                false => (hi[k], lo[k], 1.0),
            };
            let t_near = (near - src_pt[k]) / d;
            exit = exit.min((far - src_pt[k]) / d);
            if t_near > latest {
                latest = t_near;
                entry = Some((t_near, k, sign));
            }
        }
        match entry {
            Some((at, dim, sign)) if at <= exit && (0.0..1.0).contains(&at) => Touch::Face { at, dim, sign },
            _ => Touch::Clear,
        }
    }
}

struct Room(Vec<Block>);

impl Room {
    /// Flat ground at z = 0, 4000 units out each way.
    fn floor() -> Self {
        Room(vec![Block { lo: [-4000.0, -4000.0, -64.0], hi: [4000.0, 4000.0, 0.0], flags: 0 }])
    }

    fn plus(self, lo: [f32; 3], hi: [f32; 3], flags: u32) -> Self {
        let Room(mut blocks) = self;
        blocks.push(Block { lo, hi, flags });
        Room(blocks)
    }
}

fn strike(at: f32, len: f32, dim: usize, sign: f32, flags: u32) -> Trace {
    let mut facing = [0.0_f32; 3];
    facing[dim] = sign;
    let mut hit = Trace::new();
    hit.fraction = ((at * len - SKIN) / len).max(0.0);
    (hit.normal, hit.sflags, hit.cflags) = (facing, flags, 1);
    hit.impact_category = trace_t6::TraceHitType::Entity;
    hit.struck_entity_num = trace_t6::ENTITYNUM_WORLD;
    hit.walkable = dim == 2 && sign >= 0.7;
    hit
}

impl movement_t6::Bo2Collision for Room {}

impl SolidProbe for Room {
    fn trace(&self, input: SweepAsk) -> Trace {
        let step: [f32; 3] = std::array::from_fn(|k| input.end[k] - input.start[k]);
        let len = step.iter().map(|d| d * d).sum::<f32>().sqrt();
        let mut nearest: Option<Trace> = None;
        let (mut began_inside, mut stuck) = (false, false);
        let (lo, hi) = (input.lo_corner, input.hi_corner);
        for block in &self.0 {
            match block.touch(input.start, input.end, step, lo, hi) {
                Touch::Inside { to_the_end } => {
                    began_inside = true;
                    stuck |= to_the_end;
                }
                Touch::Face { at, dim, sign } => {
                    let t = strike(at, len, dim, sign, block.flags);
                    if t.fraction < nearest.as_ref().map_or(1.0, |n| n.fraction) {
                        nearest = Some(t);
                    }
                }
                Touch::Clear => {}
            }
        }
        let clear = Trace::new();
        let mut out = nearest.unwrap_or(clear);
        if began_inside {
            out.starts_inside = true;
            out.fully_embedded = stuck;
            if stuck {
                out.fraction = 0.0_f32;
            }
        }
        out
    }
}

// ---------------------------------------------------------------- the rules

/// bo2mp's multiplayer rules, as the sim builds them from BO2's dvars.
fn bo2_feel() -> Bo2Feel {
    Bo2Feel {
        sprint_strafe_speed_scale: 0.667,
        jump_slowdown: true,
        fall_damage_min_height: 128.0,
        fall_damage_max_height: 300.0,
        zombies: false,
        gravity: 800,
        fast_ads_multiplier: 0.5,
        sprint_multiplier: 2.0,
        speed_multiplier: 1.07,
        g_speed: 190.0,
        dive: true,
        omni: false,
        mantle_weapon_height: 39.0,
        mantle_weapon_anim_height: 32.0,
        leap_height: 39.0_f32,
        ..Bo2Feel::BO2_MP
    }
}

const NO_SHOCK_AIR: AirborneStepArgs = AirborneStepArgs {
    observer_pace_ratio: 1.0,
    concussed_fall_ratio: 1.0,
    concussed_fall_offset: 0.0,
};

fn walk_rules(last_tick_buttons: u32) -> FootedStepArgs {
    let cmd_scale = StrideScaleArgs {
        retreat_pace_ratio: 0.7,
        sidestep_pace_ratio: 0.8,
        dash_pace_ratio: 1.5,
        downed_crawl_pace_ratio: 0.15,
        gun_stride_ratio: 1.0,
        scoped_carry_pace: 1.0,
        shellshock_movement_scale: 1.0,
        prone_lerp_ms: PRONE_LERP_MS,
    };
    let jump = LeapKickoffArgs {
        leap_height: 39.0_f32,
        dive: Default::default(),
        squat_leap_ratio: 1.0,
        rung_kickoff_speed: 128.0,
    };
    FootedStepArgs {
        cmd_scale,
        gun_carry_pace: 1.0,
        last_tick_buttons,
        jump,
        air: NO_SHOCK_AIR,
        feel: bo2_feel(),
        lunge: None,
    }
}

fn sprint_rules() -> DashArgs {
    DashArgs {
        gun_dash_cap_secs: 4000,
        endless_dash: false,
        dash_least_secs: 1.0,
        dash_warmup_secs: 0.0,
        dash_thrust_floor: 105,
        rise_space_free: true,
        dash_refill_hold_secs: 0.0,
    }
}

fn aim_rules() -> (ScopeWishArgs, ScopeBlendArgs) {
    let intent = ScopeWishArgs {
        sights_permitted: true,
        scoped_weapon: false,
        dash_keeps_scope: false,
    };
    let frac = ScopeBlendArgs {
        scoped_aiming: false,
        scope_raise_speed: 1.0 / 200.0,
        scope_lower_speed: 1.0 / 200.0,
        rack_bolt_when_scoped: true,
        fire_only_when_scoped: false,
        ..ScopeBlendArgs::default()
    };
    (intent, frac)
}

/// Everything one move needs, given last frame's buttons and stick.
fn rules(last_tick_buttons: u32, old_move: [i8; 2]) -> OneTickMotionArgs {
    let (ads_intent, ads_frac) = aim_rules();
    OneTickMotionArgs {
        walk: walk_rules(last_tick_buttons),
        air: NO_SHOCK_AIR,
        bounds: HullExtents {
            lo_corner: [-15.0_f32, -15.0, 0.0],
            hi_corner: [15.0_f32, 15.0, 70.0],
            collide_filter: 0x0281_0011,
        },
        look_rotation: LookRotationLimit {
            tilt_upper_limit: 85.0,
            tilt_lower_limit: 85.0,
            free_tilt_flag: false,
        },
        sprint: sprint_rules(),
        ads_intent,
        ads_frac,
        melee_charge: LungeGunTimings::default(),
        knife_reach: movement_t6::bridge::MELEE_CHARGE_PLAYER_MELEE_RANGE_DEFAULT,
        last_tick_buttons,
        old_move,
        gun_forbids_lying: false,
    }
}

/// A standing player on the ground, 190 speed, 800 gravity.
fn fresh_player() -> AvatarState {
    AvatarState {
        gravity: 800,
        speed: 190,
        run_speed_modifier: 1.0,
        standing_on: 1022,
        eye_height_goal: 60,
        eye_height_now: 60.0,
        last_cmd_ms: 1000,
        ..AvatarState::ZERO
    }
}

// ---------------------------------------------------------------- driving

/// Feeds commands to the real frame, carrying last frame's buttons and stick.
struct Driver<'w> {
    avatar: AvatarState,
    room: &'w Room,
    prev: (u32, [i8; 2]),
}

impl<'w> Driver<'w> {
    fn new(room: &'w Room, avatar: AvatarState) -> Self {
        Driver { avatar, room, prev: (0, [0, 0]) }
    }

    fn step(&mut self, msec: i32, ahead: i8, lateral: i8, held: u32, yaw: f32) {
        self.step_with(msec, ahead, lateral, held, yaw, |_| {});
    }

    fn step_with(&mut self, msec: i32, ahead: i8, lateral: i8, held: u32, yaw: f32, tweak: impl FnOnce(&mut OneTickMotionArgs)) {
        let mut ucmd = InputCommand {
            authority_clock_ms: self.avatar.last_cmd_ms + msec,
            thrust_ahead: ahead,
            thrust_lateral: lateral,
            buttons: held,
            ..InputCommand::default()
        };
        ucmd.angles[1] = ((yaw * 65536.0 / 360.0) as i32) & 0xffff;
        let mut ctx = rules(self.prev.0, self.prev.1);
        tweak(&mut ctx);
        movement_t6::pmove(&mut self.avatar, &mut ucmd, ctx, self.room, &FixedClimbClipDuration::default(), &StillClimbOffset);
        self.prev = (ucmd.buttons, [ucmd.thrust_ahead, ucmd.thrust_lateral]);
    }
}

// ---------------------------------------------------------------- the bench

struct Run {
    name: &'static str,
    room: fn() -> Room,
    spawn_at: [f32; 3],
    frames: usize,
    msec: i32,
    script: fn(usize) -> (i8, i8, u32, f32),
    prep: fn(&mut AvatarState),
}

impl Run {
    /// On the floor, from the origin, 50 ms frames, the player untouched.
    fn new(name: &'static str, frames: usize, script: fn(usize) -> (i8, i8, u32, f32)) -> Self {
        Run { name, room: Room::floor, spawn_at: Default::default(), frames, msec: 50, script, prep: |_| {} }
    }
    fn room(self, room: fn() -> Room) -> Self {
        Run { room, ..self }
    }
    fn at(self, at_pos: [f32; 3]) -> Self {
        Run { spawn_at: at_pos, ..self }
    }
    fn msec(self, msec: i32) -> Self {
        Run { msec, ..self }
    }
    fn prep(self, prep: fn(&mut AvatarState)) -> Self {
        Run { prep, ..self }
    }

    /// (origin, velocity, movement_state_bits, on ground, view height) after each frame.
    fn play(&self) -> Vec<([f32; 3], [f32; 3], u32, bool, f32)> {
        let room = (self.room)();
        let mut avatar = fresh_player();
        avatar.origin = self.spawn_at;
        (self.prep)(&mut avatar);
        let mut d = Driver::new(&room, avatar);
        (0..self.frames)
            .map(|i| {
                let (f, r, b, yaw) = (self.script)(i);
                d.step(self.msec, f, r, b, yaw);
                let p = &d.avatar;
                let grounded = p.standing_on != playerstate_t6::game::NO_ENTITY_SLOT;
                (p.origin, p.velocity, p.movement_state_bits, grounded, p.eye_height_now)
            })
            .collect()
    }
}

fn wall() -> Room {
    // A square wall across the path at x = 200; the player meets it at an angle.
    Room::floor().plus([200.0, -4000.0, 0.0], [264.0, 4000.0, 200.0], 0)
}

fn step16() -> Room {
    Room::floor().plus([100.0, -4000.0, 0.0], [4000.0, 4000.0, 16.0], 0)
}

fn ledge40() -> Room {
    Room::floor().plus([60.0, -4000.0, 0.0], [4000.0, 4000.0, 40.0], 0)
}

fn drop200() -> Room {
    // A block 200 up that ends at x = 50, the player on top of it.
    Room::floor().plus([-4000.0, -4000.0, 0.0], [50.0, 4000.0, 200.0], 0)
}

fn ladder_face() -> Room {
    // A 300-tall ladder face at x = 30.
    Room::floor().plus([30.0, -40.0, 0.0], [34.0, 40.0, 300.0], movement_t6::bridge::FLOOR_IS_RUNGS)
}

fn runs() -> Vec<Run> {
    use key::*;
    use playerstate_t6::game::{MOVE_MODE_DOWNED, MOVE_MODE_INTERLUDE, MOVE_MODE_WALK_ATTACHED, MOVE_KIND_DEAD, entity_flag_bits, movement_state_bits};
    vec![
        Run::new("stand still", 40, |_| (0, 0, 0, 0.0)),
        Run::new("walk forward", 40, |_| (127, 0, 0, 0.0)),
        Run::new("walk 60fps", 120, |_| (127, 0, 0, 0.0)).msec(16),
        Run::new("strafe right", 40, |_| (0, 127, 0, 0.0)),
        Run::new("diagonal", 40, |_| (127, 127, 0, 0.0)),
        Run::new("back", 40, |_| (-127, 0, 0, 0.0)),
        Run::new("walk then stop", 40, |i| (if i < 20 { 127 } else { 0 }, 0, 0, 0.0)),
        Run::new("sprint", 80, |_| (127, 0, SPRINT, 0.0)),
        Run::new("sprint strafe", 40, |_| (127, 127, SPRINT, 0.0)),
        Run::new("jump standing", 30, |i| (0, 0, if i == 2 { LEAP } else { 0 }, 0.0)),
        Run::new("running jump", 40, |i| (127, 0, if i == 10 { LEAP } else { 0 }, 0.0)),
        Run::new("jump twice", 60, |i| (127, 0, if matches!(i, 10 | 30) { LEAP } else { 0 }, 0.0)),
        Run::new("crouch walk", 40, |_| (127, 0, DUCK_BUTTON_BIT, 0.0)),
        Run::new("prone crawl", 80, |i| (if i > 30 { 127 } else { 0 }, 0, if i == 0 { 0 } else { LYING_FLAT }, 0.0)),
        Run::new("dive", 60, |i| (127, 0, if i < 15 { SPRINT } else { LYING_FLAT }, 0.0)),
        Run::new("wall at an angle", 40, |_| (127, 0, 0, 30.0)).room(wall),
        Run::new("step up 16", 30, |_| (127, 0, 0, 0.0)).room(step16),
        Run::new("ledge 40 + jump", 40, |i| (127, 0, if i == 8 { LEAP } else { 0 }, 0.0)).room(ledge40),
        Run::new("walk off 200", 40, |_| (127, 0, 0, 0.0)).room(drop200).at([0.0, 0.0, 200.0]),
        Run::new("turning walk", 60, |i| (127, 0, 0, i as f32 * 6.0)),
        Run::new("ladder", 60, |_| (127, 0, 0, 0.0)).room(ladder_face),
        Run::new("dead slide", 20, |_| (127, 0, 0, 0.0)).prep(|avatar| {
            avatar.movement_mode = MOVE_KIND_DEAD;
            avatar.velocity = [300.0, 0.0, 0.0];
        }),
        Run::new("last stand crawl", 40, |_| (127, 0, 0, 0.0)).prep(|avatar| {
            avatar.movement_mode = MOVE_MODE_DOWNED;
            avatar.movement_state_bits = movement_state_bits::LYING_FLAT;
        }),
        Run::new("linked", 20, |_| (127, 0, 0, 0.0)).at([0.0, 0.0, 50.0]).prep(|avatar| {
            avatar.movement_mode = MOVE_MODE_WALK_ATTACHED;
            avatar.velocity = [100.0, 0.0, 0.0];
        }),
        Run::new("on a turret", 20, |_| (127, 0, 0, 0.0)).prep(|avatar| avatar.e_flags = entity_flag_bits::TURRET_STAND),
        Run::new("end of match", 20, |_| (127, 0, 0, 0.0)).prep(|avatar| {
            avatar.movement_mode = MOVE_MODE_INTERLUDE;
            avatar.velocity = [100.0, 0.0, 0.0];
        }),
        Run::new("noclip", 20, |_| (127, 0, 0, 0.0))
            .at([0.0, 0.0, 50.0])
            .prep(|avatar| avatar.movement_mode = movement_t6::bridge::ufo::PM_TYPE_NOCLIP),
    ]
}

#[test]
fn bench() {
    let trace_name = std::env::var("BENCH_TRACE").ok();
    println!("{:<18} {:>16} {:>8} {:>6} {:>6}", "scenario", "end x,z", "max_vel", "ground", "view");
    for run in runs() {
        let frames = run.play();
        let mut fastest = 0.0_f32;
        let mut grounded = 0;
        for (spot, motion_vec, _, ground, _) in &frames {
            let finite = spot.iter().chain(motion_vec.iter()).all(|v| v.is_finite());
            assert!(finite, "{}: not finite", run.name);
            let speed = (motion_vec[0].powi(2) + motion_vec[1].powi(2) + motion_vec[2].powi(2)).sqrt();
            fastest = fastest.max(speed);
            grounded += usize::from(*ground);
        }
        let (end, view) = match frames.last() {
            Some((o, _, _, _, vh)) => (format!("{:.1},{:.1}", o[0], o[2]), *vh),
            None => (String::new(), 0.0),
        };
        println!("{:<18} {:>16} {:>8.2} {:>6} {:>6.1}", run.name, end, fastest, grounded, view);
        if trace_name.as_deref() != Some(run.name) {
            continue;
        }
        for (i, (o, v, f, g, vh)) in frames.iter().enumerate() {
            println!("  {i:>3} o={o:?} v={v:?} f={f:#x} g={g} vh={vh:.1}");
        }
    }
}

/// BO2 runs the aim, sprint, walking and stance updates a second time on the
/// frame a climb starts, now off the ground: the aim lets go that frame (no
/// aiming while climbing), not a frame later.
#[test]
fn the_frame_a_climb_starts_lets_go_of_the_aim() {
    use playerstate_t6::game::movement_state_bits::{ADS_INTENT, MANTLE, WALKING};
    // The 40-unit ledge with BO2's mantle-on bit on its faces.
    let room = Room::floor().plus([60.0, -4000.0, 0.0], [4000.0, 4000.0, 40.0], 0x0200_0000);
    let mut d = Driver::new(&room, fresh_player());
    let started = (0..60).find_map(|i| {
        let jump = if d.avatar.origin[0] >= 25.0 { key::LEAP } else { 0 };
        let aiming_before = d.avatar.movement_state_bits & ADS_INTENT != 0;
        // The sim refuses the aim once the player is climbing (PM_IsAdsAllowed).
        let climbing = d.avatar.movement_state_bits & MANTLE != 0;
        d.step_with(50, 127, 0, key::ADS | jump, 0.0, |ctx| ctx.ads_intent.sights_permitted = !climbing);
        (d.avatar.movement_state_bits & MANTLE != 0).then_some((i, aiming_before))
    });
    let Some((frame, aiming_before)) = started else { panic!("the climb never started") };
    assert!(aiming_before, "frame {frame}: not aiming before the climb");
    let left = d.avatar.movement_state_bits & (ADS_INTENT | WALKING);
    assert_eq!(left, 0, "frame {frame}: {:#x}", d.avatar.movement_state_bits);
}

/// Crouched or prone, then holding sprint from frame 5. The client's stance
/// latch is kept as the game keeps it: the toggle sets it, and the predicted
/// stance events (BO2's 8 stand, 9 crouch, 10 prone) overwrite it before the
/// next move is built, one frame after they are raised.
fn crouched_then_sprint(prone: bool) -> Vec<(u32, f32)> {
    use entity_t6::event::{STANCE_FORCE_CROUCH, STANCE_FORCE_PRONE, STANCE_FORCE_STAND};
    use playerstate_t6::game::movement_state_bits;
    let room = Room::floor();
    let mut avatar = fresh_player();
    let mut latch = if prone { key::LYING_FLAT } else { key::DUCK_BUTTON_BIT };
    avatar.movement_state_bits = if prone { movement_state_bits::LYING_FLAT } else { movement_state_bits::DUCK_BUTTON_BIT };
    avatar.eye_height_goal = if prone { 11 } else { 40 };
    avatar.eye_height_now = avatar.eye_height_goal as f32;
    let mut d = Driver::new(&room, avatar);
    let mut receipt_confirmed = d.avatar.predictable_event_sequence;
    // Events raised last frame, oldest first; they land before this frame's move.
    let mut landing: Vec<i32> = Vec::new();
    let mut out = Vec::with_capacity(30);
    for i in 0..30 {
        for ev in landing.drain(..) {
            match u8::try_from(ev) {
                Ok(STANCE_FORCE_STAND) => latch = 0,
                Ok(STANCE_FORCE_CROUCH) => latch = key::DUCK_BUTTON_BIT,
                Ok(STANCE_FORCE_PRONE) => latch = key::LYING_FLAT,
                _ => {}
            }
        }
        let sprint = if i < 5 { 0 } else { key::SPRINT };
        d.step(16, 127, 0, latch | sprint, 0.0);
        let seq = d.avatar.predictable_event_sequence;
        let new_count = seq.wrapping_sub(receipt_confirmed).clamp(0, 4);
        landing.extend((1..=new_count).rev().map(|back| d.avatar.predictable_events[(seq.wrapping_sub(back) & 3) as usize]));
        receipt_confirmed = seq;
        out.push((d.avatar.movement_state_bits, d.avatar.eye_height_now));
    }
    out
}

#[test]
fn crouched_the_sprint_key_stands_and_sprints_at_once() {
    use playerstate_t6::game::movement_state_bits::{DUCK_BUTTON_BIT, LYING_FLAT, SPRINTING};
    for prone in [false, true] {
        let frames = crouched_then_sprint(prone);
        for (i, &(flags, view)) in frames.iter().enumerate().skip(5) {
            let sprinting_upright = flags & SPRINTING != 0 && flags & (DUCK_BUTTON_BIT | LYING_FLAT) == 0;
            assert!(sprinting_upright, "prone {prone} frame {i}: flags {flags:#x} view {view}");
        }
    }
}

/// One dive at full sprint, 10 ms frames: sprint 750 ms, press prone and
/// hold it, let it go at `release_ms` after the press. Returns, per frame,
/// (time since the press, origin, movement bits, view height).
fn dive_then_get_up(room: &Room, release_ms: i32) -> Vec<(i32, [f32; 3], u32, f32)> {
    let mut d = Driver::new(room, fresh_player());
    let mut out = Vec::new();
    for i in 0..400 {
        let t = (i - 75) * 10;
        let held = if t < 0 { key::SPRINT } else if t < release_ms { key::LYING_FLAT } else { 0 };
        d.step(10, 127, 0, held, 0.0);
        out.push((t, d.avatar.origin, d.avatar.movement_state_bits, d.avatar.eye_height_now));
    }
    out
}

/// The dive's times and heights beside BO2's own (bg_dtp.cpp and
/// PM_GetViewHeightLerpTime): the take-off rises to a jump's height (39) and
/// hangs there until 400 ms after take-off (dtp_max_apex_duration), the slide
/// lasts 300 ms from touch-down (dtp_max_slide_duration), and getting up from
/// prone takes 400 ms to crouch and 200 ms more to stand, 600 ms in all.
/// Run with `--nocapture` for the numbers.
#[test]
fn a_dive_rises_slides_and_gets_up_at_bo2s_times() {
    use movement_t6::dive::{PMF_DIVE, PMF_DIVE_PRONE};
    let room = Room::floor();
    let frames = dive_then_get_up(&room, 1500);
    let first = |pred: &dyn Fn(&(i32, [f32; 3], u32, f32)) -> bool| frames.iter().find(|f| pred(f)).map(|f| f.0);
    let takeoff = first(&|f| f.2 & PMF_DIVE != 0).expect("no dive");
    let apex = frames.iter().map(|f| f.1[2]).fold(0.0_f32, f32::max);
    // While the dive hangs, each frame starts at rest, so it sinks the same
    // small step every frame; the fall begins on the first frame whose drop
    // is bigger than the one before.
    let drops: Vec<(i32, f32)> = frames.windows(2).filter(|w| w[1].0 > takeoff + 100).map(|w| (w[1].0, w[0].1[2] - w[1].1[2])).collect();
    let falls = drops.windows(2).find(|w| w[1].1 > w[0].1 + 0.01).map(|w| w[1].0).expect("no fall");
    // The body rests a hair above the floor (the ground trace's gap).
    let touchdown = first(&|f| f.0 > takeoff && f.1[2] <= 0.2).expect("no touch-down");
    let slide_end = first(&|f| f.2 & PMF_DIVE_PRONE != 0).expect("no slide end");
    let lying = first(&|f| f.3 <= 11.0).expect("never lay flat");
    let crouched = first(&|f| f.0 >= 1500 && f.3 >= 40.0).expect("never crouched");
    let standing = first(&|f| f.0 >= 1500 && f.3 >= 60.0).expect("never stood");
    println!("dive (ours vs BO2):");
    println!("  take-off at {takeoff} ms after the prone press");
    println!("  apex {apex:.1} units (BO2: jump height 39)");
    println!("  holds the apex until {} ms after take-off (BO2: 400)", falls - takeoff);
    println!("  touch-down {} ms after take-off", touchdown - takeoff);
    println!("  slide {} ms from touch-down (BO2: up to 300)", slide_end - touchdown);
    println!("  view flat (11) {} ms after take-off", lying - takeoff);
    println!("  get-up: crouch view at +{} ms (BO2: 400), standing at +{} ms (BO2: 600)", crouched - 1500, standing - 1500);
    // Capped at a jump's height from the start of each frame, so the
    // take-off frame that crosses it overshoots by up to one frame's rise.
    assert!((39.0..=39.0 + 5.0).contains(&apex), "apex {apex}");
    assert!((390..=410).contains(&(falls - takeoff)), "apex hold {}", falls - takeoff);
    assert!((290..=310).contains(&(slide_end - touchdown)), "slide {}", slide_end - touchdown);
    assert!((390..=410).contains(&(crouched - 1500)), "crouch {}", crouched - 1500);
    assert!((590..=610).contains(&(standing - 1500)), "stand {}", standing - 1500);
}

/// Diving at a low box: one under a jump's height (39) is dived over, as in
/// BO2, where the dive rises to that height and hangs there; a taller one
/// stops the dive at its face.
#[test]
fn a_dive_goes_over_a_low_box_and_not_a_tall_one() {
    // The player reaches about x = 196 at the prone press and is at the
    // dive's top by x = 222; the box is 30 deep.
    for (top, over) in [(24.0_f32, true), (36.0, true), (48.0, false)] {
        let room = Room::floor().plus([240.0, -4000.0, 0.0], [270.0, 4000.0, top], 0);
        let frames = dive_then_get_up(&room, 1500);
        let at_press = frames.iter().find(|f| f.0 == 0).map(|f| f.1[0]).unwrap_or_default();
        let furthest = frames.iter().map(|f| f.1[0]).fold(f32::MIN, f32::max);
        let highest = frames.iter().filter(|f| f.1[0] > 230.0 && f.1[0] < 280.0).map(|f| f.1[2]).fold(0.0_f32, f32::max);
        println!("box {top} tall at x 240..270: press at x {at_press:.0}, furthest x {furthest:.0}, highest over it {highest:.1}");
        assert_eq!(furthest > 270.0, over, "box {top}: furthest {furthest}");
    }
}
