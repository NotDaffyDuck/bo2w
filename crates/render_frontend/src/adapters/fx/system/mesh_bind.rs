//! Tying an effect's code mesh to a catalog material: the texture it
//! samples, the sort key it draws under, and whether the emissive pass may
//! take it at all. A mesh that cannot be bound is skipped and, for the
//! causes the journal tracks, counted.

use asset_game::{FxDefinitions, OwnedFxVisual, lookup_fx_color_image};
use bevy::prelude::*;
use render_fx::{FxJournalCursor, FxWorldColorImages};

use crate::assemble::drawsurf::{
    MaterialGeneration, RuntimeMaterial, RuntimeMaterialCatalog, TechType,
    material_runtime::resolve_material_technique,
};

/// Why a code mesh is left out this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FxPresentSkip {
    /// No colour image, and the emissive technique does not sample one.
    NoColorMap,
    /// The material is not in the sorted catalog.
    NoOrdinal,
    /// The material draws in another camera region than the emissive one.
    NotEmissive,
}

pub(crate) enum FxCodeMeshBind {
    Ready {
        color: Option<Handle<Image>>,
        sort_key: u8,
        ordinal: u32,
    },
    Skip(FxPresentSkip),
}

/// Bumps the journal counter behind `cause`; a missing colour map is not
/// tracked.
pub(crate) fn count_fx_present_skip(cursor: &mut FxJournalCursor, cause: FxPresentSkip) {
    let counter = match cause {
        FxPresentSkip::NoOrdinal => &mut cursor.skipped_no_ordinal,
        FxPresentSkip::NotEmissive => &mut cursor.skipped_not_emissive,
        FxPresentSkip::NoColorMap => return,
    };
    *counter = counter.saturating_add(1);
}

/// The first visual of an effect element that names a material asset.
pub(crate) fn spark_elem_asset_id(
    catalog: &FxDefinitions,
    catalog_index: u16,
    def_index: u8,
) -> Option<usize> {
    let effect = render_fx::present::catalog_lookup(catalog, catalog_index)?;
    let elem = effect.elems.get(usize::from(def_index))?;
    elem.visuals.iter().find_map(OwnedFxVisual::bound_index)
}

/// Binds a code mesh by the material name its effect element carries.
pub(crate) fn code_mesh_bind(
    material_name: &str,
    color_images: &FxWorldColorImages,
    runtime: &MaterialGeneration,
) -> FxCodeMeshBind {
    let catalog = &runtime.catalog;
    let bind_name = asset_game::material_bind_name(material_name);
    let Some((material, ordinal)) = find_by_name(catalog, bind_name) else {
        return FxCodeMeshBind::Skip(FxPresentSkip::NoOrdinal);
    };
    let color = lookup_fx_color_image(&color_images.colors, material_name).cloned();
    let table_key = color_images
        .keys
        .get(material_name)
        .or_else(|| color_images.keys.get(bind_name))
        .map(|&(key, _)| key);
    bind_material(catalog, material, ordinal, color, table_key)
}

/// Binds a code mesh by the material's asset id (tracer beams, sparks).
pub(crate) fn code_mesh_bind_asset(
    asset_id: usize,
    color_images: &FxWorldColorImages,
    runtime: &MaterialGeneration,
) -> FxCodeMeshBind {
    let catalog = &runtime.catalog;
    let Some(material) = catalog.derived(assets::MaterialIndex::from_order(asset_id)) else {
        return FxCodeMeshBind::Skip(FxPresentSkip::NoOrdinal);
    };
    let Some(ordinal) = catalog.sorted_materials.ordinal_for_asset_id(asset_id) else {
        return FxCodeMeshBind::Skip(FxPresentSkip::NoOrdinal);
    };
    let color = color_images.colors_by_asset.get(&asset_id).cloned();
    let table_key = color_images
        .keys_by_asset
        .get(&asset_id)
        .map(|&(key, _)| key);
    bind_material(catalog, material, ordinal.get(), color, table_key)
}

/// Binds a mark surface's material: the sort key baked into the material
/// and its ordinal in the sorted catalog.
pub(crate) fn mark_mesh_bind(
    material_name: &str,
    runtime: &MaterialGeneration,
) -> Option<(u8, u32)> {
    let bind_name = asset_game::material_bind_name(material_name);
    let (material, ordinal) = find_by_name(&runtime.catalog, bind_name)?;
    Some((choose_sort_key(None, material.baked_draw_surf), ordinal))
}

fn find_by_name<'a>(
    catalog: &'a RuntimeMaterialCatalog,
    name: &str,
) -> Option<(&'a RuntimeMaterial, u32)> {
    let material = catalog.material_for_name(name)?;
    let ordinal = catalog
        .sorted_materials
        .ordinal_for_asset_id(usize::from(material.asset_id.0))?;
    Some((material, ordinal.get()))
}

/// The part both entry points share: refuse what the emissive pass cannot
/// draw, otherwise pick the sort key.
fn bind_material(
    catalog: &RuntimeMaterialCatalog,
    material: &RuntimeMaterial,
    ordinal: u32,
    color: Option<Handle<Image>>,
    table_key: Option<u8>,
) -> FxCodeMeshBind {
    let technique_has_texture = || emissive_technique_flagged(catalog, material, ordinal);
    if let Some(cause) = refusal(
        color.is_some(),
        technique_has_texture,
        material.camera_region == render_material::CAMERA_REGION_EMISSIVE,
    ) {
        return FxCodeMeshBind::Skip(cause);
    }
    FxCodeMeshBind::Ready {
        color,
        sort_key: choose_sort_key(table_key, material.baked_draw_surf),
        ordinal,
    }
}

/// Whether the material's emissive technique carries the "samples a colour
/// map" flag. Only asked when no colour image was found.
fn emissive_technique_flagged(
    catalog: &RuntimeMaterialCatalog,
    material: &RuntimeMaterial,
    ordinal: u32,
) -> bool {
    let Some(packed) = material.baked_draw_surf else {
        return false;
    };
    let key = render_material::MaterialDrawKey::new(packed, ordinal);
    resolve_material_technique(catalog, key, TechType(lighting_t6::TECHNIQUE_EMISSIVE))
        .is_ok_and(|(_, technique)| technique.flags & 1 != 0)
}

/// The skip decision, apart from the catalog so it can be tested: a mesh
/// needs a colour image or a technique that brings its own, and must sit in
/// the emissive camera region.
fn refusal(
    has_color: bool,
    technique_has_texture: impl FnOnce() -> bool,
    in_emissive_region: bool,
) -> Option<FxPresentSkip> {
    if !has_color && !technique_has_texture() {
        Some(FxPresentSkip::NoColorMap)
    } else if !in_emissive_region {
        Some(FxPresentSkip::NotEmissive)
    } else {
        None
    }
}

/// The colour table's key when it has one, otherwise the key baked into
/// the material's draw surface, otherwise 0.
fn choose_sort_key(table_key: Option<u8>, baked_draw_surf: Option<u64>) -> u8 {
    table_key
        .or_else(|| {
            baked_draw_surf
                .map(|packed| dpvs_t6::GfxDrawSurf::from_packed(packed).primary_sort_key())
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_colour_image_skips_the_technique_check() {
        let asked = std::cell::Cell::new(false);
        let got = refusal(
            true,
            || {
                asked.set(true);
                false
            },
            true,
        );
        assert_eq!(got, None);
        assert!(!asked.get());
    }

    #[test]
    fn no_image_and_no_texture_flag_is_no_colour_map() {
        assert_eq!(refusal(false, || false, true), Some(FxPresentSkip::NoColorMap));
        // The missing map is reported even when the region is wrong as well.
        assert_eq!(refusal(false, || false, false), Some(FxPresentSkip::NoColorMap));
    }

    #[test]
    fn a_technique_texture_stands_in_for_the_image() {
        assert_eq!(refusal(false, || true, true), None);
        assert_eq!(refusal(false, || true, false), Some(FxPresentSkip::NotEmissive));
    }

    #[test]
    fn the_wrong_region_is_not_emissive() {
        assert_eq!(refusal(true, || true, false), Some(FxPresentSkip::NotEmissive));
    }

    #[test]
    fn sort_key_prefers_the_table_then_the_baked_surface() {
        // The primary sort key lives in bits 58..64.
        let baked = 0x2au64 << 58;
        assert_eq!(choose_sort_key(Some(7), Some(baked)), 7);
        assert_eq!(choose_sort_key(None, Some(baked)), 0x2a);
        assert_eq!(choose_sort_key(None, None), 0);
    }

    #[test]
    fn only_tracked_causes_are_counted() {
        let mut cursor = FxJournalCursor::default();
        count_fx_present_skip(&mut cursor, FxPresentSkip::NoColorMap);
        assert_eq!((cursor.skipped_no_ordinal, cursor.skipped_not_emissive), (0, 0));
        count_fx_present_skip(&mut cursor, FxPresentSkip::NoOrdinal);
        count_fx_present_skip(&mut cursor, FxPresentSkip::NoOrdinal);
        count_fx_present_skip(&mut cursor, FxPresentSkip::NotEmissive);
        assert_eq!((cursor.skipped_no_ordinal, cursor.skipped_not_emissive), (2, 1));
    }
}
