//! bo2zm: Black Ops II GfxWorld -> WorldDraw. Not part of the upstream engine.
//!
//! The T6 world vertex is 36 bytes (measured on Nuketown, t6world probe):
//! position f32x3 @0, binormal sign f32 @12, colour @16, texcoord half2 @20
//! (u in the low half), normal @24 and tangent @28 packed 10:10:10 signed,
//! lightmap coord unorm16 x2 @32 (u low; measured: it spans the whole
//! lightmap atlas, 0..2048 x 1..3070 texels on lightmap 0, where half floats
//! give nonsense). Surfaces come in groups sharing one vertex
//! block: a surface's vertex `i` is `vd0[vertexDataOffset0 + i * 36]`, with
//! `i` from its own triangles (u16, group-relative). The world keeps each
//! row as BO2 wrote it.
//!
//! Visibility is the map's own cells, their AABB trees and portals (one
//! synthetic cell holding every surface when a world has none). Surfaces the
//! static list leaves out (submodels) sit in one extra tree past the last
//! cell, offered to every view.

use std::collections::HashMap;

use asset_t6::{WorldRef, pack_unit_vec, unpack_unit_vec};
use bevy::asset::RenderAssetUsages;
use bevy::prelude::Mesh;
use bevy::render::mesh::{Indices, PrimitiveTopology};
use dpvs_t6::{AabbTree, Bounds, CPlane};

use crate::world_lightmap::{WorldLightmap, WorldLightmapGap};
use crate::world_lights::WorldSunLight;
use crate::world_scene::{SurfaceDrawFields, WorldBatch, WorldDraw, WorldVertexPayload};
use crate::world_visibility::{
    CameraRangeKind, CameraSurfRange, CameraSurfRanges, DpvsWorldData, OwnedPortal,
};
use crate::mesh_report::{WorldMeshError, WorldMeshStats};
use crate::{SurfaceCastsSunShadow, world_capture_from_casters};

pub use asset_t6::T6_WORLD_VERTEX;

fn half(bits: u16) -> f32 {
    asset_t6::half_to_f32(bits)
}

fn normalized(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l > 1e-6 {
        [v[0] / l, v[1] / l, v[2] / l]
    } else {
        [0.0, 0.0, 1.0]
    }
}

fn rd_u32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn rd_f32(b: &[u8], at: usize) -> f32 {
    f32::from_bits(rd_u32(b, at))
}

fn half2(packed: u32) -> [f32; 2] {
    [half(packed as u16), half((packed >> 16) as u16)]
}

struct Decoded {
    packed: [u8; T6_WORLD_VERTEX],
    position: [f32; 3],
    normal: [f32; 3],
    tangent: [f32; 4],
    color: [f32; 4],
    uv: [f32; 2],
    lightmap_uv: [f32; 2],
}

fn decode_vertex(v: &[u8]) -> Decoded {
    let position = [rd_f32(v, 0), rd_f32(v, 4), rd_f32(v, 8)];
    let binormal_sign = rd_f32(v, 12);
    // T6 colour bytes are R, G, B, A (the lit shaders read them as RGBA unorm).
    let [r, g, b, a] = [v[16], v[17], v[18], v[19]];
    let uv = half2(rd_u32(v, 20));
    let normal = normalized(unpack_unit_vec(rd_u32(v, 24)));
    let tangent3 = normalized(unpack_unit_vec(rd_u32(v, 28)));
    let lm = rd_u32(v, 32);
    let lightmap_uv = [
        f32::from(lm as u16) / 65535.0,
        f32::from((lm >> 16) as u16) / 65535.0,
    ];

    Decoded {
        packed: <[u8; T6_WORLD_VERTEX]>::try_from(&v[..T6_WORLD_VERTEX]).expect("a whole world row"),
        position,
        normal,
        tangent: [tangent3[0], tangent3[1], tangent3[2], binormal_sign],
        color: [r, g, b, a].map(|c| f32::from(c) / 255.0),
        uv,
        lightmap_uv,
    }
}

/// codmp: one window as Black Ops II draws it. `GlassesClient::Update`
/// hands each window that is not shattered to `R_AddGlassBrushToScene`: its
/// brush model's own surfaces, placed at the window's origin and turned by
/// its angles. Those surfaces wear the def's pristine material while the
/// window is whole and its cracked one once damaged
/// (`GlassClient::SetBrushMaterial`). Vertices are in the world.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct T6GlassPaneDraw {
    pub vertices: Vec<T6GlassVertex>,
    /// Into `vertices`.
    pub indices: Vec<u32>,
    pub surfaces: Vec<T6GlassSurface>,
    pub pristine: String,
    pub cracked: String,
    /// What its client does as it breaks (its def's sounds and effects).
    pub client: T6GlassClientDef,
    /// What its shards are made of once it shatters.
    pub shards: T6GlassShardDef,
    /// Its window's bounds (`Glass::absmin/absmax`): a mark takes the pane
    /// only when its box overlaps them.
    pub absmin: [f32; 3],
    pub absmax: [f32; 3],
}

/// bo2mp: what a Black Ops II window's shards are cut from and drawn with
/// (`GlassClient::Shatter`, `GlassShard::Create`): its def's shard material
/// and size limits, the pane's own thickness and texture scale, whether its
/// border counts as frame (`Glass::isPlanar`), and the map's shard pools.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct T6GlassShardDef {
    pub material: String,
    pub thickness: f32,
    pub uv_scale: f32,
    pub is_planar: bool,
    pub min_shard_size: f32,
    pub max_shard_size: f32,
    pub shard_life_probability: f32,
    pub max_shards: i32,
    pub pool_shards: u32,
    pub pool_physics: u32,
}

/// bo2mp: what Black Ops II's client does with a window as it breaks
/// (`GlassClient::SetState`, `GlassClient::PlayShatterFX`): it plays its
/// def's crack sound as it cracks and its shatter sound (auto-shatter sound
/// when it broke by itself) as it shatters, both at the window's origin, and
/// its shatter effect along its outline's edges (`outline` in the plane of
/// `axis[0]`, `axis[1]` about `outline_origin`; `axis[2]` its normal).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct T6GlassClientDef {
    pub origin: [f32; 3],
    pub crack_sound: String,
    pub shatter_sound: String,
    pub auto_shatter_sound: String,
    pub crack_effect: String,
    pub shatter_effect: String,
    pub outline: Vec<[f32; 2]>,
    pub axis: [[f32; 3]; 3],
    pub outline_origin: [f32; 3],
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct T6GlassVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    /// The vertex colour, R, G, B, A in 0..1.
    pub color: [f32; 4],
    pub uv: [f32; 2],
    pub lightmap_uv: [f32; 2],
}

/// One of a window's surfaces: its run of the pane's indices, its lightmap,
/// its primary light, its reflection probe and the world surface it is
/// (for its material).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct T6GlassSurface {
    pub surface: u32,
    pub start: u32,
    pub count: u32,
    pub lightmap: u8,
    pub primary_light: u8,
    pub reflection_probe: u8,
}

/// Every window's surfaces, one entry per placement (pane `i` = piece `i`;
/// a window whose model has no surfaces gets an empty one). A surface's
/// vertex `i` is `vd0[vertexDataOffset0 + i * 36]`, as the world's are.
pub fn build_t6_glass_draws(
    world: &WorldRef,
    glasses: &[asset_t6::GlassRef],
    placements: &[crate::T6GlassPlacement],
) -> Vec<T6GlassPaneDraw> {
    placements
        .iter()
        .map(|place| {
            let mut pane = T6GlassPaneDraw::default();
            if let Some(g) = glasses.get(place.glass) {
                pane.pristine = g.pristine_material.clone();
                pane.cracked = g.cracked_material.clone();
                pane.client = T6GlassClientDef {
                    origin: g.origin,
                    crack_sound: g.crack_sound.clone(),
                    shatter_sound: g.shatter_sound.clone(),
                    auto_shatter_sound: g.auto_shatter_sound.clone(),
                    crack_effect: g.crack_effect.clone(),
                    shatter_effect: g.shatter_effect.clone(),
                    outline: g.outline.clone(),
                    axis: g.axis,
                    outline_origin: g.outline_origin,
                };
                pane.absmin = g.absmin;
                pane.absmax = g.absmax;
                pane.shards = T6GlassShardDef {
                    material: g.shard_material.clone(),
                    thickness: g.pane_thickness,
                    uv_scale: g.uv_scale,
                    is_planar: g.is_planar,
                    min_shard_size: g.min_shard_size,
                    max_shard_size: g.max_shard_size,
                    shard_life_probability: g.shard_life_probability,
                    max_shards: g.max_shards,
                    pool_shards: g.pool_shards,
                    pool_physics: g.pool_physics,
                };
            }
            let model = place.brush_model;
            let Some(&(first, count, _)) = world.brush_models.get(model as usize).filter(|_| model != 0)
            else {
                return pane;
            };
            // `AnglesToAxis`: model space -> world is origin + x*axis[0] + y*axis[1] + z*axis[2].
            let axis = game_math::angles_to_axis(place.angles);
            let turn = |v: [f32; 3]| [0, 1, 2].map(|j| v[0] * axis[0][j] + v[1] * axis[1][j] + v[2] * axis[2][j]);
            for s in first..first.saturating_add(count) {
                let Some(surf) = world.surfaces.get(s as usize) else {
                    continue;
                };
                let base = surf.base_index.max(0) as usize;
                let Some(tris) = world.indices.get(base..base + usize::from(surf.tri_count) * 3) else {
                    continue;
                };
                let Some(&top) = tris.iter().max() else {
                    continue;
                };
                let start0 = surf.vertex_data_offset0.max(0) as usize;
                let Some(block) = world
                    .vd0
                    .get(start0..start0 + (usize::from(top) + 1) * T6_WORLD_VERTEX)
                else {
                    continue;
                };
                let first_vertex = pane.vertices.len() as u32;
                for v in block.chunks_exact(T6_WORLD_VERTEX) {
                    let d = decode_vertex(v);
                    let p = turn(d.position);
                    pane.vertices.push(T6GlassVertex {
                        position: [0, 1, 2].map(|j| place.origin[j] + p[j]),
                        normal: turn(d.normal),
                        color: d.color,
                        uv: d.uv,
                        lightmap_uv: d.lightmap_uv,
                    });
                }
                let start = pane.indices.len() as u32;
                pane.indices.extend(tris.iter().map(|&i| first_vertex + u32::from(i)));
                pane.surfaces.push(T6GlassSurface {
                    surface: s,
                    start,
                    count: tris.len() as u32,
                    lightmap: surf.lightmap_index as u8,
                    primary_light: surf.primary_light_index as u8,
                    reflection_probe: surf.reflection_probe_index as u8,
                });
            }
            pane
        })
        .collect()
}

/// One report line on the windows' surfaces: how many were found, and how
/// many stand inside their window's own bounds (`Glass::absmin/absmax`).
pub fn t6_glass_draw_report(
    panes: &[T6GlassPaneDraw],
    glasses: &[asset_t6::GlassRef],
    placements: &[crate::T6GlassPlacement],
) -> String {
    let mut inside = 0;
    let mut worst = 0.0f32;
    for (pane, place) in panes.iter().zip(placements) {
        let Some(g) = glasses.get(place.glass) else {
            continue;
        };
        if pane.vertices.is_empty() {
            continue;
        }
        let out = pane
            .vertices
            .iter()
            .flat_map(|v| (0..3).map(move |j| (g.absmin[j] - v.position[j]).max(v.position[j] - g.absmax[j])))
            .fold(0.0f32, f32::max);
        worst = worst.max(out);
        if out <= 1.0 {
            inside += 1;
        }
    }
    let mut materials: Vec<(&str, &str)> = panes.iter().map(|p| (p.pristine.as_str(), p.cracked.as_str())).collect();
    materials.sort_unstable();
    materials.dedup();
    format!(
        "t6 glass draw: {} panes, {} with surfaces, {} surfaces, {} triangles, {} inside their window's bounds (worst {:.1} out), materials (pristine, cracked) {:?}",
        panes.len(),
        panes.iter().filter(|p| !p.surfaces.is_empty()).count(),
        panes.iter().map(|p| p.surfaces.len()).sum::<usize>(),
        panes.iter().map(|p| p.indices.len() / 3).sum::<usize>(),
        inside,
        worst,
        materials
    )
}

/// The lighting a T6 world draws with, besides its lightmaps (measured from
/// the lit world shaders, `pimp_shader_lmap_*`): the sun the shaders add,
/// times the lightmap's baked sun visibility, and the exposure multiplier
/// every lit result is scaled by.
#[derive(Clone, Copy, Debug)]
pub struct T6WorldLighting {
    pub sun: WorldSunLight,
    pub exposure_scale: f32,
    /// The sky material's `skyBoxRotation` x, y, z, then its brightness
    /// (`skyColorParm` y), negative when the rotation's w flips z.
    pub sky: [f32; 4],
}

/// One T6 lightmap as the engine's world lightmap. A T6 lightmap is one
/// RGBA8 image of three stacked pages, addressed at `(u, v / 3 + page / 3)`:
/// page 0 base light and page 1 directional light (both `rgb / a`), page 2
/// the light direction (`rgb * 2 - 1`) with the baked sun visibility in
/// alpha. It rides in the secondary slot, read linear; the other slots get
/// 1x1 stand-ins (T6 has no primary page).
pub fn t6_lightmap(image: &asset_t6::ImageRef) -> Option<WorldLightmap> {
    use bevy::asset::RenderAssetUsages;
    use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
    use bevy::prelude::{Image, UVec2};
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

    let embedded = image.embedded.as_ref()?;
    // DXGI_FORMAT_R8G8B8A8_UNORM.
    if embedded.dxgi_format != 28 {
        return None;
    }
    let (w, h) = (u32::from(image.width), u32::from(image.height));
    let bytes = (w * h * 4) as usize;
    let data = embedded.data.get(..bytes)?.to_vec();
    let mut page = Image::new(
        Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    page.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        ..Default::default()
    });
    let stand_in = || {
        Image::new(
            Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            vec![255, 255, 255, 255],
            TextureFormat::Rgba8Unorm,
            RenderAssetUsages::RENDER_WORLD,
        )
    };
    Some(WorldLightmap {
        primary_image: None,
        secondary_image: Some(page),
        secondary_b_image: None,
        ambient_image: stand_in(),
        directional_image: stand_in(),
        sun_mask_image: stand_in(),
        ambient_source_name: image.name.clone(),
        sun_mask_source_name: String::new(),
        ambient_size: UVec2::new(w, h),
        sun_mask_size: UVec2::ONE,
    })
}

/// codmp: a `script_brushmodel` entity (`"model" "*104"`): Black Ops II
/// draws its submodel's surfaces for the entity, where it stands
/// (`R_AddBrushModelToSceneFromAngles`), until a script hides or deletes
/// it. Nuketown's blue house floor under the carpet is one.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct T6BrushPlacement {
    pub brush_model: u32,
    pub origin: [f32; 3],
    pub angles: [f32; 3],
}

/// codmp: where a placed submodel's surfaces sit among the world's (their
/// vertices already moved to where the entity stands); the cull drops them
/// while no live entity shows that submodel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct T6BrushSurfaces {
    pub brush_model: u32,
    pub first_surf: u32,
    pub surface_count: u32,
}

/// A decoded vertex moved from a submodel's space into the world:
/// origin + x*axis[0] + y*axis[1] + z*axis[2] (`AnglesToAxis`). The row's
/// normal and tangent are packed again the way BO2 packs a turned unit vector.
fn place_vertex(d: &mut Decoded, origin: [f32; 3], axis: [[f32; 3]; 3]) {
    let turn = |v: [f32; 3]| [0, 1, 2].map(|j| v[0] * axis[0][j] + v[1] * axis[1][j] + v[2] * axis[2][j]);
    let p = turn(d.position);
    d.position = [0, 1, 2].map(|j| origin[j] + p[j]);
    d.normal = turn(d.normal);
    let t = turn([d.tangent[0], d.tangent[1], d.tangent[2]]);
    d.tangent = [t[0], t[1], t[2], d.tangent[3]];
    for k in 0..3 {
        d.packed[k * 4..k * 4 + 4].copy_from_slice(&d.position[k].to_le_bytes());
    }
    d.packed[24..28].copy_from_slice(&pack_unit_vec(d.normal).to_le_bytes());
    d.packed[28..32].copy_from_slice(&pack_unit_vec(t).to_le_bytes());
}

/// Build the engine's world from a captured T6 GfxWorld. `surface_materials`
/// holds each world surface's material as an index into the lane's catalog
/// (one entry per `world.surfaces`, in order; missing entries are `None`).
/// BO2 materials have no technique the engine can run, so their surfaces
/// draw through the renderer's fallback pass, which samples the colour map.
pub fn build_t6_world_draw(
    world: &WorldRef,
    surface_materials: &[Option<usize>],
    surface_layers: &[asset_core::T6Layers],
    lightmaps: Vec<Option<WorldLightmap>>,
    lighting: Option<T6WorldLighting>,
    brush_placements: &[T6BrushPlacement],
) -> Result<WorldDraw, WorldMeshError> {
    if world.surfaces.is_empty() || world.vd0.is_empty() {
        return Err(WorldMeshError::NoGeometry);
    }

    // codmp: the submodels a `script_brushmodel` stands for: their
    // surfaces are drawn where the entity stands (the first entity naming
    // a submodel places it).
    let mut placed: Vec<Option<usize>> = vec![None; world.surfaces.len()];
    let mut t6_brush_surfaces = Vec::new();
    for (pi, place) in brush_placements.iter().enumerate() {
        let model = place.brush_model;
        let Some(&(first, count, _)) = world.brush_models.get(model as usize).filter(|_| model != 0)
        else {
            continue;
        };
        let range = first as usize..(first as usize + count as usize).min(placed.len());
        if range.is_empty() || placed[range.clone()].iter().any(Option::is_some) {
            continue;
        }
        for slot in &mut placed[range.clone()] {
            *slot = Some(pi);
        }
        t6_brush_surfaces.push(T6BrushSurfaces {
            brush_model: model,
            first_surf: range.start as u32,
            surface_count: range.len() as u32,
        });
    }
    let place_axis: Vec<[[f32; 3]; 3]> = brush_placements
        .iter()
        .map(|p| game_math::angles_to_axis(p.angles))
        .collect();

    // Each vertex group is expanded once per second-stream layout its
    // surfaces read it with (layered surfaces add their layers' texcoords
    // from `vd1`), sized by the largest index those surfaces use; a placed
    // submodel's surfaces get their own (their vertices move).
    let group_key = |i: usize| -> (i32, i32, asset_core::T6Layers, usize) {
        let surf = &world.surfaces[i];
        let at = placed[i].unwrap_or(usize::MAX);
        match surface_layers.get(i) {
            Some(layers) if layers.any() => {
                (surf.vertex_data_offset0, surf.vertex_data_offset1, *layers, at)
            }
            _ => (
                surf.vertex_data_offset0,
                -1,
                asset_core::T6Layers::default(),
                at,
            ),
        }
    };
    let mut group_len: HashMap<(i32, i32, asset_core::T6Layers, usize), usize> = HashMap::new();
    for (i, surf) in world.surfaces.iter().enumerate() {
        let first = surf.base_index as usize;
        let count = surf.tri_count as usize * 3;
        let indices = world
            .indices
            .get(first..first + count)
            .ok_or(WorldMeshError::NoGeometry)?;
        let need = indices.iter().map(|&i| i as usize + 1).max().unwrap_or(0);
        let e = group_len.entry(group_key(i)).or_insert(0);
        *e = (*e).max(need);
    }

    let mut group_base: HashMap<(i32, i32, asset_core::T6Layers, usize), u32> = HashMap::new();
    let mut layer_uvs: Vec<[f32; 4]> = Vec::new();
    let mut packed_vertices = Vec::new();
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut tangents = Vec::new();
    let mut colors = Vec::new();
    let mut texture_uvs = Vec::new();
    let mut lightmap_uvs = Vec::new();
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];

    let mut packed_indices: Vec<u32> = Vec::new();
    let n = world.surfaces.len();
    let mut surface_index_ranges = Vec::with_capacity(n);
    let mut surface_first_vertex = Vec::with_capacity(n);
    let mut surface_draw_fields = Vec::with_capacity(n);
    let mut surface_lightmap_indices = Vec::with_capacity(n);
    let mut surface_reflection_probes = Vec::with_capacity(n);
    let mut surface_primary_lights = Vec::with_capacity(n);
    let mut surface_bounds = Vec::with_capacity(n);

    // The surfaces the game draws: the static visibility list's. Others
    // (Nuketown's orange light dome) only feed the light compiler.
    let mut drawn = vec![world.sorted_surf_index.is_empty(); world.surfaces.len()];
    for &i in &world.sorted_surf_index {
        if let Some(d) = drawn.get_mut(usize::from(i)) {
            *d = true;
        }
    }
    for (d, at) in drawn.iter_mut().zip(&placed) {
        *d |= at.is_some();
    }
    for (surf_index, surf) in world.surfaces.iter().enumerate() {
        let key = group_key(surf_index);
        let base = match group_base.get(&key) {
            Some(&base) => base,
            None => {
                let base = positions.len() as u32;
                let count = group_len[&key];
                let start = surf.vertex_data_offset0 as usize;
                let block = world
                    .vd0
                    .get(start..start + count * T6_WORLD_VERTEX)
                    .ok_or(WorldMeshError::NoGeometry)?;
                let (_, start1, layers, at) = key;
                for (vi, v) in block.chunks_exact(T6_WORLD_VERTEX).enumerate() {
                    // Layers 1 and 2's texcoords from the second stream.
                    let mut layer = [0.0f32; 4];
                    if start1 >= 0 {
                        for k in 0..2 {
                            if layers.kinds[k] == 0 {
                                continue;
                            }
                            let at = start1 as usize
                                + vi * usize::from(layers.stride)
                                + usize::from(layers.uv_offsets[k]);
                            if let Some(b) = world.vd1.get(at..at + 4) {
                                let uv = half2(rd_u32(b, 0));
                                layer[k * 2] = uv[0];
                                layer[k * 2 + 1] = uv[1];
                            }
                        }
                    }
                    layer_uvs.push(layer);
                    let mut d = decode_vertex(v);
                    if let Some(place) = brush_placements.get(at) {
                        place_vertex(&mut d, place.origin, place_axis[at]);
                    }
                    for axis in 0..3 {
                        min[axis] = min[axis].min(d.position[axis]);
                        max[axis] = max[axis].max(d.position[axis]);
                    }
                    packed_vertices.push(d.packed);
                    positions.push(d.position);
                    normals.push(d.normal);
                    tangents.push(d.tangent);
                    colors.push(d.color);
                    texture_uvs.push(d.uv);
                    lightmap_uvs.push(d.lightmap_uv);
                }
                group_base.insert(key, base);
                base
            }
        };
        let range_start = packed_indices.len() as u32;
        let first = surf.base_index as usize;
        let count = if drawn[surf_index] {
            surf.tri_count as usize * 3
        } else {
            0
        };
        for &i in &world.indices[first..first + count] {
            packed_indices.push(base + i as u32);
        }
        surface_index_ranges.push((range_start, count as u32));
        surface_first_vertex.push(base);
        let lightmap_index = surf.lightmap_index as u8;
        let probe = surf.reflection_probe_index as u8;
        let light = surf.primary_light_index as u8;
        surface_lightmap_indices.push(lightmap_index);
        surface_reflection_probes.push(probe);
        surface_primary_lights.push(light);
        surface_draw_fields.push(SurfaceDrawFields {
            first_vertex: base,
            tri_count: (count / 3) as u16,
            base_index: range_start,
            lightmap_index,
            reflection_probe_index: probe,
            primary_light_index: light,
        });
        // A placed submodel's surface: its bounds where it now stands.
        let bounds = match placed[surf_index] {
            Some(_) => world.indices[first..first + count]
                .iter()
                .filter_map(|&i| positions.get((base + u32::from(i)) as usize))
                .fold(None, |acc: Option<([f32; 3], [f32; 3])>, p| {
                    let (lo, hi) = acc.unwrap_or((*p, *p));
                    Some(([0, 1, 2].map(|k| lo[k].min(p[k])), [0, 1, 2].map(|k| hi[k].max(p[k]))))
                })
                .unwrap_or((surf.bounds[0], surf.bounds[1])),
            None => (surf.bounds[0], surf.bounds[1]),
        };
        surface_bounds.push(Bounds::from_mins_maxs(bounds.0, bounds.1));
    }

    let surface_materials: Vec<Option<usize>> = (0..n)
        .map(|i| surface_materials.get(i).copied().flatten())
        .collect();
    // A surface is lightmapped when its lightmap index names a page this
    // world built (31 = none).
    let surface_lightmapped: Vec<bool> = surface_lightmap_indices
        .iter()
        .map(|&i| lightmaps.get(usize::from(i)).is_some_and(Option::is_some))
        .collect();
    let (batches, surface_batch_ranges) = make_material_batches(
        &positions,
        &normals,
        &tangents,
        &colors,
        &texture_uvs,
        &lightmap_uvs,
        &packed_indices,
        &surface_index_ranges,
        &surface_materials,
        &surface_lightmapped,
        &surface_lightmap_indices,
        &surface_primary_lights,
        &surface_reflection_probes,
    );

    let dpvs = if world.cell_bounds.is_empty() || world.dpvs_nodes.is_empty() {
        one_cell_dpvs(world, surface_bounds)?
    } else {
        cells_dpvs(world, surface_bounds)?
    };
    let stats = WorldMeshStats {
        vertices: positions.len(),
        triangles: packed_indices.len() / 3,
        surfaces: n,
        min,
        max,
    };

    Ok(WorldDraw {
        batches,
        sky_model: None,
        lightmap: if lightmaps.iter().any(Option::is_some) {
            Ok(lightmaps)
        } else {
            Err(WorldLightmapGap::Missing)
        },
        stats,
        packed_vertices: WorldVertexPayload::T6(packed_vertices),
        vertex_layer: Vec::new(),
        surface_vertex_layer: vec![0; n],
        surface_first_vertex,
        surface_draw_fields,
        positions,
        normals,
        tangents,
        colors,
        texture_uvs,
        lightmap_uvs,
        packed_indices,
        surface_index_ranges,
        surface_batch_ranges,
        surface_lightmapped,
        surface_lightmap_indices,
        surface_reflection_probes,
        surface_primary_lights,
        sort_key_distortion: None,
        capture: world_capture_from_casters(SurfaceCastsSunShadow::with_len(n)),
        brush_models: Vec::new(),
        brush_model_bounds: Vec::new(),
        surface_materials,
        primary_lights: Vec::new(),
        light_defs: Vec::new(),
        sun_primary_light_count: 0,
        light_region_hulls: None,
        shadow_geometry: Vec::new(),
        reflection_probes: Vec::new(),
        dpvs,
        outdoor_image_name: None,
        outdoor_image: None,
        outdoor_lookup: [0; 16],
        sun_effects: None,
        // bo2zm: the T5 sun and exposure carriers hold the T6 world's; for a
        // T6 world only the renderer's fallback draw reads them.
        t5_sun_parse_exposure: lighting.map(|l| l.exposure_scale),
        t5_sky_dynamic_intensity: lighting.map(|l| l.sky),
        t5_sun_light: lighting.map(|l| l.sun),
        t5_tree_scatter_intensity: None,
        t5_tree_scatter_amount: None,
        t5_exposure_volume_count: 0,
        t6_lights: Vec::new(),
        t6_fog_banks: Vec::new(),
        t6_probes: Vec::new(),
        t6_layer_uvs: layer_uvs,
        t6_brush_surfaces,
    })
}

/// The map's cells: BO2's static surface list, each cell's AABB trees and
/// portals, and the cell tree. Surfaces the list leaves out follow it in
/// `sorted_surf_index`, under one node in an extra tree at `cell_count`.
fn cells_dpvs(
    world: &WorldRef,
    surface_bounds: Vec<Bounds>,
) -> Result<DpvsWorldData, WorldMeshError> {
    let n = world.surfaces.len();
    u16::try_from(n).map_err(|_| WorldMeshError::NoGeometry)?;
    let mut out = DpvsWorldData::new(CameraSurfRanges::new(
        CameraSurfRange {
            kind: CameraRangeKind::LitOpaque,
            begin: 0,
            end: n as u32,
        },
        Vec::new(),
    ));
    let cell_count = world.cell_bounds.len();
    out.cell_count = cell_count;
    out.lit_opaque_begin = 0;
    out.lit_opaque_end = n as u32;
    out.planes = world
        .dpvs_planes
        .iter()
        .map(|p| CPlane {
            normal: [p[0], p[1], p[2]],
            dist: p[3],
            r#type: (0..3).find(|&k| p[k] == 1.0).unwrap_or(3) as u8,
        })
        .collect();
    out.nodes.clone_from(&world.dpvs_nodes);

    let mut listed = vec![false; n];
    let mut sorted: Vec<u16> = Vec::with_capacity(n);
    for &i in &world.sorted_surf_index {
        if let Some(l) = listed.get_mut(usize::from(i)) {
            *l = true;
        }
        sorted.push(i);
    }
    let static_end = sorted.len();
    sorted.extend((0..n as u16).filter(|&i| !listed[usize::from(i)]));
    let extra = u16::try_from(sorted.len() - static_end).map_err(|_| WorldMeshError::NoGeometry)?;

    for i in 0..cell_count {
        let trees = world.cells.get(i).map_or(&[][..], Vec::as_slice);
        // BO2's nodes as stored: childrenOffset stays in bytes of 40-byte nodes.
        let nodes: Vec<AabbTree> = trees
            .iter()
            .map(|t| AabbTree {
                mins: t.mins,
                maxs: t.maxs,
                child_count: t.child_count,
                surface_count: t.surface_count,
                start_surf_index: t.start_surf_index,
                smodel_index_count: 0,
                smodel_index_start: 0,
                children_offset: t.children_offset,
            })
            .collect();
        out.aabb_trees.push(nodes);
        out.aabb_smodel_indices.push(Vec::new());
        out.cell_reflection_probes.push(Vec::new());
        let mut portals = Vec::new();
        for p in &world.cell_bounds[i].portals {
            let vert_start = out.portal_verts.len();
            out.portal_verts.extend_from_slice(&p.vertices);
            portals.push(OwnedPortal {
                plane: p.plane,
                neighbor: p.cell,
                vert_start,
                vert_count: p.vertices.len(),
                hull_axis: Some(p.hull_axis),
            });
        }
        out.portals_per_cell.push(portals);
    }
    if extra > 0 {
        let start = static_end as u16;
        out.aabb_trees.push(vec![AabbTree {
            mins: world.mins,
            maxs: world.maxs,
            surface_count: extra,
            start_surf_index: start,
            ..Default::default()
        }]);
        out.aabb_smodel_indices.push(Vec::new());
    }
    out.static_surface_count = sorted.len();
    out.static_surface_count_no_decal = sorted.len();
    out.sorted_surf_index = sorted;
    out.surface_bounds = surface_bounds;
    out.checked()
}

/// One cell, one AABB node over every surface, identity sort order.
fn one_cell_dpvs(
    world: &WorldRef,
    surface_bounds: Vec<Bounds>,
) -> Result<DpvsWorldData, WorldMeshError> {
    let n = world.surfaces.len();
    let count = u16::try_from(n).map_err(|_| WorldMeshError::NoGeometry)?;
    let mut out = DpvsWorldData::new(CameraSurfRanges::new(
        CameraSurfRange {
            kind: CameraRangeKind::LitOpaque,
            begin: 0,
            end: n as u32,
        },
        Vec::new(),
    ));
    out.cell_count = 1;
    out.static_surface_count = n;
    out.static_surface_count_no_decal = n;
    out.lit_opaque_begin = 0;
    out.lit_opaque_end = n as u32;
    out.sorted_surf_index = (0..count).collect();
    out.surface_bounds = surface_bounds;
    out.aabb_trees = vec![vec![AabbTree {
        mins: world.mins,
        maxs: world.maxs,
        surface_count: count,
        ..Default::default()
    }]];
    out.aabb_smodel_indices = vec![Vec::new()];
    out.portals_per_cell = vec![Vec::new()];
    out.cell_reflection_probes = vec![Vec::new()];
    out.checked()
}

#[derive(Default)]
struct BatchBuilder {
    material: Option<usize>,
    lightmapped: bool,
    lightmap_index: u8,
    primary_light_index: u8,
    reflection_probe_index: u8,
    vertices: HashMap<u32, u32>,
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    tangents: Vec<[f32; 4]>,
    colors: Vec<[f32; 4]>,
    texture_uvs: Vec<[f32; 2]>,
    lightmap_uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn make_material_batches(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    tangents: &[[f32; 4]],
    colors: &[[f32; 4]],
    texture_uvs: &[[f32; 2]],
    lightmap_uvs: &[[f32; 2]],
    packed_indices: &[u32],
    surface_index_ranges: &[(u32, u32)],
    surface_materials: &[Option<usize>],
    surface_lightmapped: &[bool],
    surface_lightmap_indices: &[u8],
    surface_primary_lights: &[u8],
    surface_reflection_probes: &[u8],
) -> (Vec<WorldBatch>, Vec<(usize, u32, u32)>) {
    let mut builders: Vec<BatchBuilder> = Vec::new();
    let mut surface_batch_ranges = Vec::with_capacity(surface_index_ranges.len());

    for (i, &(start, count)) in surface_index_ranges.iter().enumerate() {
        if count == 0 {
            surface_batch_ranges.push((0, 0, 0));
            continue;
        }
        let material = surface_materials.get(i).copied().flatten();
        let lightmapped = surface_lightmapped.get(i).copied().unwrap_or(false);
        let lightmap_index = surface_lightmap_indices[i];
        let primary_light_index = surface_primary_lights.get(i).copied().unwrap_or(0);
        let reflection_probe_index = surface_reflection_probes.get(i).copied().unwrap_or(0);

        let batch_index = builders
            .iter()
            .position(|batch| {
                batch.material == material
                    && batch.lightmapped == lightmapped
                    && batch.lightmap_index == lightmap_index
                    && batch.primary_light_index == primary_light_index
                    && batch.reflection_probe_index == reflection_probe_index
            })
            .unwrap_or_else(|| {
                let index = builders.len();
                builders.push(BatchBuilder {
                    material,
                    lightmapped,
                    lightmap_index,
                    primary_light_index,
                    reflection_probe_index,
                    ..Default::default()
                });
                index
            });

        let batch = &mut builders[batch_index];
        let local_start = batch.indices.len() as u32;
        for &absolute in &packed_indices[start as usize..(start + count) as usize] {
            let local = *batch.vertices.entry(absolute).or_insert_with(|| {
                let local = batch.positions.len() as u32;
                batch.positions.push(positions[absolute as usize]);
                batch.normals.push(normals[absolute as usize]);
                batch.tangents.push(tangents[absolute as usize]);
                batch.colors.push(colors[absolute as usize]);
                batch.texture_uvs.push(texture_uvs[absolute as usize]);
                batch.lightmap_uvs.push(lightmap_uvs[absolute as usize]);
                local
            });
            batch.indices.push(local);
        }
        let local_count = batch.indices.len() as u32 - local_start;
        surface_batch_ranges.push((batch_index, local_start, local_count));
    }

    let batches = builders
        .into_iter()
        .map(|batch| {
            let packed_indices = batch.indices.clone();
            let mut mesh = Mesh::new(
                PrimitiveTopology::TriangleList,
                RenderAssetUsages::default(),
            );
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, batch.positions);
            mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, batch.normals);
            mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, batch.tangents);
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, batch.colors);
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, batch.texture_uvs);
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, batch.lightmap_uvs);
            mesh.insert_indices(Indices::U32(batch.indices));
            WorldBatch {
                mesh,
                packed_indices,
                material: batch.material,
                lightmapped: batch.lightmapped,
                lightmap_index: batch.lightmap_index,
                primary_light_index: batch.primary_light_index,
                reflection_probe_index: batch.reflection_probe_index,
            }
        })
        .collect();

    (batches, surface_batch_ranges)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t6_glass_pane_stands_where_its_window_does() {
        // One triangle in model space, its normal +z, lightmap 2.
        let mut vd0 = Vec::new();
        for p in [[10.0f32, 0.0, 0.0], [0.0, 10.0, 0.0], [0.0, 0.0, 0.0]] {
            let mut v = [0u8; T6_WORLD_VERTEX];
            for k in 0..3 {
                v[k * 4..k * 4 + 4].copy_from_slice(&p[k].to_le_bytes());
            }
            v[16..20].copy_from_slice(&[255; 4]);
            v[24..28].copy_from_slice(&(0x1ffu32 << 20).to_le_bytes());
            vd0.extend_from_slice(&v);
        }
        let world = WorldRef {
            vd0,
            indices: vec![0, 1, 2],
            surfaces: vec![asset_t6::WorldSurface {
                mins: [0.0; 3],
                maxs: [0.0; 3],
                bounds: [[0.0; 3]; 2],
                vertex_data_offset0: 0,
                vertex_data_offset1: -1,
                first_vertex: 0,
                vertex_count: 3,
                tri_count: 1,
                base_index: 0,
                material: None,
                lightmap_index: 2,
                reflection_probe_index: 0,
                primary_light_index: 1,
                flags: 0,
            }],
            brush_models: vec![(0, 0, [[0.0; 3]; 2]), (0, 1, [[0.0; 3]; 2])],
            ..Default::default()
        };
        let glasses = vec![asset_t6::GlassRef {
            pristine_material: "clear".into(),
            cracked_material: "cracked".into(),
            ..Default::default()
        }];
        // Yaw 90: model x turns to world +y, model y to world -x.
        let place = crate::T6GlassPlacement {
            brush_model: 1,
            origin: [100.0, 200.0, 300.0],
            angles: [0.0, 90.0, 0.0],
            glass: 0,
        };
        let panes = build_t6_glass_draws(&world, &glasses, &[place]);
        let pane = &panes[0];
        let near = |a: [f32; 3], b: [f32; 3]| (0..3).all(|j| (a[j] - b[j]).abs() < 1e-3);
        assert!(near(pane.vertices[0].position, [100.0, 210.0, 300.0]), "{:?}", pane.vertices[0]);
        assert!(near(pane.vertices[1].position, [90.0, 200.0, 300.0]), "{:?}", pane.vertices[1]);
        assert!(near(pane.vertices[2].normal, [0.0, 0.0, 1.0]));
        assert_eq!(pane.indices, vec![0, 1, 2]);
        assert_eq!(
            pane.surfaces,
            vec![T6GlassSurface { surface: 0, start: 0, count: 3, lightmap: 2, primary_light: 1, reflection_probe: 0 }]
        );
        assert_eq!((pane.pristine.as_str(), pane.cracked.as_str()), ("clear", "cracked"));
        // The world model (0) and a missing one draw nothing.
        let none = build_t6_glass_draws(&world, &glasses, &[crate::T6GlassPlacement { brush_model: 0, ..place }]);
        assert!(none[0].surfaces.is_empty());
    }

    #[test]
    fn a_placed_row_stays_a_bo2_row() {
        let mut v = [0u8; T6_WORLD_VERTEX];
        v[16..20].copy_from_slice(&[1, 2, 3, 4]);
        v[20..24].copy_from_slice(&0x3800_3c00u32.to_le_bytes());
        v[24..28].copy_from_slice(&(0x1ffu32 << 20).to_le_bytes());
        v[28..32].copy_from_slice(&0x1ffu32.to_le_bytes());
        v[32..36].copy_from_slice(&0xffff_0000u32.to_le_bytes());
        let mut d = decode_vertex(&v);
        assert_eq!(d.packed, v);
        // A quarter turn about z: x goes to y.
        place_vertex(&mut d, [1.0, 2.0, 3.0], [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]]);
        let row = asset_t6::T6WorldVertex::read(&d.packed).unwrap();
        assert_eq!(row.base.position, [1.0, 2.0, 3.0]);
        assert_eq!(row.base.color, [1, 2, 3, 4]);
        assert_eq!(row.base.uv(), [1.0, 0.5]);
        assert_eq!(row.base.unpacked_normal(), [0.0, 0.0, 1.0]);
        assert_eq!(row.base.unpacked_tangent(), [0.0, 1.0, 0.0]);
        assert_eq!(row.lightmap_uv(), [0.0, 1.0]);
    }

    #[test]
    fn t6_cells_keep_their_trees_and_portals() {
        let surf = asset_t6::WorldSurface {
            mins: [0.0; 3],
            maxs: [0.0; 3],
            bounds: [[0.0; 3]; 2],
            vertex_data_offset0: 0,
            vertex_data_offset1: -1,
            first_vertex: 0,
            vertex_count: 3,
            tri_count: 1,
            base_index: 0,
            material: None,
            lightmap_index: 0,
            reflection_probe_index: 0,
            primary_light_index: 0,
            flags: 0,
        };
        let tree = |start: u16, count: u16, child_count: u16, children_offset: i32| asset_t6::AabbTreeRef {
            mins: [0.0; 3],
            maxs: [1.0; 3],
            child_count,
            surface_count: count,
            start_surf_index: start,
            smodel_indexes: Vec::new(),
            children_offset,
        };
        // Surface 1 is a submodel's: the static list leaves it out.
        let world = WorldRef {
            surfaces: vec![surf.clone(), surf.clone(), surf],
            sorted_surf_index: vec![2, 0],
            cells: vec![
                vec![tree(0, 2, 2, 40), tree(0, 1, 0, 0), tree(1, 1, 0, 0)],
                vec![tree(1, 1, 0, 0)],
            ],
            dpvs_planes: vec![[1.0, 0.0, 0.0, 0.0]],
            dpvs_nodes: vec![0],
            cell_bounds: vec![
                asset_t6::CellRef {
                    portals: vec![asset_t6::PortalRef {
                        plane: [1.0, 0.0, 0.0, 0.0],
                        cell: 1,
                        vertices: vec![[0.0; 3]; 4],
                        hull_axis: [[0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
                    }],
                    ..Default::default()
                },
                asset_t6::CellRef::default(),
            ],
            mins: [0.0; 3],
            maxs: [1.0; 3],
            ..Default::default()
        };
        let d = cells_dpvs(&world, vec![Bounds::from_mins_maxs([0.0; 3], [1.0; 3]); 3]).unwrap();
        assert_eq!(d.cell_count, 2);
        assert_eq!(d.sorted_surf_index, vec![2, 0, 1]);
        // childrenOffset stays in bytes: one BO2 node on is 40.
        assert_eq!(d.aabb_trees[0][0].children_offset, dpvs_t6::AABB_TREE_SIZE as i32);
        assert_eq!(d.aabb_trees.len(), 3);
        let extra = &d.aabb_trees[2][0];
        assert_eq!((extra.start_surf_index, extra.surface_count), (2, 1));
        assert_eq!(d.planes[0].r#type, 0);
        let p = &d.portals_per_cell[0][0];
        assert_eq!((p.neighbor, p.vert_start, p.vert_count), (1, 0, 4));
        assert!(p.hull_axis.is_some() && d.portals_per_cell[1].is_empty());
    }

    #[test]
    fn t6_empty_aabb_node_is_a_cleared_box() {
        // mp_la and mp_raid: cell 36 node 0 holds inverted extremes.
        let empty = asset_t6::AabbTreeRef {
            mins: [f32::MAX; 3],
            maxs: [-f32::MAX; 3],
            child_count: 0,
            surface_count: 0,
            start_surf_index: 0,
            smodel_indexes: Vec::new(),
            children_offset: 0,
        };
        let world = WorldRef {
            cells: vec![vec![empty]],
            cell_bounds: vec![asset_t6::CellRef::default()],
            mins: [0.0; 3],
            maxs: [1.0; 3],
            ..Default::default()
        };
        let d = cells_dpvs(&world, Vec::new()).unwrap();
        assert!(d.aabb_trees[0][0].bounds().is_cleared());
        assert_eq!(d.cleared_boxes, 1);
    }
}
