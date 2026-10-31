//! bo2mp: walk Black Ops II's own movement over a real map, headless and
//! read-only. `t6walk [zone.ff] [frames]` (default Nuketown 2025, 300
//! frames): from each spawn point and four headings it runs the same keys
//! (walk, sprint, jumps, a crouch, a slow turn) twice, on Black Ops II's own
//! collision and on the box traces, then reports the time per move, the runs
//! that fell out of the world or got stuck, and how far the two ended apart.

use std::path::PathBuf;
use std::time::Instant;

use asset_t6::{capture_zone, parse_entities};
use movement_t6::Bo2Collision;
use movement_t6::bridge::{
    ScopeBlendArgs, ScopeWishArgs, AirborneStepArgs, Bo2Feel, StrideScaleArgs,
    FixedClimbClipDuration, SweepAsk, LeapKickoffArgs,
    LungeGunTimings, HullExtents, PRONE_LERP_MS, OneTickMotionArgs, DashArgs,
    LookRotationLimit, FootedStepArgs, StillClimbOffset,
};
use playerstate_t6::game::{NO_ENTITY_SLOT, AvatarState, InputCommand};

fn default_zone() -> PathBuf {
    bo2zm_tools::zone_file("mp_nuketown_2020.ff")
}

const SPRINT: u32 = 0x2;
const DUCK_BUTTON_BIT: u32 = 0x200;
const LEAP: u32 = 0x400;
/// One move's length: a 60 fps client.
const MSEC: i32 = 16;
const G_GRAVITY: i32 = 800;
const BODY_HALF: f32 = 15.0;
const BODY_BOX_LOWER: [f32; 3] = [-BODY_HALF, -BODY_HALF, 0.0];
const BODY_BOX_UPPER: [f32; 3] = [BODY_HALF, BODY_HALF, 70.0];
const PLAYER_MASK: u32 = 0x0281_0011;
/// Spawn points walked, at most (spread over the map's list).
const MAX_SPAWNS: usize = 48;

/// Standing jump height, in units; also the height the weapon mantle uses.
const JUMP_HEIGHT: f32 = 39.0;
/// A scale that changes nothing.
const NEUTRAL: f32 = 1.0;
/// How far forward the stick has to be held to keep a sprint going.
const SPRINT_FORWARD_MIN: i32 = 105;
/// The weapon's sprint time, in milliseconds.
const WEAPON_SPRINT_MS: i32 = 4000;
/// The shortest sprint, in milliseconds.
const MIN_SPRINT_MS: f32 = 1000.0;

/// bo2mp's multiplayer rules, as the sim builds them from BO2's dvars.
fn bo2_feel() -> Bo2Feel {
    let mut feel = Bo2Feel::BO2_MP;
    feel.g_speed = 190.0;
    feel.gravity = G_GRAVITY;
    feel.speed_multiplier = 1.07;
    feel.sprint_multiplier = 2.0;
    feel.fast_ads_multiplier = 0.5;
    feel.sprint_strafe_speed_scale = 0.667;
    feel.fall_damage_min_height = 128.0;
    feel.fall_damage_max_height = 300.0;
    feel.mantle_weapon_height = JUMP_HEIGHT;
    feel.mantle_weapon_anim_height = 32.0;
    feel.leap_height = JUMP_HEIGHT;
    feel.jump_slowdown = true;
    feel.dive = true;
    feel.omni = false;
    feel.zombies = false;
    feel
}

/// Speed scales for the walk command scale; the shell shock and weapon
/// factors do nothing.
fn cmd_scale_ctx() -> StrideScaleArgs {
    let [retreat_pace_ratio, sidestep_pace_ratio, dash_pace_ratio, downed_crawl_pace_ratio] =
        [0.7, 0.8, 1.5, 0.15];
    StrideScaleArgs {
        prone_lerp_ms: PRONE_LERP_MS,
        scoped_carry_pace: NEUTRAL,
        gun_stride_ratio: NEUTRAL,
        shellshock_movement_scale: NEUTRAL,
        retreat_pace_ratio,
        sidestep_pace_ratio,
        dash_pace_ratio,
        downed_crawl_pace_ratio,
    }
}

/// The air-move settings: gravity and spectate speed left alone.
fn air_ctx() -> AirborneStepArgs {
    let [bias, scale, spectate] = [0.0, NEUTRAL, NEUTRAL];
    AirborneStepArgs {
        concussed_fall_ratio: scale,
        concussed_fall_offset: bias,
        observer_pace_ratio: spectate,
    }
}

/// What the last move left behind for the next one to read.
#[derive(Clone, Copy, Default)]
struct Carry {
    held: u32,
    stick: [i8; 2],
}

fn walk_ctx(prev: Carry, air: AirborneStepArgs) -> FootedStepArgs {
    let dive = false;
    let jump = LeapKickoffArgs { leap_height: JUMP_HEIGHT, rung_kickoff_speed: 128.0, squat_leap_ratio: NEUTRAL, dive };
    FootedStepArgs {
        last_tick_buttons: prev.held,
        cmd_scale: cmd_scale_ctx(),
        gun_carry_pace: NEUTRAL,
        lunge: None,
        feel: bo2_feel(),
        jump,
        air,
    }
}

fn sprint_ctx() -> DashArgs {
    // The sprint never waits to start or to recharge, and there is always
    // room to stand.
    let [rise_space_free, endless_dash] = [true, false];
    let no_wait = 0.0;
    DashArgs {
        gun_dash_cap_secs: WEAPON_SPRINT_MS,
        dash_thrust_floor: SPRINT_FORWARD_MIN,
        dash_least_secs: MIN_SPRINT_MS / 1000.0,
        dash_warmup_secs: no_wait,
        dash_refill_hold_secs: no_wait,
        rise_space_free,
        endless_dash,
    }
}

/// The sights stay down; the rates are the stock ones.
fn ads_frac_ctx() -> ScopeBlendArgs {
    ScopeBlendArgs { scoped_aiming: false, ..ScopeBlendArgs::default() }
}

fn context(prev: Carry) -> OneTickMotionArgs {
    let (air, gun_forbids_lying) = (air_ctx(), false);
    let bounds = HullExtents { collide_filter: PLAYER_MASK, lo_corner: BODY_BOX_LOWER, hi_corner: BODY_BOX_UPPER };
    OneTickMotionArgs {
        old_move: prev.stick,
        last_tick_buttons: prev.held,
        walk: walk_ctx(prev, air),
        bounds,
        air,
        look_rotation: LookRotationLimit { tilt_upper_limit: 85.0, tilt_lower_limit: 85.0, free_tilt_flag: false },
        sprint: sprint_ctx(),
        ads_intent: ScopeWishArgs { sights_permitted: true, scoped_weapon: false, dash_keeps_scope: false },
        ads_frac: ads_frac_ctx(),
        melee_charge: LungeGunTimings::default(),
        knife_reach: movement_t6::bridge::MELEE_CHARGE_PLAYER_MELEE_RANGE_DEFAULT,
        gun_forbids_lying,
    }
}

fn player(origin: [f32; 3]) -> AvatarState {
    AvatarState {
        origin,
        last_cmd_ms: 1000,
        gravity: 800,
        speed: 190,
        run_speed_modifier: 1.0,
        eye_height_goal: 60,
        eye_height_now: 60.0,
        standing_on: NO_ENTITY_SLOT,
        ..AvatarState::ZERO
    }
}

/// Frame `i`'s keys: forward, right, buttons, and how far the view turns.
fn keys(i: usize) -> (i8, i8, u32, f32) {
    const JUMP_FRAMES: [usize; 6] = [60, 61, 150, 151, 240, 241];
    let held = [(20..120, SPRINT), (170..190, DUCK_BUTTON_BIT)]
        .into_iter()
        .filter(|(frames, _)| frames.contains(&i))
        .fold(0, |mask, (_, bit)| mask | bit);
    let jump = if JUMP_FRAMES.contains(&i) { LEAP } else { 0 };
    let right = if (200..230).contains(&i) { 127 } else { 0 };
    (127, right, held | jump, 0.6)
}

/// How one run went.
struct Run {
    /// The player as the last move left them.
    fin: AvatarState,
    grounded: bool,
    /// Under the world: nothing solid below the player.
    out: bool,
    /// Moved less than a body's width in the whole run.
    stuck: bool,
    /// Started inside something.
    start_solid: bool,
    /// How far above the ground the run ended.
    height: f32,
    nanos: Vec<u32>,
}

impl Run {
    fn end(&self) -> [f32; 3] {
        self.fin.origin
    }
}

/// The view yaw as the 16-bit angle a command carries.
fn yaw_to_angle(yaw: f32) -> i32 {
    (yaw * 65536.0 / 360.0) as i32 & 0xffff
}

/// Run frame `i`'s keys through BO2's own move code. Gives back how far the
/// player slid over the floor and how many nanoseconds the move took.
fn step(world: &impl Bo2Collision, ps: &mut AvatarState, carry: &mut Carry, yaw: &mut f32, i: usize) -> (f32, u32) {
    let (ahead, right, held, turn) = keys(i);
    *yaw += turn;
    let mut cmd = InputCommand::default();
    cmd.authority_clock_ms = ps.last_cmd_ms + MSEC;
    cmd.thrust_ahead = ahead;
    cmd.thrust_lateral = right;
    cmd.buttons = held;
    cmd.angles[1] = yaw_to_angle(*yaw);
    let before = ps.origin;
    let clock = Instant::now();
    movement_t6::pmove(
        ps,
        &mut cmd,
        context(*carry),
        world,
        &FixedClimbClipDuration::default(),
        &StillClimbOffset,
    );
    let nanos = u32::try_from(clock.elapsed().as_nanos()).unwrap_or(u32::MAX);
    *carry = Carry { held: cmd.buttons, stick: [cmd.thrust_ahead, cmd.thrust_lateral] };
    (f32::hypot(ps.origin[0] - before[0], ps.origin[1] - before[1]), nanos)
}

/// A box sweep the size of the player's: the trace input for it.
fn player_box(from: [f32; 3], dest: [f32; 3]) -> SweepAsk {
    SweepAsk { start: from, end: dest, collide_filter: PLAYER_MASK, lo_corner: BODY_BOX_LOWER, hi_corner: BODY_BOX_UPPER }
}

/// How far a player-sized box got (0..1) and whether it began inside something.
fn sweep(world: &impl Bo2Collision, from: [f32; 3], dest: [f32; 3]) -> (f32, bool) {
    let hit = world.trace(player_box(from, dest));
    (hit.fraction, hit.starts_inside)
}

fn walk(world: &impl Bo2Collision, origin_at: [f32; 3], yaw: f32, frames: usize) -> Run {
    walk_path(world, origin_at, yaw, frames, None)
}

/// How far below a player to look for ground.
const GROUND_PROBE: f32 = 16384.0;

fn walk_path(
    world: &impl Bo2Collision,
    origin_at: [f32; 3],
    yaw: f32,
    frames: usize,
    mut path: Option<&mut Vec<[f32; 3]>>,
) -> Run {
    let mut ps = player(origin_at);
    let (mut carry, mut yaw) = (Carry::default(), yaw);
    let mut nanos = Vec::with_capacity(frames);
    let mut slid = 0.0;
    for i in 0..frames {
        let (moved, took) = step(world, &mut ps, &mut carry, &mut yaw, i);
        slid += moved;
        nanos.push(took);
        path.iter_mut().for_each(|trail| trail.push(ps.origin));
        if !ps.origin.iter().all(|v| v.is_finite()) {
            break;
        }
    }
    let finite = ps.origin.iter().all(|v| v.is_finite());
    let [x, y, z] = ps.origin;
    let (below, below_solid) = sweep(world, ps.origin, [x, y, z - GROUND_PROBE]);
    Run {
        grounded: ps.standing_on != NO_ENTITY_SLOT,
        out: !finite || (below >= 1.0 && !below_solid),
        stuck: slid < 30.0,
        start_solid: sweep(world, origin_at, origin_at).1,
        height: below * GROUND_PROBE,
        fin: ps,
        nanos,
    }
}

fn percentile(sorted: &[u32], p: f32) -> f32 {
    let Some(last) = sorted.len().checked_sub(1) else {
        return 0.0;
    };
    sorted[(last as f32 * p).round() as usize] as f32 / 1000.0
}

/// Everything read from the zone that the walks need.
struct World {
    brushes: Vec<sim::SolidBrush>,
    bsp: sim::CollisionTreeData,
    /// Terrain triangles, nodes and leaves, for the load line.
    sizes: [usize; 3],
    /// Spawn points (origin, yaw), one per spot, in list order.
    spawns: Vec<([f32; 3], f32)>,
}

fn read_level_geometry(path: &std::path::Path) -> Result<World, String> {
    let capture = capture_zone(path).map_err(|e| format!("t6walk: {e}"))?;
    let clip_ref = capture.clip.as_ref().ok_or_else(|| format!("t6walk: {} has no clipMap", path.to_string_lossy()))?;
    let cm = asset_world::build_t6_clip_collision(clip_ref).map_err(|e| format!("t6walk: collision: {e:?}"))?;
    let brushes: Vec<sim::SolidBrush> = cm
        .brushes
        .iter()
        .map(|b| sim::SolidBrush {
            hit_surface_flags: b.hit_surface_flags,
            solid_bits: b.contents,
            glass_hit_code: b.glass_hit_code,
            sides: b.planes.clone(),
            plane_material_flags: b.plane_material_flags.clone(),
            vertex_points: b.vertex_points.clone(),
        })
        .collect();
    let t6_glass = sim::CollisionTreeData::glass_of(&brushes);
    let bsp = sim::CollisionTreeData {
        t6_glass,
        t6: cm.t6,
        brush_leaf_indices: cm.brush_leaf_indices.to_vec(),
        t6_model_brushes: cm.t6_model_brushes.clone(),
    };
    let sizes = [cm.t6.map_or(0, |m| m.tri_indices.len()), cm.nodes.len(), cm.leaves.len()];
    Ok(World { brushes, bsp, sizes, spawns: spawn_points(&capture.map_ents) })
}

/// The map's spawn points, one per spot, in list order.
fn spawn_points(map_ents: &[String]) -> Vec<([f32; 3], f32)> {
    let wanted = |class: &str| class == "info_player_start" || (class.starts_with("mp_") && class.contains("spawn"));
    map_ents
        .iter()
        .flat_map(|text| parse_entities(text))
        .filter(|e| wanted(e.classname()))
        .filter_map(|e| Some((e.vec3("origin")?, e.vec3("angles").map_or(0.0, |a| a[1]))))
        .fold(Vec::new(), |mut kept: Vec<([f32; 3], f32)>, (o, yaw)| {
            if kept.iter().all(|(s, _)| (0..3).any(|a| (s[a] - o[a]).abs() > 8.0)) {
                kept.push((o, yaw));
            }
            kept
        })
}

/// `T6WALK_TRACE=x,y,top,bottom`: box traces straight down there, a point
/// and the player's box, and what they hit.
fn trace_probe(world: &impl Bo2Collision, spec: &str) -> Result<(), String> {
    let nums: Vec<f32> = spec.split(',').filter_map(|v| v.trim().parse().ok()).collect();
    let [x, y, top, bottom] = <[f32; 4]>::try_from(nums).map_err(|_| "t6walk: T6WALK_TRACE=x,y,top,bottom".to_string())?;
    let shapes = [("point", [0.0; 3], [0.0; 3]), ("player", BODY_BOX_LOWER, BODY_BOX_UPPER)];
    for (name, mins, maxs) in shapes {
        let input = SweepAsk { lo_corner: mins, hi_corner: maxs, ..player_box([x, y, top], [x, y, bottom]) };
        let t = world.trace(input);
        let z = top + (bottom - top) * t.fraction;
        println!(
            "{name:<6} from {top} to {bottom}: fraction {:.4} hits z {z:.1} normal {:?} startsolid {} allsolid {} entity {} contents {:#x} surface {:#x}",
            t.fraction, t.normal, t.starts_inside, t.fully_embedded, t.struck_entity_num, t.cflags, t.sflags
        );
    }
    Ok(())
}

/// The axis-aligned box of a brush with at least six planes (the first six
/// are its box sides), or `None`.
fn brush_box(planes: &[[f32; 4]]) -> Option<([f32; 3], [f32; 3])> {
    let [_, _, _, _, _, _, ..] = planes else {
        return None;
    };
    let lows = [1, 3, 5].map(|k| -planes[k][3]);
    let highs = [0, 2, 4].map(|k| planes[k][3]);
    Some((lows, highs))
}

/// `T6WALK_CLIMB=x,y,z,yaw,ahead,jump_frame`: from that spot, hold the stick
/// forward and hold jump for 30 moves from that frame (as a bot at a climb link does);
/// print each move's origin, climb and stance bits, ground and whether the
/// standing and crouched boxes start inside something. The climb plays without
/// its animations. `T6WALK_CLIMB_AT=x,y,z` also lists the brushes there, and
/// `T6WALK_SIDES=id` prints that brush's planes and corners.
fn climb_probe(name: &str, world: &impl Bo2Collision, spec: &str, frames: usize) {
    use playerstate_t6::flags::movement_state_bits::{MANTLE, STANCE_MASK};
    let nums: Vec<f32> = spec.split(',').filter_map(|v| v.trim().parse().ok()).collect();
    let [x, y, z, yaw, ahead, jf] = nums[..] else { println!("T6WALK_CLIMB=x,y,z,yaw,ahead,jump_frame"); return };
    let mut ps = player([x, y, z]);
    let mut carry = Carry::default();
    for i in 0..frames {
        let mut cmd = InputCommand::default();
        cmd.authority_clock_ms = ps.last_cmd_ms + MSEC;
        cmd.thrust_ahead = ahead as i8;
        cmd.buttons = if (jf as usize..jf as usize + 30).contains(&i) { LEAP } else { 0 };
        cmd.angles[1] = yaw_to_angle(yaw);
        movement_t6::pmove(&mut ps, &mut cmd, context(carry), world, &FixedClimbClipDuration::default(), &StillClimbOffset);
        carry = Carry { held: cmd.buttons, stick: [cmd.thrust_ahead, cmd.thrust_lateral] };
        let o = ps.origin;
        let stand = world.trace(player_box(o, o)).starts_inside;
        let low = SweepAsk { hi_corner: [BODY_HALF, BODY_HALF, 50.0], ..player_box(o, o) };
        let crouch = world.trace(low).starts_inside;
        println!(
            "{name} {i:>3} at {:.1} {:.1} {:.1} vel {:.0} {:.0} {:.0} mantle {} stance {} ground {} solid stand {stand} crouch {crouch}",
            o[0], o[1], o[2], ps.velocity[0], ps.velocity[1], ps.velocity[2],
            ps.movement_state_bits & MANTLE as u32 != 0, ps.movement_state_bits & STANCE_MASK as u32, ps.standing_on != NO_ENTITY_SLOT,
        );
    }
}

/// `T6WALK_PROBE=spawn,heading[,every[,frame]]`: walk that one run on both,
/// print the two paths and the brushes around where the box run stopped.
fn walk_probe(
    spec: &str,
    spawns: &[([f32; 3], f32)],
    brushes: &[sim::SolidBrush],
    gjk_world: &impl Bo2Collision,
    box_world: &impl Bo2Collision,
    frames: usize,
) -> Result<(), String> {
    let nums: Vec<usize> = spec.split(',').filter_map(|v| v.trim().parse().ok()).collect();
    let (&n, &h) = nums.first().zip(nums.get(1)).ok_or("t6walk: T6WALK_PROBE=spawn,heading")?;
    let (every, around_bo2) = (nums.get(2).copied(), nums.get(3).copied());
    let &(origin, yaw) = spawns.get(n).ok_or_else(|| format!("t6walk: no spawn {n}"))?;
    let start = [origin[0], origin[1], origin[2] + 1.0];
    let heading = yaw + 90.0 * h as f32;
    let (mut a, mut b) = (Vec::new(), Vec::new());
    walk_path(gjk_world, start, heading, frames, Some(&mut a));
    walk_path(box_world, start, heading, frames, Some(&mut b));
    let stride = every.unwrap_or(5).max(1);
    for (i, (p, q)) in a.iter().zip(&b).enumerate().filter(|(i, _)| i % stride == 0) {
        println!("{i:>4}  bo2 {:?}  box {:?}", p.map(|v| v.round()), q.map(|v| v.round()));
    }
    // Where the box run stopped moving sideways for good.
    let stop = b.windows(2).rposition(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]) > 0.5).map_or(0, |i| i + 1);
    let at = match around_bo2 {
        Some(f) => a.get(f).copied(),
        None => b.get(stop).copied(),
    }
    .unwrap_or(start);
    let reach = 40.0;
    let lo = [at[0] - reach, at[1] - reach, at[2] - 8.0];
    let hi = [at[0] + reach, at[1] + reach, at[2] + 80.0];
    let rounded = at.map(|v| v.round());
    if let Some(f) = around_bo2 {
        println!("bo2 run at frame {f} {rounded:?}; brushes within {reach}:");
    } else {
        println!("box run stops moving at frame {stop} {rounded:?}; brushes within {reach}:");
    }
    list_brushes(lo, hi, brushes, gjk_world);
    Ok(())
}

/// The brushes whose boxes touch `lo..hi`, and whether BO2's collision
/// query finds each.
fn list_brushes(lo: [f32; 3], hi: [f32; 3], brushes: &[sim::SolidBrush], gjk_world: &impl Bo2Collision) {
    let mut found = Vec::new();
    gjk_world.gjk_query(lo, hi, PLAYER_MASK, &mut found);
    let near = brushes.iter().enumerate().filter_map(|(id, brush)| {
        let (bmin, bmax) = brush_box(&brush.sides)?;
        let overlaps = (0..3).all(|k| bmin[k] <= hi[k] && lo[k] <= bmax[k]);
        (overlaps && bmax[0] - bmin[0] <= 20000.0).then_some((id, brush, bmin, bmax))
    });
    for (id, brush, bmin, bmax) in near {
        let in_query = found.iter().any(|g| g.key == (1 << 32 | id as u64));
        println!(
            "  brush {id}: contents {:#x} sides {} corners {} box {:?}..{:?} mask-hit {} in gjk query {in_query}",
            brush.solid_bits,
            brush.sides.len(),
            brush.vertex_points.len(),
            bmin.map(|v| v.round()),
            bmax.map(|v| v.round()),
            brush.solid_bits & PLAYER_MASK != 0
        );
        if std::env::var("T6WALK_SIDES").is_ok_and(|v| v == id.to_string()) {
            println!("    sides {:?}\n    corners {:?}", brush.sides, brush.vertex_points);
        }
    }
    let others = found.iter().filter(|g| g.key >> 32 != 1).count();
    println!("  plus {others} terrain/model/body shapes in the gjk query");
}

/// One line of timing and failure counts for a set of runs.
fn summary_line(name: &str, runs: &[Run]) -> String {
    let mut all: Vec<u32> = runs.iter().flat_map(|r| r.nanos.iter().copied()).collect();
    all.sort_unstable();
    let total: f64 = all.iter().copied().map(f64::from).sum();
    let mean = total / all.len().max(1) as f64 / 1000.0;
    let count = |pick: fn(&Run) -> bool| runs.iter().filter(|r| pick(r)).count();
    format!(
        "{name:<6} moves {:>6}  us/move mean {mean:>7.1} p50 {:>7.1} p99 {:>8.1} max {:>8.1} | out {:>3} stuck {:>3} airborne {:>3} start-solid {:>3}",
        all.len(),
        percentile(&all, 0.5),
        percentile(&all, 0.99),
        percentile(&all, 1.0),
        count(|r| r.out),
        count(|r| r.stuck),
        count(|r| !r.grounded),
        count(|r| r.start_solid),
    )
}

/// How far apart each pair of run ends is (a lost run counts as infinitely
/// far), paired with the run's index, farthest first.
fn gaps(left: &[Run], right: &[Run]) -> Vec<(f32, usize)> {
    let mut ordered: Vec<(f32, usize)> = left
        .iter()
        .zip(right)
        .map(|(a, b)| {
            let sq: f32 = a.end().iter().zip(b.end()).map(|(p, q)| (p - q).powi(2)).sum();
            sq.sqrt()
        })
        .map(|d| if d.is_finite() { d } else { f32::MAX })
        .zip(0..)
        .collect();
    ordered.sort_by(|a, b| b.0.total_cmp(&a.0));
    ordered
}

/// Every how-many-th spawn to keep so at most [`MAX_SPAWNS`] remain.
fn spawns_stride(spawns: &[([f32; 3], f32)]) -> usize {
    spawns.len().div_ceil(MAX_SPAWNS)
}

/// A spawn's run label, the way the report names it.
fn trip_label(n: usize, spot: [f32; 3], heading: f32) -> String {
    let [x, y, z] = spot;
    format!("spawn {n} ({x:.0} {y:.0} {z:.0}) yaw {heading:.0}")
}

/// Print the failures among the first collision's runs and the pairs that
/// ended farthest apart.
fn report_pairs(gjk_runs: &[Run], box_runs: &[Run], labels: &[String]) {
    let apart = gaps(gjk_runs, box_runs);
    let close = apart.iter().filter(|(d, _)| *d < 16.0).count();
    println!("ends within 16 units of each other: {close} of {}", apart.len());
    let troubled = gjk_runs.iter().zip(labels).filter(|(r, _)| r.out || r.stuck || !r.grounded);
    for (run, label) in troubled {
        let flags: String = [(run.out, "OUT "), (run.stuck, "STUCK "), (!run.grounded, "AIRBORNE")]
            .into_iter()
            .filter_map(|(on, word)| on.then_some(word))
            .collect();
        println!(
            "  bo2 {flags}: {label} ends {:?} {:.1} above the ground, moving {:?}",
            run.end().map(f32::round),
            run.height,
            run.fin.velocity.map(f32::round)
        );
    }
    println!("farthest apart (bo2 end | box end):");
    for (d, i) in apart.into_iter().take(8) {
        let (a, b) = (gjk_runs[i].end().map(f32::round), box_runs[i].end().map(f32::round));
        println!("  {d:>8.1}  {}  {a:?} | {b:?}", labels[i]);
    }
}

/// What the command line asks for: the zone and how many moves per walk.
#[derive(Debug, PartialEq)]
struct Cli {
    path: PathBuf,
    frames: usize,
}

/// `[zone.ff] [frames]`: a first word ending `.ff` is the zone, the next
/// number the frame count; anything missing or unreadable takes its default.
fn read_flags(args: impl IntoIterator<Item = String>) -> Cli {
    let mut args = args.into_iter().peekable();
    let path = args.next_if(|a| a.ends_with(".ff")).map_or_else(default_zone, PathBuf::from);
    let frames = args.next().and_then(|a| a.parse().ok()).unwrap_or(300);
    Cli { path, frames }
}

/// The spawn points the walks start from; a map with none cannot be walked.
fn require_spawns(spawns: Vec<([f32; 3], f32)>) -> Result<Vec<([f32; 3], f32)>, String> {
    (!spawns.is_empty()).then_some(spawns).ok_or_else(|| "t6walk: no spawn points in the map's entities".to_owned())
}

fn run(cli: &Cli) -> Result<(), String> {
    let Cli { path, frames } = cli;
    let frames = *frames;
    let clock = Instant::now();
    let World { brushes, bsp, sizes: [triangle_list, nodes, leaves], spawns } = read_level_geometry(path)?;
    let extra_sided = || brushes.iter().filter(|b| b.sides.len() > 6);
    let (sided, sided_bare) = (extra_sided().count(), extra_sided().filter(|b| b.vertex_points.is_empty()).count());
    let zone_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    println!(
        "{zone_name}: {} brushes ({sided} with extra sides, {sided_bare} of those with no corner points), {triangle_list} terrain tris, {nodes} nodes, {leaves} leaves; loaded in {:.1}s",
        brushes.len(),
        clock.elapsed().as_secs_f32()
    );
    // Black Ops II's player mask (ClientThink_real) is MW2's 0x0281_0011 plus 0x8000.
    let flagged = || brushes.iter().filter(|b| b.solid_bits & 0x8000 != 0);
    let only_bo2 = flagged().filter(|b| b.solid_bits & 0x0281_0011 == 0).count();
    println!("contents 0x8000: {} brushes, {only_bo2} solid to the player only by that bit", flagged().count());

    let spawns = require_spawns(spawns)?;
    let spawns: Vec<_> = spawns.iter().copied().step_by(spawns_stride(&spawns)).collect();
    println!("{} spawn points x 4 headings x {frames} moves of {MSEC} ms", spawns.len());

    let mesh = sim::CollisionTriangleData::default();
    let [gjk_world, box_world] = [true, false].map(|gjk| sim::bo2_map_collision(&brushes, &bsp, &mesh, gjk));

    if let Ok(spec) = std::env::var("T6WALK_TRACE") {
        println!("box:");
        trace_probe(&box_world, &spec)?;
        println!("bo2:");
        return trace_probe(&gjk_world, &spec);
    }
    if let Ok(spec) = std::env::var("T6WALK_CLIMB") {
        climb_probe("bo2", &gjk_world, &spec, frames);
        climb_probe("box", &box_world, &spec, frames);
        if let Ok(at) = std::env::var("T6WALK_CLIMB_AT") {
            let a: Vec<f32> = at.split(',').filter_map(|v| v.trim().parse().ok()).collect();
            list_brushes([a[0] - 20.0, a[1] - 20.0, a[2] - 4.0], [a[0] + 20.0, a[1] + 20.0, a[2] + 4.0], &brushes, &gjk_world);
        }
        return Ok(());
    }
    if let Ok(spec) = std::env::var("T6WALK_PROBE") {
        return walk_probe(&spec, &spawns, &brushes, &gjk_world, &box_world, frames);
    }

    // Every spawn, four headings a quarter turn apart, run on both collisions.
    let trips: Vec<(usize, [f32; 3], f32)> = (0..)
        .zip(&spawns)
        .flat_map(|(n, &(origin, yaw))| (0..4u8).map(move |quarter| (n, origin, yaw + 90.0 * f32::from(quarter))))
        .collect();
    let run_trips = |world: &dyn Fn([f32; 3], f32) -> Run| -> Vec<Run> {
        trips.iter().map(|&(_, [x, y, z], heading)| world([x, y, z + 1.0], heading)).collect()
    };
    let gjk_runs = run_trips(&|from, heading| walk(&gjk_world, from, heading, frames));
    let box_runs = run_trips(&|from, heading| walk(&box_world, from, heading, frames));
    let labels: Vec<String> = trips.iter().map(|&(n, origin, heading)| trip_label(n, origin, heading)).collect();

    println!("{shown}", shown = summary_line("bo2", &gjk_runs));
    println!("{shown}", shown = summary_line("box", &box_runs));
    report_pairs(&gjk_runs, &box_runs, &labels);
    Ok(())
}

fn main() -> std::process::ExitCode {
    let outcome = run(&read_flags(std::env::args().skip(1)));
    if let Err(msg) = &outcome {
        eprintln!("{msg}");
    }
    std::process::ExitCode::from(u8::from(outcome.is_err()))
}

#[cfg(test)]
mod tests {
    use super::{read_flags, Cli, default_zone, require_spawns, keys, SPRINT, LEAP, percentile, yaw_to_angle, trip_label, brush_box};

    fn words(list: &[&str]) -> Vec<String> {
        list.iter().map(|w| (*w).to_owned()).collect()
    }

    #[test]
    fn no_arguments_mean_nuketown_and_300_frames() {
        assert_eq!(read_flags(words(&[])), Cli { path: default_zone(), frames: 300 });
    }

    #[test]
    fn a_zone_word_is_taken_only_when_it_ends_in_ff() {
        assert_eq!(read_flags(words(&["a.ff", "50"])), Cli { path: "a.ff".into(), frames: 50 });
        assert_eq!(read_flags(words(&["50"])), Cli { path: default_zone(), frames: 50 });
        assert_eq!(read_flags(words(&["a.ff", "many"])).frames, 300);
    }

    #[test]
    fn a_map_without_spawns_is_refused() {
        assert_eq!(require_spawns(vec![]), Err("t6walk: no spawn points in the map's entities".to_owned()));
        assert_eq!(require_spawns(vec![([1.0, 2.0, 3.0], 90.0)]).map(|s| s.len()), Ok(1));
    }

    #[test]
    fn keys_follow_the_scripted_windows() {
        assert_eq!(keys(0), (127, 0, 0, 0.6));
        assert_eq!(keys(20).2, SPRINT);
        assert_eq!(keys(60).2, SPRINT | LEAP);
        assert_eq!(keys(119).2, SPRINT);
        assert_eq!(keys(120).2, 0);
        assert_eq!(keys(180).2, 0x200);
        assert_eq!(keys(229).1, 127);
        assert_eq!(keys(230).1, 0);
    }

    #[test]
    fn percentile_picks_the_nearest_rank() {
        assert_eq!(percentile(&[], 0.5), 0.0);
        assert_eq!(percentile(&[1000, 2000, 3000], 1.0), 3.0);
        assert_eq!(percentile(&[1000, 2000, 3000], 0.5), 2.0);
    }

    #[test]
    fn yaw_wraps_into_sixteen_bits() {
        assert_eq!(yaw_to_angle(90.0), 0x4000);
        assert_eq!(yaw_to_angle(360.0), 0);
        assert_eq!(yaw_to_angle(-90.0), 0xc000);
    }

    #[test]
    fn trip_labels_round_to_whole_units() {
        assert_eq!(trip_label(3, [1.4, -2.6, 0.0], 180.4), "spawn 3 (1 -3 0) yaw 180");
    }

    #[test]
    fn brush_box_needs_six_planes() {
        let planes = [[1.0, 0.0, 0.0, 10.0], [-1.0, 0.0, 0.0, 4.0], [0.0, 1.0, 0.0, 9.0], [0.0, -1.0, 0.0, 3.0], [0.0, 0.0, 1.0, 8.0], [0.0, 0.0, -1.0, 2.0]];
        assert_eq!(brush_box(&planes), Some(([-4.0, -3.0, -2.0], [10.0, 9.0, 8.0])));
        assert_eq!(brush_box(&planes[..5]), None);
    }
}
