//! bo2mp: what a falling body's joint meets in a Black Ops II map, read-only.
//! `t6drop [zone.ff] x y z ...` (default MP Nuketown) builds the map's
//! collision as the game does (brushes and the mesh; not the static models)
//! and, at each point, sweeps boxes of a few sizes straight down with the
//! ragdoll's mask and the player's, then drops one in ragdoll-sized steps.

use std::path::PathBuf;

use asset_t6::capture_zone;
use asset_world::CollisionClipData;

fn default_zone() -> PathBuf {
    bo2zm_tools::zone_file("mp_nuketown_2020.ff")
}

/// What the ragdoll's joints collide with: solid, glass and item clip (the
/// fx physics mask; Nuketown's ground is clip-only, no solid bit).
const MASK_RAGDOLL: u32 = 0xc11;
/// The solid bit alone (the ragdoll's first mask).
const MASK_SOLID: u32 = 1;
/// What a player walks on (MASK_PLAYERSOLID).
const MASK_PLAYER: u32 = 0x0281_0011;

const MASKS: [(&str, u32); 3] = [("ragdoll", MASK_RAGDOLL), ("solid", MASK_SOLID), ("player", MASK_PLAYER)];
/// Half-sizes of the boxes swept down at each point.
const BOX_HALVES: [f32; 4] = [0.25, 1.0, 4.0, 8.0];
/// A falling joint drops 9 units a step (800 u/s^2 for a while at 90 steps a second), for up to 30 steps.
const FALL_STEP: f32 = 9.0;
const FALL_STEPS: usize = 30;

/// The zone (a first argument ending `.ff`, else Nuketown) and the numbers after it.
fn parse_args(args: impl IntoIterator<Item = String>) -> (PathBuf, Vec<f32>) {
    let mut args = args.into_iter().peekable();
    let zone = args.next_if(|a| a.ends_with(".ff")).map_or_else(default_zone, PathBuf::from);
    (zone, args.filter_map(|a| a.parse().ok()).collect())
}

/// The x y z points: every full triple of numbers.
fn points(nums: &[f32]) -> impl Iterator<Item = [f32; 3]> + '_ {
    nums.chunks_exact(3).map(|p| [p[0], p[1], p[2]])
}

fn straight_down(at: [f32; 3], by: f32) -> [f32; 3] {
    [at[0], at[1], at[2] - by]
}

/// The fall stops once it hits something it did not start inside.
fn landed(fraction: f32, startsolid: bool) -> bool {
    fraction < 1.0 && !startsolid
}

/// Where the next step starts: stepping through a solid, else where the last one ended.
fn next_z(to: f32, end_z: f32, startsolid: bool) -> f32 {
    if startsolid { to } else { end_z }
}

/// Boxes of a few sizes swept straight down under each mask, and held in place.
fn box_lines(world: &CollisionClipData, at: [f32; 3]) -> Vec<String> {
    let cases = MASKS.iter().flat_map(|&(label, mask)| BOX_HALVES.iter().map(move |&h| (label, mask, h)));
    cases
        .map(|(label, mask, h)| {
            let trace = world.sweep_box(at, straight_down(at, 300.0), [-h; 3], [h; 3], mask);
            let still = world.sweep_box(at, at, [-h; 3], [h; 3], mask);
            format!(
                "  {label:7} half {h:5.2}: down frac {:.4} end z {:9.3} normal {:?} startsolid {} allsolid {} | in place startsolid {}",
                trace.fraction, trace.finish_point[2], trace.normal, trace.began_inside, trace.fully_embedded, still.began_inside
            )
        })
        .collect()
}

/// Which contents bits the first floor below has.
fn floor_lines(world: &CollisionClipData, at: [f32; 3]) -> Vec<String> {
    (0..32)
        .filter_map(|bit| {
            let hit = world.sweep_box(at, straight_down(at, 300.0), [-1.0; 3], [1.0; 3], 1 << bit);
            (hit.fraction < 1.0).then(|| format!("  bit {:#010x}: floor at z {:9.3}", 1u32 << bit, hit.finish_point[2]))
        })
        .collect()
}

/// A ragdoll-sized joint falling step by step until it lands.
fn fall_lines(world: &CollisionClipData, at: [f32; 3]) -> Vec<String> {
    let h = 7.0f32;
    let mut z = at[2];
    let mut notes = vec![];
    for step in 0..FALL_STEPS {
        let to = z - FALL_STEP;
        let trace = world.sweep_box([at[0], at[1], z], [at[0], at[1], to], [-h; 3], [h; 3], MASK_RAGDOLL);
        notes.push(format!(
            "  fall {step:2}: {z:8.3} -> {to:8.3}: frac {:.4} end z {:8.3} startsolid {} allsolid {}",
            trace.fraction, trace.finish_point[2], trace.began_inside, trace.fully_embedded
        ));
        if landed(trace.fraction, trace.began_inside) {
            break;
        }
        z = next_z(to, trace.finish_point[2], trace.began_inside);
    }
    notes
}

fn run(path: &std::path::Path, nums: &[f32]) -> Result<(), String> {
    let capture = capture_zone(path).map_err(|e| e.to_string())?;
    let clip = capture.clip.as_ref().ok_or_else(|| format!("{} has no clipMap", path.to_string_lossy()))?;
    let world = asset_world::build_t6_clip_collision(clip).map_err(|e| format!("collision: {e}"))?;
    println!(
        "{} brushes, {} mesh tris, {} materials",
        world.brushes.len(),
        world.t6.map_or(0, |m| m.tri_indices.len()),
        world.materials.len()
    );
    for at in points(nums) {
        println!("at {at:?}:");
        let sections = [box_lines(&world, at), floor_lines(&world, at), fall_lines(&world, at)];
        sections.iter().flatten().for_each(|line| println!("{line}"));
    }
    Ok(())
}

fn main() -> std::process::ExitCode {
    let (path, nums) = parse_args(std::env::args().skip(1));
    let outcome = run(&path, &nums);
    if let Err(e) = &outcome {
        eprintln!("t6drop: {e}");
    }
    std::process::ExitCode::from(u8::from(outcome.is_err()))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use super::{parse_args, default_zone, points, straight_down, landed, next_z};

    fn words(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn a_leading_ff_name_is_the_zone_and_the_rest_are_numbers() {
        let (zone, nums) = parse_args(words(&["a.ff", "1", "x", "2.5"]));
        assert_eq!((zone, nums), (PathBuf::from("a.ff"), vec![1.0, 2.5]));
        let (zone, nums) = parse_args(words(&["1", "2", "3"]));
        assert_eq!((zone, nums), (default_zone(), vec![1.0, 2.0, 3.0]));
        assert_eq!(parse_args(words(&["1", "a.ff"])).1, [1.0]);
    }

    #[test]
    fn only_full_triples_become_points() {
        let nums = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
        assert_eq!(points(&nums).collect::<Vec<_>>(), [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]]);
    }

    #[test]
    fn straight_down_only_changes_z() {
        assert_eq!(straight_down([1.0, 2.0, 3.0], 300.0), [1.0, 2.0, -297.0]);
    }

    #[test]
    fn a_fall_ends_on_a_hit_that_did_not_start_solid() {
        assert!(landed(0.4, false));
        assert!(!landed(1.0, false));
        assert!(!landed(0.4, true));
        assert_eq!(next_z(10.0, 12.0, true), 10.0);
        assert_eq!(next_z(10.0, 12.0, false), 12.0);
    }
}
