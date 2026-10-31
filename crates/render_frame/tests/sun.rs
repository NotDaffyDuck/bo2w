//! Sun shadow atlas geometry, clip-plane storage and the sun sprite ramp.

use render_frame::{
    SUN_CASTER_TECH, DAYLIGHT_SHADOW_FORCED_PROFILE, SUN_SHADOW_MAX_CLIP_PLANES,
    SUNLIGHT_CASCADE_TOTAL, DaylightShadowAtlasProfile, DaylightShadowCasterLists, SunShadowClipPlanes,
    SunlightShadowRegion, angular_lerp,
};

#[test]
fn the_large_atlas_is_two_wide_and_the_small_one_four() {
    let large = DaylightShadowAtlasProfile::Large;
    assert_eq!(large.target_surface_code(), 10);
    assert_eq!(large.split_size(), 0x400);
    assert_eq!(large.atlas_tiles_across(), 2);
    assert_eq!(large.atlas_px_width(), 0x800);
    let small = DaylightShadowAtlasProfile::Small;
    assert_eq!(small.target_surface_code(), 11);
    assert_eq!(small.split_size(), 0x200);
    assert_eq!(small.atlas_tiles_across(), 4);
    assert_eq!(small.atlas_px_width(), 0x800);
}

#[test]
fn partitions_stack_down_the_first_column() {
    let large = DaylightShadowAtlasProfile::Large;
    assert_eq!(
        large.slice_rect(0),
        Some(SunlightShadowRegion { x: 0, y: 0, width: 0x400, height: 0x400 })
    );
    assert_eq!(
        large.slice_rect(1),
        Some(SunlightShadowRegion { x: 0, y: 0x400, width: 0x400, height: 0x400 })
    );
    assert_eq!(large.slice_rect(SUNLIGHT_CASCADE_TOTAL), None);
    assert_eq!(
        DaylightShadowAtlasProfile::Small.slice_rect(1).map(|v| (v.y, v.width)),
        Some((0x200, 0x200))
    );
}

#[test]
fn the_forced_profile_is_the_large_one() {
    assert_eq!(DAYLIGHT_SHADOW_FORCED_PROFILE, DaylightShadowAtlasProfile::Large);
    assert_eq!(SUNLIGHT_CASCADE_TOTAL, 2);
    assert_eq!(SUN_CASTER_TECH, lighting_t6::TECHNIQUE_SUN_SHADOW_CASTER);
}

#[test]
fn clip_planes_fill_up_and_then_refuse() {
    let mut planes = SunShadowClipPlanes::BLANK;
    assert!(planes.is_empty());
    assert_eq!(planes.len(), 0);
    for i in 0..SUN_SHADOW_MAX_CLIP_PLANES {
        assert!(planes.push([i as f32, 0.0, 0.0, 1.0]));
    }
    assert_eq!(planes.len(), SUN_SHADOW_MAX_CLIP_PLANES);
    assert!(!planes.push([99.0; 4]));
    assert_eq!(planes.len(), SUN_SHADOW_MAX_CLIP_PLANES);
    assert_eq!(planes.as_slice()[3], [3.0, 0.0, 0.0, 1.0]);
    assert_eq!(planes.as_slice().last(), Some(&[9.0, 0.0, 0.0, 1.0]));
    assert!(!planes.is_empty());
}

#[test]
fn caster_lists_start_empty_for_both_partitions() {
    let lists = DaylightShadowCasterLists::default();
    assert_eq!(lists.partitions.len(), 2);
    assert!(lists.partitions.iter().all(|p| p.level_faces.is_empty()));
    assert_eq!(lists, lists.clone());
}

#[test]
fn the_ramp_is_zero_below_one_below_and_linear_between() {
    assert_eq!(angular_lerp(0.2, 0.5, 0.9), 0.0);
    assert_eq!(angular_lerp(0.5, 0.5, 0.9), 0.0);
    assert!((angular_lerp(0.7, 0.5, 0.9) - 0.5).abs() < 1e-6);
    assert_eq!(angular_lerp(0.9, 0.5, 0.9), 1.0);
    assert_eq!(angular_lerp(2.0, 0.5, 0.9), 1.0);
}

#[test]
fn a_collapsed_ramp_steps_and_bad_input_is_off() {
    assert_eq!(angular_lerp(0.6, 0.5, 0.5), 1.0);
    assert_eq!(angular_lerp(0.6, 0.5, 0.4), 1.0);
    assert_eq!(angular_lerp(f32::NAN, 0.5, 0.9), 0.0);
    assert_eq!(angular_lerp(f32::INFINITY, 0.5, 0.9), 0.0);
    assert_eq!(angular_lerp(f32::NEG_INFINITY, 0.5, 0.9), 0.0);
}
