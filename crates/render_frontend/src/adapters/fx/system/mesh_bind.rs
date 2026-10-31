//! Tying an effect's code mesh to a catalog material: the texture it
//! samples, the sort key it draws under, and whether the emissive pass may
//! take it at all. A mesh that cannot be bound is skipped and, for the
//! causes the journal tracks, counted.

use asset_game::{FxDefinitions, SelfContainedEffectLook, find_tint_picture};
use bevy::prelude::*;
use render_fx::{EffectJournalCursor, EffectLevelColorImages, JournalCount};

use crate::{
    MaterialEpoch, LiveMaterial, LiveMaterialLedger, MethodType,
    material_live::select_material_technique,
};

/// Why a code mesh is left out this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EffectPresentSkip {
    /// No colour image, and the emissive technique does not sample one.
    MissingColorMap,
    /// The material is not in the sorted catalog.
    NoOrdinal,
    /// The material draws in another camera region than the emissive one.
    NoGlow,
}

pub(crate) enum EffectEngineMeshBind {
    Ready {
        tint: Option<Handle<Image>>,
        order_key: u8,
        ord_no: u32,
    },
    Skip(EffectPresentSkip),
}

/// Bumps the journal counter behind `cause`; a missing colour map is not
/// tracked.
pub(crate) fn tally_fx_present_skip(cursor: &mut EffectJournalCursor, why: EffectPresentSkip) {
    match why {
        EffectPresentSkip::NoOrdinal => cursor.bump(JournalCount::MeshNoOrdinal),
        EffectPresentSkip::NoGlow => cursor.bump(JournalCount::MeshNotEmissive),
        EffectPresentSkip::MissingColorMap => {}
    }
}

/// The first visual of an effect element that names a material asset.
pub(crate) fn spark_elem_asset_id(
    catalog: &FxDefinitions,
    ledger_index: u16,
    definition_slot: u8,
) -> Option<usize> {
    let seat = usize::from(definition_slot);
    render_fx::effect_play::recipe_lookup(catalog, ledger_index)?
        .elems
        .get(seat)
        .and_then(|elem| elem.visuals.iter().find_map(SelfContainedEffectLook::extent_position))
}

/// Binds a code mesh by the material name its effect element carries.
pub(crate) fn code_mesh_bind(
    surface_title: &str,
    colour_targets: &EffectLevelColorImages,
    runtime: &MaterialEpoch,
) -> EffectEngineMeshBind {
    let bind_name = asset_game::material_bind_name(surface_title);
    let named = &colour_targets.by_name;
    let table_key = [surface_title, bind_name]
        .into_iter()
        .find_map(|name| named.sort_keys.get(name).copied());
    match find_by_name(&runtime.catalog, bind_name) {
        None => EffectEngineMeshBind::Skip(EffectPresentSkip::NoOrdinal),
        Some((material, ordinal)) => {
            let color = find_tint_picture(&named.colors, surface_title).cloned();
            bind_material(&runtime.catalog, material, ordinal, color, table_key)
        }
    }
}

/// Binds a code mesh by the material's asset id (tracer beams, sparks).
pub(crate) fn code_mesh_bind_asset(
    asset_no: usize,
    colour_targets: &EffectLevelColorImages,
    runtime: &MaterialEpoch,
) -> EffectEngineMeshBind {
    let catalog = &runtime.catalog;
    let found = catalog
        .derived(assets::SurfaceMaterialId::from_sequence(asset_no))
        .zip(catalog.ranked_materials.rank_for_asset_id(asset_no));
    let Some((derived, ord)) = found else {
        return EffectEngineMeshBind::Skip(EffectPresentSkip::NoOrdinal);
    };
    let images = &colour_targets.by_asset;
    bind_material(
        catalog,
        derived,
        ord.get(),
        images.colors.get(&asset_no).cloned(),
        images.sort_keys.get(&asset_no).copied(),
    )
}

/// Binds a mark surface's material: the sort key baked into the material
/// and its ordinal in the sorted catalog.
pub(crate) fn mark_mesh_bind(
    surface_title: &str,
    runtime: &MaterialEpoch,
) -> Option<(u8, u32)> {
    let bind_name = asset_game::material_bind_name(surface_title);
    find_by_name(&runtime.catalog, bind_name)
        .map(|(material, ordinal)| (choose_sort_key(None, material.baked_facet_draw), ordinal))
}

fn find_by_name<'a>(
    catalog: &'a LiveMaterialLedger,
    name: &str,
) -> Option<(&'a LiveMaterial, u32)> {
    let material = catalog.material_by_name(name)?;
    let ord = catalog
        .ranked_materials
        .rank_for_asset_id(usize::from(material.asset_id.0))?;
    Some((material, ord.get()))
}

/// The part both entry points share: refuse what the emissive pass cannot
/// draw, otherwise pick the sort key.
fn bind_material(
    catalog: &LiveMaterialLedger,
    material: &LiveMaterial,
    ord: u32,
    rgba: Option<Handle<Image>>,
    table_key: Option<u8>,
) -> EffectEngineMeshBind {
    let in_emissive_region = material.camera_zone == render_material::VIEW_STAGE_SELF_LIT;
    let refused = refusal(
        rgba.is_some(),
        || emissive_technique_flagged(catalog, material, ord),
        in_emissive_region,
    );
    match refused {
        Some(cause) => EffectEngineMeshBind::Skip(cause),
        None => EffectEngineMeshBind::Ready {
            order_key: choose_sort_key(table_key, material.baked_facet_draw),
            tint: rgba,
            ord_no: ord,
        },
    }
}

/// Whether the material's emissive technique carries the "samples a colour
/// map" flag. Only asked when no colour image was found.
fn emissive_technique_flagged(
    catalog: &LiveMaterialLedger,
    material: &LiveMaterial,
    ord: u32,
) -> bool {
    material.baked_facet_draw.is_some_and(|packed| {
        let key = render_material::SurfaceSortKeyPair::new(packed, ord);
        let emissive = MethodType(lighting_t6::TECHNIQUE_EMISSIVE);
        select_material_technique(catalog, key, emissive)
            .is_ok_and(|pair| pair.1.flags & 1 != 0)
    })
}

/// The skip decision, apart from the catalog so it can be tested: a mesh
/// needs a colour image or a technique that brings its own, and must sit in
/// the emissive camera region.
fn refusal(
    has_color: bool,
    technique_has_texture: impl FnOnce() -> bool,
    in_emissive_region: bool,
) -> Option<EffectPresentSkip> {
    if !has_color && !technique_has_texture() {
        Some(EffectPresentSkip::MissingColorMap)
    } else if !in_emissive_region {
        Some(EffectPresentSkip::NoGlow)
    } else {
        None
    }
}

/// The colour table's key when it has one, otherwise the key baked into
/// the material's draw surface, otherwise 0.
fn choose_sort_key(table_key: Option<u8>, baked_facet_draw: Option<u64>) -> u8 {
    table_key
        .or_else(|| {
            baked_facet_draw
                .map(|packed| dpvs_t6::SortedDraw::unpack_from_bits(packed).leading_sort_bucket())
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
        assert_eq!(refusal(false, || false, true), Some(EffectPresentSkip::MissingColorMap));
        // The missing map is reported even when the region is wrong as well.
        assert_eq!(refusal(false, || false, false), Some(EffectPresentSkip::MissingColorMap));
    }

    #[test]
    fn a_technique_texture_stands_in_for_the_image() {
        assert_eq!(refusal(false, || true, true), None);
        assert_eq!(refusal(false, || true, false), Some(EffectPresentSkip::NoGlow));
    }

    #[test]
    fn the_wrong_region_is_not_emissive() {
        assert_eq!(refusal(true, || true, false), Some(EffectPresentSkip::NoGlow));
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
        let mut cursor = EffectJournalCursor::default();
        tally_fx_present_skip(&mut cursor, EffectPresentSkip::MissingColorMap);
        let counted = |cursor: &EffectJournalCursor| {
            (
                cursor.tally_of(JournalCount::MeshNoOrdinal),
                cursor.tally_of(JournalCount::MeshNotEmissive),
            )
        };
        assert_eq!(counted(&cursor), (0, 0));
        tally_fx_present_skip(&mut cursor, EffectPresentSkip::NoOrdinal);
        tally_fx_present_skip(&mut cursor, EffectPresentSkip::NoOrdinal);
        tally_fx_present_skip(&mut cursor, EffectPresentSkip::NoGlow);
        assert_eq!(counted(&cursor), (2, 1));
    }
}
