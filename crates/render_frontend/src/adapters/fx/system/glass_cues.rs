//! What a Black Ops II window does when its state changes: the sound of the crack or break, and
//! the effect that plays along the edges of a shattered pane.

use asset_core::AssetCategoryScope;
use asset_game::{FxDefinitions, FxName};
use bevy::prelude::*;
use fx::{EffectSystemHost, GlassWorldContact, GlassWorldTrace};

use render_fx::effect_play::{EffectElementInfoMemo, EffectStage, play_named_oriented_in_level};
use crate::assemble::draw_items::T6GlassCue;

/// Shortest outline edge that still gets a shatter effect.
const MIN_EDGE_LENGTH: f32 = 20.0;
/// Target spacing between effects along one edge.
const EFFECT_SPACING: f32 = 55.0;
/// Placement stops once more than this many effects are down.
const MAX_SPOTS: usize = 9;

/// One shatter effect to play: where, and its orientation (facing first).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ShatterSpot {
    pub at_point: [f32; 3],
    pub axes_set: [[f32; 3]; 3],
}

/// The sound alias for what just happened to a pane. Empty when its def names none.
pub(super) fn cue_alias(cue: &T6GlassCue) -> &str {
    let def = &cue.client;
    match (cue.shattered, cue.dir_v == [0.0; 3]) {
        (false, _) => &def.crack_sound,
        (true, true) => &def.auto_shatter_sound,
        (true, false) => &def.shatter_sound,
    }
}

/// Sends each cue's sound to the audio mixer, at the pane's origin.
pub(super) fn sound_cues(cues: &[T6GlassCue], out: &mut MessageWriter<audio::AliasCommand>) {
    for cue in cues {
        let alias = cue_alias(cue);
        if alias.is_empty() {
            continue;
        }
        let fallback = None;
        out.write(audio::AliasCommand::Play(audio::TriggerCueMsg {
            cue_ns: AssetCategoryScope::T6,
            alias: String::from(alias),
            fallback,
            position_in_inches: Some(cue.client.spot),
            source_entity_no: Some(fx_t6::runtime::FX_ENTITYNUM_WORLD),
        }));
    }
}

/// Plays the shatter effect of every pane in `cues` that just broke and has one.
pub(super) fn play_shatter_effects(
    cues: &[T6GlassCue],
    host: &mut EffectSystemHost,
    defs: &FxDefinitions,
    infos: &EffectElementInfoMemo,
    scene: Option<&dyn EffectStage>,
) {
    let msec = host.clock_ms_now;
    for cue in cues.iter().filter(|cue| cue.shattered) {
        let def = &cue.client;
        if def.shatter_effect.is_empty() {
            continue;
        }
        // A pane that broke by itself faces either way; mix the pane and the clock for the flip.
        let flip = (cue.pane ^ msec as u32).wrapping_mul(0x9E37_79B9) >> 31 != 0;
        for spot in shatter_spots(def, cue.dir_v, flip) {
            let _ = play_named_oriented_in_level(
                host,
                defs,
                infos,
                FxName::new(AssetCategoryScope::T6, &def.shatter_effect),
                spot.at_point,
                spot.axes_set,
                scene,
            );
        }
    }
}

/// Where a shattered window plays its effect: along the outline's edges, longest first, one to
/// three per edge spread evenly, until an edge is too short or more than nine are placed. Each
/// faces out of the window toward the break; a pane that broke by itself (`dir` zero) faces the
/// way `flip` says.
pub(super) fn shatter_spots(
    def: &asset_world::T6GlassClientDef,
    dir: [f32; 3],
    flip: bool,
) -> Vec<ShatterSpot> {
    let outline = &def.outline;
    let mut edges: Vec<(Vec2, Vec2, f32)> = (0..outline.len())
        .map(|k| {
            let from = Vec2::from(outline[k]);
            let span = Vec2::from(outline[(k + 1) % outline.len()]) - from;
            let length = span.length();
            let unit = if length > 0.0 { span / length } else { span };
            (from, unit, length)
        })
        .collect();
    edges.sort_by(|a, b| b.2.total_cmp(&a.2));

    let [across, up, normal] = def.axes_m.map(Vec3::from);
    let facing = if dir == [0.0; 3] {
        flip
    } else {
        normal.dot(Vec3::from(dir)) > 0.0
    };
    let front = if facing { normal } else { -normal };
    let axis = [front, up, front.cross(up)].map(|v| v.to_array());
    let base = Vec3::from(def.outline_origin);

    let mut spots = Vec::new();
    for (from, unit, length) in edges {
        if spots.len() > MAX_SPOTS || length < MIN_EDGE_LENGTH {
            break;
        }
        let count = ((length / EFFECT_SPACING + 0.5) as i32).clamp(1, 3);
        let step = length / count as f32;
        for i in 0..count {
            let along = from + unit * ((i as f32 + 0.5) * step);
            spots.push(ShatterSpot {
                at_point: (base + across * along.x + up * along.y).to_array(),
                axes_set: axis,
            });
        }
    }
    spots
}

/// Lets the glass simulation sweep against the prediction world's static geometry.
pub(super) struct PredictionGlassTrace<'a>(pub &'a sim::SimWorld);

impl GlassWorldTrace for PredictionGlassTrace<'_> {
    fn sweep(&self, from_pt: [f32; 3], stop: [f32; 3]) -> Option<GlassWorldContact> {
        let strike = self
            .0
            .trace_static_world(from_pt, stop, [0.0; 3], [0.0; 3], sim::MASK_SHOT);
        let contact = |fraction, end, inside| GlassWorldContact {
            fraction,
            end,
            normal: strike.normal,
            began_inside: inside,
            pane: sim::pane_from_hit(strike.impact_category as i32, strike.struck_entity_num),
        };
        if strike.starts_inside {
            Some(contact(0.0, from_pt, true))
        } else if strike.fraction < 1.0 {
            Some(contact(strike.fraction, strike.finish_location(from_pt, stop), false))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(width: f32, height: f32) -> asset_world::T6GlassClientDef {
        asset_world::T6GlassClientDef {
            outline: vec![[0.0, 0.0], [width, 0.0], [width, height], [0.0, height]],
            axes_m: [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]],
            crack_sound: "crack".into(),
            shatter_sound: "shatter".into(),
            auto_shatter_sound: "auto".into(),
            ..Default::default()
        }
    }

    fn cue(shattered: bool, dir: [f32; 3]) -> T6GlassCue {
        T6GlassCue {
            pane: 7,
            shattered,
            dir_v: dir,
            client: window(100.0, 40.0),
        }
    }

    #[test]
    fn the_sound_follows_what_happened_to_the_pane() {
        assert_eq!(cue_alias(&cue(false, [0.0; 3])), "crack");
        assert_eq!(cue_alias(&cue(false, [0.0, 1.0, 0.0])), "crack");
        assert_eq!(cue_alias(&cue(true, [0.0; 3])), "auto");
        assert_eq!(cue_alias(&cue(true, [0.0, 1.0, 0.0])), "shatter");
    }

    #[test]
    fn long_edges_get_two_effects_and_short_ones_one() {
        let spots = shatter_spots(&window(100.0, 40.0), [0.0, -1.0, 0.0], false);
        assert_eq!(spots.len(), 6);
        assert_eq!(spots[0].at_point, [25.0, 0.0, 0.0]);
        assert_eq!(spots[1].at_point, [75.0, 0.0, 0.0]);
    }

    #[test]
    fn the_effect_faces_out_of_the_window_toward_the_break() {
        let toward_back = shatter_spots(&window(100.0, 40.0), [0.0, -1.0, 0.0], false);
        assert_eq!(toward_back[0].axes_set[0], [0.0, -1.0, 0.0]);
        let toward_front = shatter_spots(&window(100.0, 40.0), [0.0, 1.0, 0.0], false);
        assert_eq!(toward_front[0].axes_set[0], [0.0, 1.0, 0.0]);
    }

    #[test]
    fn a_pane_that_broke_by_itself_faces_by_the_flip() {
        let front = shatter_spots(&window(100.0, 40.0), [0.0; 3], true);
        let back = shatter_spots(&window(100.0, 40.0), [0.0; 3], false);
        assert_eq!(front[0].axes_set[0], [0.0, 1.0, 0.0]);
        assert_eq!(back[0].axes_set[0], [0.0, -1.0, 0.0]);
    }

    #[test]
    fn a_sliver_plays_nothing() {
        let sliver = asset_world::T6GlassClientDef {
            outline: vec![[0.0, 0.0], [15.0, 0.0], [15.0, 10.0]],
            ..window(0.0, 0.0)
        };
        assert!(shatter_spots(&sliver, [0.0; 3], true).is_empty());
    }

    #[test]
    fn a_very_long_edge_is_capped_at_three() {
        let spots = shatter_spots(&window(1000.0, 30.0), [0.0, 1.0, 0.0], false);
        // Two 1000-unit edges at three each, then the 30-unit ones at one each.
        assert_eq!(spots.len(), 3 + 3 + 1 + 1);
    }

    #[test]
    fn placement_stops_once_ten_are_down() {
        // Four 1000-unit edges would give twelve; the stop is checked before each edge.
        let def = asset_world::T6GlassClientDef {
            outline: vec![[0.0, 0.0], [1000.0, 0.0], [1000.0, 1000.0], [0.0, 1000.0]],
            ..window(0.0, 0.0)
        };
        assert_eq!(shatter_spots(&def, [0.0, 1.0, 0.0], false).len(), 12);
        let five_edges = asset_world::T6GlassClientDef {
            outline: vec![
                [0.0, 0.0],
                [1000.0, 0.0],
                [1000.0, 1000.0],
                [0.0, 1000.0],
                [0.0, 500.0],
            ],
            ..window(0.0, 0.0)
        };
        assert_eq!(shatter_spots(&five_edges, [0.0, 1.0, 0.0], false).len(), 12);
    }
}
