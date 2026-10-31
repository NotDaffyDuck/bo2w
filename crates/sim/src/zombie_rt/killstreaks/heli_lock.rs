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
use gsc_t6::{ScriptValue, Vm};

use super::super::{Closest, Zm, frame, int, origin_of, text, triggers::brush_index, vec3};
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
    pub volume: u32,
    pub origin: [f32; 3],
    pub name_tag: String,
    pub on: bool,
}

/// The ceilings as BO2's spawn records them, in map order: each brush model
/// once, the first 32, all on.
fn collect(found: impl IntoIterator<Item = (u32, [f32; 3], String)>) -> Vec<Patch> {
    let mut dest: Vec<Patch> = Vec::new();
    for (mdl, spot, target_name) in found {
        if dest.len() >= MAX_PATCHES {
            break;
        }
        if dest.iter().any(|p| p.volume == mdl) {
            continue;
        }
        dest.push(Patch { volume: mdl, origin: spot, name_tag: target_name, on: true });
    }
    dest
}

/// The height where a ceiling that is on crosses the vertical line `range`
/// above to `range` below `point`, the crossing nearest `point` winning;
/// `line` gives where along the line (0 top, 1 bottom) a ceiling is met.
fn height(
    patches: &[Patch],
    place: [f32; 3],
    range: f32,
    line: &mut dyn FnMut(&Patch, [f32; 3], [f32; 3]) -> Option<f32>,
) -> Option<f32> {
    let start = [place[0], place[1], place[2] + range];
    let finish = [place[0], place[1], place[2] - range];
    let mut best = Closest::new();
    for p in patches.iter().filter(|p| p.on) {
        let Some(fraction) = line(p, start, finish) else {
            continue;
        };
        let z = (finish[2] - start[2]) * fraction + start[2];
        let off = (z - place[2]).abs();
        best.offer(off, z);
    }
    best.into_item()
}

/// Is `xy` over a ceiling that is on (its model's box at its origin, edges
/// included)?
fn inside(patches: &[Patch], xy: [f32; 2], bounds: &dyn Fn(u32) -> Option<([f32; 3], [f32; 3])>) -> bool {
    patches.iter().filter(|p| p.on).any(|p| {
        bounds(p.volume).is_some_and(|(mins, maxs)| {
            (p.origin[0] + mins[0]..=p.origin[0] + maxs[0]).contains(&xy[0])
                && (p.origin[1] + mins[1]..=p.origin[1] + maxs[1]).contains(&xy[1])
        })
    })
}

/// Where along `start`..`end` a ceiling's model is first met (its brushes
/// and its collision triangles, any contents), as BO2's trace through one
/// brush model at the ceiling's origin; None when the line misses it.
fn line_hit(game: &mut World, pt: &Patch, from_pt: [f32; 3], finish: [f32; 3]) -> Option<f32> {
    let f = frame(game);
    let nil = [0.0; 3];
    // One BO2 trace through the model meets its brushes and its triangles
    // both.
    let map = f.collision_tree().t6?;
    let model = crate::sim_parts::T6Model::Brush(pt.volume);
    let fraction = crate::sim_parts::t6_transformed_trace(map, from_pt, finish, nil, nil, model, u32::MAX, pt.origin, nil).fraction;
    (fraction < 1.0).then_some(fraction)
}

/// The map's ceilings, read from its `heli_height_lock` entities the first
/// time a script asks.
fn patches(vm: &mut Vm<World>, game: &mut World) -> Vec<Patch> {
    if let Some(list) = &streaks(game).heli_patches {
        return list.clone();
    }
    let name = vm.intern("targetname");
    let found: Vec<(u32, [f32; 3], String)> = game
        .resource::<Zm>()
        .ents
        .values()
        .filter(|e| e.map && e.class_tag == "heli_height_lock")
        .filter_map(|e| {
            let targetname = match e.obj.map(|o| vm.raw_field(o, name)) {
                None | Some(ScriptValue::Undefined) => String::new(),
                Some(v) => vm.to_text(&v),
            };
            Some((brush_index(e)?, e.origin, targetname))
        })
        .collect();
    let list = collect(found);
    streaks(game).heli_patches = Some(list.clone());
    list
}

fn inside_lock(vm: &mut Vm<World>, game: &mut World, st: &ScriptValue) -> ScriptValue {
    let Some(o) = origin_of(vm, game, st) else {
        return ScriptValue::Int(0);
    };
    let list = patches(vm, game);
    let f = frame(game);
    let models = &f.collision_submodel_set().models;
    let bounds = |m: u32| models.get(m as usize).map(|c| (c.lo, c.hi));
    ScriptValue::bool(inside(&list, [o[0], o[1]], &bounds))
}

pub(super) fn bind(vm: &mut Vm<World>) {
    // `setheliheightpatchenabled(name, on)`: every ceiling of that name.
    vm.bind("setheliheightpatchenabled", false, |vm, world, _, a| {
        let name = text(vm, a, 0);
        let on = int(a, 1)? != 0;
        let mut list = patches(vm, world);
        for p in list.iter_mut().filter(|p| p.name_tag == name) {
            p.on = on;
        }
        streaks(world).heli_patches = Some(list);
        Ok(ScriptValue::Undefined)
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
        Ok(ScriptValue::Float(z))
    });
    // `isinsideheightlock()`, `ismissileinsideheightlock()`: is the entity
    // over a ceiling that is on?
    vm.bind("isinsideheightlock", true, |vm, world, s, _| Ok(inside_lock(vm, world, s)));
    vm.bind("ismissileinsideheightlock", true, |vm, world, s, _| Ok(inside_lock(vm, world, s)));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch(mdl: u32, z: f32, ident: &str) -> Patch {
        Patch { volume: mdl, origin: [0.0, 0.0, z], name_tag: ident.to_owned(), on: true }
    }

    /// Flat ceilings: model n sits at the patch's own height, everywhere.
    fn flat(pt: &Patch, st: [f32; 3], en: [f32; 3]) -> Option<f32> {
        let z = pt.origin[2];
        let lo = st[2].min(en[2]);
        let hi = st[2].max(en[2]);
        (lo..=hi).contains(&z).then(|| (z - st[2]) / (en[2] - st[2]))
    }

    #[test]
    fn each_model_once_and_at_most_32() {
        let found = (0..40u32).map(|i| (i % 35, [i as f32; 3], format!("p{i}")));
        let list = collect(found);
        assert_eq!(list.len(), MAX_PATCHES);
        assert!(list.iter().all(|p| p.on));
        let first = collect([(7, [1.0; 3], "a".to_owned()), (7, [2.0; 3], "b".to_owned())]);
        assert_eq!(first, vec![Patch { volume: 7, origin: [1.0; 3], name_tag: "a".into(), on: true }]);
    }

    #[test]
    fn the_nearest_ceiling_wins_and_off_ones_are_skipped() {
        let mut list = vec![patch(1, 900.0, "a"), patch(2, 700.0, "war_mode_heli_height_lock")];
        let point = [0.0, 0.0, 650.0];
        assert_eq!(height(&list, point, NEAR, &mut flat), Some(700.0));
        list[1].on = false;
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
        list[0].on = false;
        assert!(!inside(&list, [100.0, 0.0], &bounds));
        assert!(!inside(&[], [0.0, 0.0], &bounds));
    }
}
