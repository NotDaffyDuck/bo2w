//! bo2mp: `visionsetnaked( name, time )`'s fade: the frame's grade runs
//! from the vision it shows to the named one over `time` seconds, each row
//! of the two mixed (the EMP's white flash comes in over 1.5 s, the map's
//! own look back over 5). A zero time, or nothing to fade from, switches at
//! once.

use asset_world::T6Vision;
use bevy_ecs::prelude::{Resource, World};

use super::{LEVEL, Step, Zm, again};

/// A step of the fade each snapshot.
const TICK_MS: i64 = 50;

#[derive(Resource, Default)]
pub(super) struct Fade {
    run: Option<Run>,
}

struct Run {
    name: String,
    from: T6Vision,
    to: T6Vision,
    start: i64,
    len: i64,
    /// What it put in last: anything else there means something else set
    /// the vision since (a map load, a cut to black) and the fade gives way.
    last: Option<T6Vision>,
}

/// `visionsetnaked`: false when no zone holds the name (the grade stays).
pub(crate) fn naked(world: &mut World, name: &str, secs: f32) -> bool {
    let len = (secs * 1000.0).round() as i64;
    let (Some(to), Some(from)) = (asset_world::named_t6_vision(name), asset_world::t6_naked_vision()) else {
        world.get_resource_or_insert_with(Fade::default).run = None;
        return asset_world::set_t6_vision(name);
    };
    if len <= 0 {
        world.get_resource_or_insert_with(Fade::default).run = None;
        return asset_world::set_t6_vision(name);
    }
    let now = world.resource::<Zm>().now_ms;
    let mut f = world.get_resource_or_insert_with(Fade::default);
    // A fade already running has its step queued; it carries on to the new one.
    let queue = f.run.is_none();
    f.run = Some(Run { name: name.to_owned(), from, to, start: now, len, last: Some(from) });
    if queue {
        again(world, LEVEL, now, Step::VisionFade);
    }
    true
}

/// One step: the mix for now, or the named vision itself at the end.
pub(super) fn tick(world: &mut World, now: i64) {
    let Some(mut f) = world.get_resource_mut::<Fade>() else {
        return;
    };
    let Some(r) = f.run.as_mut() else {
        return;
    };
    if asset_world::t6_naked_vision() != r.last {
        f.run = None;
        return;
    }
    let t = (now - r.start) as f32 / r.len as f32;
    if t >= 1.0 {
        let name = std::mem::take(&mut r.name);
        f.run = None;
        asset_world::set_t6_vision(&name);
        return;
    }
    let v = asset_world::lerp_t6_vision(&r.from, &r.to, t);
    asset_world::set_t6_vision_values(v);
    r.last = Some(v);
    again(world, LEVEL, now + TICK_MS, Step::VisionFade);
}
