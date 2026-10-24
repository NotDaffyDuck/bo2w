//! Black Ops II's helicopter height lock: the invisible ceilings a map lays
//! over its play space for the scorestreak aircraft (`heli_height_lock`
//! entities, each a brush model whose collision triangles are the ceiling).
//! The Escort Drone flies at the ceiling over its owner
//! (`getheliheightlockheight`), out of the ceilings' outline it falls back
//! to its minimum height (`isinsideheightlock`), a Dragonfire out of the
//! outline counts down (`ismissileinsideheightlock`), and maps with a second
//! ceiling for war mode switch it off in `main`
//! (`setheliheightpatchenabled("war_mode_heli_height_lock", 0)`).
//!
//! BO2's helicopters can also hold their own height on the ceiling
//! (`setheliheightlock`), but no MP script turns that on.

use bevy_ecs::prelude::World;
use gsc_t6::{Value, Vm};

use super::super::{Zm, frame, int, origin_of, text, triggers::brush_index, vec3};
use super::streaks;

/// BO2 keeps at most this many ceilings; later ones are left out.
const MAX_PATCHES: usize = 32;
/// How far above and below a point a ceiling is looked for: near first,
/// then far.
const NEAR: f32 = 200.0;
const FAR: f32 = 3000.0;

/// One ceiling: its brush model, where it stands, its name, and whether it
/// is on.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Patch {
    pub model: u32,
    pub origin: [f32; 3],
    pub targetname: String,
    pub enabled: bool,
}

/// The ceilings as BO2's spawn records them, in map order: each brush model
/// once, the first 32, all on.
fn collect(found: impl IntoIterator<Item = (u32, [f32; 3], String)>) -> Vec<Patch> {
    let mut out: Vec<Patch> = Vec::new();
    for (model, origin, targetname) in found {
        if out.len() >= MAX_PATCHES {
            break;
        }
        if out.iter().any(|p| p.model == model) {
            continue;
        }
        out.push(Patch { model, origin, targetname, enabled: true });
    }
    out
}

/// The height where a ceiling that is on crosses the vertical line `range`
/// above to `range` below `point`, the crossing nearest `point` winning;
/// `line` gives where along the line (0 top, 1 bottom) a ceiling is met.
fn height(
    patches: &[Patch],
    point: [f32; 3],
    range: f32,
    line: &mut dyn FnMut(&Patch, [f32; 3], [f32; 3]) -> Option<f32>,
) -> Option<f32> {
    let start = [point[0], point[1], point[2] + range];
    let end = [point[0], point[1], point[2] - range];
    let mut best: Option<(f32, f32)> = None;
    for p in patches.iter().filter(|p| p.enabled) {
        let Some(fraction) = line(p, start, end) else {
            continue;
        };
        let z = (end[2] - start[2]) * fraction + start[2];
        let off = (z - point[2]).abs();
        if best.is_none_or(|(b, _)| off < b) {
            best = Some((off, z));
        }
    }
    best.map(|(_, z)| z)
}

/// Is `xy` over a ceiling that is on (its model's box at its origin, edges
/// included)?
fn inside(patches: &[Patch], xy: [f32; 2], bounds: &dyn Fn(u32) -> Option<([f32; 3], [f32; 3])>) -> bool {
    patches.iter().filter(|p| p.enabled).any(|p| {
        bounds(p.model).is_some_and(|(mins, maxs)| {
            (p.origin[0] + mins[0]..=p.origin[0] + maxs[0]).contains(&xy[0])
                && (p.origin[1] + mins[1]..=p.origin[1] + maxs[1]).contains(&xy[1])
        })
    })
}

/// Where along `start`..`end` a ceiling's model is first met (its brushes
/// and its collision triangles, any contents), as BO2's trace through one
/// brush model at the ceiling's origin; None when the line misses it.
fn line_hit(world: &mut World, p: &Patch, start: [f32; 3], end: [f32; 3]) -> Option<f32> {
    let f = frame(world);
    let zero = [0.0; 3];
    // One BO2 trace through the model meets its brushes and its triangles
    // both.
    let map = f.clip_bsp().t6?;
    let model = crate::world::T6Model::Brush(p.model);
    let fraction = crate::world::t6_transformed_trace(map, start, end, zero, zero, model, u32::MAX, p.origin, zero).fraction;
    (fraction < 1.0).then_some(fraction)
}

/// The map's ceilings, read from its `heli_height_lock` entities the first
/// time a script asks.
fn patches(vm: &mut Vm<World>, world: &mut World) -> Vec<Patch> {
    if let Some(list) = &streaks(world).heli_patches {
        return list.clone();
    }
    let name = vm.intern("targetname");
    let found: Vec<(u32, [f32; 3], String)> = world
        .resource::<Zm>()
        .ents
        .values()
        .filter(|e| e.map && e.classname == "heli_height_lock")
        .filter_map(|e| {
            let targetname = match e.obj.map(|o| vm.raw_field(o, name)) {
                None | Some(Value::Undefined) => String::new(),
                Some(v) => vm.to_text(&v),
            };
            Some((brush_index(e)?, e.origin, targetname))
        })
        .collect();
    let list = collect(found);
    streaks(world).heli_patches = Some(list.clone());
    list
}

fn inside_lock(vm: &mut Vm<World>, world: &mut World, s: &Value) -> Value {
    let Some(o) = origin_of(vm, world, s) else {
        return Value::Int(0);
    };
    let list = patches(vm, world);
    let f = frame(world);
    let models = &f.clip_cmodels().models;
    let bounds = |m: u32| models.get(m as usize).map(|c| (c.mins, c.maxs));
    Value::bool(inside(&list, [o[0], o[1]], &bounds))
}

pub(super) fn bind(vm: &mut Vm<World>) {
    // `setheliheightpatchenabled(name, on)`: every ceiling of that name.
    vm.bind("setheliheightpatchenabled", false, |vm, world, _, a| {
        let name = text(vm, a, 0);
        let on = int(a, 1)? != 0;
        let mut list = patches(vm, world);
        for p in list.iter_mut().filter(|p| p.targetname == name) {
            p.enabled = on;
        }
        streaks(world).heli_patches = Some(list);
        Ok(Value::Undefined)
    });
    // `getheliheightlockheight(point)`: the ceiling's height over (or
    // under) a point, looked for 200 up and down, then 3000; the point's own
    // height when there is none.
    vm.bind("getheliheightlockheight", true, |vm, world, _, a| {
        let point = vec3(a, 0)?;
        let list = patches(vm, world);
        let mut line = |p: &Patch, s, e| line_hit(world, p, s, e);
        let z = height(&list, point, NEAR, &mut line)
            .or_else(|| height(&list, point, FAR, &mut line))
            .unwrap_or(point[2]);
        Ok(Value::Float(z))
    });
    // `isinsideheightlock()`, `ismissileinsideheightlock()`: is the entity
    // over a ceiling that is on?
    vm.bind("isinsideheightlock", true, |vm, world, s, _| Ok(inside_lock(vm, world, s)));
    vm.bind("ismissileinsideheightlock", true, |vm, world, s, _| Ok(inside_lock(vm, world, s)));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch(model: u32, z: f32, name: &str) -> Patch {
        Patch { model, origin: [0.0, 0.0, z], targetname: name.to_owned(), enabled: true }
    }

    /// Flat ceilings: model n sits at the patch's own height, everywhere.
    fn flat(p: &Patch, s: [f32; 3], e: [f32; 3]) -> Option<f32> {
        let z = p.origin[2];
        let lo = s[2].min(e[2]);
        let hi = s[2].max(e[2]);
        (lo..=hi).contains(&z).then(|| (z - s[2]) / (e[2] - s[2]))
    }

    #[test]
    fn each_model_once_and_at_most_32() {
        let found = (0..40u32).map(|i| (i % 35, [i as f32; 3], format!("p{i}")));
        let list = collect(found);
        assert_eq!(list.len(), MAX_PATCHES);
        assert!(list.iter().all(|p| p.enabled));
        let first = collect([(7, [1.0; 3], "a".to_owned()), (7, [2.0; 3], "b".to_owned())]);
        assert_eq!(first, vec![Patch { model: 7, origin: [1.0; 3], targetname: "a".into(), enabled: true }]);
    }

    #[test]
    fn the_nearest_ceiling_wins_and_off_ones_are_skipped() {
        let mut list = vec![patch(1, 900.0, "a"), patch(2, 700.0, "war_mode_heli_height_lock")];
        let point = [0.0, 0.0, 650.0];
        assert_eq!(height(&list, point, NEAR, &mut flat), Some(700.0));
        list[1].enabled = false;
        // 900 is out of the near reach; the far look finds it.
        assert_eq!(height(&list, point, NEAR, &mut flat), None);
        assert_eq!(height(&list, point, FAR, &mut flat), Some(900.0));
        // A tie keeps the first.
        let tie = vec![patch(1, 600.0, ""), patch(2, 700.0, "")];
        assert_eq!(height(&tie, point, NEAR, &mut flat), Some(600.0));
        assert_eq!(height(&[], point, FAR, &mut flat), None);
    }

    #[test]
    fn inside_is_over_an_on_ceilings_box_edges_included() {
        let mut list = vec![Patch { origin: [100.0, 0.0, 500.0], ..patch(4, 0.0, "") }];
        let bounds = |m: u32| (m == 4).then_some(([-50.0, -20.0, -5.0], [50.0, 20.0, 5.0]));
        assert!(inside(&list, [150.0, 20.0], &bounds));
        assert!(inside(&list, [60.0, -10.0], &bounds));
        assert!(!inside(&list, [150.1, 0.0], &bounds));
        assert!(!inside(&list, [100.0, -20.5], &bounds));
        list[0].enabled = false;
        assert!(!inside(&list, [100.0, 0.0], &bounds));
        assert!(!inside(&[], [0.0, 0.0], &bounds));
    }
}
