//! bo2mp: `visionsetnaked( name, time )`'s fade: the frame's grade runs
//! from the vision it shows to the named one over `time` seconds, each row
//! of the two mixed (the EMP's white flash comes in over 1.5 s, the map's
//! own look back over 5). A zero time, or nothing to fade from, switches at
//! once.

use asset_world::T6Vision;
use bevy_ecs::prelude::{Resource, World};

use super::{LEVEL, Cue, Zm, again};

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
pub(crate) fn naked(game: &mut World, ident: &str, secs: f32) -> bool {
    let len = (secs * 1000.0).round() as i64;
    let (Some(goal), Some(from)) = (asset_world::named_t6_vision(ident), asset_world::t6_naked_vision()) else {
        game.get_resource_or_insert_with(Fade::default).run = None;
        return asset_world::set_t6_vision(ident);
    };
    if len <= 0 {
        game.get_resource_or_insert_with(Fade::default).run = None;
        return asset_world::set_t6_vision(ident);
    }
    let now_ms = game.resource::<Zm>().clock_millis;
    let mut fl = game.get_resource_or_insert_with(Fade::default);
    // A fade already running has its step queued; it carries on goal the new one.
    let queue = fl.run.is_none();
    fl.run = Some(Run { name: ident.to_owned(), from, to: goal, start: now_ms, len, last: Some(from) });
    if queue {
        again(game, LEVEL, now_ms, Cue::VisionFade);
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
    again(world, LEVEL, now + TICK_MS, Cue::VisionFade);
}
