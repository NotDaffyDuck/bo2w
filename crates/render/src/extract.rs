//! What the render world takes from the main world each frame. The sealed
//! frame, image handles, post effects and geometry copies live in the
//! submodules; the BO2 model and weapon extraction follows.
use std::{sync::Arc, time::Instant};
use bevy::{prelude::*, render::{Extract, MainWorld}};
use render_frontend::assemble::drawsurf::MaterialGeneration;
mod geometry;
mod render_world;
mod seal;
mod shares;
pub use geometry::extract_geometry;
pub use render_world::extract_exact_colour;
pub use render_world::extract_image_handles;
pub use render_world::extract_model_lighting_tiles;
pub use render_world::extract_postfx;
pub use seal::seal_render_frame;
/// bo2zm: one skinned BO2 model row (what the first-person rig writes)
/// as the fallback pass's model vertex: position, normal, colour, texcoord.
fn t6_packed_to_vertex(row: &[u8; asset_t6::T6_PACKED_VERTEX]) -> render_frame::SmodelVertex {
    let v = asset_t6::T6PackedVertex::read(row).expect("a whole BO2 model row");
    render_frame::SmodelVertex {
        position: v.position,
        normal: asset_model::normalize_or_up(v.unpacked_normal()),
        color: v.color_unorm(),
        uv0: v.uv(),
    }
}

/// `R_HashString("featherParms")`: an effect material's (1 / feather
/// distance, feather distance).
const T6_FEATHER_PARMS: u32 = 0x4d7e_a234;
/// bo2zm M3 fix list 1: `eyeOffsetParms` (its register slot in the `_eo`
/// effect techniques, from the t6shader dump): x = units the sprite is
/// drawn nearer the eye.
const T6_EYE_OFFSET_PARMS: u32 = 0x0812_c0a9;
/// bo2zm M3 fix list 1: the falloff effect techniques' `falloffParms` and
/// `falloffEndColor` (register slots from the t6shader dump): a sprite
/// fades as it turns edge-on to the eye.
const T6_FALLOFF_PARMS: u32 = 0xbdde_5cf5;
const T6_FALLOFF_END_COLOR: u32 = 0x6b1d_a6fa;

/// bo2zm: an effect material drawn with no colour map (white): logged once.
fn t6_note_untextured(material: u32, name: &str) {
    static SEEN: std::sync::Mutex<Vec<u32>> = std::sync::Mutex::new(Vec::new());
    if let Ok(mut seen) = SEEN.lock()
        && !seen.contains(&material)
    {
        seen.push(material);
        diag::warn!(World, "bo2zm fx: material `{name}` ({material}) has no colour map");
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
    generation: Option<u64>,
    /// Bumped whenever the kept shapes are thrown out and gathered again.
    epoch: u64,
    frame: u64,
    vertex_count: u32,
    index_count: u32,
    chunks: Vec<Arc<(Vec<render_frame::SmodelVertex>, Vec<u32>)>>,
    models: std::collections::HashMap<T6StaticKey, T6Kept>,
    /// Last frame's skinned poses, by address (held, so a new pose cannot
    /// take an old one's address): one handed on again holds still.
    posed_before: std::collections::HashMap<usize, Arc<render_anim::occupancy::missile::PosedVerts>>,
}

/// A kept shape: its surfaces (first index, count, colour slot, draw code),
/// its vertex count, the frame it was last drawn, and the pose it came
/// from (held, so its address names it).
struct T6Kept {
    surfaces: Vec<[u32; 4]>,
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
    fn new(view: Option<&render_scene::PreparedSceneView>) -> Self {
        let planes = match view {
            // Near, far, left, right, bottom, top: the far plane is left
            // out (a view without one puts it at the eye).
            Some(v) if v.ready && v.frustum_planes.len() == 6 => {
                let p = &v.frustum_planes;
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
        let skel = &item.skel;
        let (centre, half) = match (skel.bounds, skel.radius) {
            (Some((lo, hi)), _) => {
                let (lo, hi) = (Vec3::from(lo), Vec3::from(hi));
                ((lo + hi) * 0.5, (hi - lo).length() * 0.5)
            }
            (None, Some(r)) => (Vec3::ZERO, r),
            _ => return true,
        };
        let m = item.world_from_local;
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
    vn: usize,
    triangles: T6Triangles,
    slot: u32,
    code: u32,
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
    catalog: &render_frontend::assemble::drawsurf::RuntimeMaterialCatalog,
) -> Vec<T6Surface> {
    let skel = &item.skel;
    let mut out = Vec::new();
    let ranges = skel.surface_vertex_ranges.iter().zip(&skel.surface_index_ranges);
    for (surface, (&(vbase, vn), &(ibase, icount))) in ranges.enumerate() {
        let Some(material) = item.materials.get(surface).copied().flatten() else {
            continue;
        };
        let Some(m) = catalog.derived(assets::MaterialIndex::from_order(material as usize)) else {
            continue;
        };
        let Some(draw) = m.t6_draw else {
            continue;
        };
        if vbase + vn > skel.positions.len() || ibase + icount > skel.indices.len() {
            continue;
        }
        // bo2mp: less its hidden parts (a mannequin's shot-off head).
        let triangles = match item.drawn_indices(surface, &skel.indices[ibase..ibase + icount]) {
            None => continue,
            Some(std::borrow::Cow::Borrowed(all)) if !all.is_empty() => T6Triangles::All(ibase, icount),
            Some(std::borrow::Cow::Owned(left)) if !left.is_empty() => T6Triangles::Left(left),
            Some(_) => continue,
        };
        let slot = m
            .texture_semantic(asset_material::TS_COLOR_MAP)
            .map_or(u32::MAX, |id| id.0);
        out.push(T6Surface {
            vbase,
            vn,
            triangles,
            slot,
            code: draw.code(),
            canopy: render_frontend::assemble::drawsurf::tess::smodel::t6_canopy(m),
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
    if let Some(&(_, at)) = made.iter().find(|(k, _)| *k == key) {
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

/// Per surface (first index, count, colour slot, draw code), the first
/// index counted from `first`.
fn t6_surface_rows(surfaces: &[T6Surface], mut first: u32) -> Vec<[u32; 4]> {
    surfaces
        .iter()
        .map(|s| {
            let count = s.index_count() as u32;
            let row = [first, count, s.slot, s.code];
            first += count;
            row
        })
        .collect()
}

/// A model's surfaces' vertices (from its skinned pose when it has one)
/// and indices appended; the indices count from `first_vertex` plus where
/// its vertices land in `vertices`.
fn t6_fill_model(
    skel: &asset_model::ModelSkel,
    posed: Option<&render_anim::occupancy::missile::PosedVerts>,
    surfaces: &[T6Surface],
    vertices: &mut Vec<render_frame::SmodelVertex>,
    mut indices: Option<&mut Vec<u32>>,
    first_vertex: u32,
) {
    // bo2mp: a pose left to the graphics card is skinned here instead (a
    // corpse at rest, held from now on).
    let posed = posed.map(|p| p.on_cpu(skel));
    let (positions, normals) = match posed.as_deref() {
        Some(posed) => (posed.positions.as_slice(), posed.normals.as_slice()),
        None => (skel.positions.as_slice(), skel.normals.as_slice()),
    };
    for s in surfaces {
        let base = first_vertex + vertices.len() as u32;
        let range = s.vbase..s.vbase + s.vn;
        match (
            positions.get(range.clone()),
            normals.get(range.clone()),
            skel.colors.get(range.clone()),
            skel.uvs.get(range.clone()),
        ) {
            // Every list has the surface's vertices: a straight copy (a
            // live player's ~20k vertices every frame).
            (Some(p), Some(n), Some(c), Some(u)) => vertices.extend(p.iter().zip(n).zip(c).zip(u).map(
                |(((&position, &normal), &color), &uv0)| render_frame::SmodelVertex {
                    position,
                    normal,
                    color,
                    uv0,
                },
            )),
            _ => vertices.extend(range.map(|v| render_frame::SmodelVertex {
                position: positions.get(v).copied().unwrap_or(skel.positions[v]),
                normal: normals.get(v).copied().unwrap_or([0.0, 0.0, 1.0]),
                color: skel.colors.get(v).copied().unwrap_or([1.0; 4]),
                uv0: skel.uvs.get(v).copied().unwrap_or([0.0; 2]),
            })),
        }
        if let Some(indices) = indices.as_deref_mut() {
            t6_fill_indices(skel, s, base, indices);
        }
    }
}

/// A surface's indices appended, counting from `base` (where its first
/// vertex lands).
fn t6_fill_indices(skel: &asset_model::ModelSkel, s: &T6Surface, base: u32, indices: &mut Vec<u32>) {
    let vbase = s.vbase as u32;
    match &s.triangles {
        T6Triangles::All(first, count) => indices.extend(
            skel.indices[*first..*first + *count]
                .iter()
                .map(|&i| base + i.saturating_sub(vbase)),
        ),
        T6Triangles::Left(left) => indices.extend(left.iter().map(|&i| base + i.saturating_sub(vbase))),
    }
}

/// bo2mp: a model's shape as the graphics card skins it: every vertex in
/// its bind pose, and its four bones and weights (a vertex with none moves
/// with bone 0, as `skin_verts` does).
fn t6_skin_shape(skel: &Arc<asset_model::ModelSkel>) -> render_gpu::T6SkinShape {
    let source: Arc<dyn std::any::Any + Send + Sync> = skel.clone();
    let vertex_total = skel.positions.len();
    let skel = Arc::clone(skel);
    let fill = move |vertices: &mut Vec<render_frame::SmodelVertex>, weights: &mut Vec<[u32; 8]>| {
        const RIGID: [u32; 8] = [0, 0, 0, 0, 0x3f80_0000, 0, 0, 0];
        let n = skel.positions.len();
        let blended = skel.vert_skin.len() == n;
        vertices.extend((0..n).map(|v| render_frame::SmodelVertex {
            position: skel.positions[v],
            normal: skel.normals.get(v).copied().unwrap_or([0.0, 0.0, 1.0]),
            color: skel.colors.get(v).copied().unwrap_or([1.0; 4]),
            uv0: skel.uvs.get(v).copied().unwrap_or([0.0; 2]),
        }));
        weights.extend((0..n).map(|v| {
            let Some(s) = skel.vert_skin.get(v).filter(|_| blended) else {
                return RIGID;
            };
            let w = s.weights.map(|w| if w > 0.0 { w } else { 0.0 });
            if w.iter().sum::<f32>() <= 0.0 {
                return RIGID;
            }
            let b = s.bones.map(u32::from);
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
    catalog: &render_frontend::assemble::drawsurf::RuntimeMaterialCatalog,
    vertices: &mut Vec<render_frame::SmodelVertex>,
    indices: &mut Vec<u32>,
    offset: (u32, u32),
) -> (Vec<[u32; 4]>, Vec<Option<T6Canopy>>) {
    let surfaces = t6_model_surfaces(item, catalog);
    let rows = t6_surface_rows(&surfaces, offset.1 + indices.len() as u32);
    t6_fill_model(&item.skel, item.posed.as_deref(), &surfaces, vertices, Some(indices), offset.0);
    (rows, surfaces.iter().map(|s| s.canopy).collect())
}

/// BO2MP_T6_LITLOG=1: an animated model whose light reads (near) black,
/// once a second: where it stands. BO2MP_T6_LITLOG=<part of a name>: each
/// model whose name holds it, once per place, however it is lit.
fn t6_note_dark(item: &render_anim::occupancy::missile::T6MissileModel, lit: &lighting_t6::T6PointLight) {
    static ON: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    let Some(on) = ON.get_or_init(|| std::env::var("BO2MP_T6_LITLOG").ok()) else {
        return;
    };
    if on != "1" {
        static SEEN: std::sync::Mutex<Vec<(String, [i32; 3])>> = std::sync::Mutex::new(Vec::new());
        let at = item.light_at.map(|c| c.round() as i32);
        if !item.skel.name.contains(on.as_str()) {
            return;
        }
        if let Ok(mut seen) = SEEN.lock()
            && seen.len() < 256
            && !seen.iter().any(|(n, p)| *p == at && *n == item.skel.name)
        {
            seen.push((item.skel.name.clone(), at));
            diag::info!(
                World,
                "bo2zm model {} at {:?}: light {:?} primary {} vis {:.2} posed {}",
                item.skel.name,
                at,
                lit.light,
                lit.primary,
                lit.visibility,
                item.posed.is_some()
            );
        }
        return;
    }
    if item.posed.is_none() || lit.light.iter().sum::<f32>() >= 0.02 {
        return;
    }
    static LAST: std::sync::Mutex<u64> = std::sync::Mutex::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    if let Ok(mut last) = LAST.lock()
        && *last != now
    {
        *last = now;
        diag::info!(
            World,
            "bo2zm dark model {} at {:?}: light {:?} primary {} vis {:.2}",
            item.skel.name,
            item.light_at.map(|v| v.round()),
            lit.light,
            lit.primary,
            lit.visibility
        );
    }
}

/// bo2zm: Black Ops II things that move, every frame: the first-person gun
/// and arms (skinned by the rig into the FPV draw plan), lit by the light
/// grid where they are; and every T6 material's colour slot and draw code.
pub fn extract_t6_dynamic(
    mut commands: Commands,
    fpv: Extract<Option<Res<render_anim::FpvDrawPlan>>>,
    t6_fx: Extract<Option<Res<render_frontend::adapters::fx::system::T6FxMesh>>>,
    t6_marks: Extract<Option<Res<render_frontend::adapters::fx::system::T6MarkMesh>>>,
    t6_glass: Extract<Option<Res<render_frontend::assemble::drawsurf::T6GlassMesh>>>,
    t6_missiles: Extract<Option<Res<render_anim::occupancy::missile::T6MissileModels>>>,
    t6_script_models: Extract<Option<Res<render_anim::occupancy::missile::T6ScriptModels>>>,
    t6_bodies: Extract<Option<Res<render_anim::occupancy::t6_body::T6BodyModels>>>,
    t6_lights: Extract<Option<Res<render_frontend::adapters::fx::system::T6DynLights>>>,
    runtime: Extract<Option<Res<MaterialGeneration>>>,
    exposure: Extract<Option<Res<render_frontend::assemble::drawsurf::MapT5SunParseExposure>>>,
    scene_view: Extract<Option<Res<render_scene::PreparedSceneView>>>,
    launch: Extract<Option<Res<frame::LaunchReport>>>,
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
                materials: Arc::clone(&existing.materials),
                ..Default::default()
            }
        }
        None => render_gpu::ExtractedT6Dynamic::default(),
    };
    // bo2mp: the map's wind, which sways the tree canopies.
    out.wind = asset_core::t6_map_wind(launch.as_ref().map_or("", |l| l.zone.as_str()));
    if out.generation != catalog.generation_id || out.materials.is_empty() {
        let materials: Vec<(u32, u32)> = catalog
            .materials
            .iter()
            .filter_map(|m| {
                let draw = m.t6_draw?;
                let slot = m.texture_semantic(asset_material::TS_COLOR_MAP)?.0;
                Some((slot, draw.code()))
            })
            .collect();
        out.materials = Arc::new(materials);
        out.generation = catalog.generation_id;
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
                plan.drawgun,
                plan.fallback_rows().map_or(0, |r| r.rows.len()),
                plan.fallback_rows().map_or(0, |r| r.draws.len()),
                plan.fallback_rows().map_or(0, |r| r.draws.iter().filter(|d| d.2.is_some()).count()),
            ),
        };
        if let Ok(mut last) = LAST.lock()
            && last.as_deref() != Some(line.as_str())
        {
            diag::info!(Fpv, "bo2zm viewmodel extract: {line}");
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
                && plan.drawgun != Some(0)
                && !out.materials.is_empty()
        })
        .and_then(|plan| {
            let rows = plan.fallback_rows()?;
            let vertices = rows.rows.iter().map(t6_packed_to_vertex).collect();
            let draws = rows
                .draws
                .iter()
                .filter_map(|&(start, count, material)| {
                    let m = catalog.derived(assets::MaterialIndex::from_order(material? as usize))?;
                    let draw = m.t6_draw?;
                    let slot_of = |semantic| m.texture_semantic(semantic).map_or(u32::MAX, |id| id.0);
                    Some([
                        start,
                        count,
                        slot_of(asset_material::TS_COLOR_MAP),
                        draw.code(),
                        0,
                        slot_of(asset_material::TS_NORMAL_MAP),
                        slot_of(asset_material::TS_SPECULAR_MAP),
                    ])
                })
                .collect();
            let eye = plan.world_from_local.w_axis.truncate().to_array();
            let lit = lighting_t6::t6_light_sampler()
                .map(|sampler| sampler.at(eye))
                .unwrap_or_default();
            let mut instance = [0.0f32; 28];
            instance[..16].copy_from_slice(&plan.world_from_local.to_cols_array());
            for c in 0..3 {
                instance[16 + c] = lit.light[c] * exposure;
            }
            instance[20] = f32::from(lit.primary);
            instance[21] = lit.visibility;
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
        .map(|(sampler, plan)| sampler.at(plan.world_from_local.w_axis.truncate().to_array()))
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
            .filter_map(|&(start, count, material)| {
                let m = catalog.derived(assets::MaterialIndex::from_order(material as usize))?;
                let draw = m.t6_draw?;
                let slot = m
                    .texture_semantic(asset_material::TS_COLOR_MAP)
                    .map_or(u32::MAX, |id| id.0);
                if slot == u32::MAX {
                    t6_note_untextured(material, &m.name);
                }
                let feather = m
                    .constants
                    .iter()
                    .find(|(hash, _)| *hash == T6_FEATHER_PARMS)
                    .map_or(0.0, |(_, bits)| f32::from_bits(bits[0]));
                let eye_offset = m
                    .constants
                    .iter()
                    .find(|(hash, _)| *hash == T6_EYE_OFFSET_PARMS)
                    .map_or(0.0, |(_, bits)| f32::from_bits(bits[0]));
                let constant = |want: u32| {
                    m.constants
                        .iter()
                        .find(|(hash, _)| *hash == want)
                        .map(|(_, bits)| bits.map(f32::from_bits))
                };
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
                        "bo2zm fx draw {}: first {start} count {count} slot {slot} code {:#x}",
                        m.name,
                        draw.code()
                    );
                }
                let mut instance = [0.0f32; 28];
                instance[..16].copy_from_slice(&Mat4::IDENTITY.to_cols_array());
                for c in 0..3 {
                    instance[16 + c] = eye_light.light[c] * exposure;
                }
                // A falloff material: slots 16..18 carry falloffParms.z, .w
                // and the falloff end colour (its begin colour is white in
                // every one), slot 19 says so (the effect shaders read no
                // grid light).
                if let Some(f) = falloff {
                    instance[16..19].copy_from_slice(&f);
                    instance[19] = 1.0;
                }
                instance[20] = f32::from(eye_light.primary);
                instance[21] = eye_light.visibility;
                instance[22] = feather;
                instance[23] = eye_offset;
                instances.push(instance);
                Some([start, count, slot, draw.code(), instances.len() as u32 - 1, u32::MAX, u32::MAX])
            })
            .collect();
        // Particle clouds: the box's placement, then (slots 16..19) the
        // colour, halved as BO2's cloud shaders halve it, and (20, 21) the
        // particle's width and height.
        for cloud in clouds {
            let Some(m) = catalog.derived(assets::MaterialIndex::from_order(cloud.material as usize)) else {
                continue;
            };
            let Some(draw) = m.t6_draw.filter(|d| d.cloud) else {
                continue;
            };
            let slot = m
                .texture_semantic(asset_material::TS_COLOR_MAP)
                .map_or(u32::MAX, |id| id.0);
            let mut instance = [0.0f32; 28];
            instance[..16].copy_from_slice(&cloud.world_from_local);
            for c in 0..3 {
                instance[16 + c] = cloud.color[c] * 0.5;
            }
            instance[19] = cloud.color[3];
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
    .filter(|(_, _, draws)| !draws.is_empty())
    .collect();
    out.marks = (!lists.is_empty()).then(|| {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        let mut listed = Vec::new();
        for (verts, idx, draws) in lists {
            let (base, first) = (vertices.len() as u32, indices.len() as u32);
            vertices.extend_from_slice(verts);
            indices.extend(idx.iter().map(|&i| i + base));
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
                let m = catalog.derived(assets::MaterialIndex::from_order(material as usize))?;
                let draw = m.t6_draw?;
                let slot = m
                    .texture_semantic(asset_material::TS_COLOR_MAP)
                    .map_or(u32::MAX, |id| id.0);
                let mut row = [0.0f32; 28];
                row[..16].copy_from_slice(&Mat4::IDENTITY.to_cols_array());
                if let Some((primary, visibility)) = lightmap {
                    // bo2zm fix list 2: lit by its wall's lightmap; the
                    // vertex colours hold it (w > 0 = baked light).
                    row[19] = exposure.max(0.000001);
                    row[20] = f32::from(primary);
                    row[21] = visibility;
                } else {
                    let lit = sampler.as_ref().map(|s| s.at(at)).unwrap_or_default();
                    for c in 0..3 {
                        row[16 + c] = lit.light[c] * exposure;
                    }
                    row[20] = f32::from(lit.primary);
                    row[21] = lit.visibility;
                }
                // bo2mp: a mark lit like the one before it shares its row
                // (and, its indices following on, its draw: ~256 a frame).
                if instances.last() != Some(&row) {
                    instances.push(row);
                }
                Some([start, count, slot, draw.code(), instances.len() as u32 - 1, u32::MAX, u32::MAX])
            })
            .collect();
        render_gpu::T6DynamicDraw {
            vertices,
            indices,
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
        let generation = catalog.generation_id.0;
        if cache.generation != Some(generation) {
            *cache = T6StaticCache {
                generation: Some(generation),
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
                    generation: cache.generation,
                    epoch: cache.epoch + 1,
                    frame,
                    posed_before: std::mem::take(&mut cache.posed_before),
                    ..Default::default()
                };
            }
        }
        let mut posed_now = std::collections::HashMap::new();
        let mut new_vertices: Vec<render_frame::SmodelVertex> = Vec::new();
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
            let key = (item.skel.name.clone(), item.materials.clone(), item.hide, pose);
            let kept = cache.models.entry(key).or_insert_with(|| {
                // Where this frame's new chunk starts; the append counts on
                // from what the chunk holds already.
                let offset = (cache.vertex_count, cache.index_count);
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
            let mut row = [0.0f32; 28];
            row[..16].copy_from_slice(&item.world_from_local.to_cols_array());
            for c in 0..3 {
                row[16 + c] = lit.light[c] * exposure;
            }
            row[20] = f32::from(lit.primary);
            row[21] = lit.visibility;
            instances.push(row);
            let instance = instances.len() as u32 - 1;
            let mut canopy_rows = Vec::new();
            for (s, &canopy) in kept.surfaces.iter().zip(&kept.canopy) {
                let at = t6_canopy_row(&mut instances, &mut canopy_rows, instance, canopy);
                draws.push([s[0], s[1], s[2], s[3], at, u32::MAX, u32::MAX]);
            }
        }
        cache.posed_before = posed_now;
        if !new_vertices.is_empty() {
            cache.vertex_count += new_vertices.len() as u32;
            cache.index_count += new_indices.len() as u32;
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
        let mut draws = Vec::new();
        let mut instances = Vec::with_capacity(moving.len());
        let mut models = Vec::with_capacity(moving.len());
        let (mut vertex_count, mut index_count) = (0usize, 0usize);
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
                    let shape = *shape_of.entry(Arc::as_ptr(&item.skel) as usize).or_insert_with(|| {
                        skinned.shapes.push(t6_skin_shape(&item.skel));
                        skinned.shapes.len() as u32 - 1
                    });
                    let bones = (skinned.bones.len() as u32, posed.bones.len() as u32);
                    skinned.bones.extend(posed.bones.iter().map(|m| m.to_cols_array()));
                    for s in &plan {
                        skinned.runs.push([shape, s.vbase as u32, s.vn as u32, bones.0, bones.1]);
                        skinned.vertices += s.vn;
                    }
                }
                None => vertex_count += plan.iter().map(|s| s.vn).sum::<usize>(),
            }
            index_count += plan.iter().map(T6Surface::index_count).sum::<usize>();
            {
                use std::hash::Hash;
                (Arc::as_ptr(&item.skel) as usize, on_card.is_some()).hash(&mut key);
                for s in &plan {
                    (s.vbase, s.vn).hash(&mut key);
                    match &s.triangles {
                        T6Triangles::All(first, count) => (0u8, first, count).hash(&mut key),
                        T6Triangles::Left(left) => (1u8, left).hash(&mut key),
                    }
                }
            }
            let canopies: Vec<Option<T6Canopy>> = plan.iter().map(|s| s.canopy).collect();
            models.push((item.skel.clone(), item.posed.clone(), plan, on_card.is_some()));
            let mut row = [0.0f32; 28];
            row[..16].copy_from_slice(&item.world_from_local.to_cols_array());
            for c in 0..3 {
                row[16 + c] = lit.light[c] * exposure;
            }
            row[20] = f32::from(lit.primary);
            row[21] = lit.visibility;
            instances.push(row);
            let instance = instances.len() as u32 - 1;
            let mut canopy_rows = Vec::new();
            for (s, canopy) in surfaces.into_iter().zip(canopies) {
                let at = t6_canopy_row(&mut instances, &mut canopy_rows, instance, canopy);
                draws.push([s[0], s[1], s[2], s[3], at, u32::MAX, u32::MAX]);
            }
        }
        // The card-skinned models' vertices land after the copied ones;
        // their indices count from there.
        let copied = vertex_count as u32;
        let indices_key = {
            use std::hash::{Hash, Hasher};
            copied.hash(&mut key);
            key.finish().max(1)
        };
        let fill = move |vertices: &mut Vec<render_frame::SmodelVertex>, mut indices: Option<&mut Vec<u32>>| {
            let mut on_card = copied;
            for (skel, posed, plan, card) in &models {
                if !*card {
                    t6_fill_model(skel, posed.as_deref(), plan, vertices, indices.as_deref_mut(), 0);
                    continue;
                }
                let Some(indices) = indices.as_deref_mut() else {
                    continue;
                };
                for s in plan {
                    t6_fill_indices(skel, s, on_card, indices);
                    on_card += s.vn as u32;
                }
            }
        };
        render_gpu::T6DynamicDraw {
            vertices: Vec::new(),
            indices: Vec::new(),
            draws,
            instances,
            deferred: Some(render_gpu::T6DeferredFill {
                vertices: vertex_count,
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
                    "bo2zm t6 extract: {:.2} ms a frame, {} moving vertices a frame ({} skinned on the card); \
                     now {} moving models, \
                     {} held still, {} out of view ({} vertices kept in {} chunks), {} bodies and ground \
                     items in all; vertices/draws: moving {:?}, marks {:?}, effects {:?}",
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
    frame.clip = view.clip;
    frame.world_vertices = std::mem::take(&mut view.world_vertices);
    frame.world_indices = std::mem::take(&mut view.world_indices);
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
