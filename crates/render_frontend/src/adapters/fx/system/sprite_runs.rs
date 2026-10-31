//! How an effect frame's sprites are grouped into draws and where each one
//! stands in the world.
//!
//! Black Ops II draws its parts in the effects system's own order and then
//! sorts draws by the material's key alone, so the sprites of such a material
//! stay in runs exactly as the host listed them: one draw per run of equal
//! material. Gathering them per material instead put overlapping layers (a
//! fire's flames over its smoke) in a different order every frame, which
//! showed as flicker. Any other material is gathered once, wherever its
//! sprites appear.

use std::collections::HashMap;

use bevy::prelude::*;
use fx::EffectSpriteInstance;
use fx_t6::runtime::{tail_anchor_origin, tail_sprite_axes, tail_sprite_full_extent};

/// Items grouped by material, in the order their draws are issued.
pub(crate) struct Batching<T> {
    pub batches: Vec<(usize, Vec<T>)>,
    /// Items that named no material at all.
    pub without_material: u32,
}

/// Groups `items` by material. A material for which `keeps_order` holds only
/// merges with the run directly before it; the rest merge with the first
/// batch of their material.
pub(crate) fn batch_by_material<T>(
    items: impl IntoIterator<Item = (Option<usize>, T)>,
    keeps_order: impl Fn(usize) -> bool,
) -> Batching<T> {
    let mut batches: Vec<(usize, Vec<T>)> = Vec::new();
    let mut first_batch: HashMap<usize, usize> = HashMap::new();
    let mut without_material = 0u32;
    for (slot, item) in items {
        let Some(asset_id) = slot else {
            without_material = without_material.saturating_add(1);
            continue;
        };
        if keeps_order(asset_id) {
            match batches.last_mut() {
                Some((last, run)) if *last == asset_id => run.push(item),
                _ => batches.push((asset_id, vec![item])),
            }
            continue;
        }
        let at = *first_batch.entry(asset_id).or_insert_with(|| {
            batches.push((asset_id, vec![]));
            batches.len() - 1
        });
        batches[at].1.push(item);
    }
    Batching {
        batches,
        without_material,
    }
}

/// Where a sprite sits and how it faces, as a transform that scales the
/// unit quad to the sprite's size.
pub(crate) fn sprite_transform(spr: &EffectSpriteInstance, camera: &Transform) -> Transform {
    let spin = Quat::from_rotation_z(spr.spin_rad);
    match spr.shape_kind {
        0 => facing_camera(spr, camera, spin),
        2 => tail(spr, camera, spin),
        _ => on_authored_axes(spr, spin),
    }
}

/// A sprite with a second size is that tall; otherwise it is square.
fn quad_scale(sprite: &EffectSpriteInstance) -> Vec3 {
    let height = if sprite.second_extent > 0.0 {
        sprite.second_extent
    } else {
        sprite.start_extent
    };
    Vec3::new(sprite.start_extent * 2.0, height * 2.0, 1.0)
}

fn placed(at: [f32; 3], rotation: Quat, scale: Vec3) -> Transform {
    Transform::from_translation(Vec3::from_array(at))
        .with_rotation(rotation)
        .with_scale(scale)
}

/// A direction as a unit vector, or zero when it has no length.
fn unit(direction: [f32; 3]) -> Vec3 {
    Vec3::from_array(direction).normalize_or_zero()
}

fn facing_camera(sprite: &EffectSpriteInstance, camera: &Transform, spin: Quat) -> Transform {
    placed(sprite.where_at, camera.rotation * spin, quad_scale(sprite))
}

fn on_authored_axes(sprite: &EffectSpriteInstance, spin: Quat) -> Transform {
    let [_, right, up] = sprite.turn_rows.map(unit);
    let basis = Mat3::from_cols(right, up, right.cross(up).normalize_or_zero());
    placed(sprite.where_at, Quat::from_mat3(&basis) * spin, quad_scale(sprite))
}

/// A streak that trails behind its part (a line element runs ahead of it)
/// and turns about its own length to face the eye.
fn tail(spr: &EffectSpriteInstance, camera: &Transform, spin: Quat) -> Transform {
    let along = if spr.line {
        spr.move_dir.map(|v| -v)
    } else {
        spr.move_dir
    };
    let anchor = tail_anchor_origin(spr.where_at, along, spr.second_extent);
    let eye = camera.translation.to_array();
    let Some(axes) = tail_sprite_axes(spr.move_dir, eye, anchor) else {
        // Moving straight at or away from the eye: nothing to see.
        return placed(anchor, camera.rotation, Vec3::ZERO);
    };
    let [across, lengthwise, facing] = axes.map(unit);
    let basis = Mat3::from_cols(across, lengthwise, facing);
    let extent = |size| tail_sprite_full_extent(size);
    let scale = Vec3::new(extent(spr.start_extent), extent(spr.second_extent), 1.0);
    placed(anchor, Quat::from_mat3(&basis) * spin, scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sprite(elem_type: u8) -> EffectSpriteInstance {
        EffectSpriteInstance {
            where_at: [10.0, 20.0, 30.0],
            start_extent: 4.0,
            second_extent: 0.0,
            rgba_tint: [255; 4],
            shape_kind: elem_type,
            definition_name: "test".into(),
            ledger_index: 0,
            definition_slot: 0,
            surface_title: "mat".into(),
            shader_at: Some(1),
            turn_rows: [Vec3::X, Vec3::Y, Vec3::Z].map(|unit| unit.to_array()),
            spin_rad: 0.0,
            move_dir: [0.0, 0.0, 1.0],
            atlas: fx_t6::runtime::BillboardSheetCell::FULL,
            bit_word: 0,
            line: false,
            lit_sample: [0; 3],
            shading_blend: 0,
            packed_lighting_src: fx::BakedShadeSource::White,
        }
    }

    fn rich_first(material: usize) -> bool {
        material < 100
    }

    #[test]
    fn ordered_materials_merge_only_with_the_run_before() {
        let items = [1, 1, 2, 1].map(|m| (Some(m), m * 10));
        let got = batch_by_material(items, rich_first);
        assert_eq!(
            got.batches,
            vec![(1, vec![10, 10]), (2, vec![20]), (1, vec![10])]
        );
    }

    #[test]
    fn other_materials_gather_in_first_seen_order() {
        let items = [(Some(200), 'a'), (Some(300), 'b'), (Some(200), 'c')];
        let got = batch_by_material(items, rich_first);
        assert_eq!(got.batches, vec![(200, vec!['a', 'c']), (300, vec!['b'])]);
    }

    #[test]
    fn the_two_kinds_interleave_without_disturbing_each_other() {
        let items = [
            (Some(5), 'a'),
            (Some(200), 'b'),
            (Some(5), 'c'),
            (Some(200), 'd'),
        ];
        let got = batch_by_material(items, rich_first);
        // 5 is ordered, so its second sprite starts a new run after 200's batch.
        assert_eq!(
            got.batches,
            vec![(5, vec!['a']), (200, vec!['b', 'd']), (5, vec!['c'])]
        );
    }

    #[test]
    fn sprites_without_a_material_are_counted_and_dropped() {
        let items = [(None, 1), (Some(7), 2), (None, 3)];
        let got = batch_by_material(items, rich_first);
        assert_eq!(got.without_material, 2);
        assert_eq!(got.batches, vec![(7, vec![2])]);
    }

    #[test]
    fn a_billboard_follows_the_camera_and_doubles_its_size() {
        let camera = Transform::from_rotation(Quat::from_rotation_y(0.7));
        let t = sprite_transform(&sprite(0), &camera);
        assert_eq!(t.translation, Vec3::new(10.0, 20.0, 30.0));
        assert!(t.rotation.abs_diff_eq(camera.rotation, 1e-6));
        assert_eq!(t.scale, Vec3::new(8.0, 8.0, 1.0));
    }

    #[test]
    fn a_second_size_makes_the_quad_taller() {
        let mut s = sprite(0);
        s.second_extent = 6.0;
        let t = sprite_transform(&s, &Transform::IDENTITY);
        assert_eq!(t.scale, Vec3::new(8.0, 12.0, 1.0));
    }

    #[test]
    fn spin_turns_the_billboard_about_its_facing() {
        let mut s = sprite(0);
        s.spin_rad = std::f32::consts::FRAC_PI_2;
        let t = sprite_transform(&s, &Transform::IDENTITY);
        let turned = t.rotation * Vec3::X;
        assert!(turned.abs_diff_eq(Vec3::Y, 1e-5));
    }

    #[test]
    fn an_authored_sprite_lies_in_its_own_plane() {
        let mut s = sprite(1);
        s.turn_rows = [[0.0; 3], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let t = sprite_transform(&s, &Transform::IDENTITY);
        // right = +Y, up = +Z, so the quad faces +X.
        assert!((t.rotation * Vec3::X).abs_diff_eq(Vec3::Y, 1e-5));
        assert!((t.rotation * Vec3::Y).abs_diff_eq(Vec3::Z, 1e-5));
        assert!((t.rotation * Vec3::Z).abs_diff_eq(Vec3::X, 1e-5));
    }

    #[test]
    fn a_tail_is_anchored_behind_its_part_and_sized_by_both_halves() {
        let mut s = sprite(2);
        s.second_extent = 5.0;
        let camera = Transform::from_translation(Vec3::new(100.0, 20.0, 30.0));
        let t = sprite_transform(&s, &camera);
        // Moving up (+Z), so the tail hangs second_extent below the part.
        assert!(t.translation.abs_diff_eq(Vec3::new(10.0, 20.0, 25.0), 1e-5));
        assert_eq!(t.scale, Vec3::new(8.0, 10.0, 1.0));
    }

    #[test]
    fn a_line_element_runs_ahead_instead() {
        let mut s = sprite(2);
        s.second_extent = 5.0;
        s.line = true;
        let camera = Transform::from_translation(Vec3::new(100.0, 20.0, 30.0));
        let t = sprite_transform(&s, &camera);
        assert!(t.translation.abs_diff_eq(Vec3::new(10.0, 20.0, 35.0), 1e-5));
    }

    #[test]
    fn a_tail_pointing_at_the_eye_vanishes() {
        let mut s = sprite(2);
        s.second_extent = 5.0;
        // The part moves along +Z and the eye is straight above its anchor.
        let camera = Transform::from_translation(Vec3::new(10.0, 20.0, 500.0));
        let t = sprite_transform(&s, &camera);
        assert_eq!(t.scale, Vec3::ZERO);
    }
}
