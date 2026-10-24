//! bo2mp: walk Black Ops II's own movement over a real map, headless and
//! read-only. `t6walk [zone.ff] [frames]` (default Nuketown 2025, 300
//! frames): from each spawn point and four headings it runs the same keys
//! (walk, sprint, jumps, a crouch, a slow turn) twice, on Black Ops II's own
//! collision and on the box traces, then reports the time per move, the runs
//! that fell out of the world or got stuck, and how far the two ended apart.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use asset_t6::{capture_zone, parse_entities};
use movement_t6::Bo2Collision;
use movement_t6::bridge::CollisionBackend;
use movement_t6::bridge::{
    AdsFracContext, AdsIntentContext, AirMoveContext, Bo2Feel, CmdScaleWalkContext,
    FlatMantleAnimLength, GroundTraceInput, JumpLaunchContext,
    MeleeChargeWeaponDelays, MoveBounds, PRONE_LERP_MS, PmoveSingleContext, SprintContext,
    ViewAngleClamp, WalkMoveContext, ZeroMantleRootDelta,
};
use playerstate_t6::game::{ENTITYNUM_NONE, PlayerState, UserCmd};

const DEFAULT_ZONE: &str =
    r"C:\Program Files (x86)\Steam\steamapps\common\Call of Duty Black Ops II\zone\all\mp_nuketown_2020.ff";

const SPRINT: u32 = 0x2;
const CROUCH: u32 = 0x200;
const JUMP: u32 = 0x400;
/// One move's length: a 60 fps client.
const MSEC: i32 = 16;
const PLAYER_MINS: [f32; 3] = [-15.0, -15.0, 0.0];
const PLAYER_MAXS: [f32; 3] = [15.0, 15.0, 70.0];
const PLAYER_MASK: u32 = 0x0281_0011;
/// Spawn points walked, at most (spread over the map's list).
const MAX_SPAWNS: usize = 48;

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
            mins: PLAYER_MINS,
            maxs: PLAYER_MAXS,
            tracemask: PLAYER_MASK,
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

fn player(origin: [f32; 3]) -> PlayerState {
    let mut ps = PlayerState::ZERO;
    ps.gravity = 800;
    ps.speed = 190;
    ps.move_speed_scale_multiplier = 1.0;
    ps.ground_entity_num = ENTITYNUM_NONE;
    ps.view_height_target = 60;
    ps.view_height_current = 60.0;
    ps.command_time = 1000;
    ps.origin = origin;
    ps
}

/// Frame `i`'s keys: forward, right, buttons, and how far the view turns.
fn keys(i: usize) -> (i8, i8, u32, f32) {
    let sprint = if (20..120).contains(&i) { SPRINT } else { 0 };
    let jump = if matches!(i, 60 | 61 | 150 | 151 | 240 | 241) { JUMP } else { 0 };
    let crouch = if (170..190).contains(&i) { CROUCH } else { 0 };
    let right = if (200..230).contains(&i) { 127 } else { 0 };
    (127, right, sprint | jump | crouch, 0.6)
}

/// How one run went.
struct Run {
    end: [f32; 3],
    grounded: bool,
    /// Under the world: nothing solid below the player.
    out: bool,
    /// Moved less than a body's width in the whole run.
    stuck: bool,
    /// Started inside something.
    start_solid: bool,
    /// How far above the ground the run ended, and its speed then.
    height: f32,
    velocity: [f32; 3],
    nanos: Vec<u32>,
}

fn walk(world: &impl movement_t6::Bo2Collision, start: [f32; 3], yaw: f32, frames: usize) -> Run {
    walk_path(world, start, yaw, frames, None)
}

fn walk_path(
    world: &impl movement_t6::Bo2Collision,
    start: [f32; 3],
    yaw: f32,
    frames: usize,
    mut path: Option<&mut Vec<[f32; 3]>>,
) -> Run {
    let mut ps = player(start);
    let mut yaw = yaw;
    let (mut old_buttons, mut old_move) = (0, [0_i8; 2]);
    let mut nanos = Vec::with_capacity(frames);
    let mut travelled = 0.0_f32;
    for i in 0..frames {
        let (forward, right, buttons, turn) = keys(i);
        yaw += turn;
        let mut cmd = UserCmd {
            server_time: ps.command_time + MSEC,
            forwardmove: forward,
            rightmove: right,
            buttons,
            ..UserCmd::default()
        };
        cmd.angles[1] = ((yaw * 65536.0 / 360.0) as i32) & 0xffff;
        let before = ps.origin;
        let clock = Instant::now();
        movement_t6::pmove(
            &mut ps,
            &mut cmd,
            context(old_buttons, old_move),
            world,
            &FlatMantleAnimLength::default(),
            &ZeroMantleRootDelta,
        );
        nanos.push(u32::try_from(clock.elapsed().as_nanos()).unwrap_or(u32::MAX));
        travelled += (ps.origin[0] - before[0]).hypot(ps.origin[1] - before[1]);
        old_buttons = cmd.buttons;
        old_move = [cmd.forwardmove, cmd.rightmove];
        if let Some(path) = path.as_deref_mut() {
            path.push(ps.origin);
        }
        if !ps.origin.iter().all(|v| v.is_finite()) {
            break;
        }
    }
    let down = world.trace(GroundTraceInput {
        start: ps.origin,
        end: [ps.origin[0], ps.origin[1], ps.origin[2] - 16384.0],
        mins: PLAYER_MINS,
        maxs: PLAYER_MAXS,
        tracemask: PLAYER_MASK,
    });
    let at_start = world.trace(GroundTraceInput {
        start: start,
        end: start,
        mins: PLAYER_MINS,
        maxs: PLAYER_MAXS,
        tracemask: PLAYER_MASK,
    });
    Run {
        end: ps.origin,
        grounded: ps.ground_entity_num != ENTITYNUM_NONE,
        out: !ps.origin.iter().all(|v| v.is_finite()) || (down.fraction >= 1.0 && !down.startsolid),
        stuck: travelled < 30.0,
        start_solid: at_start.startsolid,
        height: down.fraction * 16384.0,
        velocity: ps.velocity,
        nanos,
    }
}

fn percentile(sorted: &[u32], p: f32) -> f32 {
    if sorted.is_empty() {
        return 0.0;
    }
    let i = ((sorted.len() - 1) as f32 * p).round() as usize;
    sorted[i] as f32 / 1000.0
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1).peekable();
    let path = match args.peek() {
        Some(a) if a.ends_with(".ff") => PathBuf::from(args.next().unwrap_or_default()),
        _ => PathBuf::from(DEFAULT_ZONE),
    };
    let frames: usize = args.next().and_then(|a| a.parse().ok()).unwrap_or(300);
    let loaded = Instant::now();
    let capture = match capture_zone(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("t6walk: {e}");
            return ExitCode::FAILURE;
        }
    };
    let Some(clip_ref) = capture.clip.as_ref() else {
        eprintln!("t6walk: {} has no clipMap", path.display());
        return ExitCode::FAILURE;
    };
    let clip = match asset_world::build_t6_clip_collision(clip_ref) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("t6walk: collision: {e:?}");
            return ExitCode::FAILURE;
        }
    };
    let brushes: Vec<sim::SimBrush> = clip
        .brushes
        .iter()
        .map(|b| sim::SimBrush {
            planes: b.planes.clone(),
            contents: b.contents,
            plane_surface_flags: b.plane_surface_flags.clone(),
            glass_encoded: b.glass_encoded,
            verts: b.verts.clone(),
            hit_surface_flags: b.hit_surface_flags,
        })
        .collect();
    let bsp = sim::SimClipBsp {
        leafbrushes: clip.leafbrushes.clone(),
        t6: clip.t6,
        t6_glass: sim::SimClipBsp::glass_of(&brushes),
    };
    let mesh = sim::SimClipMesh::default();
    let sided = brushes.iter().filter(|b| b.planes.len() > 6).count();
    let sided_bare = brushes.iter().filter(|b| b.planes.len() > 6 && b.verts.is_empty()).count();
    println!(
        "{}: {} brushes ({} with extra sides, {} of those with no corner points), {} terrain tris, {} nodes, {} leaves; loaded in {:.1}s",
        path.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned()),
        brushes.len(),
        sided,
        sided_bare,
        clip.t6.map_or(0, |m| m.tri_indices.len()),
        clip.nodes.len(),
        clip.leaves.len(),
        loaded.elapsed().as_secs_f32()
    );
    // Black Ops II's player mask (ClientThink_real) is MW2's 0x0281_0011 plus 0x8000.
    let with_bit = brushes.iter().filter(|b| b.contents & 0x8000 != 0).count();
    let only_bo2 = brushes
        .iter()
        .filter(|b| b.contents & 0x8000 != 0 && b.contents & 0x0281_0011 == 0)
        .count();
    println!("contents 0x8000: {with_bit} brushes, {only_bo2} solid to the player only by that bit");

    // The spawn points, one per spot, spread over the list.
    let mut spawns: Vec<([f32; 3], f32)> = Vec::new();
    for text in &capture.map_ents {
        for e in parse_entities(text) {
            let class = e.classname();
            if !(class == "info_player_start" || (class.starts_with("mp_") && class.contains("spawn"))) {
                continue;
            }
            let Some(o) = e.vec3("origin") else {
                continue;
            };
            let yaw = e.vec3("angles").map_or(0.0, |a| a[1]);
            if spawns.iter().all(|(s, _)| (0..3).any(|a| (s[a] - o[a]).abs() > 8.0)) {
                spawns.push((o, yaw));
            }
        }
    }
    if spawns.is_empty() {
        eprintln!("t6walk: no spawn points in the map's entities");
        return ExitCode::FAILURE;
    }
    let step = spawns.len().div_ceil(MAX_SPAWNS);
    let spawns: Vec<_> = spawns.into_iter().step_by(step).collect();
    println!("{} spawn points x 4 headings x {frames} moves of {MSEC} ms", spawns.len());

    let gjk_world = sim::bo2_map_collision(&brushes, &bsp, &mesh, true);
    let box_world = sim::bo2_map_collision(&brushes, &bsp, &mesh, false);

    // `T6WALK_TRACE=x,y,top,bottom`: box traces straight down there, a point
    // and the player's box, and what they hit.
    if let Ok(probe) = std::env::var("T6WALK_TRACE") {
        let v: Vec<f32> = probe.split(',').filter_map(|v| v.trim().parse().ok()).collect();
        let [x, y, top, bottom] = v[..] else {
            eprintln!("t6walk: T6WALK_TRACE=x,y,top,bottom");
            return ExitCode::FAILURE;
        };
        for (name, mins, maxs) in [("point", [0.0; 3], [0.0; 3]), ("player", PLAYER_MINS, PLAYER_MAXS)] {
            let t = box_world.trace(GroundTraceInput {
                start: [x, y, top],
                end: [x, y, bottom],
                mins,
                maxs,
                tracemask: PLAYER_MASK,
            });
            println!(
                "{name:<6} from {top} to {bottom}: fraction {:.4} hits z {:.1} normal {:?} startsolid {} allsolid {} entity {} contents {:#x} surface {:#x}",
                t.fraction,
                top + (bottom - top) * t.fraction,
                t.normal,
                t.startsolid,
                t.allsolid,
                t.hit_id,
                t.cflags,
                t.sflags
            );
        }
        return ExitCode::SUCCESS;
    }

    // `T6WALK_PROBE=spawn,heading`: walk that one run on both, print the two
    // paths and the brushes around where the box run stopped.
    if let Ok(probe) = std::env::var("T6WALK_PROBE") {
        let mut it = probe.split(',').filter_map(|v| v.trim().parse::<usize>().ok());
        let (Some(n), Some(h)) = (it.next(), it.next()) else {
            eprintln!("t6walk: T6WALK_PROBE=spawn,heading");
            return ExitCode::FAILURE;
        };
        let Some(&(origin, yaw)) = spawns.get(n) else {
            eprintln!("t6walk: no spawn {n}");
            return ExitCode::FAILURE;
        };
        let start = [origin[0], origin[1], origin[2] + 1.0];
        let heading = yaw + 90.0 * h as f32;
        let (mut a, mut b) = (Vec::new(), Vec::new());
        walk_path(&gjk_world, start, heading, frames, Some(&mut a));
        walk_path(&box_world, start, heading, frames, Some(&mut b));
        let every = it.next();
        let around_bo2 = it.next();
        for (i, (p, q)) in a.iter().zip(&b).enumerate() {
            if i % every.unwrap_or(5).max(1) == 0 {
                println!("{i:>4}  bo2 {:?}  box {:?}", p.map(|v| v.round()), q.map(|v| v.round()));
            }
        }
        // Where the box run stopped moving sideways for good.
        let stop = (1..b.len())
            .rev()
            .find(|&i| (b[i][0] - b[i - 1][0]).hypot(b[i][1] - b[i - 1][1]) > 0.5)
            .unwrap_or(0);
        let at = match around_bo2 {
            Some(f) => a.get(f).copied().unwrap_or(start),
            None => b.get(stop).copied().unwrap_or(start),
        };
        let reach = 40.0;
        let lo = [at[0] - reach, at[1] - reach, at[2] - 8.0];
        let hi = [at[0] + reach, at[1] + reach, at[2] + 80.0];
        match around_bo2 {
            Some(f) => println!("bo2 run at frame {f} {:?}; brushes within {reach}:", at.map(|v| v.round())),
            None => println!("box run stops moving at frame {stop} {:?}; brushes within {reach}:", at.map(|v| v.round())),
        }
        let mut found = Vec::new();
        gjk_world.gjk_query(lo, hi, PLAYER_MASK, &mut found);
        for (id, brush) in brushes.iter().enumerate() {
            let p = &brush.planes;
            if p.len() < 6 {
                continue;
            }
            let (bmin, bmax) = ([-p[1][3], -p[3][3], -p[5][3]], [p[0][3], p[2][3], p[4][3]]);
            if !(0..3).all(|k| bmin[k] <= hi[k] && lo[k] <= bmax[k]) || bmax[0] - bmin[0] > 20000.0 {
                continue;
            }
            let in_query = found.iter().any(|g| g.key == (1 << 32 | id as u64));
            println!(
                "  brush {id}: contents {:#x} sides {} corners {} box {:?}..{:?} mask-hit {} in gjk query {in_query}",
                brush.contents,
                p.len(),
                brush.verts.len(),
                bmin.map(|v| v.round()),
                bmax.map(|v| v.round()),
                brush.contents & PLAYER_MASK != 0
            );
        }
        let others = found.iter().filter(|g| g.key >> 32 != 1).count();
        println!("  plus {others} terrain/model/body shapes in the gjk query");
        return ExitCode::SUCCESS;
    }
    let mut gjk_runs = Vec::new();
    let mut box_runs = Vec::new();
    let mut labels = Vec::new();
    for (n, &(origin, yaw)) in spawns.iter().enumerate() {
        for h in 0..4 {
            let start = [origin[0], origin[1], origin[2] + 1.0];
            let heading = yaw + 90.0 * h as f32;
            gjk_runs.push(walk(&gjk_world, start, heading, frames));
            box_runs.push(walk(&box_world, start, heading, frames));
            labels.push(format!("spawn {n} ({:.0} {:.0} {:.0}) yaw {heading:.0}", origin[0], origin[1], origin[2]));
        }
    }

    let summary = |name: &str, runs: &[Run]| {
        let mut all: Vec<u32> = runs.iter().flat_map(|r| r.nanos.iter().copied()).collect();
        all.sort_unstable();
        let mean = all.iter().map(|&n| f64::from(n)).sum::<f64>() / all.len().max(1) as f64 / 1000.0;
        println!(
            "{name:<6} moves {:>6}  us/move mean {:>7.1} p50 {:>7.1} p99 {:>8.1} max {:>8.1} | out {:>3} stuck {:>3} airborne {:>3} start-solid {:>3}",
            all.len(),
            mean,
            percentile(&all, 0.5),
            percentile(&all, 0.99),
            percentile(&all, 1.0),
            runs.iter().filter(|r| r.out).count(),
            runs.iter().filter(|r| r.stuck).count(),
            runs.iter().filter(|r| !r.grounded).count(),
            runs.iter().filter(|r| r.start_solid).count(),
        );
    };
    summary("bo2", &gjk_runs);
    summary("box", &box_runs);

    let mut apart: Vec<(f32, usize)> = gjk_runs
        .iter()
        .zip(&box_runs)
        .enumerate()
        .map(|(i, (a, b))| {
            let d = (0..3).map(|k| (a.end[k] - b.end[k]).powi(2)).sum::<f32>().sqrt();
            (if d.is_finite() { d } else { f32::MAX }, i)
        })
        .collect();
    apart.sort_by(|a, b| b.0.total_cmp(&a.0));
    let close = apart.iter().filter(|(d, _)| *d < 16.0).count();
    println!("ends within 16 units of each other: {close} of {}", apart.len());
    for (i, run) in gjk_runs.iter().enumerate() {
        if run.out || run.stuck || !run.grounded {
            println!(
                "  bo2 {}{}{}: {} ends {:?} {:.1} above the ground, moving {:?}",
                if run.out { "OUT " } else { "" },
                if run.stuck { "STUCK " } else { "" },
                if run.grounded { "" } else { "AIRBORNE" },
                labels[i],
                run.end.map(|v| v.round()),
                run.height,
                run.velocity.map(|v| v.round())
            );
        }
    }
    println!("farthest apart (bo2 end | box end):");
    for &(d, i) in apart.iter().take(8) {
        println!(
            "  {d:>8.1}  {}  {:?} | {:?}",
            labels[i],
            gjk_runs[i].end.map(|v| v.round()),
            box_runs[i].end.map(|v| v.round())
        );
    }
    ExitCode::SUCCESS
}
