//! What the render world takes from the main world each frame. The sealed
//! frame, image handles, post effects and geometry copies live in the
//! submodules; the BO2 model and weapon extraction follows.
use std::{sync::Arc, time::Instant};
use bevy::{prelude::*, render::{Extract, MainWorld}};
use render_frontend::assemble::draw_items::MaterialEpoch;
mod shapes;
mod render_world;
mod seal;
mod shares;
pub use shapes::pull_geometry_to_render_world;
pub use render_world::pull_precise_colour;
pub use render_world::pull_image_handles_to_render_world;
pub use render_world::extract_model_lighting_tiles;
pub use render_world::pull_screen_effects_to_render_world;
pub use seal::seal_render_frame;
/// bo2zm: one skinned BO2 model row (what the first-person rig writes)
/// as the fallback pass's model vertex: position, normal, colour, uv_semantic.
fn t6_packed_to_vertex(row: &[u8; asset_t6::T6_PACKED_VERTEX]) -> render_frame::PropVertex {
    let v = asset_t6::T6PackedVertex::read(row).expect("a whole BO2 model row");
    render_frame::PropVertex {
        position: v.position,
        normal: asset_model::unit_or_vertical(v.unpacked_normal()),
        color: v.color_unorm(),
        first_texcoord: v.uv(),
    }
}

/// `R_HashString("featherParms")`: an effect material's (1 / feather
/// distance, feather distance).
const T6_FEATHER_PARMS: u32 = 0x4d7e_a234;
/// bo2zm M3 fix list 1: `eyeOffsetParms` (its register slot in the `_eo`
/// effect techniques, from the t6shader dump): x = units the sprite is
/// drawn nearer the eye.
const T6_EYE_OFFSET_PARMS: u32 = 0x0812_c0a9;
/// bo2mp: a lens flare's (`sw4_3d_superflare_*`) `FlareIntensity` and
/// `FlareScale`, and the semantic its grime map rides under (layer 1's).
const T6_FLARE_INTENSITY: u32 = 0x64b8_0bd3;
const T6_FLARE_SCALE: u32 = 0xd6d0_afa4;
const T6_LAYER1_SEMANTIC: u8 = 0xf1;
/// bo2zm M3 fix list 1: the falloff effect techniques' `falloffParms` and
/// `falloffEndColor` (register slots from the t6shader dump): a sprite
/// fades as it turns edge-on to the eye.
const T6_FALLOFF_PARMS: u32 = 0xbdde_5cf5;
const T6_FALLOFF_END_COLOR: u32 = 0x6b1d_a6fa;
/// bo2zm M4 retest 5: `alphaDissolveParms` (register slot from the t6shader
/// dump of the `_ds` blood effects): their alpha is saturate(x * (texture
/// alpha + vertex alpha - 1)), the texture's alpha a dissolve order, not a
/// coverage (multiplied, its faint floor drew each sprite's whole square).
const T6_ALPHA_DISSOLVE_PARMS: u32 = 0x8c1d_2f74;

/// bo2zm: an effect material drawn with no colour map (white): logged once.
fn t6_note_untextured(material: u32, label: &str) {
    static SEEN: std::sync::Mutex<Vec<u32>> = std::sync::Mutex::new(Vec::new());
    if let Ok(mut seen) = SEEN.lock()
        && !seen.contains(&material)
    {
        seen.push(material);
        diag::warn!(World, "bo2zm fx: no colour map on material `{label}` ({material})");
    }
}

/// bo2zm M3: the shapes of things that hold their shape, kept across
/// frames: still script models, grenades and rockets in flight, and (bo2mp)
/// guns on the ground and corpses at rest. Each new shape goes up once, in
/// a chunk of its own; a frame then sends one placement row per model.
/// Re-sending them every frame grew with each gun dropped and each body
/// that fell, to ~270,000 vertices and ~12 ms a frame late in a match.
#[derive(Default)]
pub struct T6StaticCache {
    level_gen: Option<u64>,
    /// Bumped whenever the kept shapes are thrown out and gathered again.
    epoch: u64,
    frame: u64,
    vertex_count: u32,
    indices_total: u32,
    chunks: Vec<Arc<(Vec<render_frame::PropVertex>, Vec<u32>)>>,
    models: std::collections::HashMap<T6StaticKey, T6Kept>,
    /// Last frame's skinned poses, by address (held, so a new pose cannot
    /// take an old one's address): one handed on again holds still.
    posed_before: std::collections::HashMap<usize, Arc<render_anim::occupancy::missile::PosedVerts>>,
}

/// A kept shape: its surfaces (first index, count, colour slot, draw code),
/// its vertex count, the frame it was last drawn, and the pose it came
/// from (held, so its address names it).
struct T6Kept {
    surfaces: Vec<[u32; 6]>,
    /// bo2mp: per surface, a lit tree canopy's constants.
    canopy: Vec<Option<T6Canopy>>,
    vertices: u32,
    used: u64,
    _posed: Option<Arc<render_anim::occupancy::missile::PosedVerts>>,
}

/// A kept shape: its name, materials, (bo2mp) hidden parts and, when it was
/// skinned, its pose's address.
type T6StaticKey = (String, Vec<Option<u32>>, Option<([u32; 5], u32)>, usize);

/// Kept vertices past which the shapes nothing draws any more (guns picked
/// up, corpses gone) are thrown out, once they are half of them.
const T6_KEPT_VERTICES: u32 = 400_000;

/// bo2mp: how far a posed model may reach past its bind bounds (a player
/// lying dead or prone), in game units.
const T6_POSE_MARGIN: f32 = 48.0;

/// bo2mp: how near the eye a model still casts sun shadow into view, while
/// the sun shadow is drawn (its far partition is at most 4096 across).
const T6_SHADOW_REACH: f32 = 4096.0;

/// bo2mp: the camera's view, for leaving out the models it cannot see (a
/// live player is ~20k vertices copied and drawn a frame): its near and
/// side planes and, while the sun shadow is drawn, how near a model out
/// of view still casts into it.
struct T6ViewCull {
    planes: Vec<[f32; 4]>,
    eye: Vec3,
    shadow_reach: Option<f32>,
}

impl T6ViewCull {
    fn new(view: Option<&render_scene::StagedStageEye>) -> Self {
        let planes = match view {
            // Near, far, left, right, bottom, top: the far plane is left
            // out (a view without one puts it at the eye).
            Some(v) if v.primed && v.view_volume_planes.len() == 6 => {
                let p = &v.view_volume_planes;
                vec![p[0], p[2], p[3], p[4], p[5]]
            }
            _ => Vec::new(),
        };
        Self {
            planes,
            eye: view.map_or(Vec3::ZERO, |v| v.eye),
            shadow_reach: (!asset_core::t6_fast()).then_some(T6_SHADOW_REACH),
        }
    }

    fn sees(&self, item: &render_anim::occupancy::missile::T6MissileModel) -> bool {
        if self.planes.is_empty() {
            return true;
        }
        let bone_rig = &item.bone_rig;
        let (centre, half) = match (bone_rig.bounds, bone_rig.radius) {
            (Some((lo, hi)), _) => {
                let (lo, hi) = (Vec3::from(lo), Vec3::from(hi));
                ((lo + hi) * 0.5, (hi - lo).length() * 0.5)
            }
            (None, Some(r)) => (Vec3::ZERO, r),
            _ => return true,
        };
        let m = item.model_to_world;
        let scale = m.x_axis.truncate().length().max(m.y_axis.truncate().length()).max(m.z_axis.truncate().length());
        let centre = m.transform_point3(centre);
        let radius = half * scale + T6_POSE_MARGIN;
        if self.shadow_reach.is_some_and(|reach| centre.distance(self.eye) < reach + radius) {
            return true;
        }
        !self
            .planes
            .iter()
            .any(|p| p[0] * centre.x + p[1] * centre.y + p[2] * centre.z + p[3] + radius <= 0.0)
    }
}

/// One drawn surface of a model: its vertices, its triangles (all of the
/// surface's, or those its hidden parts leave), colour slot and draw code.
struct T6Surface {
    vbase: usize,
    normal_speed: usize,
    triangles: T6Triangles,
    slot: u32,
    code: u32,
    /// Its shine maps (normal, specular): BO2's gold power-ups, perk
    /// machines and the box gloss and reflect as the gun in his hands does.
    normal: u32,
    specular: u32,
    /// A zombie's eyes (see `T6Draw::eyes`).
    eyes: bool,
    /// bo2mp: a lit tree canopy's constants (see `t6_canopy_row`).
    canopy: Option<T6Canopy>,
}

enum T6Triangles {
    /// The surface's own index range (first, count).
    All(usize, usize),
    Left(Vec<u32>),
}

impl T6Surface {
    fn index_count(&self) -> usize {
        match &self.triangles {
            T6Triangles::All(_, count) => *count,
            T6Triangles::Left(left) => left.len(),
        }
    }
}

/// A model's drawn surfaces (with a material that draws, in range, not
/// all hidden).
fn t6_model_surfaces(
    item: &render_anim::occupancy::missile::T6MissileModel,
    catalog: &render_frontend::assemble::draw_items::LiveMaterialLedger,
) -> Vec<T6Surface> {
    let bone_rig = &item.bone_rig;
    let mut out = Vec::new();
    // Surfaces left out, by why: no material, not in the catalog, no BO2 draw.
    let mut dropped = [0u32; 3];
    let ranges = bone_rig.facet_vertex_ranges.iter().zip(&bone_rig.facet_index_ranges);
    for (facet, (&(vbase, normal_speed), &(ibase, icount))) in ranges.enumerate() {
        let Some(material) = item.materials.get(facet).copied().flatten() else {
            dropped[0] += 1;
            continue;
        };
        let Some(m) = catalog.derived(assets::SurfaceMaterialId::from_sequence(material as usize)) else {
            dropped[1] += 1;
            continue;
        };
        let Some(draw) = m.t6_draw else {
            dropped[2] += 1;
            continue;
        };
        if vbase + normal_speed > bone_rig.positions.len() || ibase + icount > bone_rig.indices.len() {
            continue;
        }
        // bo2mp: less its hidden parts (a mannequin's shot-off head).
        let triangles = match item.drawn_indices(facet, &bone_rig.indices[ibase..ibase + icount]) {
            None => continue,
            Some(std::borrow::Cow::Borrowed(all)) if !all.is_empty() => T6Triangles::All(ibase, icount),
            Some(std::borrow::Cow::Owned(left)) if !left.is_empty() => T6Triangles::Left(left),
            Some(_) => continue,
        };
        let slot = m
            .semantic_for_image(asset_material::SEMANTIC_COLOR)
            .map_or(u32::MAX, |id| id.0);
        let shine = |semantic| m.semantic_for_image(semantic).map_or(u32::MAX, |id| id.0);
        out.push(T6Surface {
            vbase,
            normal_speed,
            triangles,
            slot,
            code: draw.code(),
            normal: shine(asset_material::SEMANTIC_NORMAL),
            specular: shine(asset_material::SEMANTIC_SPECULAR),
            eyes: draw.eyes,
            canopy: render_frontend::assemble::draw_items::mesh_build::prop_mesh::t6_canopy(m),
        });
    }
    out
}

/// bo2mp: a lit tree canopy material's constants (`smodel::t6_canopy`).
type T6Canopy = ([u32; 2], [f32; 3]);

/// bo2mp: the instance row a script model's surface draws with: for a lit
/// tree canopy (BO2's animated vines on Pod and Castaway), its own copy of
/// the model's row (`instance`) holding the canopy constants where the
/// placed props' canopy rows hold them (see `smodel::t6_canopy`), one copy
/// per set of constants per model (`made`); the model's own row otherwise.
fn t6_canopy_row(
    instances: &mut Vec<[f32; 28]>,
    made: &mut Vec<(([u32; 2], [u32; 3]), u32)>,
    instance: u32,
    canopy: Option<T6Canopy>,
) -> u32 {
    let Some((params, sway)) = canopy else {
        return instance;
    };
    let key = (params, sway.map(f32::to_bits));
    if let Some(&(_, at)) = made.iter().find(|(seen, _)| *seen == key) {
        return at;
    }
    let mut copy = instances[instance as usize];
    copy[20] = params[1] as f32;
    copy[23] = -1.0 - params[0] as f32;
    copy[24..27].copy_from_slice(&sway);
    instances.push(copy);
    let at = instances.len() as u32 - 1;
    made.push((key, at));
    at
}

/// Per surface (first index, count, colour slot, draw code, normal map slot,
/// specular map slot), the first index counted from `first`.
fn t6_surface_rows(surfaces: &[T6Surface], mut first: u32) -> Vec<[u32; 6]> {
    surfaces
        .iter()
        .map(|s| {
            let count = s.index_count() as u32;
            let row = [first, count, s.slot, s.code, s.normal, s.specular];
            first += count;
            row
        })
        .collect()
}

/// A model's surfaces' vertices (from its skinned pose when it has one)
/// and indices appended; the indices count from `first_vertex` plus where
/// its vertices land in `vertices`.
fn t6_fill_model(
    bone_rig: &asset_model::MeshRig,
    posed: Option<&render_anim::occupancy::missile::PosedVerts>,
    surfaces: &[T6Surface],
    vertices: &mut Vec<render_frame::PropVertex>,
    mut indices: Option<&mut Vec<u32>>,
    first_vertex: u32,
) {
    // bo2mp: a pose left to the graphics card is skinned here instead (a
    // corpse at rest, held from now on).
    let posed = posed.map(|p| p.on_cpu(bone_rig));
    let (positions, surface_directions) = match posed.as_deref() {
        Some(posed) => (posed.positions.as_slice(), posed.surface_directions.as_slice()),
        None => (bone_rig.positions.as_slice(), bone_rig.surface_directions.as_slice()),
    };
    for s in surfaces {
        let base = first_vertex + vertices.len() as u32;
        let range = s.vbase..s.vbase + s.normal_speed;
        match (
            positions.get(range.clone()),
            surface_directions.get(range.clone()),
            bone_rig.colors.get(range.clone()),
            bone_rig.texcoord_set.get(range.clone()),
        ) {
            // Every list has the surface's vertices: a straight copy (a
            // live player's ~20k vertices every frame).
            (Some(p), Some(n), Some(c), Some(u)) => vertices.extend(p.iter().zip(n).zip(c).zip(u).map(
                |(((&position, &normal), &color), &first_texcoord)| render_frame::PropVertex {
                    position,
                    normal,
                    color,
                    first_texcoord,
                },
            )),
            _ => vertices.extend(range.map(|v| render_frame::PropVertex {
                position: positions.get(v).copied().unwrap_or(bone_rig.positions[v]),
                normal: surface_directions.get(v).copied().unwrap_or([0.0, 0.0, 1.0]),
                color: bone_rig.colors.get(v).copied().unwrap_or([1.0; 4]),
                first_texcoord: bone_rig.texcoord_set.get(v).copied().unwrap_or([0.0; 2]),
            })),
        }
        if let Some(indices) = indices.as_deref_mut() {
            t6_fill_indices(bone_rig, s, base, indices);
        }
    }
}

/// A surface's indices appended, counting from `base` (where its first
/// vertex lands).
fn t6_fill_indices(bone_rig: &asset_model::MeshRig, s: &T6Surface, base: u32, indices: &mut Vec<u32>) {
    let vbase = s.vbase as u32;
    match &s.triangles {
        T6Triangles::All(first, count) => indices.extend(
            bone_rig.indices[*first..*first + *count]
                .iter()
                .map(|&i| base + i.saturating_sub(vbase)),
        ),
        T6Triangles::Left(left) => indices.extend(left.iter().map(|&i| base + i.saturating_sub(vbase))),
    }
}

/// bo2mp: a model's shape as the graphics card skins it: every vertex in
/// its bind pose, and its four bones and weights (a vertex with none moves
/// with bone 0, as `skin_verts` does).
fn t6_skin_shape(bone_rig: &Arc<asset_model::MeshRig>) -> render_gpu::T6SkinShape {
    let source: Arc<dyn std::any::Any + Send + Sync> = bone_rig.clone();
    let vertex_total = bone_rig.positions.len();
    let bone_rig = Arc::clone(bone_rig);
    let fill = move |vertices: &mut Vec<render_frame::PropVertex>, weights: &mut Vec<[u32; 8]>| {
        const RIGID: [u32; 8] = [0, 0, 0, 0, 0x3f80_0000, 0, 0, 0];
        let n = bone_rig.positions.len();
        let blended = bone_rig.vert_skin.len() == n;
        vertices.extend((0..n).map(|v| render_frame::PropVertex {
            position: bone_rig.positions[v],
            normal: bone_rig.surface_directions.get(v).copied().unwrap_or([0.0, 0.0, 1.0]),
            color: bone_rig.colors.get(v).copied().unwrap_or([1.0; 4]),
            first_texcoord: bone_rig.texcoord_set.get(v).copied().unwrap_or([0.0; 2]),
        }));
        weights.extend((0..n).map(|v| {
            let Some(s) = bone_rig.vert_skin.get(v).filter(|_| blended) else {
                return RIGID;
            };
            let w = s.weights.map(|w| if w > 0.0 { w } else { 0.0 });
            if w.iter().sum::<f32>() <= 0.0 {
                return RIGID;
            }
            let b = s.joints.map(u32::from);
            [b[0], b[1], b[2], b[3], w[0].to_bits(), w[1].to_bits(), w[2].to_bits(), w[3].to_bits()]
        }));
    };
    render_gpu::T6SkinShape {
        source,
        vertices: vertex_total,
        fill: Arc::new(fill),
    }
}

/// A draw list's vertices, written now or on the render thread.
fn t6_vertex_count(draw: &render_gpu::T6DynamicDraw) -> usize {
    draw.deferred.as_ref().map_or(draw.vertices.len(), |d| d.vertices)
        + draw.skinned.as_ref().map_or(0, |s| s.vertices)
}

/// One model's drawn surfaces appended to a geometry whose first vertex
/// and index are `offset` into the whole: per surface (first index, count,
/// colour slot, draw code); vertices from its skinned pose when it has one.
fn t6_append_model(
    item: &render_anim::occupancy::missile::T6MissileModel,
    catalog: &render_frontend::assemble::draw_items::LiveMaterialLedger,
    vertices: &mut Vec<render_frame::PropVertex>,
    index_out: &mut Vec<u32>,
    offset: (u32, u32),
) -> (Vec<[u32; 6]>, Vec<Option<T6Canopy>>) {
    let surfaces = t6_model_surfaces(item, catalog);
    let rows = t6_surface_rows(&surfaces, offset.1 + index_out.len() as u32);
    t6_fill_model(&item.bone_rig, item.posed.as_deref(), &surfaces, vertices, Some(index_out), offset.0);
    (rows, surfaces.iter().map(|s| s.canopy).collect())
}

/// BO2MP_T6_LITLOG=1: an animated model whose light reads (near) black,
/// once a second: where it stands. BO2MP_T6_LITLOG=<part of a name>: each
/// model whose name holds it, once per place, however it is lit.
/// One draw instance as the shaders read it. Slots 0..16 hold the world-from-local matrix by columns and
/// 16..19 a light colour; the rest depend on the kind of draw (for a model lit from the light grid, 20 is its
/// primary light and 21 that light's visibility).
type InstanceRow = [f32; 28];

/// A row placed by `world_from_local` with `tint`, scaled by `scale`, in the light-colour slots.
fn instance_row(world_from_local: &[f32; 16], tint: [f32; 3], scale: f32) -> InstanceRow {
    let mut row = [0.0; 28];
    row[..16].copy_from_slice(world_from_local);
    row[16..19].copy_from_slice(&tint.map(|channel| channel * scale));
    row
}

/// A row for a model lit by the light grid sample `lit`, its light scaled by `exposure`.
fn grid_lit_row(world_from_local: &[f32; 16], lit: &lighting_t6::T6PointLight, exposure: f32) -> InstanceRow {
    let mut row = instance_row(world_from_local, lit.light, exposure);
    row[20] = f32::from(lit.primary);
    row[21] = lit.seen_share;
    row
}

/// codmp: a BO2 glass material's own constants (`sw4_3d_glass_scaled`, a
/// broken window's shards; fxc disassembly) for instance slots 24-26, read
/// by the GLASS shader: 1 + Specular_Amount, 1 + Reflection_Amount, and
/// 1 + the colour, normal and specular-gloss map scales (each times 16,
/// eight bits; the x scale, the data's y being the same). A missing one is
/// 1, as on the riot shield, whose row leaves the slots 0 (all 1).
fn t6_glass_params(constants: &[(u32, [u32; 4])]) -> [f32; 3] {
    let constant = |hash: u32, or: f32| {
        constants.iter().find(|(h, _)| *h == hash).map_or(or, |(_, bits)| f32::from_bits(bits[0]))
    };
    let sixteenths = |hash| (constant(hash, 1.0) * 16.0).round().clamp(0.0, 255.0);
    [
        1.0 + constant(GLASS_SPECULAR_AMOUNT, 1.0),
        1.0 + constant(GLASS_REFLECTION_AMOUNT, 1.0),
        1.0 + sixteenths(GLASS_COLOR_MAP_SCALE)
            + 256.0 * sixteenths(GLASS_NORMAL_MAP_SCALE)
            + 65536.0 * sixteenths(GLASS_SPECGLOSS_MAP_SCALE),
    ]
}

const GLASS_SPECULAR_AMOUNT: u32 = 0xa8cd_1103;
const GLASS_REFLECTION_AMOUNT: u32 = 0x3ccf_fe8b;
const GLASS_COLOR_MAP_SCALE: u32 = 0x215d_6223;
const GLASS_NORMAL_MAP_SCALE: u32 = 0x6951_4aed;
const GLASS_SPECGLOSS_MAP_SCALE: u32 = 0x4012_233f;

#[cfg(test)]
mod glass_tests {
    use super::*;

    /// mc/glass_clear_shatter_mp_model's constants (mp_drone): specular 0.5,
    /// reflection 1, every map scaled 2x; the riot shield's material has none.
    #[test]
    fn shard_glass_params() {
        let v = |x: f32| [x, x, 0.0, 1.0].map(f32::to_bits);
        let shards = [
            (GLASS_COLOR_MAP_SCALE, v(2.0)),
            (GLASS_REFLECTION_AMOUNT, v(1.0)),
            (GLASS_SPECGLOSS_MAP_SCALE, v(2.0)),
            (GLASS_NORMAL_MAP_SCALE, v(2.0)),
            (GLASS_SPECULAR_AMOUNT, v(0.5)),
        ];
        assert_eq!(t6_glass_params(&shards), [1.5, 2.0, 1.0 + 32.0 + 256.0 * 32.0 + 65536.0 * 32.0]);
        assert_eq!(t6_glass_params(&[]), [2.0, 2.0, 1.0 + 16.0 + 256.0 * 16.0 + 65536.0 * 16.0]);
    }
}

fn t6_note_dark(item: &render_anim::occupancy::missile::T6MissileModel, lit: &lighting_t6::T6PointLight) {
    static ON: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    let Some(on) = ON.get_or_init(|| std::env::var("BO2MP_T6_LITLOG").ok()) else {
        return;
    };
    if on != "1" {
        static SEEN: std::sync::Mutex<Vec<(String, [i32; 3])>> = std::sync::Mutex::new(Vec::new());
        let at = item.light_at.map(|c| c.round() as i32);
        if !item.bone_rig.name.contains(on.as_str()) {
            return;
        }
        if let Ok(mut seen) = SEEN.lock()
            && seen.len() < 256
            && !seen.iter().any(|(n, p)| *p == at && *n == item.bone_rig.name)
        {
            seen.push((item.bone_rig.name.clone(), at));
            diag::info!(
                World,
                "bo2zm model {} lit at {:?}: grid light {:?}, primary {}, visibility {:.2}, posed {}",
                item.bone_rig.name,
                at,
                lit.light,
                lit.primary,
                lit.seen_share,
                item.posed.is_some()
            );
        }
        return;
    }
    if item.posed.is_none() || lit.light.iter().sum::<f32>() >= 0.02 {
        return;
    }
    static LAST: std::sync::Mutex<u64> = std::sync::Mutex::new(0);
    let now = std::time::UNIX_EPOCH.elapsed().map_or(0, |d| d.as_secs());
    if let Ok(mut last) = LAST.lock()
        && *last != now
    {
        *last = now;
        diag::info!(
            World,
            "bo2zm model {} too dark at {:?}: grid light {:?}, primary {}, visibility {:.2}",
            item.bone_rig.name,
            item.light_at.map(|v| v.round()),
            lit.light,
            lit.primary,
            lit.seen_share
        );
    }
}

/// bo2zm: Black Ops II things that move, every frame: the first-person gun
/// and arms (skinned by the rig into the FPV draw plan), lit by the light
/// grid where they are; and every T6 material's colour slot and draw code.
pub fn extract_t6_dynamic(
    mut commands: Commands,
    fpv: Extract<Option<Res<render_anim::FirstPersonPaintPlan>>>,
    t6_fx: Extract<Option<Res<render_frontend::adapters::fx::system::T6FxMesh>>>,
    t6_marks: Extract<Option<Res<render_frontend::adapters::fx::system::T6MarkMesh>>>,
    t6_glass: Extract<Option<Res<render_frontend::assemble::draw_items::T6GlassMesh>>>,
    t6_missiles: Extract<Option<Res<render_anim::occupancy::missile::T6MissileModels>>>,
    t6_script_models: Extract<Option<Res<render_anim::occupancy::missile::T6ScriptModels>>>,
    t6_bodies: Extract<Option<Res<render_anim::occupancy::t6_body::T6BodyModels>>>,
    t6_lights: Extract<Option<Res<render_frontend::adapters::fx::system::T6DynLights>>>,
    runtime: Extract<Option<Res<MaterialEpoch>>>,
    exposure: Extract<Option<Res<render_frontend::assemble::draw_items::MapT5DaylightParseExposure>>>,
    scene_view: Extract<Option<Res<render_scene::StagedStageEye>>>,
    launch: Extract<Option<Res<frame::StartupReport>>>,
    existing: Option<ResMut<render_gpu::ExtractedT6Dynamic>>,
    mut static_cache: Local<T6StaticCache>,
) {
    let t0 = Instant::now();
    let catalog = match runtime.as_deref() {
        Some(materials) => &materials.catalog,
        None => return,
    };
    // Only the material table carries over; every draw list is rebuilt
    // below (cloning last frame's vertex lists cost as much as building
    // them).
    let mut out = match existing {
        Some(existing) => {
            render_gpu::ExtractedT6Dynamic {
                generation: existing.generation,
                mat_list: Arc::clone(&existing.mat_list),
                ..Default::default()
            }
        }
        None => render_gpu::ExtractedT6Dynamic::default(),
    };
    // bo2mp: the map's wind, which sways the tree canopies.
    out.wind = asset_core::t6_map_wind(launch.as_ref().map_or("", |l| l.zone.as_str()));
    if out.generation != catalog.revision_stamp || out.mat_list.is_empty() {
        let known = &catalog.materials;
        let materials: Vec<(u32, u32)> = known
            .iter()
            .filter_map(|m| {
                let draw = m.t6_draw?;
                let colour = m.semantic_for_image(asset_material::SEMANTIC_COLOR)?.0;
                Some((colour, draw.code()))
            })
            .collect();
        out.mat_list = Arc::new(materials);
        out.generation = catalog.revision_stamp;
    }
    let exposure = exposure.as_ref().map_or(1.0, |e| e.exposure);
    // bo2zm: what the gun extraction sees, on change.
    {
        use std::sync::Mutex;
        static LAST: Mutex<Option<String>> = Mutex::new(None);
        let line = match fpv.as_ref() {
            None => "no FPV plan".to_owned(),
            Some(plan) => format!(
                "plan visible={} geometry_ok={} placement_ok={} drawgun={:?} rows={} draws={} t6_materials={}",
                plan.visible,
                plan.geometry_ok,
                plan.placement_ok,
                plan.viewmodel_pass_flag,
                plan.fallback_rows().map_or(0, |r| r.rows.len()),
                plan.fallback_rows().map_or(0, |r| r.draws.len()),
                plan.fallback_rows().map_or(0, |r| r.draws.iter().filter(|d| d.2.is_some()).count()),
            ),
        };
        if let Ok(mut last) = LAST.lock()
            && last.as_deref() != Some(line.as_str())
        {
            diag::info!(ViewModelPerspective, "bo2zm viewmodel extract: {line}");
            *last = Some(line);
        }
    }
    out.viewmodel = fpv
        .as_ref()
        // The engine's own visibility also needs its MW2-style eye
        // lighting, which a BO2 map does not have; the fallback draw lights
        // the gun itself, so built + placed + not hidden is enough.
        .filter(|plan| {
            plan.geometry_ok
                && plan.placement_ok
                && plan.viewmodel_pass_flag != Some(0)
                && !out.mat_list.is_empty()
        })
        .and_then(|plan| {
            let rows = plan.fallback_rows()?;
            let vertices = rows.rows.iter().map(t6_packed_to_vertex).collect();
            let draws = rows
                .draws
                .iter()
                .filter_map(|&(start, count, mat_index)| {
                    let m = catalog.derived(assets::SurfaceMaterialId::from_sequence(mat_index? as usize))?;
                    let draw = m.t6_draw?;
                    let slot_of = |semantic| m.semantic_for_image(semantic).map_or(u32::MAX, |id| id.0);
                    Some([
                        start,
                        count,
                        slot_of(asset_material::SEMANTIC_COLOR),
                        draw.code(),
                        0,
                        slot_of(asset_material::SEMANTIC_NORMAL),
                        slot_of(asset_material::SEMANTIC_SPECULAR),
                    ])
                })
                .collect();
            let eye = plan.model_to_world.w_axis.truncate().to_array();
            let lit = lighting_t6::t6_light_sampler()
                .map(|sampler| sampler.at(eye))
                .unwrap_or_default();
            let mut instance = grid_lit_row(&plan.model_to_world.to_cols_array(), &lit, exposure);
            // The gun reflects the probe nearest the eye (plus one; 0 = none).
            instance[23] = lighting_t6::t6_nearest_probe(eye).map_or(0.0, |p| p as f32 + 1.0);
            Some(render_gpu::T6DynamicDraw {
                vertices,
                indices: rows.indices.to_vec(),
                draws,
                instances: vec![instance],
                deferred: None,
                skinned: None,
            })
        });
    // Effect sprites: world space, lit (when their material is) by the
    // grid at the eye.
    let eye_light = lighting_t6::t6_light_sampler()
        .zip(fpv.as_ref())
        .map(|(sampler, plan)| sampler.at(plan.model_to_world.w_axis.truncate().to_array()))
        .unwrap_or_default();
    // Each effect draw gets its own instance row: the eye's light, and in
    // slot 22 the material's `featherParms.x` (1 / its feather distance):
    // BO2's effect shaders fade a sprite out as it nears the camera; in
    // slot 23 its `eyeOffsetParms.x` (drawn that much nearer the eye).
    let fx_draw = |list: &Vec<(u32, u32, u32)>,
                   clouds: &[render_frontend::adapters::fx::system::T6CloudDraw],
                   mesh: &render_frontend::adapters::fx::system::T6FxMesh| {
        let mut instances: Vec<[f32; 28]> = Vec::with_capacity(list.len() + clouds.len());
        let mut draws: Vec<[u32; 7]> = list
            .iter()
            .filter_map(|&(start, count, mat_index)| {
                let m = catalog.derived(assets::SurfaceMaterialId::from_sequence(mat_index as usize))?;
                let draw = m.t6_draw?;
                let slot = m
                    .semantic_for_image(asset_material::SEMANTIC_COLOR)
                    .map_or(u32::MAX, |id| id.0);
                if slot == u32::MAX {
                    t6_note_untextured(mat_index, &m.name);
                }
                let constant = |want: u32| {
                    m.constants
                        .iter()
                        .find(|(name_hash, _)| *name_hash == want)
                        .map(|(_, bits)| bits.map(f32::from_bits))
                };
                let feather = constant(T6_FEATHER_PARMS).map_or(0.0, |p| p[0]);
                let gaze_lift = constant(T6_EYE_OFFSET_PARMS).map_or(0.0, |p| p[0]);
                let dissolve = constant(T6_ALPHA_DISSOLVE_PARMS).map_or(0.0, |p| p[0]);
                let falloff = constant(T6_FALLOFF_PARMS).map(|p| {
                    let end = constant(T6_FALLOFF_END_COLOR).map_or(1.0, |c| c[0]);
                    [p[2], p[3], end]
                });
                // BO2MP_T6_DRAWLOG_MAT=<material part>: that material's effect
                // draws as they leave for the graphics card.
                static WANT: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
                if let Some(want) = WANT.get_or_init(|| std::env::var("BO2MP_T6_DRAWLOG_MAT").ok())
                    && m.name.contains(want.as_str())
                {
                    diag::info!(
                        World,
                        "bo2zm fx {} draw: starts at {start}, {count} indices, colour slot {slot}, draw code {:#x}",
                        m.name,
                        draw.code()
                    );
                }
                let mut instance = instance_row(&Mat4::IDENTITY.to_cols_array(), eye_light.light, exposure);
                // A falloff material: slots 16..18 carry falloffParms.z, .w
                // and the falloff end colour (its begin colour is white in
                // every one), slot 19 says so (the effect shaders read no
                // grid light).
                if let Some(f) = falloff {
                    instance[16..19].copy_from_slice(&f);
                    instance[19] = 1.0;
                }
                // Slot 20: the primary light (0 or more), or for a dissolve
                // material (effect shaders, which read no primary light)
                // minus its alphaDissolveParms.x.
                instance[20] = if dissolve > 0.0 {
                    -dissolve
                } else {
                    f32::from(eye_light.primary)
                };
                instance[21] = eye_light.seen_share;
                instance[22] = feather;
                instance[23] = gaze_lift;
                // bo2zm M4 retest 5: a projected decal (a hit's blood on a
                // zombie): its box from the world in the columns, the grid's
                // light at the box.
                if draw.projected_decal {
                    let Some((box_from_world, at)) = mesh.model_boxes.get(&start) else {
                        return None;
                    };
                    instance[..16].copy_from_slice(box_from_world);
                    let lit = lighting_t6::t6_light_sampler()
                        .map(|sampler| sampler.at(*at))
                        .unwrap_or_default();
                    for c in 0..3 {
                        instance[16 + c] = lit.light[c] * exposure;
                    }
                    instance[19] = 0.0;
                }
                // bo2mp: a lens flare: slots 16 and 17 carry FlareIntensity
                // and FlareScale, and the draw's sixth word its grime map
                // (in layer 1's place).
                let mut grime = u32::MAX;
                if draw.flare {
                    instance[16] = constant(T6_FLARE_INTENSITY).map_or(1.0, |c| c[0]);
                    instance[17] = constant(T6_FLARE_SCALE).map_or(1.0, |c| c[0]);
                    instance[18] = 0.0;
                    instance[19] = 0.0;
                    grime = m.semantic_for_image(T6_LAYER1_SEMANTIC).map_or(u32::MAX, |id| id.0);
                }
                instances.push(instance);
                Some([start, count, slot, draw.code(), instances.len() as u32 - 1, grime, u32::MAX])
            })
            .collect();
        // Particle clouds: the box's placement, then (slots 16..19) the
        // colour, halved as BO2's cloud shaders halve it, and (20, 21) the
        // particle's width and height.
        for cloud in clouds {
            let Some(m) = catalog.derived(assets::SurfaceMaterialId::from_sequence(cloud.material as usize)) else {
                continue;
            };
            let Some(draw) = m.t6_draw.filter(|d| d.cloud) else {
                continue;
            };
            let slot = m
                .semantic_for_image(asset_material::SEMANTIC_COLOR)
                .map_or(u32::MAX, |id| id.0);
            // A cloud's colour goes in at half strength; its alpha and size fill the slots after it.
            let [red, green, blue, alpha] = cloud.color;
            let mut instance = instance_row(&cloud.world_from_local, [red, green, blue], 0.5);
            instance[19] = alpha;
            instance[20] = cloud.size[0];
            instance[21] = cloud.size[1];
            instances.push(instance);
            draws.push([
                cloud.first_index,
                cloud.index_count,
                slot,
                draw.code(),
                instances.len() as u32 - 1,
                u32::MAX,
                u32::MAX,
            ]);
        }
        render_gpu::T6DynamicDraw {
            vertices: mesh.vertices.clone(),
            indices: mesh.indices.clone(),
            draws,
            instances,
            deferred: None,
            skinned: None,
        }
    };
    // Marks: one instance per mark, lit by the grid where it landed.
    // codmp: the map's windows go out with them (their vertices and indices
    // after the marks').
    let sampler = lighting_t6::t6_light_sampler();
    let lists: Vec<_> = [
        t6_marks.as_ref().map(|m| (&m.vertices, &m.indices, &m.draws)),
        t6_glass.as_ref().map(|g| (&g.vertices, &g.indices, &g.draws)),
    ]
    .into_iter()
    .flatten()
    .filter(|entry| !entry.2.is_empty())
    .collect();
    out.marks = (!lists.is_empty()).then(|| {
        let mut vertices = Vec::new();
        let mut joined = Vec::new();
        let mut listed = Vec::new();
        for (vertex_points, idx, draws) in lists {
            let (base, first) = (vertices.len() as u32, joined.len() as u32);
            vertices.extend_from_slice(vertex_points);
            joined.extend(idx.iter().map(|&i| i + base));
            listed.extend(
                draws
                    .iter()
                    .map(|&(start, count, material, at, lit)| (start + first, count, material, at, lit)),
            );
        }
        let mut instances = Vec::with_capacity(listed.len());
        let draws = listed
            .iter()
            .filter_map(|&(start, count, material, at, lightmap)| {
                let m = catalog.derived(assets::SurfaceMaterialId::from_sequence(material as usize))?;
                let draw = m.t6_draw?;
                let slot_of = |semantic| m.semantic_for_image(semantic).map_or(u32::MAX, |id| id.0);
                let slot = slot_of(asset_material::SEMANTIC_COLOR);
                let identity = Mat4::IDENTITY.to_cols_array();
                let mut row = match lightmap {
                    // Lit by its wall's lightmap: the vertex colours carry the light, and a positive slot 19
                    // tells the shader so.
                    Some((light_index, vis)) => {
                        let mut baked = instance_row(&identity, [0.0; 3], 0.0);
                        baked[19] = exposure.max(0.000001);
                        baked[20] = f32::from(light_index);
                        baked[21] = vis;
                        baked
                    }
                    None => {
                        let lit = sampler.as_ref().map(|grid| grid.at(at)).unwrap_or_default();
                        grid_lit_row(&identity, &lit, exposure)
                    }
                };
                // codmp: a broken window's shards are BO2's glass, like the
                // riot shield's: their normal and specular maps, the probe
                // nearest them to reflect (plus one; 0 = none) and their
                // material's amounts and map scales.
                let (normal, specular) = if draw.glass {
                    row[23] = lighting_t6::t6_nearest_probe(at).map_or(0.0, |p| p as f32 + 1.0);
                    row[24..27].copy_from_slice(&t6_glass_params(&m.constants));
                    (slot_of(asset_material::SEMANTIC_NORMAL), slot_of(asset_material::SEMANTIC_SPECULAR))
                } else {
                    (u32::MAX, u32::MAX)
                };
                // bo2mp: a mark lit like the one before it shares its row
                // (and, its indices following on, its draw: ~256 a frame).
                if instances.last() != Some(&row) {
                    instances.push(row);
                }
                Some([start, count, slot, draw.code(), instances.len() as u32 - 1, normal, specular])
            })
            .collect();
        render_gpu::T6DynamicDraw {
            vertices,
            indices: joined,
            draws,
            instances,
            deferred: None,
            skinned: None,
        }
    });
    out.dlights = t6_lights.as_ref().map(|l| l.lights.clone()).unwrap_or_default();
    // Projectiles in flight: each model in its own space, its instance
    // row its placement and the grid's light where it is.
    // bo2zm M3: the map's script models (perk machines, the box, ...) draw
    // the same way, and (bo2mp) other players, corpses, guns and bags on
    // the ground.
    // A model that holds its shape (no pose, or the same pose handed on
    // again: a gun on the ground, a corpse at rest) is drawn from shapes
    // uploaded once; only what was skinned anew this frame is sent whole.
    let all = t6_missiles
        .as_ref()
        .map(|m| m.items.iter())
        .into_iter()
        .flatten()
        .chain(t6_script_models.as_ref().map(|m| m.items.iter()).into_iter().flatten())
        .chain(t6_bodies.as_ref().map(|m| m.items.iter()).into_iter().flatten());
    let mut moving: Vec<&render_anim::occupancy::missile::T6MissileModel> = Vec::new();
    let view_cull = T6ViewCull::new(scene_view.as_deref());
    let mut unseen = 0usize;
    let held_models: usize;
    {
        let cache = &mut *static_cache;
        let generation = catalog.revision_stamp.0;
        if cache.level_gen != Some(generation) {
            *cache = T6StaticCache {
                level_gen: Some(generation),
                epoch: cache.epoch + 1,
                ..Default::default()
            };
        }
        cache.frame += 1;
        let frame = cache.frame;
        if cache.vertex_count > T6_KEPT_VERTICES {
            let drawn: u32 = cache
                .models
                .values()
                .filter(|m| m.used + 1 >= frame)
                .map(|m| m.vertices)
                .sum();
            if drawn * 2 < cache.vertex_count {
                *cache = T6StaticCache {
                    level_gen: cache.level_gen,
                    epoch: cache.epoch + 1,
                    frame,
                    posed_before: std::mem::take(&mut cache.posed_before),
                    ..Default::default()
                };
            }
        }
        let mut posed_now = std::collections::HashMap::new();
        let mut new_vertices: Vec<render_frame::PropVertex> = Vec::new();
        let mut new_indices: Vec<u32> = Vec::new();
        let mut draws: Vec<[u32; 7]> = Vec::new();
        let mut instances: Vec<[f32; 28]> = Vec::new();
        for item in all {
            // Out of view: not copied or drawn (its pose is still noted, so
            // a corpse at rest stays held).
            let seen = view_cull.sees(item);
            unseen += usize::from(!seen);
            let pose = match &item.posed {
                None => 0,
                Some(posed) => {
                    let at = Arc::as_ptr(posed) as usize;
                    let held = cache.posed_before.contains_key(&at);
                    posed_now.insert(at, Arc::clone(posed));
                    if !held {
                        if seen {
                            moving.push(item);
                        }
                        continue;
                    }
                    at
                }
            };
            if !seen {
                continue;
            }
            let key = (item.bone_rig.name.clone(), item.materials.clone(), item.hide, pose);
            let kept = cache.models.entry(key).or_insert_with(|| {
                // Where this frame's new chunk starts; the append counts on
                // from what the chunk holds already.
                let offset = (cache.vertex_count, cache.indices_total);
                let before = new_vertices.len();
                let (surfaces, canopy) =
                    t6_append_model(item, catalog, &mut new_vertices, &mut new_indices, offset);
                T6Kept {
                    surfaces,
                    canopy,
                    vertices: (new_vertices.len() - before) as u32,
                    used: frame,
                    _posed: item.posed.clone(),
                }
            });
            kept.used = frame;
            if kept.surfaces.is_empty() {
                continue;
            }
            let lit = sampler.as_ref().map(|s| s.at(item.light_at)).unwrap_or_default();
            t6_note_dark(item, &lit);
            let mut row = grid_lit_row(&item.model_to_world.to_cols_array(), &lit, exposure);
            // Its reflection probe (the nearest, plus one), for its shine.
            let at = item.model_to_world.w_axis.truncate().to_array();
            row[23] = lighting_t6::t6_nearest_probe(at).map_or(0.0, |p| p as f32 + 1.0);
            instances.push(row);
            let instance = instances.len() as u32 - 1;
            let mut canopy_rows = Vec::new();
            for (s, &canopy) in kept.surfaces.iter().zip(&kept.canopy) {
                let at = t6_canopy_row(&mut instances, &mut canopy_rows, instance, canopy);
                draws.push([s[0], s[1], s[2], s[3], at, s[4], s[5]]);
            }
        }
        cache.posed_before = posed_now;
        if !new_vertices.is_empty() {
            cache.vertex_count += new_vertices.len() as u32;
            cache.indices_total += new_indices.len() as u32;
            cache.chunks.push(Arc::new((new_vertices, new_indices)));
        }
        held_models = instances.len();
        out.script_static = (!draws.is_empty()).then(|| render_gpu::T6StaticDraw {
            epoch: cache.epoch,
            chunks: cache.chunks.clone(),
            draws,
            instances,
        });
    }
    // The moving models' vertices (the live players' ~250k a frame) are
    // copied on the render thread: here only what is drawn where, and the
    // poses to copy from (shared, not copied). bo2mp: a pose that is only
    // bones is skinned on the graphics card, after the copied vertices.
    out.missiles = (!moving.is_empty()).then(|| {
        let mut draw_rows = Vec::new();
        let mut instances = Vec::with_capacity(moving.len());
        let mut models = Vec::with_capacity(moving.len());
        let (mut moving_vertices, mut index_count) = (0usize, 0usize);
        let mut skinned = render_gpu::T6Skinned::default();
        let mut shape_of = std::collections::HashMap::new();
        // What the indices are made from: the same as last frame's and
        // they are not made again.
        let mut key = std::hash::DefaultHasher::new();
        for item in moving.iter().copied() {
            let lit = sampler.as_ref().map(|s| s.at(item.light_at)).unwrap_or_default();
            t6_note_dark(item, &lit);
            let plan = t6_model_surfaces(item, catalog);
            if plan.is_empty() {
                continue;
            }
            let surfaces = t6_surface_rows(&plan, index_count as u32);
            let on_card = item.posed.as_ref().filter(|p| p.on_gpu());
            match on_card {
                Some(posed) => {
                    let shape = *shape_of.entry(Arc::as_ptr(&item.bone_rig) as usize).or_insert_with(|| {
                        skinned.shapes.push(t6_skin_shape(&item.bone_rig));
                        skinned.shapes.len() as u32 - 1
                    });
                    let bones = (skinned.bones.len() as u32, posed.bones.len() as u32);
                    skinned.bones.extend(posed.bones.iter().map(|m| m.to_cols_array()));
                    for s in &plan {
                        skinned.runs.push([shape, s.vbase as u32, s.normal_speed as u32, bones.0, bones.1]);
                        skinned.vertices += s.normal_speed;
                    }
                }
                None => moving_vertices += plan.iter().map(|s| s.normal_speed).sum::<usize>(),
            }
            index_count += plan.iter().map(T6Surface::index_count).sum::<usize>();
            {
                use std::hash::Hash;
                (Arc::as_ptr(&item.bone_rig) as usize, on_card.is_some()).hash(&mut key);
                for s in &plan {
                    (s.vbase, s.normal_speed).hash(&mut key);
                    match &s.triangles {
                        T6Triangles::All(first, count) => (0u8, first, count).hash(&mut key),
                        T6Triangles::Left(left) => (1u8, left).hash(&mut key),
                    }
                }
            }
            let canopies: Vec<(Option<T6Canopy>, bool)> = plan.iter().map(|s| (s.canopy, s.eyes)).collect();
            models.push((item.bone_rig.clone(), item.posed.clone(), plan, on_card.is_some()));
            let mut row = grid_lit_row(&item.model_to_world.to_cols_array(), &lit, exposure);
            // Its reflection probe (the nearest, plus one), for its shine.
            let at = item.model_to_world.w_axis.truncate().to_array();
            row[23] = lighting_t6::t6_nearest_probe(at).map_or(0.0, |p| p as f32 + 1.0);
            instances.push(row);
            let instance = instances.len() as u32 - 1;
            // bo2zm round 1: a zombie's eyes get their own row: its eye
            // brightness rides negated in slot 22, which the unlit shader
            // scales the eye texture by (nothing else makes it negative).
            let eye_instance = match item.eye_luminance {
                Some(luminance) => {
                    row[22] = -luminance;
                    instances.push(row);
                    instances.len() as u32 - 1
                }
                None => instance,
            };
            let mut canopy_rows = Vec::new();
            for (s, (canopy, eyes)) in surfaces.into_iter().zip(canopies) {
                let base = if eyes { eye_instance } else { instance };
                let at = t6_canopy_row(&mut instances, &mut canopy_rows, base, canopy);
                draw_rows.push([s[0], s[1], s[2], s[3], at, s[4], s[5]]);
            }
        }
        // The card-skinned models' vertices land after the copied ones;
        // their indices count from there.
        let copied = moving_vertices as u32;
        let indices_key = {
            use std::hash::{Hash, Hasher};
            copied.hash(&mut key);
            key.finish().max(1)
        };
        let fill = move |vertices: &mut Vec<render_frame::PropVertex>, mut indices: Option<&mut Vec<u32>>| {
            let mut on_card = copied;
            for (bone_rig, posed, plan, card) in &models {
                if !*card {
                    t6_fill_model(bone_rig, posed.as_deref(), plan, vertices, indices.as_deref_mut(), 0);
                    continue;
                }
                let Some(indices) = indices.as_deref_mut() else {
                    continue;
                };
                for s in plan {
                    t6_fill_indices(bone_rig, s, on_card, indices);
                    on_card += s.normal_speed as u32;
                }
            }
        };
        render_gpu::T6DynamicDraw {
            vertices: Vec::new(),
            indices: Vec::new(),
            draws: draw_rows,
            instances,
            deferred: Some(render_gpu::T6DeferredFill {
                vertices: moving_vertices,
                indices: index_count,
                indices_key,
                fill: Arc::new(fill),
            }),
            skinned: (skinned.vertices > 0).then_some(skinned),
        }
    });
    out.effects = t6_fx
        .as_ref()
        .filter(|mesh| !mesh.draws.is_empty() || !mesh.clouds.is_empty())
        .map(|mesh| fx_draw(&mesh.draws, &mesh.clouds, mesh));
    out.viewmodel_effects = t6_fx
        .as_ref()
        .filter(|mesh| !mesh.viewmodel_draws.is_empty())
        .map(|mesh| fx_draw(&mesh.viewmodel_draws, &[], mesh));
    // BO2MP_T6_PROF=1: this extract's cost, every 300 frames, with what it
    // carried on the last of them.
    static PROF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *PROF.get_or_init(|| std::env::var_os("BO2MP_T6_PROF").is_some()) {
        static ACC: std::sync::Mutex<(u32, f64, usize, usize)> = std::sync::Mutex::new((0, 0.0, 0, 0));
        if let Ok(mut a) = ACC.lock() {
            a.0 += 1;
            a.1 += t0.elapsed().as_secs_f64() * 1000.0;
            a.2 += out.missiles.as_ref().map_or(0, t6_vertex_count);
            a.3 += out.missiles.as_ref().and_then(|m| m.skinned.as_ref()).map_or(0, |s| s.vertices);
            if a.0 >= 300 {
                let list = |d: Option<&render_gpu::T6DynamicDraw>| {
                    d.map_or((0, 0), |d| (t6_vertex_count(d), d.draws.len()))
                };
                let kept = static_cache.vertex_count;
                diag::info!(
                    World,
                    "bo2zm t6 extract: {:.2} ms per frame, {} moving vertices per frame ({} skinned on the card); \
                     this frame {} moving models, \
                     {} kept still, {} unseen ({} vertices held in {} chunks), {} bodies and ground \
                     items altogether; vertices and draws for moving {:?}, marks {:?}, effects {:?}",
                    a.1 / f64::from(a.0),
                    a.2 / a.0 as usize,
                    a.3 / a.0 as usize,
                    moving.len(),
                    held_models,
                    unseen,
                    kept,
                    static_cache.chunks.len(),
                    t6_bodies.as_ref().map_or(0, |b| b.items.len()),
                    list(out.missiles.as_ref()),
                    list(out.marks.as_ref()),
                    list(out.effects.as_ref()),
                );
                *a = (0, 0.0, 0, 0);
            }
        }
    }
    commands.insert_resource(out);
}

/// bo2zm: the Special Weapons tab's guest weapon in his hands and the
/// arrows in the map, to the render world (each texture converted once).
pub fn extract_guest_weapons(
    mut main_world: ResMut<MainWorld>,
    mut frame: ResMut<render_gpu::GuestViewFrame>,
    mut textures: Local<[Option<(usize, Arc<render_gpu::GuestViewTexture>)>; 2]>,
) {
    let Some(mut view) = main_world.get_resource_mut::<render_anim::guest_weapons::GuestWeaponView>() else {
        return;
    };
    frame.vertices = std::mem::take(&mut view.vertices);
    frame.indices = std::mem::take(&mut view.indices);
    frame.solid = view.solid;
    frame.to_clip = view.to_clip;
    frame.level_vertex_total = std::mem::take(&mut view.level_vertex_total);
    frame.level_indices = std::mem::take(&mut view.level_indices);
    frame.clock = view.clock;
    let mut convert = |image: &Option<Arc<render_anim::guest_weapons::GuestTexture>>, slot: usize| {
        image.as_ref().map(|image| {
            let key = Arc::as_ptr(image) as usize;
            if let Some((held, converted)) = textures[slot].as_ref()
                && *held == key
            {
                return converted.clone();
            }
            let converted = Arc::new(render_gpu::GuestViewTexture { size: image.size, layers: image.layers.clone() });
            textures[slot] = Some((key, converted.clone()));
            converted
        })
    };
    frame.hl2 = convert(&view.hl2, 0);
    frame.items = convert(&view.items, 1);
}
