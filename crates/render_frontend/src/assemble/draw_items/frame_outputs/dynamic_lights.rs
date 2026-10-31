//! The extra list for dynamic lights.
//!
//! Dynamic lights (muzzle flashes, explosions, ...) do not get a place in the
//! map's light table. Each one lights the colour draws it reaches by drawing
//! them once more, additively, with the light's index stamped into the sort
//! key. This module builds that list from the finished colour list.

use render_frame::OutputTarget;
use render_scene::VisualStage;

use super::super::lamp_receivers::dlight_receivers_kept;
use super::super::kept_lanes::HeldDrawItem;
use super::super::material_setup::MethodType;
use super::super::scene_light_index_of;
use super::bank::COLOUR_MISSING;
use super::in_set::FrameBuildFeed;
use super::seat::{Build, ListSlot};
use super::technique_ix::TechRule;
use crate::prepare::scene::world::LevelStage;

/// What decides whether a draw is within reach of a light.
#[derive(Clone, Copy)]
pub(super) struct Reach<'a> {
    pub(super) scene: Option<&'a LevelStage>,
    pub(super) gfx: Option<&'a VisualStage>,
    pub(super) level_run_facets: &'a [u16],
}

/// The lights added this frame on top of the map's, with their index in the
/// frame's combined light list.
fn added_lights(
    inputs: &FrameBuildFeed,
) -> impl Iterator<Item = (usize, &lighting_t6::LampRecord)> {
    inputs
        .key_lights
        .iter()
        .enumerate()
        .skip(inputs.map_light_tally)
}

/// Digest of the colour composition plus every added light's place, size and
/// type, so the list counts as changed when a light moves or grows.
pub(super) fn digest_with_lights(colour_digest: u64, inputs: &FrameBuildFeed) -> u64 {
    added_lights(inputs).fold(colour_digest, |acc, (at, lamp)| {
        let [x, y, z] = lamp.origin.map(|v| u64::from(v.to_bits()));
        acc ^ u64::from(at as u32).rotate_left(8)
            ^ x
            ^ y.rotate_left(11)
            ^ z.rotate_left(22)
            ^ u64::from(lamp.radius.to_bits()).rotate_left(3)
            ^ u64::from(lamp.kind)
    })
}

/// Appends to `out` one copy of every colour draw per light that `reaches`
/// it, keyed to that light. Lights beyond what a sort key can address are
/// skipped.
fn collect_receivers<'a>(
    colour: &[HeldDrawItem],
    lights: impl Iterator<Item = (usize, &'a lighting_t6::LampRecord)>,
    reaches: impl Fn(&HeldDrawItem, &lighting_t6::LampRecord) -> bool,
    out: &mut Vec<HeldDrawItem>,
) {
    for (at, light) in lights {
        let Ok(index) = u8::try_from(at) else {
            continue;
        };
        for draw in colour {
            if reaches(draw, light) {
                let mut copy = *draw;
                copy.key = scene_light_index_of(draw.key, index);
                out.push(copy);
            }
        }
    }
}

/// Rebuilds `slot` for this frame from the finished `colour` list.
///
/// `tech` is the technique recorded in the product's status; the rows
/// themselves each get theirs from the light they were made for.
pub(super) fn fill_light_list(
    slot: &mut ListSlot,
    colour: &[HeldDrawItem],
    inputs: &FrameBuildFeed,
    reach: Reach<'_>,
    rule: &TechRule<'_>,
    tech: MethodType,
    build: &Build<'_>,
) {
    slot.begin(COLOUR_MISSING);
    if inputs.key_lights.len() <= inputs.map_light_tally {
        return;
    }
    collect_receivers(
        colour,
        added_lights(inputs),
        |draw, lamp| {
            dlight_receivers_kept(
                draw,
                lamp.origin,
                lamp.radius,
                reach.scene,
                reach.gfx,
                reach.level_run_facets,
            )
        },
        &mut slot.product.draw_sequence,
    );
    let stamped = Build {
        digest: digest_with_lights(build.digest, inputs),
        ..*build
    };
    slot.collapse(&stamped, rule, tech, OutputTarget::MainViewColor, false);
}

#[cfg(test)]
mod tests {
    use super::super::lane_set::tests::row;
    use super::*;

    fn light(origin: [f32; 3], radius: f32) -> lighting_t6::LampRecord {
        let mut light = lighting_t6::pack_point_lamp(origin, radius, [1.0; 3]);
        light.kind = 1;
        light
    }

    fn with_lights(map_n: usize, lights: Vec<lighting_t6::LampRecord>) -> FrameBuildFeed {
        FrameBuildFeed {
            map_light_tally: map_n,
            key_lights: lights,
            ..Default::default()
        }
    }

    #[test]
    fn only_the_added_lights_count() {
        let inputs = with_lights(2, vec![light([0.0; 3], 1.0); 5]);
        let seen: Vec<_> = added_lights(&inputs).map(|(at, _)| at).collect();
        assert_eq!(seen, [2, 3, 4]);
    }

    #[test]
    fn without_added_lights_the_digest_is_the_colour_digest() {
        let inputs = with_lights(2, vec![light([0.0; 3], 1.0); 2]);
        assert_eq!(digest_with_lights(77, &inputs), 77);
    }

    #[test]
    fn a_light_that_moves_or_grows_changes_the_digest() {
        let base = with_lights(1, vec![light([9.0; 3], 1.0), light([1.0, 2.0, 3.0], 5.0)]);
        let moved = with_lights(1, vec![light([9.0; 3], 1.0), light([1.0, 2.5, 3.0], 5.0)]);
        let grown = with_lights(1, vec![light([9.0; 3], 1.0), light([1.0, 2.0, 3.0], 6.0)]);
        let map_changed = with_lights(1, vec![light([8.0; 3], 1.0), light([1.0, 2.0, 3.0], 5.0)]);
        let d = digest_with_lights(5, &base);
        assert_ne!(d, 5);
        assert_ne!(d, digest_with_lights(5, &moved));
        assert_ne!(d, digest_with_lights(5, &grown));
        // map lights are part of the colour digest, not of this one
        assert_eq!(d, digest_with_lights(5, &map_changed));
    }

    #[test]
    fn two_equal_lights_at_different_places_in_the_list_differ() {
        let one = with_lights(0, vec![light([1.0; 3], 2.0)]);
        let two = with_lights(0, vec![light([1.0; 3], 2.0), light([1.0; 3], 2.0)]);
        assert_ne!(digest_with_lights(0, &one), digest_with_lights(0, &two));
    }

    #[test]
    fn each_light_gets_its_own_keyed_copy_of_the_draws_it_reaches() {
        let colour = [row(0x100, 1), row(0x200, 2), row(0x300, 3)];
        let lights = [light([0.0; 3], 1.0), light([5.0; 3], 1.0)];
        let mut out = Vec::new();
        // light 0 reaches surfaces 1 and 3, light 1 reaches surface 2
        let reaches = |draw: &HeldDrawItem, light: &lighting_t6::LampRecord| {
            let near_origin = light.origin[0] == 0.0;
            let odd = draw.key == 0x100 || draw.key == 0x300;
            near_origin == odd
        };
        collect_receivers(
            &colour,
            lights.iter().enumerate().map(|(at, l)| (at + 4, l)),
            reaches,
            &mut out,
        );
        let keys: Vec<_> = out.iter().map(|r| r.key).collect();
        assert_eq!(
            keys,
            [
                scene_light_index_of(0x100, 4),
                scene_light_index_of(0x300, 4),
                scene_light_index_of(0x200, 5),
            ]
        );
    }

    #[test]
    fn lights_past_the_addressable_range_are_dropped() {
        let colour = [row(1, 1)];
        let lights = [light([0.0; 3], 1.0)];
        let mut out = Vec::new();
        collect_receivers(
            &colour,
            lights.iter().map(|l| (256usize, l)),
            |_, _| true,
            &mut out,
        );
        assert!(out.is_empty());
    }
}
