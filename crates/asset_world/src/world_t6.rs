//! bo2zm: Black Ops II GfxWorld -> LevelDrawCall. Not part of the upstream engine.
//!
//! The T6 world vertex is 36 bytes (measured on Nuketown, t6world probe):
//! position f32x3 @0, binormal sign f32 @12, colour @16, uv_semantic half2 @20
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

use std::collections::BTreeSet;
use std::collections::HashMap;
use std::collections::hash_map::Entry;

use asset_t6::{WorldRef, pack_unit_vec, decode_unit_normal};
use bevy::asset::RenderAssetUsages;
use bevy::prelude::Mesh;
use bevy::render::mesh::{Indices, PrimitiveTopology};
use dpvs_t6::{AabbTree, Bounds, PlaneEquation};

use crate::world_lightmap::{LevelLightbake, LevelLightmapHole};
use crate::world_lights::LevelSunlamp;
use crate::world_scene::{FaceDrawAttributes, LevelDrawGroup, LevelDrawCall, WorldVertexPayload};
use crate::world_visibility::{
    CameraSpanKind, ViewSurfaceSpan, ViewSurfaceSpans, VisibilityLevelData, SelfContainedVisibilityPortal,
};
use crate::mesh_report::{LevelMeshFault, LevelMeshMetrics};
use crate::{FaceBlocksSunlight, snapshot_from_sun_blockers};

pub use asset_t6::T6_WORLD_VERTEX;

/// A unit vector, or straight up when the input is (nearly) zero or not a number.
fn normalized(v: [f32; 3]) -> [f32; 3] {
    let len = v.iter().map(|c| c * c).sum::<f32>().sqrt();
    (len > 1e-6).then(|| v.map(|c| c / len)).unwrap_or([0.0, 0.0, 1.0])
}

fn rd_u32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().expect("four bytes"))
}

fn rd_f32(b: &[u8], at: usize) -> f32 {
    f32::from_bits(rd_u32(b, at))
}

fn half2(packed: u32) -> [f32; 2] {
    [packed as u16, (packed >> 16) as u16].map(asset_t6::short_float_to_full)
}

struct Decoded {
    bytes: [u8; T6_WORLD_VERTEX],
    pos: [f32; 3],
    normal: [f32; 3],
    tang: [f32; 4],
    color: [f32; 4],
    tex_uv: [f32; 2],
    lightmap_uv: [f32; 2],
}

fn decode_vertex(v: &[u8]) -> Decoded {
    let word = |at: usize| rd_u32(v, at);
    let tan_dir = normalized(decode_unit_normal(word(28)));
    let lightmap = word(32);
    Decoded {
        bytes: v[..T6_WORLD_VERTEX].try_into().expect("a whole world row"),
        pos: [0, 4, 8].map(|at| rd_f32(v, at)),
        normal: normalized(decode_unit_normal(word(24))),
        tang: [tan_dir[0], tan_dir[1], tan_dir[2], rd_f32(v, 12)],
        // T6 colour bytes are R, G, B, A (the lit shaders read them as RGBA unorm).
        color: [v[16], v[17], v[18], v[19]].map(|c| f32::from(c) / 255.0),
        tex_uv: half2(word(20)),
        lightmap_uv: [lightmap as u16, (lightmap >> 16) as u16].map(|h| f32::from(h) / 65535.0),
    }
}

/// A model-space direction turned into the world by `AnglesToAxis` rows.
fn turned(axis: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|j| (0..3).map(|k| v[k] * axis[k][j]).sum())
}

/// A model-space point moved to where its entity stands.
fn placed_at(base: &[f32; 3], axis: &[[f32; 3]; 3], spot: [f32; 3]) -> [f32; 3] {
    let moved = turned(axis, spot);
    std::array::from_fn(|j| base[j] + moved[j])
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
    pub tri_list: Vec<u32>,
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
    pub mat_slot: String,
    pub depth_units: f32,
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
    pub spot: [f32; 3],
    pub crack_sound: String,
    pub shatter_sound: String,
    pub auto_shatter_sound: String,
    pub crack_effect: String,
    pub shatter_effect: String,
    pub outline: Vec<[f32; 2]>,
    pub axes_m: [[f32; 3]; 3],
    pub outline_origin: [f32; 3],
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct T6GlassVertex {
    pub position: [f32; 3],
    pub hit_facing: [f32; 3],
    /// The vertex colour, R, G, B, A in 0..1.
    pub rgba: [f32; 4],
    pub glass_uv: [f32; 2],
    pub lightmap_uv: [f32; 2],
}

/// One of a window's surfaces: its run of the pane's indices, its lightmap,
/// its primary light, its reflection probe and the world surface it is
/// (for its material).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct T6GlassSurface {
    pub surface: u32,
    pub first: u32,
    pub count: u32,
    pub lightmap: u8,
    pub dominant_light: u8,
    pub reflection_probe: u8,
}

/// The surfaces `model` stands for, as (first surface, how many); none for
/// the world model (0) or a number the map does not have.
fn brush_span(world: &WorldRef, model: u32) -> Option<(u32, u32)> {
    let &(first, count, _) = world.solid_models.get(model as usize)?;
    (model != 0).then_some((first, count))
}

/// A surface's triangle indices and the vertex rows they reach, when the
/// world holds both.
fn surface_rows<'w>(world: &'w WorldRef, surf: &asset_t6::WorldSurface) -> Option<(&'w [u16], &'w [u8])> {
    let base = surf.base_index.max(0) as usize;
    let triangle_list = world.indices.get(base..base + usize::from(surf.tri_tally) * 3)?;
    let rows = usize::from(*triangle_list.iter().max()?) + 1;
    let at = surf.vertex_data_offset0.max(0) as usize;
    Some((triangle_list, world.vd0.get(at..at + rows * T6_WORLD_VERTEX)?))
}

/// A pane carrying only what its window's def says (no geometry yet).
fn pane_from_def(g: &asset_t6::GlassRef) -> T6GlassPaneDraw {
    T6GlassPaneDraw {
        pristine: g.pristine_material.clone(),
        cracked: g.cracked_material.clone(),
        client: T6GlassClientDef {
            spot: g.origin,
            crack_sound: g.crack_sound.clone(),
            shatter_sound: g.shatter_sound.clone(),
            auto_shatter_sound: g.auto_shatter_sound.clone(),
            crack_effect: g.crack_effect.clone(),
            shatter_effect: g.shatter_effect.clone(),
            outline: g.outline.clone(),
            axes_m: g.axis,
            outline_origin: g.outline_origin,
        },
        shards: T6GlassShardDef {
            mat_slot: g.shard_material.clone(),
            depth_units: g.pane_thickness,
            uv_scale: g.uv_scale,
            is_planar: g.is_planar,
            min_shard_size: g.min_shard_size,
            max_shard_size: g.max_shard_size,
            shard_life_probability: g.shard_life_probability,
            max_shards: g.max_shards,
            pool_shards: g.pool_shards,
            pool_physics: g.pool_physics,
        },
        absmin: g.absmin,
        absmax: g.absmax,
        ..Default::default()
    }
}

/// Append one world surface to a pane, its rows moved to the window's spot.
fn add_glass_surface(
    pane: &mut T6GlassPaneDraw,
    (origin, axis): (&[f32; 3], &[[f32; 3]; 3]),
    number: u32,
    surf: &asset_t6::WorldSurface,
    (triangle_list, block): (&[u16], &[u8]),
) {
    let first_vertex = pane.vertices.len() as u32;
    pane.vertices.extend(block.chunks_exact(T6_WORLD_VERTEX).map(|row| {
        let mut d = decode_vertex(row);
        place_vertex(&mut d, *origin, *axis);
        T6GlassVertex { position: d.pos, hit_facing: d.normal, rgba: d.color, glass_uv: d.tex_uv, lightmap_uv: d.lightmap_uv }
    }));
    let start = pane.tri_list.len() as u32;
    pane.tri_list.extend(triangle_list.iter().map(|&i| first_vertex + u32::from(i)));
    pane.surfaces.push(T6GlassSurface {
        surface: number,
        first: start,
        count: triangle_list.len() as u32,
        lightmap: surf.lightbake_index as u8,
        dominant_light: surf.key_light_index as u8,
        reflection_probe: surf.env_probe_index as u8,
    });
}

/// Every window's surfaces, one entry per placement (pane `i` = piece `i`;
/// a window whose model has no surfaces gets an empty one). A surface's
/// vertex `i` is `vd0[vertexDataOffset0 + i * 36]`, as the world's are.
pub fn build_t6_glass_draws(
    world: &WorldRef,
    glasses: &[asset_t6::GlassRef],
    placements: &[crate::T6GlassPlacement],
) -> Vec<T6GlassPaneDraw> {
    let one_pane = |place: &crate::T6GlassPlacement| {
        let mut pane = glasses.get(place.glass).map(pane_from_def).unwrap_or_default();
        let Some((first, count)) = brush_span(world, place.solid_model) else {
            return pane;
        };
        // `AnglesToAxis`: model space -> world is origin + x*axis[0] + y*axis[1] + z*axis[2].
        let axis = game_math::euler_to_rows(place.angles);
        let found = (first..first.saturating_add(count)).filter_map(|surf_no| {
            let surf = world.surfaces.get(surf_no as usize)?;
            Some((surf_no, surf, surface_rows(world, surf)?))
        });
        for (surf_no, surf, rows) in found {
            add_glass_surface(&mut pane, (&place.origin, &axis), surf_no, surf, rows);
        }
        pane
    };
    placements.iter().map(one_pane).collect()
}

/// How far a pane's vertices stray outside its window's box (0 when inside).
fn stray_distance(pane: &T6GlassPaneDraw, g: &asset_t6::GlassRef) -> f32 {
    pane.vertices
        .iter()
        .flat_map(|v| v.position.into_iter().zip(g.absmin.into_iter().zip(g.absmax)))
        .map(|(p, (low, high))| (low - p).max(p - high))
        .fold(0.0f32, f32::max)
}

/// One report line on the windows' surfaces: how many were found, and how
/// many stand inside their window's own bounds (`Glass::absmin/absmax`).
pub fn t6_glass_draw_report(
    panes: &[T6GlassPaneDraw],
    glasses: &[asset_t6::GlassRef],
    placements: &[crate::T6GlassPlacement],
) -> String {
    let strays: Vec<f32> = panes
        .iter()
        .zip(placements)
        .filter(|(pane, _)| !pane.vertices.is_empty())
        .filter_map(|(pane, place)| Some(stray_distance(pane, glasses.get(place.glass)?)))
        .collect();
    let inside = strays.iter().filter(|&&s| s <= 1.0).count();
    let worst = strays.iter().copied().fold(0.0f32, f32::max);
    let materials: Vec<(&str, &str)> = panes
        .iter()
        .map(|p| (p.pristine.as_str(), p.cracked.as_str()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let total = |count: fn(&T6GlassPaneDraw) -> usize| panes.iter().map(count).sum::<usize>();
    let with_surfaces = total(|p| usize::from(!p.surfaces.is_empty()));
    let surfaces = total(|p| p.surfaces.len());
    let triangles = total(|p| p.tri_list.len() / 3);
    format!(
        "t6 glass draw: {} panes, {with_surfaces} with surfaces, {surfaces} surfaces, {triangles} triangles, {inside} inside their window's bounds (worst {worst:.1} out), materials (pristine, cracked) {materials:?}",
        panes.len(),
    )
}

/// The lighting a T6 world draws with, besides its lightmaps (measured from
/// the lit world shaders, `pimp_shader_lmap_*`): the sun the shaders add,
/// times the lightmap's baked sun visibility, and the exposure multiplier
/// every lit result is scaled by.
#[derive(Clone, Copy, Debug)]
pub struct T6WorldLighting {
    pub sun: LevelSunlamp,
    pub exposure_scale: f32,
    /// The sky material's `skyBoxRotation` x, y, z, then its brightness
    /// (`skyColorParm` y), negative when the rotation's w flips z.
    pub sky: [f32; 4],
    /// The saturation of the map's vision (`vc_FSM` w of
    /// `vision/<map>.vision`; 1 when it has none), `None` without one.
    pub saturation: Option<f32>,
    /// The sky material's `skyColorParm` x (above 0: the cube is `rgb / a`,
    /// else `rgb^2`) and w (how much of the fog the sky takes; Buried 0.16).
    pub sky_parms: [f32; 2],
    /// The map's vision colour grade, as the game's colour table shader
    /// reads it (tone band slopes and offsets, the nine tone matrix rows,
    /// gamma, saturation, blend), `None` without a vision.
    pub grade: Option<[[f32; 4]; 14]>,
}

/// One T6 lightmap as the engine's world lightmap. A T6 lightmap is one
/// RGBA8 image of three stacked pages, addressed at `(u, v / 3 + page / 3)`:
/// page 0 base light and page 1 directional light (both `rgb / a`), page 2
/// the light direction (`rgb * 2 - 1`) with the baked sun visibility in
/// alpha. It rides in the secondary slot, read linear; the other slots get
/// 1x1 stand-ins (T6 has no primary page).
pub fn t6_lightmap(image: &asset_t6::ImageRef) -> Option<LevelLightbake> {
    use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
    use bevy::prelude::{Image, UVec2};
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

    // DXGI_FORMAT_R8G8B8A8_UNORM only.
    let embedded = image.embedded.as_ref().filter(|e| e.dxgi_format == 28)?;
    let size = UVec2::new(u32::from(image.width), u32::from(image.height));
    let texels = embedded.data.get(..(size.x * size.y * 4) as usize)?;
    let usage = RenderAssetUsages::RENDER_WORLD;
    let texture = |UVec2 { x: width, y: height }: UVec2, data: Vec<u8>| {
        let one_layer = Extent3d { width, height, depth_or_array_layers: 1 };
        Image::new(one_layer, TextureDimension::D2, data, TextureFormat::Rgba8Unorm, usage)
    };
    let mut page = texture(size, texels.to_vec());
    let mut smooth = ImageSamplerDescriptor {
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        ..ImageSamplerDescriptor::default()
    };
    smooth.set_address_mode(ImageAddressMode::ClampToEdge);
    page.sampler = ImageSampler::Descriptor(smooth);
    let white = || texture(UVec2::ONE, vec![0xff; 4]);
    Some(LevelLightbake {
        first_slot_texture: None,
        auxiliary_texture: Some(page),
        second_b_map: None,
        background_light_texture: white(),
        heading_texture: white(),
        daylight_mask_image: white(),
        background_light_origin_label: image.name.clone(),
        daylight_mask_source_name: String::new(),
        background_light_extent: size,
        daylight_mask_size: UVec2::ONE,
    })
}

/// codmp: a `script_brushmodel` entity (`"model" "*104"`): Black Ops II
/// draws its submodel's surfaces for the entity, where it stands
/// (`R_AddBrushModelToSceneFromAngles`), until a script hides or deletes
/// it. Nuketown's blue house floor under the carpet is one.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct T6BrushPlacement {
    pub solid_model: u32,
    pub origin: [f32; 3],
    pub angles: [f32; 3],
}

/// codmp: where a placed submodel's surfaces sit among the world's (their
/// vertices already moved to where the entity stands); the cull drops them
/// while no live entity shows that submodel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct T6BrushSurfaces {
    pub solid_model: u32,
    pub lead_skin: u32,
    pub facet_tally: u32,
}

/// A decoded vertex moved from a submodel's space into the world:
/// origin + x*axis[0] + y*axis[1] + z*axis[2] (`AnglesToAxis`). The row's
/// normal and tangent are packed again the way BO2 packs a turned unit vector.
fn place_vertex(d: &mut Decoded, base: [f32; 3], basis: [[f32; 3]; 3]) {
    let [tx, ty, tz, sign] = d.tang;
    let tan_dir = turned(&basis, [tx, ty, tz]);
    d.pos = placed_at(&base, &basis, d.pos);
    d.normal = turned(&basis, d.normal);
    d.tang = [tan_dir[0], tan_dir[1], tan_dir[2], sign];
    let rewrites = [
        (0, d.pos[0].to_bits()),
        (4, d.pos[1].to_bits()),
        (8, d.pos[2].to_bits()),
        (24, pack_unit_vec(d.normal)),
        (28, pack_unit_vec(tan_dir)),
    ];
    for (at, word) in rewrites {
        d.bytes[at..at + 4].copy_from_slice(&word.to_le_bytes());
    }
}

fn need_geometry<T>(found: Option<T>) -> Result<T, LevelMeshFault> {
    found.ok_or(LevelMeshFault::EmptyWorld)
}

/// The corners of the box around some points, or none for no points.
fn extent(points: impl IntoIterator<Item = [f32; 3]>) -> Option<([f32; 3], [f32; 3])> {
    let mut points = points.into_iter().peekable();
    points.peek()?;
    let open = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    Some(points.fold(open, |(low, high), p| {
        (
            std::array::from_fn(|k| low[k].min(p[k])),
            std::array::from_fn(|k| high[k].max(p[k])),
        )
    }))
}

/// Which surfaces `sorted_facet_index` names.
fn listed_mask(world: &WorldRef) -> Vec<bool> {
    let mut mask = vec![false; world.surfaces.len()];
    for &entry in &world.sorted_facet_index {
        if let Some(flag) = mask.get_mut(usize::from(entry)) {
            *flag = true;
        }
    }
    mask
}

/// Give each submodel a `script_brushmodel` stands for to the first entity
/// naming it: which placement owns each surface, and the runs claimed.
fn claim_brush_surfaces(
    world: &WorldRef,
    placements: &[T6BrushPlacement],
) -> (Vec<Option<usize>>, Vec<T6BrushSurfaces>) {
    let mut owner: Vec<Option<usize>> = vec![None; world.surfaces.len()];
    let mut claimed = Vec::new();
    let spans = placements
        .iter()
        .enumerate()
        .filter_map(|(pi, p)| Some((pi, p.solid_model, brush_span(world, p.solid_model)?)));
    for (pi, model, (first, count)) in spans {
        let run = first as usize..(first as usize + count as usize).min(owner.len());
        if run.is_empty() || owner[run.clone()].iter().any(Option::is_some) {
            continue;
        }
        owner[run.clone()].fill(Some(pi));
        claimed.push(T6BrushSurfaces {
            solid_model: model,
            lead_skin: run.start as u32,
            facet_tally: run.len() as u32,
        });
    }
    (owner, claimed)
}

/// Each vertex group is expanded once per second-stream layout its surfaces
/// read it with (layered surfaces add their layers' texcoords from `vd1`); a
/// placed submodel's surfaces get their own group (their vertices move).
fn vertex_group(
    surf: &asset_t6::WorldSurface,
    layers: Option<&asset_core::T6Layers>,
    owner: Option<usize>,
) -> (i32, i32, asset_core::T6Layers, usize) {
    let (second, layout) = match layers {
        Some(l) if l.any() => (surf.vertex_data_offset1, *l),
        _ => (-1, asset_core::T6Layers::default()),
    };
    (surf.vertex_data_offset0, second, layout, owner.unwrap_or(usize::MAX))
}

/// For each group, the rows it needs: one past the highest index any of its
/// surfaces uses.
fn group_extents(world: &WorldRef, keys: &[(i32, i32, asset_core::T6Layers, usize)]) -> Result<HashMap<(i32, i32, asset_core::T6Layers, usize), usize>, LevelMeshFault> {
    let mut rows_needed: HashMap<(i32, i32, asset_core::T6Layers, usize), usize> = HashMap::new();
    for (surf, &key) in world.surfaces.iter().zip(keys) {
        let first = surf.base_index as usize;
        let triangle_list = need_geometry(world.indices.get(first..first + surf.tri_tally as usize * 3))?;
        let rows = triangle_list.iter().map(|&ix| usize::from(ix) + 1).max().unwrap_or(0);
        rows_needed.entry(key).and_modify(|have| *have = (*have).max(rows)).or_insert(rows);
    }
    Ok(rows_needed)
}

/// The per-vertex lanes a world or one of its batches keeps.
#[derive(Default)]
struct Columns {
    pos: Vec<[f32; 3]>,
    nrm: Vec<[f32; 3]>,
    tan: Vec<[f32; 4]>,
    rgba: Vec<[f32; 4]>,
    first_texcoord: Vec<[f32; 2]>,
    uv1: Vec<[f32; 2]>,
}

impl Columns {
    fn len(&self) -> usize {
        self.pos.len()
    }

    fn push_decoded(&mut self, d: &Decoded) {
        self.pos.push(d.pos);
        self.nrm.push(d.normal);
        self.tan.push(d.tang);
        self.rgba.push(d.color);
        self.first_texcoord.push(d.tex_uv);
        self.uv1.push(d.lightmap_uv);
    }

    fn push_copy(&mut self, from: &VertexStreams, src: usize) {
        self.pos.push(from.pos[src]);
        self.nrm.push(from.nrm[src]);
        self.tan.push(from.tan[src]);
        self.rgba.push(from.rgba[src]);
        self.first_texcoord.push(from.first_texcoord[src]);
        self.uv1.push(from.uv1[src]);
    }
}

/// What a built world keeps per vertex: the lanes, the rows as BO2 wrote
/// them, and the second-stream texcoords.
#[derive(Default)]
struct WorldVertices {
    lanes: Columns,
    rows: Vec<[u8; T6_WORLD_VERTEX]>,
    layer_uvs: Vec<[f32; 4]>,
}

impl WorldVertices {
    /// Decode one group's block into new rows, moving them with the entity
    /// that owns the group.
    fn add_group(
        &mut self,
        world: &WorldRef,
        (offset0, stream1, layers, _): (i32, i32, asset_core::T6Layers, usize),
        row_total: usize,
        stands: Option<(&T6BrushPlacement, &[[f32; 3]; 3])>,
    ) -> Result<(), LevelMeshFault> {
        let from = offset0 as usize;
        let block = need_geometry(world.vd0.get(from..from + row_total * T6_WORLD_VERTEX))?;
        for (row, raw) in block.chunks_exact(T6_WORLD_VERTEX).enumerate() {
            let mut vert = decode_vertex(raw);
            if let Some((placement, axis)) = stands {
                place_vertex(&mut vert, placement.origin, *axis);
            }
            self.layer_uvs.push(second_stream_uvs(&world.vd1, layers, stream1, row));
            self.lanes.push_decoded(&vert);
            self.rows.push(vert.bytes);
        }
        Ok(())
    }
}

/// One surface as the draw tables want it.
struct SurfaceRecord {
    fields: FaceDrawAttributes,
    span: (u32, u32),
    bounds: Bounds,
}

/// Build the engine's world from a captured T6 GfxWorld. `material_lookup`
/// holds each world surface's material as an index into the lane's catalog
/// (one entry per `world.surfaces`, in order; missing entries are `None`).
/// BO2 materials have no technique the engine can run, so their surfaces
/// draw through the renderer's fallback pass, which samples the colour map.
pub fn build_t6_world_draw(
    world: &WorldRef,
    material_lookup: &[Option<usize>],
    surface_layers: &[asset_core::T6Layers],
    lightmaps: Vec<Option<LevelLightbake>>,
    lighting: Option<T6WorldLighting>,
    brush_placements: &[T6BrushPlacement],
) -> Result<LevelDrawCall, LevelMeshFault> {
    if world.surfaces.is_empty() || world.vd0.is_empty() {
        return Err(LevelMeshFault::EmptyWorld);
    }
    let surf_total = world.surfaces.len();

    // codmp: the submodels a `script_brushmodel` stands for are drawn where
    // the entity stands.
    let (owner, t6_brush_surfaces) = claim_brush_surfaces(world, brush_placements);
    let place_axis: Vec<[[f32; 3]; 3]> = brush_placements
        .iter()
        .map(|p| game_math::euler_to_rows(p.angles))
        .collect();

    let keys: Vec<(i32, i32, asset_core::T6Layers, usize)> = world
        .surfaces
        .iter()
        .enumerate()
        .map(|(i, surf)| vertex_group(surf, surface_layers.get(i), owner[i]))
        .collect();
    let rows_needed = group_extents(world, &keys)?;

    // Groups are expanded in the order the surfaces first reach them.
    let mut vertex_points = WorldVertices::default();
    let mut group_base = HashMap::new();
    for &key in &keys {
        if let Entry::Vacant(slot) = group_base.entry(key) {
            slot.insert(vertex_points.lanes.len() as u32);
            let stands = brush_placements.get(key.3).map(|p| (p, &place_axis[key.3]));
            vertex_points.add_group(world, key, rows_needed[&key], stands)?;
        }
    }

    // The surfaces the game draws: the static visibility list's (all when
    // there is none), and every placed submodel's. Others (Nuketown's orange
    // light dome) only feed the light compiler.
    let mut drawn = listed_mask(world);
    if world.sorted_facet_index.is_empty() {
        drawn.fill(true);
    }
    drawn.iter_mut().zip(&owner).for_each(|(flag, who)| *flag |= who.is_some());

    let mut index_pool: Vec<u32> = Vec::new();
    let records: Vec<SurfaceRecord> = world
        .surfaces
        .iter()
        .zip(&keys)
        .enumerate()
        .map(|(i, (surf, key))| {
            let base = group_base[key];
            let first = surf.base_index as usize;
            let used = if drawn[i] { usize::from(surf.tri_tally) * 3 } else { 0 };
            let triangle_list = &world.indices[first..first + used];
            let start = index_pool.len() as u32;
            index_pool.extend(triangle_list.iter().map(|&ix| base + u32::from(ix)));
            // A placed submodel's surface: its bounds where it now stands.
            let listed_bounds = (surf.bounds[0], surf.bounds[1]);
            let (low, high) = if owner[i].is_some() {
                let moved = triangle_list
                    .iter()
                    .filter_map(|&ix| vertex_points.lanes.pos.get((base + u32::from(ix)) as usize).copied());
                extent(moved).unwrap_or(listed_bounds)
            } else {
                listed_bounds
            };
            SurfaceRecord {
                fields: FaceDrawAttributes {
                    vertex_from: base,
                    tri_tally: (used / 3) as u16,
                    index_from: start,
                    lightbake_index: surf.lightbake_index as u8,
                    env_probe_index: surf.env_probe_index as u8,
                    key_light_index: surf.key_light_index as u8,
                },
                span: (start, used as u32),
                bounds: Bounds::from_corner_extremes(low, high),
            }
        })
        .collect();

    let index_spans: Vec<(u32, u32)> = records.iter().map(|r| r.span).collect();
    let first_vert_of: Vec<u32> = records.iter().map(|r| r.fields.vertex_from).collect();
    let small_column = |pick: fn(&FaceDrawAttributes) -> u8| -> Vec<u8> {
        records.iter().map(|r| pick(&r.fields)).collect()
    };
    let page_of = small_column(|f| f.lightbake_index);
    let probe_of = small_column(|f| f.env_probe_index);
    let light_of = small_column(|f| f.key_light_index);
    let (draw_fields, bounds_of): (Vec<_>, Vec<_>) = records.into_iter().map(|r| (r.fields, r.bounds)).unzip();

    // One entry per world surface, `None` past the end of the lookup.
    let mat_of_surface: Vec<Option<usize>> = material_lookup
        .iter()
        .copied()
        .chain(std::iter::repeat(None))
        .take(surf_total)
        .collect();
    // A surface is lightmapped when its lightmap index names a page this
    // world built (31 = none).
    let lit_by_page: Vec<bool> = page_of
        .iter()
        .map(|&page| matches!(lightmaps.get(usize::from(page)), Some(Some(_))))
        .collect();
    let lanes = &vertex_points.lanes;
    let Batched { batches, spans: batch_spans } = batch_by_look(
        &lanes.pos,
        &lanes.nrm,
        &lanes.tan,
        &lanes.rgba,
        &lanes.first_texcoord,
        &lanes.uv1,
        &index_pool,
        &index_spans,
        &mat_of_surface,
        &lit_by_page,
        &page_of,
        &light_of,
        &probe_of,
    );

    let single_cell = world.cell_bounds.is_empty() || world.dpvs_nodes.is_empty();
    let make_dpvs = if single_cell { one_cell_dpvs } else { cells_dpvs };
    let dpvs = make_dpvs(world, bounds_of)?;

    let corners = extent(lanes.pos.iter().copied())
        .unwrap_or(([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]));
    let stats = mesh_stats(lanes.len(), index_pool.len(), surf_total, corners);

    let lightmap = lightmaps
        .iter()
        .any(Option::is_some)
        .then_some(lightmaps)
        .ok_or(LevelLightmapHole::Missing);
    let WorldVertices { lanes, rows, layer_uvs } = vertex_points;
    let draw = LevelDrawCall {
        batches,
        sky_dome_model: None,
        lightmap,
        tallies: stats,
        compact_vertices: WorldVertexPayload::T6(rows),
        vertex_tier: Default::default(),
        facet_vertex_layer: vec![0; surf_total],
        facet_first_vertex: first_vert_of,
        facet_draw_fields: draw_fields,
        spots: lanes.pos,
        surface_directions: lanes.nrm,
        tangent_rows: lanes.tan,
        colors: lanes.rgba,
        texture_coords: lanes.first_texcoord,
        lightbake_uvs: lanes.uv1,
        compact_indices: index_pool,
        facet_index_ranges: index_spans,
        facet_batch_ranges: batch_spans,
        face_has_baked_light: lit_by_page,
        facet_lightbake_indices: page_of,
        facet_env_probes: probe_of,
        facet_key_lights: light_of,
        warp_sort_key: Default::default(),
        capture: snapshot_from_sun_blockers(FaceBlocksSunlight::with_len(surf_total)),
        solid_models: Default::default(),
        solid_model_bounds: Default::default(),
        facet_materials: mat_of_surface,
        key_lights: Vec::new(),
        light_defs: Vec::new(),
        daylight_key_light_tally: Default::default(),
        light_zone_hulls: Default::default(),
        shadow_geometry: Default::default(),
        reflection_probes: Default::default(),
        dpvs,
        exterior_texture_label: Default::default(),
        openair_image: None,
        openair_lookup: Default::default(),
        sun_effects: None,
        // bo2zm: the T5 sun and exposure carriers hold the T6 world's; for a
        // T6 world only the renderer's fallback draw reads them.
        classic_daylight_parse_exposure: lighting.map(|l| l.exposure_scale),
        classic_sky_dynamic_intensity: lighting.map(|l| l.sky),
        t5_sun_light: lighting.map(|l| l.sun),
        t5_foliage_scatter_strength: Default::default(),
        t5_foliage_scatter_share: Default::default(),
        classic_exposure_volume_tally: Default::default(),
        t6_vision_saturation: lighting.and_then(|l| l.saturation),
        t6_sky_parms: lighting.map(|l| l.sky_parms),
        t6_vision_grade: lighting.and_then(|l| l.grade),
        t6_script_grades: Vec::new(),
        t6_lights: Vec::new(),
        t6_fog_banks: Vec::new(),
        t6_fog_modifiers: Vec::new(),
        t6_exposure_volumes: Vec::new(),
        t6_lut: None,
        t6_lut_volumes: Vec::new(),
        t6_light_tweaks: None,
        t6_flicker: Vec::new(),
        t6_probes: Vec::new(),
        t6_layer_uvs: layer_uvs,
        t6_brush_surfaces,
    };
    Ok(draw)
}

/// The size report of a world with `vertex_total` vertices, `index_total`
/// indices and `surface_total` surfaces inside the box `(min, max)`.
fn mesh_stats(vertex_total: usize, index_total: usize, surface_total: usize, (min, max): ([f32; 3], [f32; 3])) -> LevelMeshMetrics {
    LevelMeshMetrics { vertices: vertex_total, triangles: index_total / 3, surfaces: surface_total, min, max }
}

/// The texcoords layers 1 and 2 read from the second vertex stream for the
/// row at `row` (zeros when the group has no second stream).
fn second_stream_uvs(vd1: &[u8], layers: asset_core::T6Layers, stream_start: i32, row: usize) -> [f32; 4] {
    let mut texcoord_set = [0.0f32; 4];
    if stream_start >= 0 {
        let row_first_cell = stream_start as usize + row * usize::from(layers.stride);
        let used_slots = texcoord_set
            .chunks_exact_mut(2)
            .zip(layers.kinds.iter().zip(&layers.uv_offsets))
            .filter(|slot| *slot.1.0 != 0);
        for (pair, (_, &offset)) in used_slots {
            let at = row_first_cell + usize::from(offset);
            if let Some(word) = vd1.get(at..at + 4) {
                pair.copy_from_slice(&half2(rd_u32(word, 0)));
            }
        }
    }
    texcoord_set
}

/// A dpvs shell whose one visible range is every surface, lit and opaque.
fn lit_opaque_dpvs(surf_total: usize) -> VisibilityLevelData {
    let end = surf_total as u32;
    let all = ViewSurfaceSpan { kind: CameraSpanKind::LitOpaque, from: 0, until: end };
    let spans = ViewSurfaceSpans::new(all, Vec::new());
    VisibilityLevelData { lit_solid_begin: 0, lit_solid_end: end, ..VisibilityLevelData::new(spans) }
}

/// Fill in the sort order and surface bounds the two builders share, then
/// have the shell checked.
fn seal_dpvs(mut dpvs: VisibilityLevelData, order: Vec<u16>, facet_bounds: Vec<Bounds>) -> Result<VisibilityLevelData, LevelMeshFault> {
    dpvs.static_facet_tally = order.len();
    dpvs.static_facet_tally_no_stain = order.len();
    dpvs.sorted_facet_index = order;
    dpvs.facet_bounds = facet_bounds;
    dpvs.vetted()
}

/// BO2's plane as the cull wants it, with its axis type (3 when not along an axis).
fn cull_plane(plane: &[f32; 4]) -> PlaneEquation {
    PlaneEquation {
        normal: [plane[0], plane[1], plane[2]],
        offset: plane[3],
        r#type: plane[..3].iter().position(|&c| c == 1.0).unwrap_or(3) as u8,
    }
}

/// BO2's node as stored: childrenOffset stays in bytes of 40-byte nodes.
fn plain_tree(tree: &asset_t6::AabbTreeRef) -> AabbTree {
    AabbTree {
        lo: tree.lo_corner,
        hi: tree.hi_corner,
        sub_element_total: tree.sub_element_total,
        facet_tally: tree.facet_tally,
        start_surf_index: tree.start_surf_index,
        sub_elements_start: tree.sub_elements_start,
        static_model_slot_total: 0,
        static_model_slot_first: 0,
    }
}

/// The static list's surfaces first, then the ones it leaves out; also where
/// the static part ends.
fn static_then_leftover(world: &WorldRef) -> (Vec<u16>, usize) {
    let listed = listed_mask(world);
    let mut order = world.sorted_facet_index.clone();
    let static_end = order.len();
    let missing = (0..world.surfaces.len() as u16).filter(|&s| !listed[usize::from(s)]);
    order.extend(missing);
    (order, static_end)
}

/// The map's cells: BO2's static surface list, each cell's AABB trees and
/// portals, and the cell tree. Surfaces the list leaves out follow it in
/// `sorted_facet_index`, under one node in an extra tree at `cell_count`.
fn cells_dpvs(world: &WorldRef, facet_bounds: Vec<Bounds>) -> Result<VisibilityLevelData, LevelMeshFault> {
    let surf_total = world.surfaces.len();
    u16::try_from(surf_total).map_err(|_| LevelMeshFault::EmptyWorld)?;
    let mut dpvs = lit_opaque_dpvs(surf_total);
    dpvs.cell_total = world.cell_bounds.len();
    dpvs.planes = world.dpvs_planes.iter().map(cull_plane).collect();
    dpvs.tree_nodes.clone_from(&world.dpvs_nodes);

    let (order, static_end) = static_then_leftover(world);
    let leftover = u16::try_from(order.len() - static_end).map_err(|_| LevelMeshFault::EmptyWorld)?;

    for (cell, cell_bounds) in world.cell_bounds.iter().enumerate() {
        let trees = world.cells.get(cell).map(Vec::as_slice).unwrap_or_default();
        dpvs.aabb_trees.push(trees.iter().map(plain_tree).collect());
        dpvs.aabb_prop_indices.push(Vec::new());
        dpvs.room_env_probes.push(Vec::new());
        let portals: Vec<SelfContainedVisibilityPortal> = cell_bounds
            .portals
            .iter()
            .map(|portal| {
                let vertex_first = dpvs.portal_vertices.len();
                dpvs.portal_vertices.extend_from_slice(&portal.vertices);
                SelfContainedVisibilityPortal {
                    split: portal.plane,
                    adjacent_cell: portal.cell,
                    vertex_first,
                    vertex_tally: portal.vertices.len(),
                    hull_axes_dir: Some(portal.hull_axes_dir),
                }
            })
            .collect();
        dpvs.portals_per_room.push(portals);
    }
    if leftover > 0 {
        let rest = AabbTree {
            lo: world.lo_corner,
            hi: world.hi_corner,
            facet_tally: leftover,
            start_surf_index: static_end as u16,
            ..Default::default()
        };
        dpvs.aabb_trees.push(vec![rest]);
        dpvs.aabb_prop_indices.push(Vec::new());
    }
    seal_dpvs(dpvs, order, facet_bounds)
}

/// One cell, one AABB node over every surface, identity sort order.
fn one_cell_dpvs(world: &WorldRef, facet_bounds: Vec<Bounds>) -> Result<VisibilityLevelData, LevelMeshFault> {
    let surf_total = world.surfaces.len();
    let count = u16::try_from(surf_total).map_err(|_| LevelMeshFault::EmptyWorld)?;
    let mut dpvs = lit_opaque_dpvs(surf_total);
    dpvs.cell_total = 1;
    let mut whole = AabbTree::default();
    whole.lo = world.lo_corner;
    whole.hi = world.hi_corner;
    whole.facet_tally = count;
    dpvs.aabb_trees = vec![vec![whole]];
    dpvs.aabb_prop_indices = vec![Vec::new()];
    dpvs.portals_per_room = vec![Vec::new()];
    dpvs.room_env_probes = vec![Vec::new()];
    seal_dpvs(dpvs, (0..count).collect(), facet_bounds)
}

/// What makes two surfaces share a batch: material, whether lightmapped, and
/// the lightmap page, primary light and reflection probe numbers.
#[derive(Clone, Copy, Default, PartialEq)]
struct BatchLook {
    material_row: Option<usize>,
    lit: bool,
    page: u8,
    light: u8,
    probe: u8,
}

/// The per-vertex streams a batch copies its vertices out of.
struct VertexStreams<'a> {
    pos: &'a [[f32; 3]],
    nrm: &'a [[f32; 3]],
    tan: &'a [[f32; 4]],
    rgba: &'a [[f32; 4]],
    first_texcoord: &'a [[f32; 2]],
    uv1: &'a [[f32; 2]],
}

/// A batch's own copy of the world vertices its triangles use.
#[derive(Default)]
struct LocalMesh {
    ids: HashMap<u32, u32>,
    lanes: Columns,
    triangle_list: Vec<u32>,
}

impl LocalMesh {
    /// Add a run of world vertex numbers as triangle corners, copying each
    /// vertex in on first sight. Returns where the run starts in `triangle_list`.
    fn append_run(&mut self, from: &VertexStreams, run: &[u32]) -> u32 {
        let first = self.triangle_list.len() as u32;
        for &absolute in run {
            let fresh = self.lanes.len() as u32;
            let local = *self.ids.entry(absolute).or_insert(fresh);
            if local == fresh {
                self.lanes.push_copy(from, absolute as usize);
            }
            self.triangle_list.push(local);
        }
        first
    }
}

/// A batch still taking surfaces.
struct OpenBatch {
    look: BatchLook,
    mesh: LocalMesh,
}

impl OpenBatch {
    fn seal(self) -> LevelDrawGroup {
        let BatchLook { material_row, lit, page, light, probe } = self.look;
        let LocalMesh { lanes, triangle_list, .. } = self.mesh;
        let mut built = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
        built.insert_attribute(Mesh::ATTRIBUTE_POSITION, lanes.pos);
        built.insert_attribute(Mesh::ATTRIBUTE_NORMAL, lanes.nrm);
        built.insert_attribute(Mesh::ATTRIBUTE_TANGENT, lanes.tan);
        built.insert_attribute(Mesh::ATTRIBUTE_COLOR, lanes.rgba);
        built.insert_attribute(Mesh::ATTRIBUTE_UV_0, lanes.first_texcoord);
        built.insert_attribute(Mesh::ATTRIBUTE_UV_1, lanes.uv1);
        built.insert_indices(Indices::U32(triangle_list.clone()));
        LevelDrawGroup {
            built_mesh: built,
            compact_indices: triangle_list,
            mat_slot: material_row,
            has_baked_light: lit,
            lightbake_index: page,
            key_light_index: light,
            env_probe_index: probe,
        }
    }
}

/// The batches, and for each surface the (batch, first index, index count)
/// its triangles landed at.
struct Batched {
    batches: Vec<LevelDrawGroup>,
    spans: Vec<(usize, u32, u32)>,
}

#[allow(clippy::too_many_arguments)]
fn batch_by_look(
    pos: &[[f32; 3]],
    nrm: &[[f32; 3]],
    tan: &[[f32; 4]],
    rgba: &[[f32; 4]],
    first_texcoord: &[[f32; 2]],
    uv1: &[[f32; 2]],
    pool: &[u32],
    runs: &[(u32, u32)],
    mats: &[Option<usize>],
    lit: &[bool],
    pages: &[u8],
    lights: &[u8],
    probes: &[u8],
) -> Batched {
    let streams = VertexStreams { pos, nrm, tan, rgba, first_texcoord, uv1 };
    let look_of = |i: usize| BatchLook {
        material_row: mats.get(i).copied().flatten(),
        lit: lit.get(i) == Some(&true),
        page: pages[i],
        light: lights.get(i).copied().unwrap_or_default(),
        probe: probes.get(i).copied().unwrap_or_default(),
    };
    let mut open: Vec<OpenBatch> = vec![];
    let spans: Vec<(usize, u32, u32)> = runs
        .iter()
        .enumerate()
        .map(|(i, &(start, count))| {
            let Some(run) = (count > 0).then(|| &pool[start as usize..][..count as usize]) else {
                return Default::default();
            };
            let look = look_of(i);
            let slot = open.iter().position(|b| b.look == look).unwrap_or_else(|| {
                open.push(OpenBatch { look, mesh: LocalMesh::default() });
                open.len() - 1
            });
            (slot, open[slot].mesh.append_run(&streams, run), count)
        })
        .collect();
    Batched { batches: open.into_iter().map(OpenBatch::seal).collect(), spans }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-triangle world surface at the start of the shared vertex block.
    fn flat_surface(lightmap_page: i8, light_slot: i8) -> asset_t6::WorldSurface {
        asset_t6::WorldSurface {
            tri_tally: 1,
            vertex_count: 3,
            lightbake_index: lightmap_page,
            key_light_index: light_slot,
            vertex_data_offset1: -1,
            flags: Default::default(),
            env_probe_index: Default::default(),
            material: Default::default(),
            base_index: Default::default(),
            first_vertex: Default::default(),
            vertex_data_offset0: Default::default(),
            bounds: Default::default(),
            hi_corner: Default::default(),
            lo_corner: Default::default(),
        }
    }

    fn aabb_ref(
        mins: [f32; 3],
        maxs: [f32; 3],
        first_surface: u16,
        surfaces: u16,
        kids: u16,
        byte_offset: i32,
    ) -> asset_t6::AabbTreeRef {
        asset_t6::AabbTreeRef {
            sub_element_total: kids,
            facet_tally: surfaces,
            start_surf_index: first_surface,
            sub_elements_start: byte_offset,
            prop_indexes: Vec::new(),
            lo_corner: mins,
            hi_corner: maxs,
        }
    }

    /// A world row at `at` with +z for its normal and white colour.
    fn upward_row(at: [f32; 3]) -> [u8; T6_WORLD_VERTEX] {
        let mut cells = [0u8; T6_WORLD_VERTEX];
        for (k, c) in at.iter().enumerate() {
            cells[k * 4..(k + 1) * 4].copy_from_slice(&c.to_le_bytes());
        }
        cells[16..20].fill(255);
        cells[24..28].copy_from_slice(&(0x1ffu32 << 20).to_le_bytes());
        cells
    }

    #[test]
    fn t6_glass_pane_stands_where_its_window_does() {
        // One triangle in model space, its normal +z, lightmap 2.
        let corners = [[10.0f32, 0.0, 0.0], [0.0, 10.0, 0.0], [0.0, 0.0, 0.0]];
        let world = WorldRef {
            vd0: corners.iter().flat_map(|&c| upward_row(c)).collect(),
            indices: vec![0, 1, 2],
            surfaces: vec![flat_surface(2, 1)],
            solid_models: vec![(0, 0, [[0.0; 3]; 2]), (0, 1, [[0.0; 3]; 2])],
            ..Default::default()
        };
        let glasses = vec![asset_t6::GlassRef {
            pristine_material: "clear".into(),
            cracked_material: "cracked".into(),
            ..Default::default()
        }];
        // Yaw 90: model x turns to world +y, model y to world -x.
        let place = crate::T6GlassPlacement {
            solid_model: 1,
            origin: [100.0, 200.0, 300.0],
            angles: [0.0, 90.0, 0.0],
            glass: 0,
        };
        let panes = build_t6_glass_draws(&world, &glasses, &[place]);
        let pane = &panes[0];
        let near = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-3);
        assert!(near(pane.vertices[0].position, [100.0, 210.0, 300.0]), "{:?}", pane.vertices[0]);
        assert!(near(pane.vertices[1].position, [90.0, 200.0, 300.0]), "{:?}", pane.vertices[1]);
        assert!(near(pane.vertices[2].hit_facing, [0.0, 0.0, 1.0]));
        assert_eq!(pane.tri_list, vec![0, 1, 2]);
        let expected = T6GlassSurface { surface: 0, first: 0, count: 3, lightmap: 2, dominant_light: 1, reflection_probe: 0 };
        assert_eq!(pane.surfaces, vec![expected]);
        assert_eq!((pane.pristine.as_str(), pane.cracked.as_str()), ("clear", "cracked"));
        // The world model (0) and a missing one draw nothing.
        let third_person_mesh = crate::T6GlassPlacement { solid_model: 0, ..place };
        assert!(build_t6_glass_draws(&world, &glasses, &[third_person_mesh])[0].surfaces.is_empty());
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
        assert_eq!(d.bytes, v);
        // A quarter turn about z: x goes to y.
        let quarter_turn = [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
        place_vertex(&mut d, [1.0, 2.0, 3.0], quarter_turn);
        let row = asset_t6::T6WorldVertex::read(&d.bytes).unwrap();
        assert_eq!(row.base.position, [1.0, 2.0, 3.0]);
        assert_eq!(row.base.color, [1, 2, 3, 4]);
        assert_eq!(row.base.uv(), [1.0, 0.5]);
        assert_eq!(row.base.unpacked_normal(), [0.0, 0.0, 1.0]);
        assert_eq!(row.base.unpacked_tangent(), [0.0, 1.0, 0.0]);
        assert_eq!(row.lightmap_uv(), [0.0, 1.0]);
    }

    #[test]
    fn t6_cells_keep_their_trees_and_portals() {
        let tree = |start: u16, count: u16, kids: u16, offset: i32| {
            aabb_ref([0.0; 3], [1.0; 3], start, count, kids, offset)
        };
        let door = asset_t6::PortalRef {
            plane: [1.0, 0.0, 0.0, 0.0],
            cell: 1,
            vertices: vec![[0.0; 3]; 4],
            hull_axes_dir: [[0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        };
        // Surface 1 is a submodel's: the static list leaves it out.
        let world = WorldRef {
            surfaces: vec![flat_surface(0, 0); 3],
            sorted_facet_index: vec![2, 0],
            cells: vec![
                vec![tree(0, 2, 2, 40), tree(0, 1, 0, 0), tree(1, 1, 0, 0)],
                vec![tree(1, 1, 0, 0)],
            ],
            dpvs_planes: vec![[1.0, 0.0, 0.0, 0.0]],
            dpvs_nodes: vec![0],
            cell_bounds: vec![
                {
                    let mut cell = asset_t6::CellRef::default();
                    cell.portals = vec![door];
                    cell
                },
                asset_t6::CellRef::default(),
            ],
            lo_corner: [0.0; 3],
            hi_corner: [1.0; 3],
            ..Default::default()
        };
        let unit_box = Bounds::from_corner_extremes([0.0; 3], [1.0; 3]);
        let d = cells_dpvs(&world, vec![unit_box; 3]).unwrap();
        assert_eq!((d.cell_total, d.aabb_trees.len()), (2, 3));
        assert_eq!(d.sorted_facet_index, vec![2, 0, 1]);
        // childrenOffset stays in bytes: one BO2 node on is 40.
        assert_eq!(d.aabb_trees[0][0].sub_elements_start, dpvs_t6::AABB_TREE_SIZE as i32);
        let extra = &d.aabb_trees[2][0];
        assert_eq!((extra.start_surf_index, extra.facet_tally), (2, 1));
        assert_eq!(d.planes[0].r#type, 0);
        let p = &d.portals_per_room[0][0];
        assert_eq!((p.adjacent_cell, p.vertex_first, p.vertex_tally), (1, 0, 4));
        assert!(p.hull_axes_dir.is_some());
        assert!(d.portals_per_room[1].is_empty());
    }

    #[test]
    fn batches_split_by_look_and_share_vertices() {
        // Four vertices in a row; surfaces 0 and 2 share a look, 1 is lit,
        // 3 is empty. Surface 2 reuses vertex 1 and 2 of surface 0.
        let pos: Vec<[f32; 3]> = (0..4).map(|k| [k as f32, 0.0, 0.0]).collect();
        let nrm = vec![[0.0, 0.0, 1.0]; 4];
        let tan = vec![[1.0, 0.0, 0.0, 1.0]; 4];
        let rgba = vec![[1.0; 4]; 4];
        let first_texcoord: Vec<[f32; 2]> = (0..4).map(|k| [k as f32, 1.0]).collect();
        let uv1 = vec![[0.5, 0.5]; 4];
        let pool = [0u32, 1, 2, 1, 2, 3, 1, 2, 3, 0, 1, 2];
        let spans = [(0u32, 3u32), (3, 3), (6, 3), (9, 0)];
        let mats = [Some(4usize), Some(4), Some(4), None];
        let lit = [false, true, false, false];
        let page = [31u8, 2, 31, 31];
        let light = [0u8, 5, 0, 0];
        let probe = [0u8, 0, 7, 0];
        let Batched { batches, spans: ranges } = batch_by_look(
            &pos, &nrm, &tan, &rgba, &first_texcoord, &uv1, &pool, &spans, &mats, &lit, &page, &light, &probe,
        );
        assert_eq!(batches.len(), 3);
        assert_eq!(ranges, vec![(0, 0, 3), (1, 0, 3), (2, 0, 3), (0, 0, 0)]);
        let look = |b: &LevelDrawGroup| {
            (b.mat_slot, b.has_baked_light, b.lightbake_index, b.key_light_index, b.env_probe_index)
        };
        assert_eq!(look(&batches[0]), (Some(4), false, 31, 0, 0));
        assert_eq!(look(&batches[1]), (Some(4), true, 2, 5, 0));
        assert_eq!(look(&batches[2]), (Some(4), false, 31, 0, 7));
        assert_eq!(batches[1].compact_indices, vec![0, 1, 2]);
        let first_uvs = match batches[1].built_mesh.attribute(Mesh::ATTRIBUTE_UV_0) {
            Some(bevy::render::mesh::VertexAttributeValues::Float32x2(v)) => v.clone(),
            other => panic!("{other:?}"),
        };
        assert_eq!(first_uvs, vec![[1.0, 1.0], [2.0, 1.0], [3.0, 1.0]]);
        // A run that names a vertex twice copies it once.
        let Batched { batches: again, .. } = batch_by_look(
            &pos, &nrm, &tan, &rgba, &first_texcoord, &uv1, &[1, 2, 1, 1, 2, 3], &[(0, 3), (3, 3)],
            &[None, None], &[], &[0, 0], &[], &[],
        );
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].compact_indices, vec![0, 1, 0, 0, 1, 2]);
    }

    #[test]
    fn t6_empty_aabb_node_is_a_cleared_box() {
        // mp_la and mp_raid: cell 36 node 0 holds inverted extremes.
        let world = WorldRef {
            cells: vec![vec![aabb_ref([f32::MAX; 3], [-f32::MAX; 3], 0, 0, 0, 0)]],
            cell_bounds: vec![asset_t6::CellRef::default()],
            lo_corner: [0.0; 3],
            hi_corner: [1.0; 3],
            ..Default::default()
        };
        let d = cells_dpvs(&world, Vec::new()).unwrap();
        assert!(d.aabb_trees[0][0].bounds().has_been_reset());
        assert_eq!(d.emptied_volumes, 1);
    }
}
