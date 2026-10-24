//! Black Ops 1's Gersh Device, the client half
//! (clientscripts/_zombiemode_weap_black_hole_bomb.csc on the Declassified
//! maps): the purple portal and its flare on the thrown bomb, the event
//! horizon when it closes, the break-up on each zombie it pulls in, and
//! the black hole's vision on a player near it who can see it. The server
//! scripts do the pull, the kills and the bomb's end.

use bevy_ecs::prelude::{Resource, World};

use super::Zm;

const PORTAL: &str = "maps/zombie/fx_zmb_blackhole_looping";
const FLARE: &str = "maps/zombie/fx_zmb_blackhole_flare_marker";
const HORIZON: &str = "maps/zombie/fx_zmb_blackhole_implode";
const PULL: &str = "maps/zombie/fx_blackhole_zombie_breakup";

/// The bomb's field (the Declassified port's `setclientfield`; BO1 set a
/// client flag) and the pulled zombie's.
const BOMB: &str = "blackhole";
const PULLED: &str = "black_hole_bomb_being_pulled_fx";

/// black_hole_vision_watcher: in reach and in sight of an open bomb.
const VISION_RANGE: f32 = 512.0;
const VISION_SIGHT: f32 = 0.85;
/// Its seconds in, out, and out when the last bomb is gone.
const FADE_IN: f32 = 2.0;
const FADE_OUT: f32 = 2.0;
const FADE_GONE: f32 = 1.0;

#[derive(Resource, Default)]
struct BlackHoles {
    /// Each open bomb and where it is.
    bombs: Vec<(u32, [f32; 3])>,
    /// Each player's black hole vision: how far in, and where it is going.
    vision: std::collections::HashMap<u32, Fade>,
    last_ms: Option<i64>,
}

#[derive(Clone, Copy, Debug, Default)]
struct Fade {
    t: f32,
    sent: f32,
}

/// An entity's black hole field changed. True when it was one of these.
pub(super) fn entity_field(world: &mut World, n: u32, name: &str, value: i64) -> bool {
    match name {
        BOMB if world.resource::<Zm>().actors.contains_key(&n) => pull(world, n, value != 0),
        BOMB => {
            if value == 0 {
                return true;
            }
            let Some(origin) = world.resource::<Zm>().ents.get(&n).map(|e| e.origin) else {
                return true;
            };
            // A `tag_origin` spot at the bomb, its angles none: the portal
            // stands upright over it whichever way the bomb landed.
            for fx in [PORTAL, FLARE] {
                super::natives_fx::effect_held(world, fx, origin, [1.0, 0.0, 0.0], Some([0.0, 0.0, 1.0]));
            }
            let mut holes = world.get_resource_or_insert_with(BlackHoles::default);
            if !holes.bombs.iter().any(|b| b.0 == n) {
                holes.bombs.push((n, origin));
            }
        }
        PULLED => pull(world, n, value != 0),
        _ => return false,
    }
    true
}

/// black_hole_bomb_being_pulled_fx: the zombie breaks up as it goes in.
fn pull(world: &mut World, n: u32, on: bool) {
    super::natives_fx::bolt_effect(world, n, PULL, "tag_origin", true);
    if on {
        super::clientfields::bolt_queued(world, n, PULL, "tag_origin", false);
    }
}

/// Each tick: a bomb gone closes in its event horizon, and each player's
/// vision fades toward the black hole's while one is near him and in sight.
pub(super) fn tick(world: &mut World) {
    let Some(mut holes) = world.remove_resource::<BlackHoles>() else { return };
    let (now, moon) = {
        let zm = world.resource::<Zm>();
        (zm.now_ms, zm.map.eq_ignore_ascii_case("zm_moon"))
    };
    let dt = holes.last_ms.map_or(0.0, |l| (now - l).max(0) as f32 / 1000.0);
    holes.last_ms = Some(now);
    let gone: Vec<[f32; 3]> = {
        let zm = world.resource::<Zm>();
        let mut gone = Vec::new();
        holes.bombs.retain(|(n, origin)| {
            let open = zm.ents.contains_key(n);
            if !open {
                gone.push(*origin);
            }
            open
        });
        gone
    };
    // The spot goes 0.2 s after the event horizon, its portal with it.
    for origin in gone {
        super::natives_fx::effect_axis(world, HORIZON, origin, [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
        for fx in [PORTAL, FLARE] {
            super::natives_fx::effect_stop_world(world, fx, origin);
        }
    }
    let players: Vec<(u32, [f32; 3], bool)> = {
        let zm = world.resource::<Zm>();
        zm.players
            .iter()
            .filter_map(|(c, p)| zm.ents.get(c).map(|e| (*c, e.origin, p.laststand)))
            .collect()
    };
    for (c, origin, down) in players {
        let eye = [origin[0], origin[1], origin[2] + 60.0];
        let near = holes
            .bombs
            .iter()
            .map(|b| b.1)
            .min_by(|a, b| dist2(eye, *a).total_cmp(&dist2(eye, *b)))
            .filter(|b| dist2(eye, *b) <= VISION_RANGE * VISION_RANGE)
            .is_some_and(|b| {
                let mask = crate::bullet_collision::MASK_SHOT;
                let t = super::frame(world).trace_static_world(eye, b, [0.0; 3], [0.0; 3], mask);
                t.fraction > VISION_SIGHT
            });
        let fade = holes.vision.entry(c).or_default();
        let secs = if near {
            FADE_IN
        } else if holes.bombs.is_empty() {
            FADE_GONE
        } else {
            FADE_OUT
        };
        let step = if secs > 0.0 { dt / secs } else { 1.0 };
        fade.t = if near { (fade.t + step).min(1.0) } else { (fade.t - step).max(0.0) };
        // In steps of 5%: his client re-grades on each new value.
        let t = (fade.t * 20.0).round() / 20.0;
        if t == fade.sent || down {
            continue;
        }
        fade.sent = t;
        let text = {
            let zm = world.resource::<Zm>();
            let fx = &zm.client_fx;
            if t > 0.0 {
                let name = if moon { "zombie_moon_black_hole" } else { "zombie_black_hole" };
                Some(format!("{name} {t}"))
            } else {
                fx.restore_text(c)
            }
        };
        if let Some(text) = text {
            super::set_client_dvar(world, c, "bo2zm_vision", &text);
        }
    }
    holes.vision.retain(|_, f| f.t > 0.0 || f.sent > 0.0);
    if !holes.bombs.is_empty() || !holes.vision.is_empty() {
        world.insert_resource(holes);
    }
}

fn dist2(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)
}
