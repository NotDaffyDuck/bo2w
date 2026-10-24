//! Black Ops II decal marks (bullet holes, blood) for the fallback pass, and the filter that picks
//! which Black Ops II effect materials draw at all.
//!
//! A mark's material has no technique, so the engine's own mark plan would refuse it; the marks of
//! a frame are taken out of the mark mesh here first.

use bevy::prelude::*;
use fx::FxSystemHost;
use render_frame::SmodelVertex;

use crate::assemble::drawsurf::MaterialGeneration;

/// The most corners the light cache holds before it starts over: more than any match leaves
/// marks for.
const MAX_CACHED_CORNERS: usize = 1 << 16;

/// A frame that spends longer than this (ms) lighting marks from the lightmap logs a warning.
const SLOW_LIGHTING_MS: f64 = 2.0;

/// A mark draw: first index, index count, catalog material, where the mark landed, and the
/// primary light with its baked visibility when the mark sits on a lightmapped surface.
pub type MarkDraw = (u32, u32, u32, [f32; 3], Option<(u8, f32)>);

/// World-space mark triangles with the mark's own texcoords.
///
/// A mark on a lightmapped wall is lit as the wall is (the mark shaders read the wall's
/// lightmap): its vertex colours then hold that light, and the draw's last field names the
/// primary light and its baked visibility, times the tint. `None` means the light grid where the
/// mark landed does the lighting.
#[derive(Resource, Clone, Debug, Default)]
pub struct T6MarkMesh {
    pub vertices: Vec<SmodelVertex>,
    pub indices: Vec<u32>,
    pub draws: Vec<MarkDraw>,
}

impl T6MarkMesh {
    fn clear(&mut self) {
        self.vertices.clear();
        self.indices.clear();
        self.draws.clear();
    }
}

/// Whether the catalog material is a Black Ops II one, which draws in the fallback pass.
pub(super) fn t6_fx_material(runtime: &MaterialGeneration, asset_id: usize) -> bool {
    runtime
        .catalog
        .derived(assets::MaterialIndex::from_order(asset_id))
        .is_some_and(|material| material.t6_draw.is_some())
}

/// Which effect materials to hide, by a word in their name: `BO2MP_T6_FX_ONLY=a,b` keeps only
/// the materials whose name has one of the words, `BO2MP_T6_FX_SKIP=a,b` hides those that do.
#[derive(Debug, Default, PartialEq, Eq)]
struct MaterialFilter {
    only: Vec<String>,
    skip: Vec<String>,
}

impl MaterialFilter {
    fn parse(only: &str, skip: &str) -> Self {
        let words = |list: &str| {
            list.split(',')
                .filter(|word| !word.is_empty())
                .map(str::to_owned)
                .collect()
        };
        Self {
            only: words(only),
            skip: words(skip),
        }
    }

    fn from_env() -> Self {
        let var = |name| std::env::var(name).unwrap_or_default();
        Self::parse(&var("BO2MP_T6_FX_ONLY"), &var("BO2MP_T6_FX_SKIP"))
    }

    fn is_empty(&self) -> bool {
        self.only.is_empty() && self.skip.is_empty()
    }

    fn hides(&self, material_name: &str) -> bool {
        let named = |words: &[String]| words.iter().any(|word| material_name.contains(word.as_str()));
        (!self.only.is_empty() && !named(&self.only)) || named(&self.skip)
    }
}

/// Whether the test-aid filter hides this effect material.
pub(super) fn t6_fx_filtered_out(runtime: &MaterialGeneration, asset_id: usize) -> bool {
    static FILTER: std::sync::OnceLock<MaterialFilter> = std::sync::OnceLock::new();
    let filter = FILTER.get_or_init(MaterialFilter::from_env);
    if filter.is_empty() {
        return false;
    }
    runtime
        .catalog
        .derived(assets::MaterialIndex::from_order(asset_id))
        .is_some_and(|material| filter.hides(&material.name))
}

/// The lightmap light at a mark corner, by lightmap, coordinate and normal (their exact bits),
/// for the lightmaps of one generation. Every mark is rebuilt each frame but a corner's light
/// never changes, so each is looked up once: 268 marks cost 2-4 ms a frame read straight from
/// the lightmap.
#[derive(Default)]
struct CornerLightCache {
    generation: u64,
    lights: std::collections::HashMap<(u8, [u32; 2], [u32; 3]), Option<([f32; 3], f32)>>,
}

impl CornerLightCache {
    /// Starts over when the lightmaps changed or the cache has grown too large.
    fn sync(&mut self, generation: u64) {
        if self.generation != generation || self.lights.len() > MAX_CACHED_CORNERS {
            self.generation = generation;
            self.lights.clear();
        }
    }

    fn light_with(
        &mut self,
        lmap: u8,
        uv: [f32; 2],
        normal: [f32; 3],
        lookup: impl FnOnce() -> Option<([f32; 3], f32)>,
    ) -> Option<([f32; 3], f32)> {
        *self
            .lights
            .entry((lmap, uv.map(f32::to_bits), normal.map(f32::to_bits)))
            .or_insert_with(lookup)
    }
}

static CORNER_LIGHTS: std::sync::LazyLock<std::sync::Mutex<CornerLightCache>> =
    std::sync::LazyLock::new(Default::default);

/// One mark corner's lightmap light: the vertex it colours, the light, and its baked visibility.
type CornerLight = (usize, [f32; 3], f32);

/// Recolours the lit corners as the tint (taken from the first) times the square root of the
/// light over 32, the form the prop shader bakes light in. Returns the mean baked visibility
/// times the mean squared tint, or `None` when no corner was lit.
fn relight_corners(vertices: &mut [SmodelVertex], lit: &[CornerLight]) -> Option<f32> {
    let &(first, _, _) = lit.first()?;
    let [r, g, b, _] = vertices[first].color;
    let tint = [r, g, b];
    for &(at, light, _) in lit {
        let color = &mut vertices[at].color;
        for channel in 0..3 {
            color[channel] = tint[channel] * (light[channel] / 32.0).clamp(0.0, 1.0).sqrt();
        }
    }
    let visibility = lit.iter().map(|corner| corner.2).sum::<f32>() / lit.len() as f32;
    let tint_squared = tint.iter().map(|c| c * c).sum::<f32>() / 3.0;
    Some(visibility * tint_squared)
}

/// A mark on a lightmapped world surface takes that surface's lightmap light at each corner, as
/// the game's world mark shaders do (the light grid in front of a wall can be far darker than the
/// wall, which turns holes into black blotches). Returns the surface's primary light and the mark's
/// baked visibility, or `None` to keep the light grid: the surface has no lightmap, or one of the
/// mark's corners has no sample.
fn light_from_lightmap(
    surf: &fx::GfxMarkMeshSurf,
    indices: &[u16],
    points: &[marks_t6::FxMarkStagingPoint],
    vertices: &mut [SmodelVertex],
    cache: &mut CornerLightCache,
) -> Option<(u8, f32)> {
    let lmap = surf.context.lmap_index;
    if lmap == marks_t6::GFX_SURFACE_LIGHTMAP_NONE {
        return None;
    }
    let mut corners = indices.to_vec();
    corners.sort_unstable();
    corners.dedup();

    let mut lit = Vec::with_capacity(corners.len());
    for corner in corners {
        let at = usize::from(corner);
        let (Some(point), true) = (points.get(at), at < vertices.len()) else {
            continue;
        };
        let (light, visibility) = cache.light_with(lmap, point.lmap_coord, point.normal, || {
            lighting_t6::t6_lightmap_light(lmap, point.lmap_coord, point.normal)
        })?;
        lit.push((at, light, visibility));
    }
    let baked = relight_corners(vertices, &lit)?;
    Some((surf.context.primary_light_index, baked))
}

/// The vertices of the whole mark mesh, converted from the packed Black Ops II rows.
fn mark_vertices(mesh: &fx::GfxMarkMeshCensus) -> Vec<SmodelVertex> {
    mesh.packed
        .iter()
        .zip(&mesh.verts)
        .map(|(row, point)| {
            let packed = asset_t6::T6PackedVertex::read(row).expect("a whole BO2 mark row");
            SmodelVertex {
                position: point.xyz,
                normal: point.normal,
                color: packed.color_unorm(),
                uv0: packed.uv(),
            }
        })
        .collect()
}

/// Moves the frame's Black Ops II marks out of the mark mesh into `out`, before the engine's own
/// mark plan sees it.
pub(super) fn take_t6_marks(
    host: &FxSystemHost,
    mesh: Option<&fx::GfxMarkMeshCensus>,
    runtime: &MaterialGeneration,
    out: &mut T6MarkMesh,
) {
    out.clear();
    let Some(mesh) = mesh else {
        return;
    };
    let mut cache = CORNER_LIGHTS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    cache.sync(lighting_t6::t6_lightmap_generation());

    let mut lighting_ms = 0.0;
    for surf in mesh.surfs.iter().filter(|surf| surf.index_count != 0) {
        let Some(name) = host.marks.material_name(surf.mark_slot) else {
            continue;
        };
        let Some(material) = runtime
            .catalog
            .material_for_name(asset_game::material_bind_name(name))
            .filter(|material| material.t6_draw.is_some())
        else {
            continue;
        };
        let first = surf.index_start as usize;
        let Some(indices) = mesh
            .indices
            .get(first..first.saturating_add(surf.index_count as usize))
        else {
            continue;
        };
        if out.vertices.is_empty() {
            out.vertices = mark_vertices(mesh);
        }
        let start = out.indices.len() as u32;
        out.indices.extend(indices.iter().map(|&i| u32::from(i)));
        let landed = indices
            .first()
            .and_then(|&i| mesh.verts.get(usize::from(i)))
            .map_or([0.0; 3], |point| point.xyz);

        let began = std::time::Instant::now();
        let lit = light_from_lightmap(surf, indices, &mesh.verts, &mut out.vertices, &mut cache);
        lighting_ms += began.elapsed().as_secs_f64() * 1000.0;

        out.draws.push((
            start,
            surf.index_count,
            u32::from(material.asset_id.0),
            landed,
            lit,
        ));
    }
    if lighting_ms > SLOW_LIGHTING_MS {
        diag::warn!(
            World,
            "marks: lighting {} marks from the lightmap took {lighting_ms:.1} ms",
            out.draws.len()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corner(color: [f32; 4]) -> SmodelVertex {
        SmodelVertex {
            position: [0.0; 3],
            normal: [0.0, 0.0, 1.0],
            color,
            uv0: [0.0; 2],
        }
    }

    #[test]
    fn an_empty_filter_hides_nothing() {
        let filter = MaterialFilter::parse("", "");
        assert!(filter.is_empty());
        assert!(!filter.hides("anything"));
    }

    #[test]
    fn only_keeps_the_named_materials() {
        let filter = MaterialFilter::parse("smoke,spark", "");
        assert!(!filter.hides("fx_smoke_01"));
        assert!(filter.hides("fx_blood_01"));
    }

    #[test]
    fn skip_hides_the_named_materials_and_ignores_empty_words() {
        let filter = MaterialFilter::parse("", "blood,,");
        assert_eq!(filter.skip, vec!["blood".to_owned()]);
        assert!(filter.hides("fx_blood_01"));
        assert!(!filter.hides("fx_smoke_01"));
    }

    #[test]
    fn skip_wins_over_only() {
        let filter = MaterialFilter::parse("fx_", "blood");
        assert!(filter.hides("fx_blood_01"));
        assert!(!filter.hides("fx_smoke_01"));
    }

    #[test]
    fn a_corner_is_looked_up_once_per_generation() {
        let mut cache = CornerLightCache::default();
        let lookups = std::cell::Cell::new(0);
        let look = |cache: &mut CornerLightCache| {
            cache.light_with(2, [0.5, 0.25], [0.0, 0.0, 1.0], || {
                lookups.set(lookups.get() + 1);
                Some(([8.0; 3], 0.5))
            })
        };
        assert_eq!(look(&mut cache), Some(([8.0; 3], 0.5)));
        assert_eq!(look(&mut cache), Some(([8.0; 3], 0.5)));
        assert_eq!(lookups.get(), 1);
        cache.sync(0);
        assert_eq!(look(&mut cache), Some(([8.0; 3], 0.5)));
        assert_eq!(lookups.get(), 1);
        cache.sync(1);
        assert_eq!(look(&mut cache), Some(([8.0; 3], 0.5)));
        assert_eq!(lookups.get(), 2);
    }

    #[test]
    fn a_missing_sample_is_remembered_too() {
        let mut cache = CornerLightCache::default();
        assert_eq!(cache.light_with(1, [0.0; 2], [0.0; 3], || None), None);
        let again = cache.light_with(1, [0.0; 2], [0.0; 3], || Some(([1.0; 3], 1.0)));
        assert_eq!(again, None);
    }

    #[test]
    fn corners_take_the_first_tint_times_the_root_of_the_light() {
        let mut vertices = [corner([0.5, 0.5, 0.5, 1.0]), corner([1.0, 1.0, 1.0, 1.0])];
        let lit = [(0, [32.0, 8.0, 0.0], 1.0), (1, [32.0, 32.0, 32.0], 0.5)];
        let baked = relight_corners(&mut vertices, &lit).unwrap();
        assert_eq!(vertices[0].color, [0.5, 0.25, 0.0, 1.0]);
        // The second corner uses the first corner's tint, not its own.
        assert_eq!(vertices[1].color, [0.5, 0.5, 0.5, 1.0]);
        // Mean visibility 0.75 times the mean squared tint 0.25.
        assert!((baked - 0.1875).abs() < 1e-6);
    }

    #[test]
    fn light_above_the_ceiling_is_clamped() {
        let mut vertices = [corner([1.0, 1.0, 1.0, 1.0])];
        relight_corners(&mut vertices, &[(0, [1000.0; 3], 1.0)]).unwrap();
        assert_eq!(vertices[0].color, [1.0, 1.0, 1.0, 1.0]);
    }

    #[test]
    fn no_lit_corner_leaves_the_light_grid() {
        let mut vertices = [corner([1.0; 4])];
        assert_eq!(relight_corners(&mut vertices, &[]), None);
        assert_eq!(vertices[0].color, [1.0; 4]);
    }

    #[test]
    fn clearing_empties_every_list() {
        let mut mesh = T6MarkMesh::default();
        mesh.vertices.push(corner([1.0; 4]));
        mesh.indices.push(0);
        mesh.draws.push((0, 1, 2, [0.0; 3], None));
        mesh.clear();
        assert!(mesh.vertices.is_empty() && mesh.indices.is_empty() && mesh.draws.is_empty());
    }
}
