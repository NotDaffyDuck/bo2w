//! The bench: Black Ops II's own frame (`movement_t6`) on scripted keys in a
//! small world of boxes. Run with
//! `cargo test -p movement_t6 --test compare -- --nocapture` for the table.
//! (It used to set the MW2 frame beside it; that frame is gone.)

use movement_t6::bridge::{
    AdsFracContext, AdsIntentContext, AirMoveContext, Bo2Feel, CmdScaleWalkContext,
    CollisionBackend, FlatMantleAnimLength, GroundTraceInput, JumpLaunchContext,
    MeleeChargeWeaponDelays, MoveBounds, PRONE_LERP_MS, PmoveSingleContext, SprintContext,
    ViewAngleClamp, WalkMoveContext, ZeroMantleRootDelta,
};
use playerstate_t6::game::{PlayerState, UserCmd};
use trace_t6::Trace;

const SPRINT: u32 = 0x2;
const PRONE: u32 = 0x100;
const CROUCH: u32 = 0x200;
const JUMP: u32 = 0x400;

/// The trace stops this far short of what it hits (the engine's own gap).
const CLIP_EPSILON: f32 = 0.125;

/// A solid box, with the surface flags its faces report.
#[derive(Clone, Copy)]
struct Solid {
    mins: [f32; 3],
    maxs: [f32; 3],
    surface_flags: u32,
}

struct World {
    solids: Vec<Solid>,
}

impl World {
    /// A floor at z = 0 out to 4000 units.
    fn floor() -> Self {
        Self {
            solids: vec![Solid {
                mins: [-4000.0, -4000.0, -64.0],
                maxs: [4000.0, 4000.0, 0.0],
                surface_flags: 0,
            }],
        }
    }

    fn with(mut self, mins: [f32; 3], maxs: [f32; 3], surface_flags: u32) -> Self {
        self.solids.push(Solid {
            mins,
            maxs,
            surface_flags,
        });
        self
    }
}

impl movement_t6::Bo2Collision for World {}

impl CollisionBackend for World {
    fn trace(&self, input: GroundTraceInput) -> Trace {
        let delta = [
            input.end[0] - input.start[0],
            input.end[1] - input.start[1],
            input.end[2] - input.start[2],
        ];
        let length = (delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]).sqrt();
        let mut best = Trace {
            fraction: 1.0,
            ..Trace::default()
        };
        let mut start_solid = false;
        let mut all_solid = false;
        for solid in &self.solids {
            // The box grown by the player's own box, swept as a point.
            let lo = [
                solid.mins[0] - input.maxs[0],
                solid.mins[1] - input.maxs[1],
                solid.mins[2] - input.maxs[2],
            ];
            let hi = [
                solid.maxs[0] - input.mins[0],
                solid.maxs[1] - input.mins[1],
                solid.maxs[2] - input.mins[2],
            ];
            let inside = |p: [f32; 3]| (0..3).all(|i| p[i] > lo[i] && p[i] < hi[i]);
            if inside(input.start) {
                start_solid = true;
                if inside(input.end) {
                    all_solid = true;
                }
                continue;
            }
            let mut enter = -1.0_f32;
            let mut leave = 1.0_f32;
            let mut axis = 3;
            let mut sign = 0.0;
            let mut missed = false;
            for i in 0..3 {
                if delta[i] == 0.0 {
                    if input.start[i] <= lo[i] || input.start[i] >= hi[i] {
                        missed = true;
                        break;
                    }
                    continue;
                }
                let (t0, t1, s) = if delta[i] > 0.0 {
                    ((lo[i] - input.start[i]) / delta[i], (hi[i] - input.start[i]) / delta[i], -1.0)
                } else {
                    ((hi[i] - input.start[i]) / delta[i], (lo[i] - input.start[i]) / delta[i], 1.0)
                };
                if t0 > enter {
                    enter = t0;
                    axis = i;
                    sign = s;
                }
                leave = leave.min(t1);
            }
            if missed || axis == 3 || enter > leave || !(0.0..1.0).contains(&enter) {
                continue;
            }
            let fraction = ((enter * length - CLIP_EPSILON) / length).max(0.0);
            if fraction < best.fraction {
                let mut normal = [0.0; 3];
                normal[axis] = sign;
                best = Trace {
                    fraction,
                    normal,
                    sflags: solid.surface_flags,
                    cflags: 1,
                    hit_type: trace_t6::TraceHitType::Entity,
                    hit_id: trace_t6::ENTITYNUM_WORLD,
                    walkable: normal[2] >= 0.7,
                    ..Trace::default()
                };
            }
        }
        if start_solid {
            best.startsolid = true;
            best.allsolid = all_solid;
            if all_solid {
                best.fraction = 0.0;
            }
        }
        best
    }
}

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
        jump_height: 39.0,
        ..Bo2Feel::BO2_MP
    }
}

fn context(old_buttons: u32, old_move: [i8; 2]) -> PmoveSingleContext {
    let feel = bo2_feel();
    let air = AirMoveContext {
        player_spectate_speed_scale: 1.0,
        shellshock_gravity_scale: 1.0,
        shellshock_gravity_bias: 0.0,
    };
    PmoveSingleContext {
        walk: WalkMoveContext {
            cmd_scale: CmdScaleWalkContext {
                player_back_speed_scale: 0.7,
                player_strafe_speed_scale: 0.8,
                player_sprint_speed_scale: 1.5,
                player_last_stand_crawl_speed_scale: 0.15,
                weapon_move_speed_scale: 1.0,
                weapon_ads_move_speed_scale: 1.0,
                shellshock_movement_scale: 1.0,
                prone_lerp_ms: PRONE_LERP_MS,
            },
            weapon_move_scale: 1.0,
            old_buttons,
            jump: JumpLaunchContext {
                jump_height: 39.0,
                dive: false,
                crouch_jump_scale: 1.0,
                jump_ladder_push_vel: 128.0,
            },
            air,
            feel,
            lunge: None,
        },
        air,
        bounds: MoveBounds {
            mins: [-15.0, -15.0, 0.0],
            maxs: [15.0, 15.0, 70.0],
            tracemask: 0x0281_0011,
        },
        view_angles: ViewAngleClamp {
            pitch_up: 85.0,
            pitch_down: 85.0,
            unclamped_pitch_bit: false,
        },
        sprint: SprintContext {
            weapon_max_sprint_time: 4000,
            sprint_forever: false,
            min_sprint_time_seconds: 1.0,
            sprint_delay_seconds: 0.0,
            sprint_forward_minimum: 105,
            stand_up_clear: true,
            sprint_recharge_pause_seconds: 0.0,
        },
        ads_intent: AdsIntentContext {
            ads_allowed: true,
            weapon_def_scope: false,
            sprint_hold_ads: false,
        },
        ads_frac: AdsFracContext {
            aim_down_sight: false,
            ads_in_rate: 1.0 / 200.0,
            ads_out_rate: 1.0 / 200.0,
            rechamber_while_ads: true,
            ads_fire_only: false,
            ..AdsFracContext::default()
        },
        melee_charge: MeleeChargeWeaponDelays {
            melee_delay_ms: 0,
            melee_charge_delay_ms: 0,
        },
        player_melee_range: movement_t6::bridge::MELEE_CHARGE_PLAYER_MELEE_RANGE_DEFAULT,
        old_buttons,
        old_move,
        weapon_blocks_prone: false,
    }
}

fn player() -> PlayerState {
    let mut ps = PlayerState::ZERO;
    ps.gravity = 800;
    ps.speed = 190;
    ps.move_speed_scale_multiplier = 1.0;
    ps.ground_entity_num = 1022;
    ps.view_height_target = 60;
    ps.view_height_current = 60.0;
    ps.command_time = 1000;
    ps
}

/// One frame's keys: forward, right, buttons, yaw in degrees.
#[derive(Clone, Copy)]
struct Keys {
    forward: i8,
    right: i8,
    buttons: u32,
    yaw: f32,
}

const fn keys(forward: i8, right: i8, buttons: u32) -> Keys {
    Keys {
        forward,
        right,
        buttons,
        yaw: 0.0,
    }
}

struct Scenario {
    name: &'static str,
    world: fn() -> World,
    start: [f32; 3],
    frames: usize,
    msec: i32,
    keys: fn(usize) -> Keys,
    setup: fn(&mut PlayerState),
}

fn as_is(_: &mut PlayerState) {}

/// Where a run stood after each frame.
#[derive(Clone, Copy)]
struct Sample {
    origin: [f32; 3],
    velocity: [f32; 3],
    pm_flags: u32,
    ground: bool,
    view_height: f32,
}

type Frame = fn(&mut PlayerState, &mut UserCmd, PmoveSingleContext, &World);

fn new_frame(ps: &mut PlayerState, cmd: &mut UserCmd, context: PmoveSingleContext, world: &World) {
    movement_t6::pmove(
        ps,
        cmd,
        context,
        world,
        &FlatMantleAnimLength::default(),
        &ZeroMantleRootDelta,
    );
}

fn run(scenario: &Scenario, frame: Frame) -> Vec<Sample> {
    let world = (scenario.world)();
    let mut ps = player();
    ps.origin = scenario.start;
    (scenario.setup)(&mut ps);
    let mut old_buttons = 0;
    let mut old_move = [0_i8; 2];
    let mut out = Vec::with_capacity(scenario.frames);
    for i in 0..scenario.frames {
        let k = (scenario.keys)(i);
        let mut cmd = UserCmd {
            server_time: ps.command_time + scenario.msec,
            forwardmove: k.forward,
            rightmove: k.right,
            buttons: k.buttons,
            ..UserCmd::default()
        };
        cmd.angles[1] = ((k.yaw * 65536.0 / 360.0) as i32) & 0xffff;
        frame(&mut ps, &mut cmd, context(old_buttons, old_move), &world);
        old_buttons = cmd.buttons;
        old_move = [cmd.forwardmove, cmd.rightmove];
        out.push(Sample {
            origin: ps.origin,
            velocity: ps.velocity,
            pm_flags: ps.pm_flags,
            ground: ps.ground_entity_num != playerstate_t6::game::ENTITYNUM_NONE,
            view_height: ps.view_height_current,
        });
    }
    out
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

fn walled() -> World {
    // A wall across the path at x = 200, turned 30 degrees off square would
    // need a slanted box; a square wall the player meets at an angle does.
    World::floor().with([200.0, -4000.0, 0.0], [264.0, 4000.0, 200.0], 0)
}

fn stepped() -> World {
    // A 16-unit step at x = 100.
    World::floor().with([100.0, -4000.0, 0.0], [4000.0, 4000.0, 16.0], 0)
}

fn ledge() -> World {
    // A 40-unit ledge at x = 60 (a climb).
    World::floor().with([60.0, -4000.0, 0.0], [4000.0, 4000.0, 40.0], 0)
}

fn cliff() -> World {
    // Standing on a block 200 up that ends at x = 50.
    World::floor().with([-4000.0, -4000.0, 0.0], [50.0, 4000.0, 200.0], 0)
}

fn ladder() -> World {
    // A ladder face at x = 30, 300 tall.
    World::floor().with([30.0, -40.0, 0.0], [34.0, 40.0, 300.0], movement_t6::bridge::SURF_LADDER)
}

fn scenarios() -> Vec<Scenario> {
    vec![
        Scenario { name: "stand still", world: World::floor, start: [0.0; 3], frames: 40, msec: 50, keys: |_| keys(0, 0, 0), setup: as_is },
        Scenario { name: "walk forward", world: World::floor, start: [0.0; 3], frames: 40, msec: 50, keys: |_| keys(127, 0, 0), setup: as_is },
        Scenario { name: "walk 60fps", world: World::floor, start: [0.0; 3], frames: 120, msec: 16, keys: |_| keys(127, 0, 0), setup: as_is },
        Scenario { name: "strafe right", world: World::floor, start: [0.0; 3], frames: 40, msec: 50, keys: |_| keys(0, 127, 0), setup: as_is },
        Scenario { name: "diagonal", world: World::floor, start: [0.0; 3], frames: 40, msec: 50, keys: |_| keys(127, 127, 0), setup: as_is },
        Scenario { name: "back", world: World::floor, start: [0.0; 3], frames: 40, msec: 50, keys: |_| keys(-127, 0, 0), setup: as_is },
        Scenario { name: "walk then stop", world: World::floor, start: [0.0; 3], frames: 40, msec: 50, keys: |i| keys(if i < 20 { 127 } else { 0 }, 0, 0), setup: as_is },
        Scenario { name: "sprint", world: World::floor, start: [0.0; 3], frames: 80, msec: 50, keys: |_| keys(127, 0, SPRINT), setup: as_is },
        Scenario { name: "sprint strafe", world: World::floor, start: [0.0; 3], frames: 40, msec: 50, keys: |_| keys(127, 127, SPRINT), setup: as_is },
        Scenario { name: "jump standing", world: World::floor, start: [0.0; 3], frames: 30, msec: 50, keys: |i| keys(0, 0, if i == 2 { JUMP } else { 0 }), setup: as_is },
        Scenario { name: "running jump", world: World::floor, start: [0.0; 3], frames: 40, msec: 50, keys: |i| keys(127, 0, if i == 10 { JUMP } else { 0 }), setup: as_is },
        Scenario { name: "jump twice", world: World::floor, start: [0.0; 3], frames: 60, msec: 50, keys: |i| keys(127, 0, if i == 10 || i == 30 { JUMP } else { 0 }), setup: as_is },
        Scenario { name: "crouch walk", world: World::floor, start: [0.0; 3], frames: 40, msec: 50, keys: |_| keys(127, 0, CROUCH), setup: as_is },
        Scenario { name: "prone crawl", world: World::floor, start: [0.0; 3], frames: 80, msec: 50, keys: |i| keys(if i > 30 { 127 } else { 0 }, 0, if i >= 1 { PRONE } else { 0 }), setup: as_is },
        Scenario { name: "dive", world: World::floor, start: [0.0; 3], frames: 60, msec: 50, keys: |i| keys(127, 0, if i >= 15 { PRONE } else { SPRINT }), setup: as_is },
        Scenario { name: "wall at an angle", world: walled, start: [0.0; 3], frames: 40, msec: 50, keys: |_| Keys { forward: 127, right: 0, buttons: 0, yaw: 30.0 }, setup: as_is },
        Scenario { name: "step up 16", world: stepped, start: [0.0; 3], frames: 30, msec: 50, keys: |_| keys(127, 0, 0), setup: as_is },
        Scenario { name: "ledge 40 + jump", world: ledge, start: [0.0; 3], frames: 40, msec: 50, keys: |i| keys(127, 0, if i == 8 { JUMP } else { 0 }), setup: as_is },
        Scenario { name: "walk off 200", world: cliff, start: [0.0, 0.0, 200.0], frames: 40, msec: 50, keys: |_| keys(127, 0, 0), setup: as_is },
        Scenario { name: "turning walk", world: World::floor, start: [0.0; 3], frames: 60, msec: 50, keys: |i| Keys { forward: 127, right: 0, buttons: 0, yaw: i as f32 * 6.0 }, setup: as_is },
        Scenario { name: "ladder", world: ladder, start: [0.0; 3], frames: 60, msec: 50, keys: |_| keys(127, 0, 0), setup: as_is },
        Scenario { name: "dead slide", world: World::floor, start: [0.0; 3], frames: 20, msec: 50, keys: |_| keys(127, 0, 0), setup: |ps| { ps.pm_type = playerstate_t6::game::PM_TYPE_DEAD; ps.velocity = [300.0, 0.0, 0.0]; } },
        Scenario { name: "last stand crawl", world: World::floor, start: [0.0; 3], frames: 40, msec: 50, keys: |_| keys(127, 0, 0), setup: |ps| { ps.pm_type = playerstate_t6::game::PM_TYPE_LAST_STAND; ps.pm_flags = playerstate_t6::game::pm_flags::PRONE; } },
        Scenario { name: "linked", world: World::floor, start: [0.0, 0.0, 50.0], frames: 20, msec: 50, keys: |_| keys(127, 0, 0), setup: |ps| { ps.pm_type = playerstate_t6::game::PM_TYPE_NORMAL_LINKED; ps.velocity = [100.0, 0.0, 0.0]; } },
        Scenario { name: "on a turret", world: World::floor, start: [0.0; 3], frames: 20, msec: 50, keys: |_| keys(127, 0, 0), setup: |ps| { ps.e_flags = 0x400; } },
        Scenario { name: "end of match", world: World::floor, start: [0.0; 3], frames: 20, msec: 50, keys: |_| keys(127, 0, 0), setup: |ps| { ps.pm_type = playerstate_t6::game::PM_TYPE_INTERMISSION; ps.velocity = [100.0, 0.0, 0.0]; } },
        Scenario { name: "noclip", world: World::floor, start: [0.0, 0.0, 50.0], frames: 20, msec: 50, keys: |_| keys(127, 0, 0), setup: |ps| { ps.pm_type = movement_t6::bridge::ufo::PM_TYPE_NOCLIP; } },
    ]
}

#[test]
fn bench() {
    println!(
        "{:<18} {:>16} {:>8} {:>6} {:>6}",
        "scenario", "end x,z", "max_vel", "ground", "view"
    );
    for scenario in scenarios() {
        let new = run(&scenario, new_frame);
        let mut max_vel = 0.0_f32;
        for b in &new {
            assert!(b.origin.iter().chain(&b.velocity).all(|v| v.is_finite()), "{}: not finite", scenario.name);
            max_vel = max_vel.max(distance([0.0; 3], b.velocity));
        }
        let end = new.last().map_or(String::new(), |s| format!("{:.1},{:.1}", s.origin[0], s.origin[2]));
        let ground = new.iter().filter(|s| s.ground).count();
        let view = new.last().map_or(0.0, |s| s.view_height);
        println!("{:<18} {:>16} {:>8.2} {:>6} {:>6.1}", scenario.name, end, max_vel, ground, view);
        if std::env::var("BENCH_TRACE").is_ok_and(|v| v == scenario.name) {
            for (i, b) in new.iter().enumerate() {
                println!(
                    "  {i:>3} o={:?} v={:?} f={:#x} g={} vh={:.1}",
                    b.origin, b.velocity, b.pm_flags, b.ground, b.view_height
                );
            }
        }
    }
}

/// BO2 runs the aim, sprint, walking and stance updates a second time on the
/// frame a climb starts, now off the ground: the aim lets go that frame (no
/// aiming while climbing), not a frame later.
#[test]
fn the_frame_a_climb_starts_lets_go_of_the_aim() {
    use playerstate_t6::game::pm_flags::{ADS_INTENT, MANTLE, WALKING};
    const ADS: u32 = 0x800;
    // The 40-unit ledge, its face climbable onto (BO2's mantle-on bit).
    let world = World::floor().with([60.0, -4000.0, 0.0], [4000.0, 4000.0, 40.0], 0x0200_0000);
    let mut ps = player();
    let mut old_buttons = 0;
    let mut old_move = [0_i8; 2];
    let mut climb = None;
    for i in 0..60 {
        let buttons = ADS | if ps.origin[0] >= 25.0 { JUMP } else { 0 };
        let mut cmd = UserCmd {
            server_time: ps.command_time + 50,
            forwardmove: 127,
            buttons,
            ..UserCmd::default()
        };
        // The sim refuses the aim once the player is climbing (PM_IsAdsAllowed).
        let mut ctx = context(old_buttons, old_move);
        ctx.ads_intent.ads_allowed = ps.pm_flags & MANTLE == 0;
        let aimed = ps.pm_flags & ADS_INTENT != 0;
        new_frame(&mut ps, &mut cmd, ctx, &world);
        old_buttons = cmd.buttons;
        old_move = [cmd.forwardmove, cmd.rightmove];
        if ps.pm_flags & MANTLE != 0 {
            climb = Some((i, aimed));
            break;
        }
    }
    let (frame, aimed) = climb.expect("the climb never started");
    assert!(aimed, "frame {frame}: not aiming before the climb");
    assert_eq!(ps.pm_flags & (ADS_INTENT | WALKING), 0, "frame {frame}: {:#x}", ps.pm_flags);
}

/// The client's stance latch as the game keeps it: the crouch toggle sets
/// it, and the predicted state's stance events (BO2's 8 stand, 9 crouch,
/// 10 prone) overwrite it before the next move is built, as BO2's client does.
fn down_then_sprint(prone: bool) -> Vec<(u32, f32)> {
    const SPRINT: u32 = 0x2;
    const CROUCH: u32 = 0x200;
    const PRONE: u32 = 0x100;
    let world = World::floor();
    let mut ps = player();
    let (flags, view, mut latch) = if prone {
        (playerstate_t6::game::pm_flags::PRONE, 11, PRONE)
    } else {
        (playerstate_t6::game::pm_flags::CROUCH, 40, CROUCH)
    };
    ps.pm_flags = flags;
    ps.view_height_target = view;
    ps.view_height_current = view as f32;
    let mut seen = ps.predictable_event_sequence;
    let mut pending: Vec<(usize, i32)> = Vec::new();
    let mut old_buttons = 0;
    let mut old_move = [0_i8; 2];
    let mut out = Vec::new();
    for i in 0..30 {
        for &(_, ev) in pending.iter().filter(|(at, _)| *at <= i) {
            use entity_t6::event::{STANCE_FORCE_CROUCH, STANCE_FORCE_PRONE, STANCE_FORCE_STAND};
            latch = match u8::try_from(ev) {
                Ok(STANCE_FORCE_STAND) => 0,
                Ok(STANCE_FORCE_CROUCH) => CROUCH,
                Ok(STANCE_FORCE_PRONE) => PRONE,
                _ => latch,
            };
        }
        pending.retain(|(at, _)| *at > i);
        let mut cmd = UserCmd {
            server_time: ps.command_time + 16,
            forwardmove: 127,
            buttons: latch | if i >= 5 { SPRINT } else { 0 },
            ..UserCmd::default()
        };
        new_frame(&mut ps, &mut cmd, context(old_buttons, old_move), &world);
        old_buttons = cmd.buttons;
        old_move = [cmd.forwardmove, cmd.rightmove];
        let events = [ps.predictable_events[0], ps.predictable_events[1], ps.predictable_events[2], ps.predictable_events[3]];
        let fresh = ps.predictable_event_sequence.wrapping_sub(seen).clamp(0, 4);
        for age in (0..fresh).rev() {
            let seq = ps.predictable_event_sequence.wrapping_sub(1 + age);
            pending.push((i + 1, events[(seq & 3) as usize]));
        }
        seen = ps.predictable_event_sequence;
        out.push((ps.pm_flags, ps.view_height_current));
    }
    out
}

#[test]
fn crouched_the_sprint_key_stands_and_sprints_at_once() {
    use playerstate_t6::game::pm_flags::{CROUCH, PRONE, SPRINTING};
    for prone in [false, true] {
        for (i, (flags, view)) in down_then_sprint(prone).iter().enumerate().skip(5) {
            assert!(
                flags & SPRINTING != 0 && flags & (CROUCH | PRONE) == 0,
                "prone {prone} frame {i}: flags {flags:#x} view {view}"
            );
        }
    }
}
