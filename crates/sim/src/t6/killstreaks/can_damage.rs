//! What a point sees of an entity, as Black Ops II's `CanDamage` measures it:
//! a blast's reach to a vehicle, and `sightconetrace` / `damageconetrace`
//! (a helicopter's or a Guardian's sight of its target, a blast's share).
//!
//! A player is seen by the middle of his body and four points round it, 15
//! units to each side and up and down by half his height: a third of him
//! for each one seen, all of him past three. Anything else is seen by the
//! middle of its box and four points across it as the point faces it: all
//! of it when one is seen. With a cone (a direction and an angle) only the
//! points inside it count. Sight looks through what eyes look through
//! (BO2's mask 0x3801), damage through what a bullet goes through
//! (0x802013).

use bevy_ecs::prelude::World;
use gsc_t6::math::{cross, normalize, sub};
use gsc_t6::{Value, Vm};

use super::super::{Zm, arg, entnum, frame};
use crate::world::ClientId;

/// `sightconetrace`'s trace mask.
pub(super) const SIGHT_MASK: u32 = 0x0000_3801;
/// `damageconetrace`'s and a blast's trace mask.
pub(super) const DAMAGE_MASK: u32 = 0x0080_2013;
/// The cone when a direction is given without an angle, in degrees.
const CONE_DEGREES: f32 = 65.0;

/// The five points looked for on a box (its middle and half size in the
/// world) from `from`: the middle, then the four corners of the box's
/// outline as `from` sees it.
pub(super) fn box_points(mid: [f32; 3], half: [f32; 3], from: [f32; 3]) -> [[f32; 3]; 5] {
    let mut d = normalize(sub(from, mid));
    if d[0] * d[0] + d[1] * d[1] < 0.001 {
        d = [1.0, 0.0, 0.0];
    }
    let right = normalize([-d[1], d[0], 0.0]);
    let up = cross(d, right);
    let reach_r = (half[0] * right[0]).abs() + (half[1] * right[1]).abs();
    let reach_u = (up[0] * half[0]).abs() + (up[1] * half[1]).abs() + (up[2] * half[2]).abs();
    let at = |r: f32, u: f32| -> [f32; 3] {
        std::array::from_fn(|i| mid[i] + right[i] * reach_r * r + up[i] * reach_u * u)
    };
    [
        at(0.0, 0.0),
        at(1.0, 1.0),
        at(-1.0, 1.0),
        at(1.0, -1.0),
        at(-1.0, -1.0),
    ]
}

/// The five points looked for on a player from `from`: the middle of his
/// body (feet to eye, or his feet alone in a vehicle) and four round it, 15
/// units to each side as `from` faces him, up and down by half his height.
pub(super) fn body_points(feet: [f32; 3], eye: [f32; 3], from: [f32; 3]) -> [[f32; 3]; 5] {
    let mid: [f32; 3] = std::array::from_fn(|i| (feet[i] + eye[i]) * 0.5);
    let half = (eye[2] - feet[2]) * 0.5;
    let (dx, dy) = (from[0] - feet[0], from[1] - feet[1]);
    let len = (dx * dx + dy * dy).sqrt();
    // Straight overhead there is no side to step to.
    let inv = if len == 0.0 { 1.0 } else { 1.0 / len };
    let side = [-dy * inv * 15.0, dx * inv * 15.0];
    let at = |s: f32, u: f32| [mid[0] + side[0] * s, mid[1] + side[1] * s, mid[2] + half * u];
    [
        at(0.0, 0.0),
        at(1.0, 1.0),
        at(-1.0, 1.0),
        at(1.0, -1.0),
        at(-1.0, -1.0),
    ]
}

/// How much of a player is seen when `seen` of his five points are: none,
/// a third each, all of him past three.
pub(super) fn body_share(seen: usize) -> f32 {
    match seen {
        0 => 0.0,
        1..=3 => seen as f32 * 0.333_333_34,
        _ => 1.0,
    }
}

/// The points `from` sees, keeping to the cone (a direction, the cosine of
/// its angle) when there is one.
fn seen(world: &mut World, from: [f32; 3], points: [[f32; 3]; 5], cone: Option<([f32; 3], f32)>, mask: u32) -> usize {
    let f = frame(world);
    points
        .into_iter()
        .filter(|&p| cone.is_none_or(|(dir, cos)| gsc_t6::math::dot(normalize(sub(p, from)), dir) >= cos))
        .filter(|&p| {
            let t = f.trace_world(from, p, [0.0; 3], [0.0; 3], mask);
            t.fraction >= 1.0 && !t.startsolid
        })
        .count()
}

/// Does `from` see a box (middle and half size in the world): its middle or
/// one of its four outline points.
pub(super) fn sees_box(
    world: &mut World,
    mid: [f32; 3],
    half: [f32; 3],
    from: [f32; 3],
    cone: Option<([f32; 3], f32)>,
    mask: u32,
) -> bool {
    let points = box_points(mid, half, from);
    let f = frame(world);
    points
        .into_iter()
        .filter(|&p| cone.is_none_or(|(dir, cos)| gsc_t6::math::dot(normalize(sub(p, from)), dir) >= cos))
        .any(|p| {
            let t = f.trace_world(from, p, [0.0; 3], [0.0; 3], mask);
            t.fraction >= 1.0 && !t.startsolid
        })
}

/// A box (lowest and highest corner about the origin) turned by `angles`
/// and placed at `origin`: the world box round it, as its middle and half
/// size.
pub(super) fn turned_box(origin: [f32; 3], angles: [f32; 3], lo: [f32; 3], hi: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let (f, r, u) = gsc_t6::math::angle_vectors(angles);
    let mut mins = [f32::MAX; 3];
    let mut maxs = [f32::MIN; 3];
    for c in 0..8 {
        let p = [
            if c & 1 == 0 { lo[0] } else { hi[0] },
            if c & 2 == 0 { lo[1] } else { hi[1] },
            if c & 4 == 0 { lo[2] } else { hi[2] },
        ];
        for i in 0..3 {
            // Local y is left (the right vector's opposite).
            let w = origin[i] + f[i] * p[0] - r[i] * p[1] + u[i] * p[2];
            mins[i] = mins[i].min(w);
            maxs[i] = maxs[i].max(w);
        }
    }
    (
        std::array::from_fn(|i| (mins[i] + maxs[i]) * 0.5),
        std::array::from_fn(|i| (maxs[i] - mins[i]) * 0.5),
    )
}

/// An entity's box in the world (middle, half size): a script model's is
/// its model's, on its origin; anything else's is the world box round its
/// own (a dog's body, a trigger's, a model's turned with it), a point on
/// its origin when it has none.
fn ent_box(world: &mut World, n: u32) -> Option<([f32; 3], [f32; 3])> {
    const BODY: ([f32; 3], [f32; 3]) = ([-15.0, -15.0, 0.0], [15.0, 15.0, 72.0]);
    let zm = world.resource::<Zm>();
    let actor = zm.actors.contains_key(&n);
    let e = zm.ents.get(&n)?;
    let (origin, angles, model, script_model) =
        (e.origin, e.angles, e.model.clone(), e.classname == "script_model");
    let (box_dims, radius, height) = (e.box_dims, e.radius, e.height);
    let model_box = if model.is_empty() || model.starts_with('*') {
        None
    } else {
        frame(world).model_capability(&model).flatten().and_then(|c| c.bounds)
    };
    if script_model && let Some((mid, half)) = model_box {
        return Some((std::array::from_fn(|i| origin[i] + mid[i]), half));
    }
    let (lo, hi) = if actor {
        BODY
    } else if let Some([w, l, h]) = box_dims {
        ([-w * 0.5, -l * 0.5, -h * 0.5], [w * 0.5, l * 0.5, h * 0.5])
    } else if radius > 0.0 {
        ([-radius, -radius, 0.0], [radius, radius, height])
    } else if let Some((mid, half)) = model_box {
        (
            std::array::from_fn(|i| mid[i] - half[i]),
            std::array::from_fn(|i| mid[i] + half[i]),
        )
    } else {
        ([0.0; 3], [0.0; 3])
    };
    Some(turned_box(origin, angles, lo, hi))
}

/// `entity sightconetrace(point, ignore, cone_dir, cone_degrees)` and
/// `damageconetrace`: how much of the entity `point` sees, 0 to 1.
fn cone_trace(vm: &mut Vm<World>, world: &mut World, s: &Value, a: &[Value], mask: u32) -> Result<Value, String> {
    let from = super::super::vec3(a, 0)?;
    let Some(n) = entnum(vm, s) else {
        return Ok(Value::Float(0.0));
    };
    // What it looks past rides on it: it sees it.
    if let Some(i) = entnum(vm, arg(a, 1))
        && world
            .resource::<Zm>()
            .ents
            .get(&i)
            .and_then(|e| e.link)
            .is_some_and(|(parent, _, _)| parent == n)
    {
        return Ok(Value::Float(1.0));
    }
    let cone = match a.get(2) {
        Some(Value::Vec3(dir)) => {
            let deg = arg(a, 3).as_float().unwrap_or(CONE_DEGREES);
            Some((normalize(*dir), deg.to_radians().cos()))
        }
        _ => None,
    };
    if world.resource::<Zm>().players.contains_key(&n) {
        let Some(feet) = frame(world).player(ClientId(n)).map(|ps| ps.origin) else {
            return Ok(Value::Float(0.0));
        };
        let eye = if super::riding(world, n) {
            feet
        } else {
            frame(world)
                .player(ClientId(n))
                .map_or(feet, |ps| [feet[0], feet[1], feet[2] + ps.view_height_current])
        };
        let count = seen(world, from, body_points(feet, eye, from), cone, mask);
        return Ok(Value::Float(body_share(count)));
    }
    let Some((mid, half)) = ent_box(world, n) else {
        return Ok(Value::Float(0.0));
    };
    let hit = sees_box(world, mid, half, from, cone, mask);
    Ok(Value::Float(if hit { 1.0 } else { 0.0 }))
}

pub(super) fn bind(vm: &mut Vm<World>) {
    vm.bind("sightconetrace", true, |vm, world, s, a| cone_trace(vm, world, s, a, SIGHT_MASK));
    vm.bind("damageconetrace", true, |vm, world, s, a| cone_trace(vm, world, s, a, DAMAGE_MASK));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-3)
    }

    #[test]
    fn a_box_is_looked_for_at_its_middle_and_the_corners_facing_the_point() {
        // A 20x40x10 box seen from straight down its x axis: the corners
        // are its y and z extents either side.
        let p = box_points([0.0; 3], [10.0, 20.0, 5.0], [100.0, 0.0, 0.0]);
        assert!(near(p[0], [0.0; 3]));
        assert!(near(p[1], [0.0, 20.0, 5.0]));
        assert!(near(p[2], [0.0, -20.0, 5.0]));
        assert!(near(p[3], [0.0, 20.0, -5.0]));
        assert!(near(p[4], [0.0, -20.0, -5.0]));
        // From straight above, the box is looked at as if from its +x side.
        let p = box_points([0.0; 3], [10.0, 20.0, 5.0], [0.0, 0.0, 100.0]);
        assert!(near(p[1], [0.0, 20.0, 5.0]));
    }

    #[test]
    fn a_player_is_looked_for_round_his_body() {
        // Feet at 0, eye at 60, the point off along +x: 15 to each side
        // along y, 30 up and down from his middle at 30.
        let p = body_points([0.0; 3], [0.0, 0.0, 60.0], [200.0, 0.0, 30.0]);
        assert!(near(p[0], [0.0, 0.0, 30.0]));
        assert!(near(p[1], [0.0, 15.0, 60.0]));
        assert!(near(p[4], [0.0, -15.0, 0.0]));
        // Straight overhead: all five on his middle line.
        let p = body_points([0.0; 3], [0.0, 0.0, 60.0], [0.0, 0.0, 500.0]);
        assert!(near(p[1], [0.0, 0.0, 60.0]));
    }

    #[test]
    fn a_players_share_is_a_third_a_point_and_all_past_three() {
        assert_eq!(body_share(0), 0.0);
        assert!((body_share(1) - 0.333_333_34).abs() < 1e-6);
        assert!((body_share(3) - 1.0).abs() < 1e-6);
        assert_eq!(body_share(4), 1.0);
        assert_eq!(body_share(5), 1.0);
    }

    #[test]
    fn a_turned_box_grows_to_hold_its_corners() {
        // A 20-long, 4-wide box turned a quarter: 4 along x, 20 along y.
        let (mid, half) = turned_box([100.0, 0.0, 0.0], [0.0, 90.0, 0.0], [-10.0, -2.0, 0.0], [10.0, 2.0, 8.0]);
        assert!(near(mid, [100.0, 0.0, 4.0]));
        assert!(near(half, [2.0, 10.0, 4.0]));
    }
}
