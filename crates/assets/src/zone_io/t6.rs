//! bo2zm: the Black Ops II lane.
//!
//! A T6 map loads through `asset_t6`: the zone is walked once, capturing the
//! world, collision and map entities; `asset_world` turns them into the
//! engine's world draw and clip collision. Materials have no route yet, so
//! world surfaces draw through the renderer's diagnostic path; the player
//! starts at the map's own `info_player_start` through the start scripts in
//! `t6_scripts`.

use std::collections::BTreeMap;
use std::path::Path;

use super::{SharedAssetTally, ResidentLevel, ShaderUsageSurvey, ZoneChannel};
use crate::load_stages::StagedLevel;
use asset_transport::progress::{LoadCompletion, StageId};
use asset_transport::{T6ZoneMemory, ZoneTextureData};
use asset_world::LevelRenderRules;

pub struct T6Lane;

/// One line per zone: asset count by type, or why the asset list did not open.
pub fn asset_census(path: &Path, image: &ZoneTextureData) -> String {
    let zone = zone_stem(path);
    let header = match image.t6_header() {
        Ok(hdr) => hdr,
        Err(error) => return format!("t6 census {zone}: zone header: {error}"),
    };
    let mut memory = T6ZoneMemory::for_header(&header);
    let mut zone_stream = match memory.stream(&image.bytes) {
        Ok(zstream) => zstream,
        Err(error) => return format!("t6 census {zone}: zone arenas: {error}"),
    };
    let table = match fastfile_t6::read_asset_directory(&mut zone_stream) {
        Ok(table) => table,
        Err(error) => return format!("t6 census {zone}: asset list: {error}"),
    };
    let mut counts: BTreeMap<u32, usize> = BTreeMap::new();
    for i in 0..table.count() {
        match table.raw_kind(&zone_stream, i) {
            Ok(raw) => *counts.entry(raw).or_default() += 1,
            Err(error) => return format!("t6 census {zone}: asset {i}: {error}"),
        }
    }
    let by_type: Vec<String> = counts
        .iter()
        .map(|(raw, n)| match fastfile_t6::XAssetKind::from_u32(*raw) {
            Some(img_ty) => format!("{}={n}", img_ty.name()),
            None => format!("pool{raw:#x}={n}"),
        })
        .collect();
    format!(
        "t6 census {zone}: {} assets, {} script strings: {}",
        table.count(),
        table.strings.count(),
        by_type.join(" ")
    )
}

/// bo2zm fix list 2: the clip map's placed models (`staticModelList`: cars,
/// fences, trees, porches, rocks) that have a model, and how many of those
/// have collision triangles, which bullets and thrown grenades hit (the
/// static world `attach_t6_static_world` files). Returns (placed, with
/// triangles).
fn t6_static_model_census(capture: &asset_t6::ZoneCapture) -> (usize, usize) {
    let props = capture.clip.as_ref().map_or(&[][..], |clip| &clip.prop_set[..]);
    let solid = |model: &&&asset_t6::XModelRef| {
        model.collision_detail_tier >= 0 && model.collision_faces.iter().any(|face| !face.triangle_list.is_empty())
    };
    let models: Vec<_> = props
        .iter()
        .filter_map(|prop| prop.model.and_then(|key| capture.xmodels.get(key.index)))
        .collect();
    (models.len(), models.iter().filter(solid).count())
}

/// The zone's file name without its extension (empty when it has none).
fn zone_stem(path: &Path) -> String {
    path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The map's player start. Zombies maps start players at `initial_spawn`
/// structs whose `script_string` names the mode and location
/// (`zstandard_nuked`); their `info_player_start` can sit inside player
/// clip. Then any `initial_spawn` struct; a multiplayer map's own first
/// free-for-all spawn point (bo2mp: BO2 never starts a player at the
/// `info_player_start`, which Hijacked lacks and Overflow puts in a
/// dumpster); the `info_player_start`; and last the world centre.
fn player_start(
    capture: &asset_t6::ZoneCapture,
    map: &str,
    notes: &mut Vec<String>) -> ([f32; 3], f32) {
    let location = map.strip_prefix("zm_").unwrap_or(map);
    // A chosen start (Tranzit's Town survival: `zstandard_town`) first.
    let chosen = frame::bo2_zm_start(map).map(|(loc, mode)| format!("{mode}_{loc}"));
    let wanted: Vec<String> = chosen
        .into_iter()
        .chain([
            format!("zstandard_{location}"),
            format!("zclassic_{location}"),
        ])
        .collect();
    let ents: Vec<asset_t6::MapEntity> = capture
        .map_ents
        .iter()
        .flat_map(|text| asset_t6::parse_entities(text))
        .collect();
    let initial = || {
        ents.iter()
            .filter(|e| e.get("script_noteworthy") == Some("initial_spawn"))
    };
    let named = wanted.iter().find_map(|w| {
        initial().find(|e| {
            e.get("script_string")
                .is_some_and(|s| s.split_whitespace().any(|t| t == w))
        })
    });
    // bo2mp: a multiplayer map's spawn points by kind (`mp_dm_spawn`,
    // `mp_tdm_spawn`, `mp_tdm_spawn_allies_start`, ...).
    let is_mp_spawn = |e: &&asset_t6::MapEntity| {
        let c = e.classname();
        c.starts_with("mp_") && c.contains("_spawn")
    };
    let mp = super::t6_m2::is_mp_map(map);
    if mp {
        let mut kinds: std::collections::BTreeMap<&str, usize> = Default::default();
        for e in ents.iter().filter(is_mp_spawn) {
            *kinds.entry(e.classname()).or_default() += 1;
        }
        notes.push(format!(
            "t6 mp spawns: {}",
            kinds
                .iter()
                .map(|(k, n)| format!("{k} {n}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let picks = [
        named.map(|e| (e, "initial_spawn for this location")),
        initial().next().map(|e| (e, "initial_spawn")),
        ents.iter()
            .find(|e| e.classname() == "mp_dm_spawn")
            .or_else(|| ents.iter().find(is_mp_spawn))
            .filter(|_| mp)
            .map(|e| (e, e.classname())),
        ents.iter()
            .find(|e| e.classname() == "info_player_start")
            .map(|e| (e, "info_player_start")),
    ];
    for (e, what) in picks.into_iter().flatten() {
        if let Some(origin) = e.vec3("origin") {
            let yaw = e.vec3("angles").map_or(0.0, |a| a[1]);
            notes.push(format!(
                "t6 spawn: {what} at [{:.1}, {:.1}, {:.1}] yaw {yaw:.1}",
                origin[0], origin[1], origin[2]
            ));
            return (origin, yaw);
        }
    }
    let w = capture.world.as_ref();
    let c = w.map_or([0.0; 3], |w| {
        [
            (w.lo_corner[0] + w.hi_corner[0]) * 0.5,
            (w.lo_corner[1] + w.hi_corner[1]) * 0.5,
            w.hi_corner[2],
        ]
    });
    notes.push("t6 spawn gap: no info_player_start; world centre used".to_owned());
    (c, 0.0)
}

/// bo2zm: the props a map zone only names. TranZit's placed models live
/// in its area zones (`zm_transit_gump_busstation.ff`, ...), which the game
/// streams in by area; the map zone holds a comma-named reference to each
/// (the Bus Depot's route-map kiosk, most of the Diner, Farm and Town).
#[derive(Default)]
struct AreaModels {
    captures: Vec<asset_t6::ZoneCapture>,
    /// A referenced model's name -> (capture, its model index).
    model_list: std::collections::HashMap<String, (usize, usize)>,
    /// Per capture: its material index -> catalog row.
    local: Vec<std::collections::HashMap<usize, usize>>,
    /// The map zone's own materials by name (an area material can be a
    /// reference to one of them).
    map_materials: std::collections::HashMap<String, usize>,
}

impl AreaModels {
    /// The area zones beside the map that define the models its static
    /// models reference; none for a map whose props are all its own.
    fn read(path: &Path, map: &str, capture: &asset_t6::ZoneCapture, world: &asset_t6::WorldRef) -> Self {
        let want_set: std::collections::BTreeSet<&str> = world
            .prop_set
            .iter()
            .filter_map(|sm| sm.model)
            .filter_map(|k| capture.xmodels.get(k.index))
            .filter_map(|m| m.name.strip_prefix(','))
            .collect();
        if want_set.is_empty() { return Self::default(); }
        let dir = asset_transport::t6_extra::install_zone_dir(path);
        let prefix = format!("{map}_gump_");
        let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path()).filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(&prefix) && n.ends_with(".ff"))
            })
            .collect();
        files.sort();
        let mut out = Self::default();
        for file in files {
            let Ok(area) = asset_t6::capture_zone(&file) else { continue };
            let ci = out.captures.len();
            for (mi, m) in area.xmodels.iter().enumerate() {
                if want_set.contains(m.name.as_str()) {
                    out.model_list.entry(m.name.clone()).or_insert((ci, mi));
                }
            }
            out.captures.push(area);
        }
        out.map_materials = capture
            .materials
            .iter()
            .enumerate()
            .filter(|(_, m)| !m.name.is_empty() && !m.name.starts_with(','))
            .map(|(i, m)| (m.name.clone(), i))
            .collect();
        out
    }

    /// Each area model surface's material: (capture, material index), the
    /// area's own or, for a reference, the map zone's (capture `None`).
    fn facet_materials(&self) -> impl Iterator<Item = (Option<usize>, usize)> + '_ {
        self.model_list.values().flat_map(move |&(ci, mi)| {
            let c = &self.captures[ci];
            c.xmodels[mi].lod0.iter().filter_map(move |srf| {
                let k = srf.material?;
                let name = &c.materials.get(k.index)?.name;
                match name.strip_prefix(',') {
                    Some(n) => self.map_materials.get(n).map(|&i| (None, i)),
                    None => Some((Some(ci), k.index)),
                }
            })
        })
    }

    /// Link every area model's own materials into the catalog, images by
    /// name from the map zone and the areas.
    fn link(
        &mut self,
        mat_list: &mut asset_material::SurfaceShaderRoster,
        capture: &asset_t6::ZoneCapture,
        packs: Option<&asset_t6::PackSet>,
        load_log: &mut Vec<String>,
    ) {
        let mut img_by_name: std::collections::HashMap<String, asset_t6::ImageRef> = Default::default();
        for img in capture.images.iter().chain(self.captures.iter().flat_map(|c| c.images.iter())) {
            if !img.name.is_empty() && !img.name.starts_with(',') {
                img_by_name.entry(img.name.clone()).or_insert_with(|| img.clone());
            }
        }
        let mut wanted: Vec<std::collections::BTreeSet<usize>> = vec![Default::default(); self.captures.len()];
        for (ci, mi) in self.facet_materials() {
            if let Some(ci) = ci {
                wanted[ci].insert(mi);
            }
        }
        self.local = self
            .captures
            .iter()
            .zip(&wanted)
            .map(|(c, want)| {
                let linked = super::t6_materials::link_materials_with(
                    mat_list,
                    c,
                    packs,
                    want.iter().copied(),
                    &c.zone,
                    Some(&img_by_name),
                );
                load_log.extend(linked.load_log.into_iter().map(|l| format!("t6 area ({}): {l}", c.zone)));
                linked.local
            })
            .collect();
        load_log.push(format!(
            "t6 area zones: {} read, {} referenced props found",
            self.captures.len(), self.model_list.len()
        ));
    }

    /// The real model for a map model, with its surfaces' catalog rows.
    fn mdl<'a>(
        &'a self,
        local_map: &'a asset_t6::XModelRef,
        map_local: &'a std::collections::HashMap<usize, usize>,
    ) -> (&'a asset_t6::XModelRef, Box<dyn Fn(asset_t6::AssetIdentity) -> Option<usize> + 'a>) {
        let Some(&(ci, mi)) = local_map.name.strip_prefix(',').and_then(|n| self.model_list.get(n)) else {
            return (local_map, Box::new(move |k| map_local.get(&k.index).copied()));
        };
        let c = &self.captures[ci];
        let local = self.local.get(ci);
        (
            &c.xmodels[mi],
            Box::new(move |k| {
                let name = &c.materials.get(k.index)?.name;
                match name.strip_prefix(',') {
                    Some(n) => map_local.get(self.map_materials.get(n)?).copied(),
                    None => local?.get(&k.index).copied(),
                }
            }),
        )
    }
}

/// The placed static models: one mesh per model used (its first LOD), a
/// mesh of its own for each instance with baked vertex light, and every
/// placement with its light. bo2mp: as the game lights them, each placement
/// takes the light grid coefficient set the map's compiler stored for it
/// (`colorsIndex`): the 56 directional colours of its model lighting block
/// and the set's SH constants (`lighting_t6::T6ModelLight`), installed as
/// `lighting_t6::t6_model_lights`.
fn build_props(
    capture: &asset_t6::ZoneCapture,
    world: &asset_t6::WorldRef,
    local_materials: &std::collections::HashMap<usize, usize>,
    areas: &AreaModels,
    solids: Option<&asset_world::CollisionClipData>,
    load_log: &mut Vec<String>,
) -> (
    Vec<asset_world::StaticMeshData>,
    Vec<Option<asset_world::FixedPropPlacement>>,
) {
    use bevy::prelude::{Mat3, Quat, Transform, Vec3};

    let grid = world
        .probe_grid
        .as_ref()
        .and_then(lighting_t6::t6_light_grid);
    // Places outside the light grid (the distant terrain) take their light
    // from a sky grid volume: one coefficient set, one primary light and
    // its visibility.
    let raw_grid = world.probe_grid.as_ref();
    let in_grid = |p: [f32; 3]| -> bool {
        let Some(g) = raw_grid else { return false };
        let cell = [
            (p[0].floor() as i32).wrapping_add(0x20000) >> 5,
            (p[1].floor() as i32).wrapping_add(0x20000) >> 5,
            (p[2].floor() as i32).wrapping_add(0x20000) >> 6,
        ];
        (0..3).all(|a| cell[a] >= i32::from(g.lo_corner[a]) && cell[a] <= i32::from(g.hi_corner[a]))
    };
    let sky_volume = |p: [f32; 3]| {
        raw_grid.and_then(|g| {
            g.sky_volumes
                .iter()
                .find(|v| (0..3).all(|a| p[a] >= v.lo_corner[a] && p[a] <= v.hi_corner[a]))
        })
    };
    // A coefficient set as the game decodes it.
    let coeffs_at = |index: u16| -> Option<[[f32; 3]; 9]> {
        let at = usize::from(index) * lighting_t6::T6_COEFF_SET_BYTES;
        lighting_t6::t6_grid_coeffs(raw_grid?.coeffs.get(at..)?)
    };
    // The mean of a set's 56 directional colours (linear, before exposure).
    let cube_mean = |c: &[[f32; 3]; 9]| -> [f32; 3] {
        let cube = lighting_t6::t6_grid_cube(c);
        [0, 1, 2].map(|ch| cube.iter().map(|rgb| rgb[ch]).sum::<f32>() / 56.0)
    };
    let sky_light = |v: &asset_t6::SkyGridVolumeRef| coeffs_at(v.vertex_color_slot).map(|c| cube_mean(&c));
    let mut sky_n = 0usize;
    let mut meshes: Vec<asset_world::StaticMeshData> = Vec::new();
    let mut shared: std::collections::HashMap<usize, Option<usize>> = Default::default();
    // The baked light's mean against the mean of the placement's own set:
    // the same light, so near 1 when the set is decoded right.
    let mut ratios: Vec<f32> = Vec::new();
    let mut light_rows: Vec<lighting_t6::T6ModelLight> = Vec::new();
    let mut light_of: std::collections::HashMap<u16, u32> = Default::default();
    let mut pending: Vec<(usize, usize, Option<[f32; 3]>, bool, u32)> = Vec::new();
    // The materials whose technique reads no baked light (decals, screens).
    let unlit: std::collections::HashSet<usize> = local_materials
        .iter()
        .filter(|(m, _)| {
            capture.materials[**m].method_set.is_some_and(|t| {
                asset_core::T6Draw::from_technique_set(&capture.technique_sets[t.index].name, 0, None)
                    .unlit
            })
        })
        .map(|(_, local)| *local)
        .collect();
    // bo2mp: the lit tree canopies, whose baked light keeps its alpha.
    let foliage: std::collections::HashSet<usize> = local_materials
        .iter()
        .filter(|(m, _)| {
            capture.materials[**m].method_set.is_some_and(|t| {
                let draw =
                    asset_core::T6Draw::from_technique_set(&capture.technique_sets[t.index].name, 0, None);
                draw.foliage && !draw.unlit
            })
        })
        .map(|(_, local)| *local)
        .collect();
    for (i, sm) in world.prop_set.iter().enumerate() {
        let Some(key) = sm.model else { continue };
        // A referenced model (TranZit's) draws from its area zone.
        let (xm, material) = areas.mdl(&capture.xmodels[key.index], local_materials);
        let base = *shared.entry(key.index).or_insert_with(|| {
            let mesh = asset_world::build_t6_model_mesh(xm, |k| material(k))?;
            meshes.push(mesh);
            Some(meshes.len() - 1)
        });
        let Some(base) = base else { continue };
        let xm = &capture.xmodels[key.index];
        // The placement's own set (the game fills the model's lighting
        // block from it).
        let own = coeffs_at(sm.vertex_color_slot);
        let handle = own.map_or(0, |c| {
            *light_of.entry(sm.vertex_color_slot).or_insert_with(|| {
                light_rows.push(lighting_t6::T6ModelLight {
                    cube: lighting_t6::t6_grid_cube(&c).map(|rgb| rgb.map(lighting_t6::t6_light_byte)),
                    sh: lighting_t6::t6_grid_sh(&c),
                });
                light_rows.len() as u32
            })
        });
        // Without a set (no grid): the grid at the model's middle, or the
        // sky volume holding it.
        let grid_light = own.map(|c| cube_mean(&c)).or_else(|| {
            let mid_pt: [f32; 3] = std::array::from_fn(|dim| (xm.lo_corner[dim] + xm.hi_corner[dim]) * 0.5 * sm.scale);
            let light_at: [f32; 3] = std::array::from_fn(|dim| {
                sm.origin[dim] + mid_pt[0] * sm.axis[0][dim] + mid_pt[1] * sm.axis[1][dim] + mid_pt[2] * sm.axis[2][dim]
            });
            if in_grid(light_at) {
                grid.as_ref().and_then(|g| lighting_t6::t6_grid_light(g, light_at))
            } else {
                sky_volume(light_at).and_then(sky_light)
            }
        });
        // BO2MP_T6_SMLIGHT_LOG=<name part>: such placements' own set (test aid).
        if let Ok(want) = std::env::var("BO2MP_T6_SMLIGHT_LOG")
            && xm.name.contains(&want)
        {
            diag::info!(
                World,
                "bo2zm t6 smlight {} at {:?}: own set {} mean {:?} sh {:?}",
                xm.name,
                sm.origin,
                sm.vertex_color_slot,
                grid_light,
                own.map(|c| lighting_t6::t6_grid_sh(&c))
            );
        }
        let baked = (!sm.lmap_colors.is_empty())
            .then(|| {
                asset_world::t6_vertex_lit_mesh(
                    &meshes[base],
                    &sm.lmap_colors,
                    |m| m.is_some_and(|m| unlit.contains(&m)),
                    |m| m.is_some_and(|m| foliage.contains(&m)),
                )
            })
            .flatten();
        let (mesh, vertex_lit) = match baked {
            Some(lit) => {
                if let Some(g) = own.map(|c| cube_mean(&c)) {
                    let mean: f32 = sm
                        .lmap_colors
                        .iter()
                        .map(|c| {
                            let b = c.to_le_bytes();
                            (0..3)
                                .map(|k| (f32::from(b[k]) / 255.0).powi(2) * 32.0)
                                .sum::<f32>()
                                / 3.0
                        })
                        .sum::<f32>()
                        / sm.lmap_colors.len() as f32;
                    let grid_mean = (g[0] + g[1] + g[2]) / 3.0;
                    if grid_mean > 1e-4 {
                        ratios.push(mean / grid_mean);
                    }
                }
                meshes.push(lit);
                (meshes.len() - 1, true)
            }
            None => (base, false),
        };
        pending.push((i, mesh, grid_light, vertex_lit, handle));
    }
    ratios.sort_by(f32::total_cmp);
    let at = |q: f32| {
        ratios
            .get(((ratios.len() as f32 - 1.0) * q).round() as usize)
            .copied().unwrap_or(0.0)
    };
    load_log.push(format!(
        "t6 props: baked light against the placement's own grid set, {} props: 10% {:.2}, 25% {:.2}, median {:.2}, 75% {:.2}, 90% {:.2}",
        ratios.len(),
        at(0.1),
        at(0.25),
        at(0.5),
        at(0.75),
        at(0.9)
    ));
    let mut placements = vec![None; world.prop_set.len()];
    let (mut gridded, mut baked_n, mut unlit) = (0usize, 0usize, 0usize);
    let mut seen_total = 0usize;
    let mut vis_diff = [0usize; 2];
    for (i, mesh, grid_light, vertex_lit, handle) in pending {
        let sm = &world.prop_set[i];
        let compact_illum = if vertex_lit {
            baked_n += 1;
            Some([0, 0, 0, 255])
        } else if let Some(light) = grid_light {
            gridded += 1;
            Some([
                lighting_t6::t6_light_byte(light[0]),
                lighting_t6::t6_light_byte(light[1]),
                lighting_t6::t6_light_byte(light[2]),
                0,
            ])
        } else {
            unlit += 1;
            None
        };
        // The primary light the game lights this placement with, and how
        // much of it reaches it: the map's compiler measured that for each
        // placement and stored it with it, and that is what the game reads.
        // (The light grid's visibility at the origin missed it both ways:
        // Raid's trees outside the grid got none, its dinner plates and
        // club ceiling lights indoors got full sun.)
        let primary = sm.key_light_index;
        let visibility = sm.visibility;
        if !in_grid(sm.origin) {
            sky_n += 1;
        }
        if visibility > 0 {
            seen_total += 1;
        }
        // BO2MP_T6_VISLOG=<name part>: the stored visibility against the one
        // the light grid (or outside it, the sky volume) gives (test aid).
        if let Ok(want) = std::env::var("BO2MP_T6_VISLOG") {
            let worked_out = if !in_grid(sm.origin) {
                sky_volume(sm.origin)
                    .filter(|v| v.key_light_index == primary)
                    .map_or(0, |v| v.visibility)
            } else {
                grid.as_ref()
                    .and_then(|g| lighting_t6::t6_grid_visibility(g, sm.origin, primary))
                    .unwrap_or(0)
            };
            let name = sm.model.map_or("", |k| capture.xmodels[k.index].name.as_str());
            vis_diff[usize::from(worked_out.abs_diff(visibility) > 64)] += 1;
            if name.contains(&want) {
                diag::info!(
                    World,
                    "bo2mp t6 vis {name} at ({:.0} {:.0} {:.0}) in grid {} primary {primary} worked out {worked_out} stored {} baked {}",
                    sm.origin[0],
                    sm.origin[1],
                    sm.origin[2],
                    in_grid(sm.origin),
                    sm.visibility,
                    vertex_lit
                );
            }
        }
        // Each axis row is one column of the model's rotation.
        let turn = Quat::from_mat3(&Mat3::from_cols_array_2d(&sm.axis));
        let transform = Transform::from_translation(Vec3::from(sm.origin))
            .with_rotation(turn)
            .with_scale(Vec3::splat(sm.scale));
        placements[i] = Some(asset_world::FixedPropPlacement {
            mesh,
            transform,
            origin: sm.origin,
            axis: sm.axis,
            scale: sm.scale,
            // TODO(cull-dist-f32): BO2's `cullDist` is a float compared
            // linearly with the LOD-ramped distance (facts
            // dpvs-scene-shadow-t6.md); the placement still holds MW2's u16,
            // so fractions drop and distances past 65535 clamp.
            hide_dist: sm.hide_dist.clamp(0.0, f32::from(u16::MAX)) as u16,
            env_probe_index: sm.env_probe_index,
            key_light_index: sm.key_light_index,
            flags: sm.flags as u8,
            compact_illum,
            t6_primary: [primary, visibility],
            t6_light: handle,
        });
    }
    let light_n = light_rows.len();
    lighting_t6::install_t6_model_lights(light_rows);
    // bo2zm M2: what lights moving things (the first-person gun, effects).
    // bo2mp: the grid decoded as the game does needs no scale.
    lighting_t6::install_t6_light_sampler(raw_grid.map(|g| lighting_t6::T6LightSampler {
        grid: grid.clone(),
        low: g.lo_corner,
        high: g.hi_corner,
        sky: g
            .sky_volumes
            .iter()
            .map(|v| {
                (
                    v.lo_corner,
                    v.hi_corner,
                    sky_light(v).unwrap_or_default(),
                    v.key_light_index,
                    v.visibility,
                )
            })
            .collect(),
        scale: 1.0,
        sunlight_primary_flag_index: g.sunlight_lamp_slot as u8,
        sight: solids.map(|c| {
            let c = c.clone();
            std::sync::Arc::new(move |a: [f32; 3], b: [f32; 3]| {
                c.box_sight_clear(a, b, lighting_t6::GRID_LINE_OF_SIGHT_CONTENT_BITS)
            }) as lighting_t6::SightCheck
        }),
        cache: Default::default(),
    }));
    load_log.push(format!(
        "t6 props: {} placed, {} meshes; {baked_n} with baked vertex light, {gridded} lit by the light grid, {unlit} without light; {light_n} own grid lights; {seen_total} see their primary light; {sky_n} outside the grid (sky volume)",
        placements.iter().filter(|p| p.is_some()).count(),
        meshes.len(),
    ));
    if std::env::var_os("BO2MP_T6_VISLOG").is_some() {
        load_log.push(format!(
            "t6 props: visibility worked out against stored: {} within 64, {} further",
            vis_diff[0], vis_diff[1]
        ));
    }
    (meshes, placements)
}

/// The map's fog as its createart script sets it, from the map's zone or
/// its patch zone (`<map>_patch.ff` beside it).
fn art_fog(capture: &asset_t6::ZoneCapture, path: &Path, map: &str) -> Option<asset_t6::ArtFog> {
    let want = format!("script:maps/mp/createart/{map}_art.gsc");
    let find = |c: &asset_t6::ZoneCapture| {
        c.raw_files
            .iter()
            .find(|(name, _)| *name == want)
            .and_then(|(_, bytes)| asset_t6::parse_art_fog(bytes))
    };
    find(capture).or_else(|| {
        let patch = asset_transport::t6_extra::file_in(
            &asset_transport::t6_extra::install_zone_dir(path),
            &format!("{map}_patch.ff"),
        );
        asset_t6::capture_zone(&patch).ok().as_ref().and_then(find)
    })
}

/// bo2mp: the map's final colour: its vision file (`vision/<map>.vision`)
/// and band 0 of its colour table (`GfxWorld::lutMaterial`'s image), built
/// into the table BO2's `hdr_create_lut2dv` makes (`asset_world::t6_grade`).
fn t6_grade(
    capture: &asset_t6::ZoneCapture,
    world: &asset_t6::WorldRef,
    packs: Option<&asset_t6::PackSet>,
    map: &str,
) -> (Option<asset_world::T6Grade>, String) {
    let want = format!("vision/{map}.vision");
    let vision = capture
        .raw_files
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(&want))
        .and_then(|(_, bytes)| asset_world::parse_t6_vision(&String::from_utf8_lossy(bytes)));
    let image = world
        .lut_material
        .and_then(|k| capture.materials.get(k.index))
        .and_then(|m| m.textures.first())
        .and_then(|tex| tex.image)
        .and_then(|k| capture.images.get(k.index));
    let band = 1024 * 32 * 4;
    let base = match (image, packs) {
        (Some(img), Some(packs)) => match packs.locate(img) {
            Some(asset_t6::ImageSource::Pack(i, entry)) => {
                packs.packs[i].read(entry).ok().and_then(|bytes| {
                    let iwi = ipak_t6::parse_iwi(&bytes).ok()?;
                    (iwi.format == ipak_t6::IwiFormat::Rgba8 && iwi.texel_width == 1024)
                        .then(|| iwi.level(0).get(..band).map(<[u8]>::to_vec))
                        .flatten()
                })
            }
            _ => None,
        },
        _ => None,
    };
    let summary = format!(
        "t6 grade: vision {want} {}, colour table {} {}",
        if vision.is_some() { "read" } else { "MISSING" },
        image.map_or("(none)", |i| i.name.as_str()),
        if base.is_some() {
            "band 0"
        } else {
            "MISSING (identity)"
        },
    );
    // The scripts' `visionsetnaked` (mpOutro at a match's end, then this
    // map's own again) builds over the same table band.
    asset_world::set_t6_vision_base(base.clone());
    if let Some(v) = &vision {
        asset_world::register_t6_visions(vec![(map.to_owned(), *v)]);
    }
    let grade = vision.map(|vision| asset_world::T6Grade {
        lut: asset_world::t6_grade_lut(&vision, base.as_deref()),
        vision,
    });
    (grade, summary)
}

/// The light table: every primary light (a surface's light index is a
/// byte; Die Rise has 214, Origins 148), then the two fog rows.
const MAX_LIGHTS: usize = 256;
const SUN_FOG_SLOT: usize = MAX_LIGHTS;
const FOG_SLOT: usize = MAX_LIGHTS + 1;

/// Where a map's fog came from.
#[derive(Clone, Copy, Debug)]
enum T6Fog {
    World(asset_t6::WorldFogRef),
    Art(asset_t6::ArtFog),
}

/// The map's flickering lamps: `light` entities whose targetname names one
/// of the engine's built-in mixer behaviours (`_lights.csc` setup_mixer_lights
/// -> `ismixerlightbehaviorhardcoded` -> `setmixerlightbehavior`), with the
/// keys init_mixer_lights passes as mixer params. `pl#` is the primary light.
fn flicker_lights(capture: &asset_t6::ZoneCapture, lights: usize) -> Vec<asset_world::T6Flicker> {
    let num = |e: &asset_t6::MapEntity, key: &str| e.get(key).and_then(|v| v.trim().parse::<f32>().ok());
    capture
        .map_ents
        .iter()
        .flat_map(|text| asset_t6::parse_entities(text))
        .filter(|e| e.classname() == "light")
        .filter_map(|e| {
            let electrical = match e.get("targetname")? {
                "fire_flicker" => false,
                "electrical_flicker" => true,
                _ => return None,
            };
            let light = e.get("pl#")?.trim().parse::<u16>().ok()?;
            if usize::from(light) >= lights {
                return None;
            }
            let base = num(&e, "intensity").unwrap_or(1.0);
            Some(asset_world::T6Flicker {
                light,
                electrical,
                base,
                min: num(&e, "script_intensity_min").unwrap_or(base * 0.25),
                max: num(&e, "script_intensity_max").unwrap_or(base),
                delay: [
                    num(&e, "script_delay_min").unwrap_or(0.1),
                    num(&e, "script_delay_max").unwrap_or(0.5),
                ],
                burst: [
                    num(&e, "script_burst_min").unwrap_or(2.0),
                    num(&e, "script_burst_max").unwrap_or(6.0),
                ],
                burst_time: num(&e, "script_burst_time").unwrap_or(0.1),
                wait: [
                    num(&e, "script_wait_min").unwrap_or(1.0),
                    num(&e, "script_wait_max").unwrap_or(4.0),
                ],
            })
        })
        .collect()
}

/// The map's primary lights for the renderer's fallback draw, four vec4
/// each: origin and type (`ComPrimaryLight::type`: 1 sun, 2 spot, 5 omni),
/// colour and radius, direction and the cone's outer cosine, then
/// `dAttenuation` and the inner cosine. Slots 256 and 257 carry the fog
/// (`GfxWorldFog` terms): 257 = (start distance, half distance, opacity, 1),
/// (fog colour HDR, base height), (sun fog colour HDR, sun fog opacity),
/// (direction toward the sun fog, half height); 256 = the sun fog blend
/// t = saturate(y * dot(dir, view) + x) as (x, y, 0, 0).
fn t6_light_table(capture: &asset_t6::ZoneCapture, fog: Option<T6Fog>) -> Vec<[f32; 16]> {
    let mut table: Vec<[f32; 16]> = capture
        .key_lights
        .iter()
        .take(MAX_LIGHTS)
        .map(|p| {
            [
                p.origin[0],
                p.origin[1],
                p.origin[2],
                f32::from(p.ty),
                p.color[0],
                p.color[1],
                p.color[2],
                p.radius,
                p.heading[0],
                p.heading[1],
                p.heading[2],
                p.cos_half_fov_outer,
                p.d_attenuation,
                p.cos_half_fov_inner,
                0.0,
                0.0,
            ]
        })
        .collect();
    let Some(fog) = fog else { return table };
    table.resize(FOG_SLOT + 1, [0.0; 16]);
    let [sun_row, fog_row] = fog_rows(fog);
    table[FOG_SLOT] = fog_row;
    table[SUN_FOG_SLOT] = sun_row;
    table
}

/// The light table's two fog rows for a fog (slots 256 and 257, as
/// `t6_light_table` lays them out).
fn fog_rows(fog: T6Fog) -> [[f32; 16]; 2] {
    let (slot, sun) = match fog {
        T6Fog::World(f) => {
            // Pitch and yaw as the game's angles: pitch down is positive.
            let (p, y) = (f.sun_fog_pitch.to_radians(), f.sun_fog_yaw.to_radians());
            let dir = [p.cos() * y.cos(), p.cos() * y.sin(), -p.sin()];
            let (ci, co) = (
                f.sun_fog_inner.to_radians().cos(),
                f.sun_fog_outer.to_radians().cos(),
            );
            let scale = if (ci - co).abs() > 1e-4 {
                1.0 / (ci - co)
            } else {
                0.0
            };
            (
                [
                    f.base_dist,
                    f.half_dist,
                    f.fog_opacity,
                    1.0,
                    f.fog_color[0],
                    f.fog_color[1],
                    f.fog_color[2],
                    f.floor_elevation,
                    f.sun_fog_color[0],
                    f.sun_fog_color[1],
                    f.sun_fog_color[2],
                    f.sun_fog_opacity,
                    dir[0],
                    dir[1],
                    dir[2],
                    // BO2: a half height under 1 is 1.
                    f.half_height.max(1.0),
                ],
                [-co * scale, scale],
            )
        }
        T6Fog::Art(art) => {
            // Script fog has no separate sun tint: the sun colour repeats
            // the fog colour and the sun points straight down the z axis.
            let [r, g, b] = art.color.map(|channel| channel * art.scale);
            let opacity = art.peak_alpha;
            let row = [
                art.fog_begin, art.half_dist, opacity, 1.0,
                r, g, b, art.floor_elevation,
                r, g, b, opacity,
                0.0, 0.0, 1.0, art.half_height,
            ];
            (row, [0.0; 2])
        }
    };
    let mut sun_row = [0.0; 16];
    sun_row[0] = sun[0];
    sun_row[1] = sun[1];
    [sun_row, slot]
}

/// The world's fog banks (the client scripts' `setworldfogactivebank`):
/// each fog volume's bank is its control word's low byte (Nuketown: three
/// bank-1 volumes with the map's fog, one bank-2 volume with none, which
/// its game-over shot switches to). Per bank, the first volume's rows.
fn fog_banks(world: &asset_t6::WorldRef) -> Vec<asset_world::T6FogVolume> {
    // Like an exposure volume's, a fog volume's control word holds its first
    // plane (bits 16..) and plane count less one (bits 8..15); its low byte
    // is the banks it belongs to, a bit mask (Origins' volumes 15, 7, 4, 2,
    // 1; Mob's csc sets banks 4, 8 and 1). Bank-less volumes never draw.
    world
        .fog_volumes
        .iter()
        .filter(|(_, _, control, _, _)| control & 0xff != 0)
        .map(|(_, _, control, _, fog)| {
            let start = (control >> 16) as usize;
            let count = ((control >> 8) & 0xff) as usize + 1;
            asset_world::T6FogVolume {
                planes: world
                    .fog_volume_planes
                    .get(start..start + count)
                    .map(|planes| planes.to_vec())
                    .unwrap_or_default(),
                banks: control & 0xff,
                rows: fog_rows(T6Fog::World(*fog)),
            }
        })
        .collect()
}

/// The world's fog modifier volumes: their control words pick planes and
/// banks as fog volumes' do. Bank-less volumes never apply.
fn fog_modifiers(world: &asset_t6::WorldRef) -> Vec<asset_world::T6FogModifier> {
    world
        .fog_modifier_volumes
        .iter()
        .filter(|(control, _, _)| control & 0xff != 0)
        .map(|(control, scales, color_adjust)| {
            let start = (control >> 16) as usize;
            let count = ((control >> 8) & 0xff) as usize + 1;
            asset_world::T6FogModifier {
                planes: world
                    .fog_modifier_volume_planes
                    .get(start..start + count)
                    .map(|planes| planes.to_vec())
                    .unwrap_or_default(),
                banks: control & 0xff,
                depth_scale: scales[1],
                color_adjust: *color_adjust,
            }
        })
        .collect()
}

/// The world's exposure volumes with their bounding planes: a volume's
/// control word holds its first plane (bits 16..) and its plane count less
/// one (bits 8..15) (measured: Nuketown 3 volumes of 10 planes at 20, 10
/// and 0; Tranzit 8 of 6; Mob's 8- and 9-plane volumes tile its 121).
fn exposure_volumes(world: &asset_t6::WorldRef) -> Vec<asset_world::T6ExposureVolume> {
    world
        .exposure_volumes
        .iter()
        .zip(&world.exposure_volume_controls)
        .map(|(v, control)| {
            let start = (control >> 16) as usize;
            let count = ((control >> 8) & 0xff) as usize + 1;
            asset_world::T6ExposureVolume {
                planes: world
                    .exposure_volume_planes
                    .get(start..start + count)
                    .map(|planes| planes.to_vec())
                    .unwrap_or_default(),
                exposure: v[0],
                increase_scale: v[1],
                decrease_scale: v[2],
                feather_range: v[3],
            }
        })
        .collect()
}

/// The map's colour tables: the first image of its `lutMaterial` (RGBA8 in
/// the packs; Buried's `zm_buried_lut_win` is 1024x96, three tables).
fn world_lut(
    capture: &asset_t6::ZoneCapture,
    world: &asset_t6::WorldRef,
    packs: Option<&asset_t6::PackSet>,
) -> Result<asset_world::T6Lut, String> {
    let material = &capture.materials[world.lut_material.ok_or("no lutMaterial")?.index];
    let image = material
        .textures
        .first()
        .and_then(|tex| tex.image)
        .map(|k| &capture.images[k.index])
        .ok_or("lutMaterial has no image")?;
    let Some(asset_t6::ImageSource::Pack(i, entry)) = packs.and_then(|p| p.locate(image)) else {
        return Err(format!("{} is in no pack", image.name));
    };
    let bytes = packs.ok_or("no packs")?.packs[i].read(entry)?;
    let iwi = ipak_t6::parse_iwi(&bytes).map_err(|e| e.to_string())?;
    let rgba = iwi.level(0);
    if iwi.format != ipak_t6::IwiFormat::Rgba8
        || iwi.texel_width != 1024
        || iwi.height % 32 != 0
        || rgba.len() < (iwi.texel_width * iwi.height * 4) as usize
    {
        return Err(format!("{} is {:?} {}x{}", image.name, iwi.format, iwi.texel_width, iwi.height));
    }
    Ok(asset_world::T6Lut {
        width: iwi.texel_width,
        height: iwi.height,
        rgba: rgba[..(iwi.texel_width * iwi.height * 4) as usize].to_vec(),
    })
}

impl ZoneChannel for T6Lane {
    fn read_level_geometry(
        &self,
        path: &Path,
        image: &ZoneTextureData,
        progress: &LoadCompletion,
        _shared_surfaces: asset_model::PooledMeshSkins,
        material_seed: asset_material::SurfaceShaderRoster,
        _common_film_visions: &mut std::collections::BTreeMap<String,
            Result<asset_world::ScreenGradeLook, asset_world::GradeVisionDecodeFault>,
        >,
    ) -> ResidentLevel {
        let mut load_log = Vec::with_capacity(32);
        load_log.push(format!("game: T6 ({})", path.display()));
        load_log.push(asset_census(path, image));
        let stage = progress.begin_scoped(StageId::MapAssets, "walk", None);
        let capture = match asset_t6::capture_image(path, image) {
            Ok(capture) => {
                stage.done();
                capture
            }
            Err(error) => {
                stage.fail();
                return ResidentLevel::with_gap(
                    LevelRenderRules::t6(),
                    format!("T6 zone walk: {error}"),
                );
            }
        };
        load_log.push(format!(
            "t6 capture: {} images, {} materials, {} technique sets, {} xmodels, {} unresolved refs",
            capture.images.len(),
            capture.materials.len(),
            capture.technique_sets.len(),
            capture.xmodels.len(),
            capture.unresolved_refs
        ));

        let map = zone_stem(path);
        let (spawn_at, yaw) = player_start(&capture, &map, &mut load_log);
        let scripts = super::t6_scripts::start_scripts(spawn_at, yaw, map.starts_with("zm_"));

        // bo2mp: the map's breakable windows.
        let t6_glass = asset_world::build_t6_glass(&capture.glasses);
        load_log.push(match t6_glass.as_ref() {
            Some((g, _)) => format!(
                "t6 glass: {} windows read, {}, materials {:?}",
                capture.glasses.len(),
                g.summary_text(),
                g.fallback_shaders
            ),
            None => format!("t6 glass: {} windows read, none built", capture.glasses.len()),
        });
        let built_clip = capture.clip.as_ref().map(|raw| (raw, asset_world::build_t6_clip_collision(raw)));
        let collision = match built_clip {
            None => {
                load_log.push("t6 clip gap: zone has no clipMap".to_owned());
                None
            }
            Some((_, Err(why))) => {
                load_log.push(format!("t6 clip gap: {why}"));
                None
            }
            Some((raw, Ok(mut clip_map))) => {
                let (placed, with_tris) = t6_static_model_census(&capture);
                if let Err(e) = asset_world::attach_t6_static_world(&mut clip_map, raw, &capture.xmodels) {
                    load_log.push(format!("t6 static world gap: {e}"));
                }
                load_log.push(format!(
                    "t6 clip static models: {placed} placed, {with_tris} with collision triangles"
                ));
                if let Some((_, placements)) = t6_glass.as_ref() {
                    load_log.push(match asset_world::attach_t6_glass_brushes(&mut clip_map, placements) {
                        Ok(c) => format!(
                            "t6 glass clip: {} panes, {} brushes, {} leaf links, {} models missing, {} shared, contents {:#x}",
                            c.panes, c.brushes, c.leaf_links, c.missing_models, c.shared_brushes, c.contents
                        ),
                        Err(error) => format!("t6 glass clip gap: {error}"),
                    });
                }
                let m = asset_world::attach_t6_model_collision(
                    &mut clip_map,
                    capture
                        .xmodels
                        .iter()
                        .map(|x| (x.name.as_str(), x.collmap_brushes.as_slice())),
                );
                load_log.push(format!(
                    "t6 model collision: {} models, {} brushes, {} skipped",
                    m.models, m.brushes, m.skipped
                ));
                let world_brushes = clip_map.cmodels.first().map_or(0, |m| m.solid_volume_total);
                let (mesh_verts, mesh_tris) =
                    clip_map.t6.map_or((0, 0), |m| (m.vertex_points.len(), m.tri_indices.len()));
                let counts = [clip_map.brushes.len(), clip_map.nodes.len(), clip_map.leaves.len(), clip_map.cmodels.len()];
                let [brushes, nodes, leaves, brush_models] = counts;
                let materials = clip_map.materials.len();
                load_log.push(format!(
                    "t6 clip: {brushes} brushes ({world_brushes} in the world model), {nodes} nodes, {leaves} leaves, {brush_models} brush models, {mesh_verts} mesh verts, {mesh_tris} mesh tris, {materials} materials"
                ));
                Some(clip_map)
            }
        };

        let spawns = vec![asset_world::RespawnMarker {
            spawn_class: "mp_dm_spawn".to_owned(),
            origin: spawn_at,
            angles: [0.0, yaw, 0.0],
            attached_to: String::new(), script_destructable_area: String::new(),
        }];

        let Some(world) = capture.world.as_ref() else {
            let mut read_back = ResidentLevel::with_gap(LevelRenderRules::t6(), "T6 zone has no GfxWorld");
            read_back.scripts = scripts;
            read_back.collision = collision;
            read_back.spawns = spawns;
            read_back.load_log.extend(load_log); return read_back;
        };
        // Materials: every world surface's, linked into the lane's catalog
        // with its colour map decoded from the packs beside the zone.
        let stage = progress.begin_scoped(StageId::MapAssets, "world materials", None);
        // BO2MP_HEADLESS=1: nothing is drawn, so no colour map is read or
        // decoded; the materials still link, without their images.
        let packs = match Some(asset_transport::t6_extra::install_zone_dir(path))
            .filter(|_| !frame::Headless::requested())
            .map(|dir| asset_t6::PackSet::open_dir(&dir))
        {
            Some(Ok(packs)) => Some(packs),
            Some(Err(fault)) => {
                load_log.push(format!("t6 image packs: {fault}"));
                None
            }
            None => None,
        };
        let mut mat_list = material_seed;
        mat_list.choose_scope(asset_core::AssetCategoryScope::T6);
        let sky_materials = capture
            .xmodels
            .iter()
            .filter(|m| m.name == world.sky_box_model)
            .flat_map(|m| m.lod0.iter())
            .filter_map(|srf| srf.material.map(|k| k.index));
        let model_materials = world
            .prop_set
            .iter()
            .filter_map(|sm| sm.model)
            .flat_map(|k| capture.xmodels[k.index].lod0.iter())
            .filter_map(|srf| srf.material.map(|k| k.index))
            .chain(sky_materials);
        // codmp: the windows' own materials (no world surface names them).
        let glass_names: std::collections::HashSet<&str> = t6_glass
            .iter()
            .flat_map(|(g, _)| g.fallback_shaders.iter())
            .flat_map(|(a, b)| [a.as_str(), b.as_str()])
            // The cracked material a damaged window wears.
            .chain(capture.glasses.iter().map(|g| g.cracked_material.as_str()))
            // bo2mp: and the one its shards are drawn with once it shatters.
            .chain(capture.glasses.iter().map(|g| g.shard_material.as_str()))
            .filter(|n| !n.is_empty())
            .collect();
        let glass_materials: Vec<usize> = capture
            .materials
            .iter()
            .enumerate()
            .filter(|(_, m)| glass_names.contains(m.name.trim_start_matches(',')))
            .map(|(i, _)| i)
            .collect();
        let mut areas = AreaModels::read(path, &map, &capture, world);
        let area_map_materials: Vec<usize> = areas
            .facet_materials()
            .filter_map(|(ci, mi)| ci.is_none().then_some(mi))
            .collect();
        let linked = super::t6_materials::link_materials(
            &mut mat_list, &capture,
            packs.as_ref(),
            world
                .surfaces
                .iter()
                .filter_map(|s| s.material.map(|k| k.index))
                .chain(model_materials)
                .chain(glass_materials)
                .chain(area_map_materials),
            &map,
        );
        stage.done();
        load_log.extend(linked.load_log);
        areas.link(&mut mat_list, &capture, packs.as_ref(), &mut load_log);
        let facet_materials: Vec<Option<usize>> = world
            .surfaces
            .iter()
            .map(|s| s.material.and_then(|k| linked.local.get(&k.index).copied()))
            .collect();
        // Lighting: each lightmap's secondary image (the only one T6 keeps),
        // the world's sun, and its exposure (exposure values are stops: the
        // scale is 2^-exposure).
        let lightmaps: Vec<Option<asset_world::LevelLightbake>> = world
            .light_atlases
            .iter()
            .map(|pair| {
                pair[1]
                    .and_then(|k| capture.images.get(k.index))
                    .and_then(asset_world::t6_lightmap)
            })
            .collect();
        // bo2zm fix list 2: the same pages on the CPU, for decals on walls.
        lighting_t6::install_t6_lightmaps(
            world
                .light_atlases
                .iter()
                .map(|pair| {
                    let image = pair[1].and_then(|k| capture.images.get(k.index))?;
                    let embedded = image.embedded.as_ref().filter(|e| e.dxgi_format == 28)?;
                    let (width, height) = (u32::from(image.width), u32::from(image.height));
                    let rgba = embedded.data.get(..(width * height * 4) as usize)?.to_vec();
                    Some(lighting_t6::T6LightmapPage {
                        width,
                        height,
                        rgba,
                    })
                })
                .collect(),
        );
        // bo2mp: a map with no exposure volume (Cove, Plaza, Pod, Uplink)
        // takes its world sun's own exposure (`initWorldSun.exposure`), not
        // 0 stops (drawn four to eight times too bright).
        let exposure = world
            .exposure_volumes
            .first()
            .map_or(world.init_sun_exposure, |v| v[0]);
        let (grade, grade_line) = t6_grade(&capture, world, packs.as_ref(), &map);
        asset_world::install_t6_grade(grade);
        load_log.push(grade_line);
        // The sky: its material's rotation and brightness constants.
        let sky_mat = capture
            .xmodels
            .iter()
            .find(|m| m.name == world.sky_box_model)
            .and_then(|m| m.lod0.first())
            .and_then(|srf| srf.material)
            .map(|k| &capture.materials[k.index]);
        let constant = |name: &str| {
            sky_mat
                .and_then(|m| m.constants.iter().find(|c| name.starts_with(&c.1)))
                .map(|c| c.2)
        };
        let rotation = constant("skyBoxRotation").unwrap_or([1.0, 0.0, -0.0, 1.0]);
        let brightness = constant("skyColorParm").map_or(1.0, |c| c[1]);
        let sky_parms = constant("skyColorParm").map_or([1.0, 1.0], |c| [c[0], c[3]]);
        // `r_skyTransition` blends the sky shader's two cube lookups, the
        // second with z flipped the other way. Die Rise's client script sets
        // it to 1 (`clientscripts/mp/zm_highrise.csc` main), so it draws the
        // cube's other half: the dark storm, not the blue-white day.
        let sky_transition = map == "zm_highrise";
        let sky = [
            rotation[0],
            rotation[1],
            rotation[2],
            if (rotation[3] < 0.0) != sky_transition {
                -brightness
            } else {
                brightness
            },
        ];
        // The map's vision (`vision/<map>.vision`; Origins starts on
        // zm_tomb): the final image's saturation and colour grade. Die Rise's
        // client script picks its vision by fog volume with the power's
        // suffix (`fog_vol_to_visionset_set_suffix("_off")` in
        // `clientscripts/mp/zm_highrise.csc`), so it starts on
        // zm_highrise_off; its plain vision is an identity grade.
        let vision = if map == "zm_highrise" {
            format!("vision/{map}_off.vision")
        } else {
            format!("vision/{map}.vision")
        };
        let vision_bytes = capture
            .raw_files
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(&vision))
            .map(|(_, bytes)| bytes.as_slice());
        let saturation = vision_bytes.map(vision_saturation);
        let grade = vision_bytes.map(vision_grade);
        load_log.push(format!("t6 vision: {vision} saturation {saturation:?} grade {grade:?}"));
        let lighting = world.sun.map(|sun| asset_world::T6WorldLighting {
            sun: asset_world::LevelSunlamp {
                direction: sun.heading,
                diffuse: [sun.color[0], sun.color[1], sun.color[2], 1.0],
                specular: [sun.color[0], sun.color[1], sun.color[2], 1.0],
            },
            exposure_scale: (-exposure).exp2(),
            sky,
            sky_parms,
            saturation,
            grade,
        });
        load_log.push(format!(
            "t6 lighting: {} of {} lightmaps built, sun {:?}, exposure {exposure} (scale {:.4})",
            lightmaps.iter().filter(|l| l.is_some()).count(),
            lightmaps.len(),
            world.sun.map(|s| s.color),
            (-exposure).exp2()
        ));
        // Layered surfaces: each one's extra layers and their second-stream
        // layout, from its technique set's name.
        let surface_layers: Vec<asset_core::T6Layers> = world
            .surfaces
            .iter()
            .map(|s| {
                s.material
                    .and_then(|k| capture.materials[k.index].method_set)
                    .map(|k| {
                        asset_core::T6Layers::from_technique_set(
                            &capture.technique_sets[k.index].name,
                        )
                    })
                    .unwrap_or_default()
            })
            .collect();
        load_log.push(format!(
            "t6 layered surfaces: {} of {}",
            surface_layers.iter().filter(|l| l.any()).count(),
            surface_layers.len()
        ));
        // Each `script_brushmodel` places its submodel's surfaces (authored
        // about the entity) where the map stands it; the cull draws them
        // while its live entity shows them.
        let brush_placements: Vec<asset_world::T6BrushPlacement> = capture
            .map_ents
            .iter()
            .flat_map(|text| asset_t6::parse_entities(text))
            .filter(|e| e.classname() == "script_brushmodel")
            .filter_map(|e| {
                let solid_model = e.get("model")?.strip_prefix('*')?.parse().ok()?;
                Some(asset_world::T6BrushPlacement {
                    solid_model,
                    origin: e.vec3("origin").unwrap_or_default(),
                    angles: e.vec3("angles").unwrap_or_default(),
                })
            })
            .collect();
        match asset_world::build_t6_world_draw(
            world,
            &facet_materials,
            &surface_layers,
            lightmaps,
            lighting,
            &brush_placements,
        ) {
            Ok(mut world_draw) => {
                load_log.push(format!(
                    "t6 placed submodels: {} with surfaces of {} script brushes",
                    world_draw.t6_brush_surfaces.len(),
                    brush_placements.len()
                ));
                // The map's fog: its own initial world fog (what the game
                // draws; the client script picks among its fog banks), else
                // its createart script's setvolfog.
                let fog = world
                    .init_fog
                    .filter(|f| f.half_dist > 0.0 && f.fog_opacity > 0.0)
                    .map(T6Fog::World)
                    .or_else(|| art_fog(&capture, path, &map).map(T6Fog::Art));
                load_log.push(format!("t6 fog: {fog:?}"));
                world_draw.t6_lights = t6_light_table(&capture, fog);
                world_draw.t6_fog_banks = fog_banks(world);
                world_draw.t6_fog_modifiers = fog_modifiers(world);
                load_log.push(format!(
                    "t6 fog modifiers: {:?}",
                    world_draw.t6_fog_modifiers
                        .iter()
                        .map(|m| (m.banks, m.planes.len(), m.depth_scale, m.color_adjust))
                        .collect::<Vec<_>>()
                ));
                world_draw.t6_exposure_volumes = exposure_volumes(world);
                // The map's own colour tables, graded before its vision
                // (`hdr_create_lut2dv`), and the volumes picking among them.
                match world_lut(&capture, world, packs.as_ref()) {
                    Ok(lut) => {
                        load_log.push(format!(
                            "t6 colour table: {}x{} ({} tables)",
                            lut.width,
                            lut.height,
                            lut.height / 32
                        ));
                        world_draw.t6_lut = Some(std::sync::Arc::new(lut));
                    }
                    Err(e) => load_log.push(format!("t6 colour table: none ({e})")),
                }
                // bo2mp Raid: the outdoor lookup's picture (`$outdoor`, held in
                // the zone) and its matrix; no material names it.
                let outdoor = world.outdoor_image.and_then(|k| capture.images.get(k.index));
                match (outdoor, packs.as_ref()) {
                    (Some(img), Some(p)) => match super::t6_materials::link_outdoor_image(&mut mat_list, p, img) {
                        Ok(_) => {
                            world_draw.exterior_texture_label = Some(img.name.clone());
                            world_draw.openair_lookup = world.outdoor_lookup.map(f32::to_bits);
                        }
                        Err(e) => load_log.push(format!("t6 outdoor image {}: {e}", img.name)),
                    },
                    _ => load_log.push("t6 outdoor image: none in GfxWorld".to_owned()),
                }
                // The sun light and colour the client scripts'
                // r_lightTweakSunLight / SunColor replace (Buried 13, TranZit
                // 12, Nuketown 15; Origins 0.38 0.78 1) and the sky intensity
                // r_sky_intensity_* replaces.
                world_draw.t6_light_tweaks = world
                    .init_sun_raw
                    .get(11)
                    .map(|w| f32::from_bits(*w))
                    .filter(|v| v.is_finite() && *v > 0.0)
                    .map(|sun_light| asset_world::T6LightTweaks {
                        sun_light,
                        sun_color: [8, 9, 10].map(|i| {
                            world.init_sun_raw.get(i).map_or(1.0, |w| f32::from_bits(*w))
                        }),
                        sky_intensity: world.sky_dyn_intensity,
                    });
                load_log.push(format!("t6 light tweaks: {:?}", world_draw.t6_light_tweaks));
                load_log.push(format!(
                    "t6 exposure volumes: {:?}",
                    world_draw.t6_exposure_volumes
                        .iter()
                        .map(|v| (v.exposure, v.planes.len()))
                        .collect::<Vec<_>>()
                ));
                world_draw.t6_flicker = flicker_lights(&capture, world_draw.t6_lights.len().min(MAX_LIGHTS));
                load_log.push(format!(
                    "t6 flickering lamps: {}",
                    world_draw.t6_flicker
                        .iter()
                        .map(|f| format!(
                            "pl{} {} {}..{} of {}",
                            f.light,
                            if f.electrical { "electrical" } else { "fire" },
                            f.min,
                            f.max,
                            f.base
                        ))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
                load_log.push(format!(
                    "t6 fog banks: {:?}",
                    world_draw.t6_fog_banks
                        .iter()
                        .map(|v| (v.banks, v.planes.len(), v.rows[1][2]))
                        .collect::<Vec<_>>()
                ));
                // Reflection probes: their cube maps (one cube array) and
                // the lighting each was captured in.
                world_draw.t6_probes = world.probe_list.iter().map(|p| p.lighting_sh).collect();
                lighting_t6::install_t6_probe_origins(
                    world.probe_list.iter().map(|p| p.origin).collect(),
                );
                if let Some(line) =
                    super::t6_materials::link_reflection_probes(&mut mat_list, &capture, packs.as_ref(), world, &map)
                {
                    load_log.push(line);
                }
                load_log.push(format!(
                    "t6 primary lights: {} (types {:?})",
                    world_draw.t6_lights.len(),
                    capture
                        .key_lights
                        .iter()
                        .map(|p| p.ty)
                        .collect::<Vec<_>>()
                ));
                load_log.push(format!(
                    "fallback world draw (colour maps): {} verts / {} tris / {} surfs",
                    world_draw.tallies.vertices, world_draw.tallies.triangles, world_draw.tallies.surfaces
                ));
                let (min, max) = (world_draw.tallies.min, world_draw.tallies.max);
                let props = build_props(&capture, world, &linked.local, &areas, collision.as_ref(), &mut load_log);
                // bo2zm M2: guns, arms, animations, effects and sound from
                // every zone the map loads.
                let stage = progress.begin_scoped(StageId::MapAssets, "weapons, effects, sound", None);
                let combat =
                    super::t6_m2::load_t6_combat(path, &capture, &mut mat_list, packs.as_ref());
                stage.done();
                load_log.extend(combat.load_log);
                // The visions a script turns on (Mob's Afterlife), graded
                // as the map's own.
                world_draw.t6_script_grades = combat
                    .visions
                    .iter()
                    .map(|(name, bytes)| (name.clone(), vision_grade(bytes)))
                    .collect();
                load_log.push(format!(
                    "t6 script visions: {}",
                    world_draw.t6_script_grades
                        .iter()
                        .map(|(name, _)| name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
                let prepare = script_sound_aliases(&combat.scripts);
                load_log.push(format!("t6 script sounds prepared at load: {}", prepare.len()));
                let facts = crate::MapFacts {
                    t6_hud_icons: combat.hud_icons,
                    t6_hud_fonts: combat.hud_fonts,
                    t6_ui: combat.ui,
                    t6_playeranim: combat.scripts.playeranim.clone(),
                    t6_gun_facts: combat.gun_facts,
                    t6_ragdoll: combat.scripts.ragdoll.clone(),
                    t6_anim_speeds: combat
                        .scripts
                        .anims
                        .iter()
                        .filter(|a| a.name.starts_with("pb_") || a.name.starts_with("pl_"))
                        .filter_map(|a| {
                            let len = f32::from(a.frame_total) / a.fps;
                            let (first, last) = (a.delta_trans.first()?.1, a.delta_trans.last()?.1);
                            let v = [last[0] - first[0], last[1] - first[1], last[2] - first[2]];
                            // T5 BG_AnimParseAnimScript: an animation that mostly
                            // rises (x + y <= 0.8 z, a ladder climb up) moves by its
                            // full 3D delta, every other one by its ground delta.
                            let d = if v[0] + v[1] <= v[2] * 0.8 {
                                (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
                            } else {
                                (v[0] * v[0] + v[1] * v[1]).sqrt()
                            };
                            (len > 0.0 && d > 1.0).then(|| (a.name.to_ascii_lowercase(), d / len))
                        })
                        .collect(),
                    t6_screen_filters: combat.screen_filters,
                    script_sound: asset_audio::LevelScriptAudioSummary {
                        background_sound_entry: combat.room_tone,
                        source_script: Some(format!("clientscripts/mp/{map}_amb.csc")),
                        prepare,
                        ..Default::default()
                    },
                    // bo2mp Raid: getnorthyaw reads the worldspawn `northyaw`,
                    // 0 (north up) on a map without one, as Raid is.
                    compass_heading: Some(
                        capture
                            .map_ents
                            .iter()
                            .flat_map(|text| asset_t6::parse_entities(text))
                            .find(|e| e.classname() == "worldspawn")
                            .and_then(|e| e.get("northyaw")?.trim().parse().ok())
                            .unwrap_or(0.0),
                    ),
                    ..Default::default()
                };
                // codmp: each window's own surfaces, where it stands (what
                // BO2 draws while it is not shattered).
                let t6_glass = t6_glass.map(|(mut g, placements)| {
                    g.panes = asset_world::build_t6_glass_draws(world, &capture.glasses, &placements);
                    load_log.push(asset_world::t6_glass_draw_report(&g.panes, &capture.glasses, &placements));
                    g
                });
                let mut scripts = scripts;
                scripts.t6 = Some(combat.scripts);
                ResidentLevel {
                    scripts,
                    world: StagedLevel {
                        draw: Some(world_draw), min,
                        max,
                        policy: LevelRenderRules::t6(),
                        prop_meshes: props.0,
                        prop_instances: props.1,
                        fx: combat.fx,
                        effect_meshes: combat.effect_meshes,
                        impact_effect: combat.impact_effect,
                        t6_glass,
                        ..Default::default()
                    },
                    mat_list,
                    collision,
                    spawns,
                    first_person_meshes: combat.fpv,
                    animation_refs: combat.animation_refs,
                    weapons: combat.weapons,
                    level_placed_weapons: combat.level_placed_weapons,
                    tracers: combat.tracers,
                    pen_table: combat.pen_table,
                    lochit_table: combat.lochit_table,
                    sound: Some(combat.sound),
                    facts,
                    load_log,
                    ..Default::default()
                }
            }
            Err(error) => {
                let mut loaded = ResidentLevel::with_gap(
                    LevelRenderRules::t6(),
                    format!("T6 world mesh: {error}"),
                );
                loaded.scripts = scripts;
                loaded.collision = collision;
                loaded.spawns = spawns;
                loaded.load_log.extend(load_log);
                loaded
            }
        }
    }

    fn read_shared_multiplayer(
        &self,
        path: &Path,
        image: &ZoneTextureData,
        _progress: &LoadCompletion,
        _decode_color_maps: bool,
        _material_seed: asset_material::SurfaceShaderRoster,
    ) -> SharedAssetTally {
        SharedAssetTally {
            load_log: vec![
                format!(
                    "Black Ops II common assets are not read yet (bo2zm M1): {}",
                    path.display()
                ),
                asset_census(path, image),
            ],
            ..Default::default()
        }
    }

    fn read_material_census(
        &self,
        path: &Path,
        image: &ZoneTextureData,
        _progress: &LoadCompletion,
        mat_seed: asset_material::SurfaceShaderRoster,
    ) -> ShaderUsageSurvey {
        ShaderUsageSurvey {
            mat_list: mat_seed,
            load_log: vec![
                format!(
                    "Black Ops II materials are not read yet (bo2zm M1): {}",
                    path.display()
                ),
                asset_census(path, image),
            ],
            ..Default::default()
        }
    }
}

/// bo2zm: every sound alias a map script names as a whole string (the
/// banks' aliases that appear in a script's string table).
fn script_sound_aliases(scripts: &crate::T6ScriptSet) -> Vec<String> {
    let known: std::collections::HashSet<&str> =
        scripts.sound_aliases.iter().map(String::as_str).collect();
    let mut out: Vec<String> = scripts
        .objects
        .iter()
        .filter_map(|bytes| asset_t6::gsc::GscObject::parse(bytes).ok())
        .flat_map(|g| g.strings.into_values())
        .map(|s| s.to_ascii_lowercase())
        .filter(|s| known.contains(s.as_str()))
        .collect();
    // Breakable props' break sounds (a mannequin's head popping off).
    for d in &scripts.destructibles {
        for p in &d.pieces {
            for st in &p.stages {
                if let Some(snd) = st.shatter_audio.as_ref().map(|s| s.to_ascii_lowercase())
                    && known.contains(snd.as_str())
                {
                    out.push(snd);
                }
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// A T6 vision file's saturation: the w of `vc_FSM "r g b saturation"` (its
/// Rec. 709 weights then the mix toward colour), 1 when it has none.
fn vision_saturation(bytes: &[u8]) -> f32 {
    vision_value(bytes, "vc_FSM").map_or(1.0, |v| v[3])
}

/// A T6 vision value `vc_<key> "x y z w"`.
fn vision_value(bytes: &[u8], key: &str) -> Option<[f32; 4]> {
    String::from_utf8_lossy(bytes).lines().find_map(|line| {
        let rest = line.trim().strip_prefix(key)?;
        if !rest.starts_with([' ', '\t']) {
            return None;
        }
        let mut v = [0.0f32; 4];
        let mut parts = rest.trim().trim_matches('"').split_whitespace();
        for x in &mut v {
            *x = parts.next()?.parse::<f32>().ok().filter(|f| f.is_finite())?;
        }
        Some(v)
    })
}

/// A T6 vision's colour grade as the game's colour table shader
/// (`hdr_create_lut2dv`) reads it, rows for `asset_world::T6WorldLighting::grade`:
/// the tone bands' slopes and offsets against the brightness_weights of the display value
/// (shadow falling over `vc_RS.x..vc_RE.x`, highlight rising over `.y`, mid
/// rising over `.z` and falling over `.w`), the red, green and blue rows of
/// the shadow, mid and highlight joint_transforms (`vc_SM*`, `vc_MM*`, `vc_HM*`; w a
/// bias), then `vc_FGM` (gamma, as its reciprocal power), `vc_FSM` (brightness_weights weights, saturation) and
/// `vc_FBM` (how much of the grade is kept).
fn vision_grade(bytes: &[u8]) -> [[f32; 4]; 14] {
    let get = |key: &str, default: [f32; 4]| vision_value(bytes, key).unwrap_or(default);
    let rs = get("vc_RS", [0.0, 1.0, 0.0, 1.0]);
    let re = get("vc_RE", [1.0, 2.0, 1.0, 2.0]);
    let (mut slope, mut offset) = ([0.0f32; 4], [0.0f32; 4]);
    for i in 0..4 {
        let width = (re[i] - rs[i]).max(0.0001);
        // The shadow and the mid's fall go down, the others up.
        if i == 0 || i == 3 {
            slope[i] = -1.0 / width;
            offset[i] = re[i] / width;
        } else {
            slope[i] = 1.0 / width;
            offset[i] = -rs[i] / width;
        }
    }
    // The shader raises to cb131; an artist's gamma brightens above 1, so
    // the table takes its reciprocal (Die Rise's warm 1.05 / 0.995 / 0.957
    // as given turns its spawn blue, R/B 1.03 against the real 1.31).
    let fgm = get("vc_FGM", [1.0, 1.0, 1.0, 0.0]);
    let stow_kind = |g: f32| if g > 0.0001 { 1.0 / g } else { 1.0 };
    let gamma = [stow_kind(fgm[0]), stow_kind(fgm[1]), stow_kind(fgm[2]), fgm[3]];
    let row = |key: &str, i: usize| {
        let mut r = [0.0; 4];
        r[i] = 1.0;
        get(key, r)
    };
    [
        slope,
        offset,
        row("vc_SMR", 0),
        row("vc_MMR", 0),
        row("vc_HMR", 0),
        row("vc_SMG", 1),
        row("vc_MMG", 1),
        row("vc_HMG", 1),
        row("vc_SMB", 2),
        row("vc_MMB", 2),
        row("vc_HMB", 2),
        gamma,
        get("vc_FSM", [0.212585, 0.715195, 0.07222, 1.0]),
        get("vc_FBM", [1.0, 1.0, 1.0, 0.0]),
    ]
}
