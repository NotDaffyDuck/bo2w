//! Turning the map's local material numbers into catalog ids, and the authored
//! sun sprite and flare into the definition the frame reads.

use assets::SurfaceMaterialId;

use crate::assemble::draw_items::LiveMaterialLedger;

/// The catalog id behind a map-local material slot, when the slot is bound.
pub(super) fn map_local_material(
    local: Option<usize>,
    asset_nos: &[Option<usize>],
) -> Option<SurfaceMaterialId> {
    let slot = local?;
    let order = asset_nos.get(slot).copied().flatten()?;
    Some(SurfaceMaterialId::from_sequence(order))
}

/// The colour image a sun sprite or flare material samples.
fn color_image(catalog: &LiveMaterialLedger, material: Option<SurfaceMaterialId>) -> Option<u32> {
    let derived = catalog.derived(material?)?;
    derived
        .semantic_for_image(asset_material::SEMANTIC_COLOR)
        .or_else(|| derived.semantic_for_image(asset_material::TS_2D))
        .map(|image| image.0)
}

pub(super) fn install_sun_effects(
    capture: Option<&asset_world::SunEffectsCapture>,
    asset_nos: &[Option<usize>],
    catalog: &LiveMaterialLedger,
) -> Option<render_frame::SunEffectsDef> {
    let capture = capture?;
    let sprite = map_local_material(capture.sprite_material, asset_nos);
    let flare = map_local_material(capture.flare_material, asset_nos);
    let order = |material: Option<assets::SurfaceMaterialId>| {
        material.and_then(|id| u32::try_from(id.order()).ok())
    };
    Some(render_frame::SunEffectsDef {
        sprite_material: order(sprite),
        flare_material: order(flare),
        sprite_image: color_image(catalog, sprite),
        flare_image: color_image(catalog, flare),
        sprite_size: capture.sprite_size,
        flare_min_size: capture.flare_min_size,
        flare_min_dot: capture.flare_min_dot,
        flare_max_size: capture.flare_max_size,
        flare_max_dot: capture.flare_max_dot,
        flare_max_alpha: capture.flare_max_alpha,
        flare_fade_in_ms: capture.flare_fade_in_ms,
        flare_fade_out_ms: capture.flare_fade_out_ms,
        blind_min_dot: capture.blind_min_dot,
        blind_max_dot: capture.blind_max_dot,
        blind_max_darken: capture.blind_max_darken,
        blind_fade_in_ms: capture.blind_fade_in_ms,
        blind_fade_out_ms: capture.blind_fade_out_ms,
        glare_min_dot: capture.glare_min_dot,
        glare_max_dot: capture.glare_max_dot,
        glare_max_lighten: capture.glare_max_lighten,
        glare_fade_in_ms: capture.glare_fade_in_ms,
        glare_fade_out_ms: capture.glare_fade_out_ms,
        direction: capture.heading,
    })
}
