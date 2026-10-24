use lighting_t6::SmodelSurfPath;
use render_frame::FrameProductKind;
use render_material::MaterialDrawKey;

use super::{RetainedDrawItem, RetainedDrawKind};
use crate::assemble::drawsurf::list::mix_content_id;
use crate::assemble::drawsurf::{
    RuntimeMaterialCatalog, SurfaceSamplerInputs, frame_product_kind_for_camera_region,
    resolve_sorted_material,
};
use crate::assemble::pack::{PackDraw, PackKind};

/// Builds an item, taking its material id and camera region from the sorted catalog.
///
/// A rank the catalog cannot resolve leaves both unset; routing then files the item as colour.
pub(crate) fn catalog_item(
    key: u64,
    material_rank: u32,
    kind: RetainedDrawKind,
    surface_samplers: SurfaceSamplerInputs,
    catalog: &RuntimeMaterialCatalog,
) -> RetainedDrawItem {
    let material_id = catalog
        .material_for_sorted_ordinal(material_rank)
        .map(|material| material.asset_id);
    let camera_region = resolve_sorted_material(catalog, MaterialDrawKey::new(key, material_rank))
        .ok()
        .map(|material| material.camera_region);
    RetainedDrawItem {
        material_id,
        key,
        material_rank,
        kind,
        surface_samplers,
        camera_region,
    }
}

/// Files `item` under the colour or emissive product its camera region names; regions that
/// draw in neither are dropped there. A caller that passes `distortion` gets the item in that
/// list as well, whatever its region.
pub(crate) fn route_to_products(
    colour: &mut Vec<RetainedDrawItem>,
    emissive: &mut Vec<RetainedDrawItem>,
    distortion: Option<&mut Vec<RetainedDrawItem>>,
    item: RetainedDrawItem,
) {
    if let Some(distortion) = distortion {
        distortion.push(item);
    }
    let target = match frame_product_kind_for_camera_region(item.camera_region) {
        Some(FrameProductKind::Colour) => colour,
        Some(FrameProductKind::Emissive) => emissive,
        _ => return,
    };
    target.push(item);
}

/// Breaks ties between items with the same sort key: the surface or draw index the item came
/// from, so equal keys come out in the same order every frame.
pub(crate) fn draw_order_tie(kind: &RetainedDrawKind) -> u32 {
    match *kind {
        RetainedDrawKind::World { surf, .. } => u32::from(surf),
        RetainedDrawKind::Smodel { surface, .. } => surface,
        RetainedDrawKind::XModel { surface, .. } => surface,
        RetainedDrawKind::CodeMesh { draw, .. } => draw,
        RetainedDrawKind::ParticleCloud { draw, .. } => draw,
        RetainedDrawKind::MarkMesh { draw, .. } => draw,
        RetainedDrawKind::Glass { draw, .. } => draw,
    }
}

/// The full order of a retained list: sort key first, then the tie.
pub(crate) fn draw_order(item: &RetainedDrawItem) -> ((u32, u32, u32), u32) {
    (item.host_sort_key(), draw_order_tie(&item.kind))
}

/// The numbers that tell one draw of a kind from another: which draw it is and what its
/// lighting or stream choice was. Fields a kind does not have are zero.
fn identity_fields(kind: &RetainedDrawKind) -> [u64; 4] {
    match *kind {
        RetainedDrawKind::World {
            surf, run, run_off, ..
        } => [0, u64::from(surf) | u64::from(run) << 16, u64::from(run_off), 0],
        RetainedDrawKind::Smodel {
            placement,
            surface,
            lighting_handle,
            stream,
            pretess,
            ..
        } => {
            let stream = match stream {
                None => 0,
                Some(SmodelSurfPath::Rigid) => 1,
                Some(SmodelSurfPath::Skinned) => 2,
                Some(SmodelSurfPath::Cached) => 3,
                Some(SmodelSurfPath::Pretess) => 4,
            };
            let range = pretess.map_or(0, |r| u64::from(r.start) << 32 ^ u64::from(r.count));
            [
                1,
                u64::from(placement),
                u64::from(surface) ^ u64::from(lighting_handle).rotate_left(16),
                stream ^ range,
            ]
        }
        RetainedDrawKind::XModel {
            surface, object_id, ..
        } => [2, u64::from(surface), u64::from(object_id), 0],
        RetainedDrawKind::CodeMesh {
            draw,
            arg_count,
            viewmodel,
            ..
        } => [3, u64::from(draw), u64::from(arg_count), u64::from(viewmodel)],
        RetainedDrawKind::ParticleCloud { draw, .. } => [4, u64::from(draw), 0, 0],
        RetainedDrawKind::MarkMesh { draw, .. } => [5, u64::from(draw), 0, 0],
        RetainedDrawKind::Glass {
            draw,
            lighting_handle,
            ..
        } => [6, u64::from(draw), u64::from(lighting_handle), 0],
    }
}

/// Folds what makes `item` the same draw from frame to frame into `id`: its key, which
/// samplers it reads and the identity of the draw behind it.
pub(crate) fn mix_identity(id: &mut u64, item: &RetainedDrawItem) {
    // 0 is "no sampler", so a sampler number n becomes n + 1.
    let slot = |sampler: Option<u8>| sampler.map_or(0, |n| u64::from(n) + 1);
    let samplers = &item.surface_samplers;
    mix_content_id(id, item.key);
    mix_content_id(id, slot(samplers.reflection_probe.map(|probe| probe.0)));
    mix_content_id(id, slot(samplers.primary_lightmap.map(|map| map.0)));
    mix_content_id(id, slot(samplers.secondary_lightmap.map(|map| map.0)));
    for field in identity_fields(&item.kind) {
        mix_content_id(id, field);
    }
}

/// What the pack step reads from an item: its key and rank, and where its geometry comes from.
/// Items that pack no geometry of their own (effect meshes, marks, glass) come out as `Skip`.
pub(crate) fn pack_draw(item: &RetainedDrawItem) -> PackDraw {
    let kind = match item.kind {
        RetainedDrawKind::World {
            surf, run, run_off, ..
        } => PackKind::World { surf, run, run_off },
        RetainedDrawKind::Smodel {
            surface,
            lighting_handle,
            stream,
            pretess,
            ..
        } => match (stream, pretess) {
            (Some(SmodelSurfPath::Rigid), _) => PackKind::SmodelRigid {
                surface,
                lighting_handle,
            },
            (Some(SmodelSurfPath::Skinned), _) => PackKind::SmodelSkinned {
                surface,
                lighting_handle,
            },
            (Some(SmodelSurfPath::Pretess), Some(dest)) => PackKind::SmodelPretess {
                lighting_handle,
                dest,
            },
            (Some(SmodelSurfPath::Cached), Some(dest)) => PackKind::SmodelCached {
                lighting_handle,
                dest,
            },
            _ => PackKind::Skip,
        },
        RetainedDrawKind::XModel {
            surface,
            lighting_handle,
            ..
        } => PackKind::XModel {
            surface,
            lighting_handle,
        },
        _ => PackKind::Skip,
    };
    PackDraw {
        key: item.key,
        material_rank: item.material_rank,
        kind,
    }
}
