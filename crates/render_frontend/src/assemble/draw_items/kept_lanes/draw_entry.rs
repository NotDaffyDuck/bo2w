use lighting_t6::PropFacetPath;
use render_frame::FrameOutputKind;
use render_material::SurfaceSortKeyPair;

use crate::assemble::draw_items::kept_lanes::{HeldDrawItem, HeldDrawKind};
use crate::assemble::draw_items::list::blend_content_id;
use crate::{
    LiveMaterialLedger, FacetSamplerInputs, frame_product_kind_for_camera_region,
    lookup_ordered_material,
};
use crate::assemble::packed_word::{PackableDrawCall, BundleKind};

/// Builds an item, taking its material id and camera region from the sorted catalog.
///
/// A rank the catalog cannot resolve leaves both unset; routing then files the item as colour.
pub(crate) fn catalog_item(
    key: u64,
    material_order_no: u32,
    kind: HeldDrawKind,
    facet_samplers: FacetSamplerInputs,
    catalog: &LiveMaterialLedger,
) -> HeldDrawItem {
    let material_id = catalog
        .by_sort_position(material_order_no)
        .map(|entry| entry.asset_id);
    let camera_zone = lookup_ordered_material(catalog, SurfaceSortKeyPair::new(key, material_order_no))
        .ok()
        .map(|material| material.camera_zone);
    HeldDrawItem {
        material_key: material_id,
        key,
        material_order_no,
        kind,
        facet_samplers,
        camera_zone,
    }
}

/// Files `item` under the colour or emissive product its camera region names; regions that
/// draw in neither are dropped there. A caller that passes `distortion` gets the item in that
/// list as well, whatever its region.
pub(crate) fn route_to_products(
    colour: &mut Vec<HeldDrawItem>,
    emissive: &mut Vec<HeldDrawItem>,
    distortion: Option<&mut Vec<HeldDrawItem>>,
    item: HeldDrawItem,
) {
    if let Some(warp_pass) = distortion {
        warp_pass.push(item);
    }
    let target = match frame_product_kind_for_camera_region(item.camera_zone) {
        Some(FrameOutputKind::Colour) => colour,
        Some(FrameOutputKind::Emissive) => emissive,
        _ => return,
    };
    target.push(item);
}

/// Breaks ties between items with the same sort key: the surface or draw index the item came
/// from, so equal keys come out in the same order every frame.
pub(crate) fn draw_order_tie(flavor: &HeldDrawKind) -> u32 {
    match *flavor {
        HeldDrawKind::World { surf, .. } => u32::from(surf),
        HeldDrawKind::Smodel { surface: facet, .. } => facet,
        HeldDrawKind::XModel { surface: facet, .. } => facet,
        HeldDrawKind::CodeMesh { draw, .. } => draw,
        HeldDrawKind::SparkleSwarm { draw, .. } => draw,
        HeldDrawKind::MarkMesh { draw, .. } => draw,
        HeldDrawKind::Glass { draw, .. } => draw,
    }
}

/// The full order of a retained list: sort key first, then the tie.
pub(crate) fn draw_order(item: &HeldDrawItem) -> ((u32, u32, u32), u32) {
    (item.submission_order_key(), draw_order_tie(&item.kind))
}

/// The numbers that tell one draw of a kind from another: which draw it is and what its
/// lighting or stream choice was. Fields a kind does not have are zero.
fn identity_fields(sort: &HeldDrawKind) -> [u64; 4] {
    match *sort {
        HeldDrawKind::World {
            surf, run, run_start_offset, ..
        } => [0, u64::from(surf) | u64::from(run) << 16, u64::from(run_start_offset), 0],
        HeldDrawKind::Smodel {
            placement: pose_at,
            surface,
            illum_handle,
            stream,
            pretess,
            ..
        } => {
            let stream = match stream {
                None => 0,
                Some(PropFacetPath::Rigid) => 1,
                Some(PropFacetPath::Skinned) => 2,
                Some(PropFacetPath::Cached) => 3,
                Some(PropFacetPath::Pretess) => 4,
            };
            let range = pretess.map_or(0, |r| u64::from(r.start) << 32 ^ u64::from(r.count));
            [
                1,
                u64::from(pose_at),
                u64::from(surface) ^ u64::from(illum_handle).rotate_left(16),
                stream ^ range,
            ]
        }
        HeldDrawKind::XModel {
            surface: facet, object_tag, ..
        } => [2, u64::from(facet), u64::from(object_tag), 0],
        HeldDrawKind::CodeMesh {
            draw: draw_item,
            n_args: args,
            gun_view: viewmodel,
            ..
        } => [3, u64::from(draw_item), u64::from(args), u64::from(viewmodel)],
        HeldDrawKind::SparkleSwarm { draw: dr, .. } => [4, u64::from(dr), 0, 0],
        HeldDrawKind::MarkMesh { draw: mark, .. } => [5, u64::from(mark), 0, 0],
        HeldDrawKind::Glass {
            draw: draw_item,
            illum_handle,
            ..
        } => [6, u64::from(draw_item), u64::from(illum_handle), 0],
    }
}

/// Folds what makes `item` the same draw from frame to frame into `id`: its key, which
/// samplers it reads and the identity of the draw behind it.
pub(crate) fn mix_identity(id: &mut u64, item: &HeldDrawItem) {
    // 0 is "no sampler", so a sampler number n becomes n + 1.
    let slot = |sampler: Option<u8>| sampler.map_or(0, |n| u64::from(n) + 1);
    let samplers = &item.facet_samplers;
    blend_content_id(id, item.key);
    blend_content_id(id, slot(samplers.reflection_probe.map(|probe| probe.0)));
    blend_content_id(id, slot(samplers.primary_lightmap.map(|map| map.0)));
    blend_content_id(id, slot(samplers.secondary_lightmap.map(|map| map.0)));
    for field in identity_fields(&item.kind) {
        blend_content_id(id, field);
    }
}

/// What the pack step reads from an item: its key and rank, and where its geometry comes from.
/// Items that pack no geometry of their own (effect meshes, marks, glass) come out as `Skip`.
pub(crate) fn pack_draw(draw_entry: &HeldDrawItem) -> PackableDrawCall {
    let sort = match draw_entry.kind {
        HeldDrawKind::World {
            surf, run, run_start_offset, ..
        } => BundleKind::World { surf, run, run_start_offset },
        HeldDrawKind::Smodel {
            surface,
            illum_handle,
            stream,
            pretess,
            ..
        } => match (stream, pretess) {
            (Some(PropFacetPath::Rigid), _) => BundleKind::PropStatic {
                surface,
                illum_handle,
            },
            (Some(PropFacetPath::Skinned), _) => BundleKind::PropSkinned {
                surface,
                illum_handle,
            },
            (Some(PropFacetPath::Pretess), Some(write_target)) => BundleKind::PropPretess {
                illum_handle,
                write_target,
            },
            (Some(PropFacetPath::Cached), Some(write_target)) => BundleKind::PropCached {
                illum_handle,
                write_target,
            },
            _ => BundleKind::Skip,
        },
        HeldDrawKind::XModel {
            surface,
            illum_handle,
            ..
        } => BundleKind::XModel {
            surface,
            illum_handle,
        },
        _ => BundleKind::Skip,
    };
    PackableDrawCall {
        key: draw_entry.key,
        material_order_no: draw_entry.material_order_no,
        kind: sort,
    }
}
