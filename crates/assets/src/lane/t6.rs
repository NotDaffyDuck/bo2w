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

use super::{CommonCensus, LoadedWorld, MaterialPopulation, ZoneLane};
use crate::session_load::PreparedWorld;
use asset_transport::progress::{LoadProgress, StageId};
use asset_transport::{T6ZoneMemory, ZoneImage};
use asset_world::WorldDrawPolicy;

pub struct T6Lane;

/// One line per zone: asset count by type, or why the asset list did not open.
pub fn asset_census(path: &Path, image: &ZoneImage) -> String {
    let zone = path
        .file_stem()
        .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    let header = match image.t6_header() {
        Ok(header) => header,
        Err(error) => return format!("t6 census {zone}: zone header: {error}"),
    };
    let mut memory = T6ZoneMemory::for_header(&header);
    let mut stream = match memory.stream(&image.bytes) {
        Ok(stream) => stream,
        Err(error) => return format!("t6 census {zone}: zone arenas: {error}"),
    };
    let table = match fastfile_t6::open_asset_table(&mut stream) {
        Ok(table) => table,
        Err(error) => return format!("t6 census {zone}: asset list: {error}"),
    };
    let mut counts: BTreeMap<u32, usize> = BTreeMap::new();
    for i in 0..table.count() {
        match table.raw_kind(&stream, i) {
            Ok(raw) => *counts.entry(raw).or_default() += 1,
            Err(error) => return format!("t6 census {zone}: asset {i}: {error}"),
        }
    }
    let by_type: Vec<String> = counts
        .iter()
        .map(|(raw, n)| match fastfile_t6::AssetType::from_u32(*raw) {
            Some(ty) => format!("{}={n}", ty.name()),
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
    let Some(source) = capture.clip.as_ref() else {
        return (0, 0);
    };
    let mut placed = 0;
    let mut with_tris = 0;
    for sm in &source.static_models {
        let Some(model) = sm.model.and_then(|k| capture.xmodels.get(k.index)) else {
            continue;
        };
        placed += 1;
        if model.coll_lod >= 0 && model.coll_surfs.iter().any(|s| !s.tris.is_empty()) {
            with_tris += 1;
        }
    }
    (placed, with_tris)
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
    report: &mut Vec<String>,
) -> ([f32; 3], f32) {
    let location = map.strip_prefix("zm_").unwrap_or(map);
    let wanted = [
        format!("zstandard_{location}"),
        format!("zclassic_{location}"),
    ];
    let ents: Vec<asset_t6::MapEntity> = capture
        .map_ents
        .iter()
        .flat_map(|text| asset_t6::parse_entities(text))
        .collect();
    let initial = || {
        ents.iter()
            .filter(|e| e.get("script_noteworthy") == Some("initial_spawn"))
    };
    let named = initial().find(|e| {
        e.get("script_string")
            .is_some_and(|s| s.split_whitespace().any(|t| wanted.iter().any(|w| w == t)))
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
        report.push(format!(
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
            report.push(format!(
                "t6 spawn: {what} at [{:.1}, {:.1}, {:.1}] yaw {yaw:.1}",
                origin[0], origin[1], origin[2]
            ));
            return (origin, yaw);
        }
    }
    let w = capture.world.as_ref();
    let c = w.map_or([0.0; 3], |w| {
        [
            (w.mins[0] + w.maxs[0]) * 0.5,
            (w.mins[1] + w.maxs[1]) * 0.5,
            w.maxs[2],
        ]
    });
    report.push("t6 spawn gap: no info_player_start; world centre used".to_owned());
    (c, 0.0)
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
    clip: Option<&asset_world::ClipCollision>,
    report: &mut Vec<String>,
) -> (
    Vec<asset_world::ModelMesh>,
    Vec<Option<asset_world::StaticModelPlacement>>,
) {
    use bevy::prelude::{Mat3, Quat, Transform, Vec3};

    let grid = world
        .light_grid
        .as_ref()
        .and_then(lighting_t6::t6_light_grid);
    // Places outside the light grid (the distant terrain) take their light
    // from a sky grid volume: one coefficient set, one primary light and
    // its visibility.
    let raw_grid = world.light_grid.as_ref();
    let in_grid = |p: [f32; 3]| -> bool {
        let Some(g) = raw_grid else { return false };
        let cell = [
            (p[0].floor() as i32).wrapping_add(0x20000) >> 5,
            (p[1].floor() as i32).wrapping_add(0x20000) >> 5,
            (p[2].floor() as i32).wrapping_add(0x20000) >> 6,
        ];
        (0..3).all(|a| cell[a] >= i32::from(g.mins[a]) && cell[a] <= i32::from(g.maxs[a]))
    };
    let sky_volume = |p: [f32; 3]| {
        raw_grid.and_then(|g| {
            g.sky_volumes
                .iter()
                .find(|v| (0..3).all(|a| p[a] >= v.mins[a] && p[a] <= v.maxs[a]))
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
    let sky_light = |v: &asset_t6::SkyGridVolumeRef| coeffs_at(v.colors_index).map(|c| cube_mean(&c));
    let mut sky_n = 0usize;
    let mut meshes: Vec<asset_world::ModelMesh> = Vec::new();
    let mut shared: std::collections::HashMap<usize, Option<usize>> = Default::default();
    // The baked light's mean against the mean of the placement's own set:
    // the same light, so near 1 when the set is decoded right.
    let mut ratios: Vec<f32> = Vec::new();
    let mut lights: Vec<lighting_t6::T6ModelLight> = Vec::new();
    let mut light_of: std::collections::HashMap<u16, u32> = Default::default();
    let mut pending: Vec<(usize, usize, Option<[f32; 3]>, bool, u32)> = Vec::new();
    // The materials whose technique reads no baked light (decals, screens).
    let unlit: std::collections::HashSet<usize> = local_materials
        .iter()
        .filter(|(m, _)| {
            capture.materials[**m].technique_set.is_some_and(|t| {
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
            capture.materials[**m].technique_set.is_some_and(|t| {
                let draw =
                    asset_core::T6Draw::from_technique_set(&capture.technique_sets[t.index].name, 0, None);
                draw.foliage && !draw.unlit
            })
        })
        .map(|(_, local)| *local)
        .collect();
    for (i, sm) in world.static_models.iter().enumerate() {
        let Some(key) = sm.model else { continue };
        let base = *shared.entry(key.index).or_insert_with(|| {
            let mesh = asset_world::build_t6_model_mesh(&capture.xmodels[key.index], |k| {
                local_materials.get(&k.index).copied()
            })?;
            meshes.push(mesh);
            Some(meshes.len() - 1)
        });
        let Some(base) = base else { continue };
        let xm = &capture.xmodels[key.index];
        // The placement's own set (the game fills the model's lighting
        // block from it).
        let own = coeffs_at(sm.colors_index);
        let handle = own.map_or(0, |c| {
            *light_of.entry(sm.colors_index).or_insert_with(|| {
                lights.push(lighting_t6::T6ModelLight {
                    cube: lighting_t6::t6_grid_cube(&c).map(|rgb| rgb.map(lighting_t6::t6_light_byte)),
                    sh: lighting_t6::t6_grid_sh(&c),
                });
                lights.len() as u32
            })
        });
        // Without a set (no grid): the grid at the model's middle, or the
        // sky volume holding it.
        let grid_light = own.map(|c| cube_mean(&c)).or_else(|| {
            let mid: [f32; 3] = std::array::from_fn(|i| (xm.mins[i] + xm.maxs[i]) * 0.5 * sm.scale);
            let light_at: [f32; 3] = std::array::from_fn(|i| {
                sm.origin[i] + mid[0] * sm.axis[0][i] + mid[1] * sm.axis[1][i] + mid[2] * sm.axis[2][i]
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
                sm.colors_index,
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
            .copied()
            .unwrap_or(0.0)
    };
    report.push(format!(
        "t6 props: baked light against the placement's own grid set, {} props: 10% {:.2}, 25% {:.2}, median {:.2}, 75% {:.2}, 90% {:.2}",
        ratios.len(),
        at(0.1),
        at(0.25),
        at(0.5),
        at(0.75),
        at(0.9)
    ));
    let mut placements = vec![None; world.static_models.len()];
    let (mut gridded, mut baked_n, mut unlit) = (0usize, 0usize, 0usize);
    let mut visible_n = 0usize;
    let mut vis_diff = [0usize; 2];
    for (i, mesh, grid_light, vertex_lit, handle) in pending {
        let sm = &world.static_models[i];
        let packed_lighting = if vertex_lit {
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
        let primary = sm.primary_light_index;
        let visibility = sm.visibility;
        if !in_grid(sm.origin) {
            sky_n += 1;
        }
        if visibility > 0 {
            visible_n += 1;
        }
        // BO2MP_T6_VISLOG=<name part>: the stored visibility against the one
        // the light grid (or outside it, the sky volume) gives (test aid).
        if let Ok(want) = std::env::var("BO2MP_T6_VISLOG") {
            let worked_out = if !in_grid(sm.origin) {
                sky_volume(sm.origin)
                    .filter(|v| v.primary_light_index == primary)
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
        let basis = Mat3::from_cols(
            Vec3::from_array(sm.axis[0]),
            Vec3::from_array(sm.axis[1]),
            Vec3::from_array(sm.axis[2]),
        );
        placements[i] = Some(asset_world::StaticModelPlacement {
            mesh,
            transform: Transform {
                translation: Vec3::from_array(sm.origin),
                rotation: Quat::from_mat3(&basis),
                scale: Vec3::splat(sm.scale),
            },
            origin: sm.origin,
            axis: sm.axis,
            scale: sm.scale,
            // TODO(cull-dist-f32): BO2's `cullDist` is a float compared
            // linearly with the LOD-ramped distance (facts
            // dpvs-scene-shadow-t6.md); the placement still holds MW2's u16,
            // so fractions drop and distances past 65535 clamp.
            cull_dist: sm.cull_dist.clamp(0.0, f32::from(u16::MAX)) as u16,
            reflection_probe_index: sm.reflection_probe_index,
            primary_light_index: sm.primary_light_index,
            flags: sm.flags as u8,
            packed_lighting,
            t6_primary: [primary, visibility],
            t6_light: handle,
        });
    }
    let light_n = lights.len();
    lighting_t6::install_t6_model_lights(lights);
    // bo2zm M2: what lights moving things (the first-person gun, effects).
    // bo2mp: the grid decoded as the game does needs no scale.
    lighting_t6::install_t6_light_sampler(raw_grid.map(|g| lighting_t6::T6LightSampler {
        grid: grid.clone(),
        mins: g.mins,
        maxs: g.maxs,
        sky: g
            .sky_volumes
            .iter()
            .map(|v| {
                (
                    v.mins,
                    v.maxs,
                    sky_light(v).unwrap_or_default(),
                    v.primary_light_index,
                    v.visibility,
                )
            })
            .collect(),
        scale: 1.0,
        sun_primary: g.sun_primary_light_index as u8,
        sight: clip.map(|c| {
            let c = c.clone();
            std::sync::Arc::new(move |a: [f32; 3], b: [f32; 3]| {
                c.box_sight_clear(a, b, lighting_t6::LIGHT_GRID_SIGHT_CONTENT_MASK)
            }) as lighting_t6::SightCheck
        }),
        cache: Default::default(),
    }));
    report.push(format!(
        "t6 props: {} placed, {} meshes; {baked_n} with baked vertex light, {gridded} lit by the light grid, {unlit} without light; {light_n} own grid lights; {visible_n} see their primary light; {sky_n} outside the grid (sky volume)",
        placements.iter().filter(|p| p.is_some()).count(),
        meshes.len(),
    ));
    if std::env::var_os("BO2MP_T6_VISLOG").is_some() {
        report.push(format!(
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
        let patch = path.with_file_name(format!("{map}_patch.ff"));
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
        .and_then(|t| t.image)
        .and_then(|k| capture.images.get(k.index));
    let band = 1024 * 32 * 4;
    let base = match (image, packs) {
        (Some(img), Some(packs)) => match packs.locate(img) {
            Some(asset_t6::ImageSource::Pack(i, entry)) => {
                packs.packs[i].read(entry).ok().and_then(|bytes| {
                    let iwi = ipak_t6::parse_iwi(&bytes).ok()?;
                    (iwi.format == ipak_t6::IwiFormat::Rgba8 && iwi.width == 1024)
                        .then(|| iwi.level(0).get(..band).map(<[u8]>::to_vec))
                        .flatten()
                })
            }
            _ => None,
        },
        _ => None,
    };
    let line = format!(
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
    (grade, line)
}

/// Light-table slots that carry the fog (no map has this many lights).
const FOG_SLOT: usize = 31;
const SUN_FOG_SLOT: usize = 30;

/// Where a map's fog came from.
#[derive(Clone, Copy, Debug)]
enum T6Fog {
    World(asset_t6::WorldFogRef),
    Art(asset_t6::ArtFog),
}

/// The map's primary lights for the renderer's fallback draw, four vec4
/// each: origin and type (`ComPrimaryLight::type`: 1 sun, 2 spot, 5 omni),
/// colour and radius, direction and the cone's outer cosine, then
/// `dAttenuation` and the inner cosine. Slots 30 and 31 carry the fog
/// (`GfxWorldFog` terms): 31 = (start distance, half distance, opacity, 1),
/// (fog colour HDR, base height), (sun fog colour HDR, sun fog opacity),
/// (direction toward the sun fog, half height); 30 = the sun fog blend
/// t = saturate(y * dot(dir, view) + x) as (x, y, 0, 0).
fn t6_light_table(capture: &asset_t6::ZoneCapture, fog: Option<T6Fog>) -> Vec<[f32; 16]> {
    let mut table: Vec<[f32; 16]> = capture
        .primary_lights
        .iter()
        .take(SUN_FOG_SLOT)
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
                p.dir[0],
                p.dir[1],
                p.dir[2],
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

/// The light table's two fog rows for a fog (slots 30 and 31, as
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
                    f.base_height,
                    f.sun_fog_color[0],
                    f.sun_fog_color[1],
                    f.sun_fog_color[2],
                    f.sun_fog_opacity,
                    dir[0],
                    dir[1],
                    dir[2],
                    f.half_height,
                ],
                [-co * scale, scale],
            )
        }
        T6Fog::Art(f) => (
            [
                f.start_dist,
                f.half_dist,
                f.max_opacity,
                1.0,
                f.color[0] * f.scale,
                f.color[1] * f.scale,
                f.color[2] * f.scale,
                f.base_height,
                f.color[0] * f.scale,
                f.color[1] * f.scale,
                f.color[2] * f.scale,
                f.max_opacity,
                0.0,
                0.0,
                1.0,
                f.half_height,
            ],
            [0.0, 0.0],
        ),
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
fn fog_banks(world: &asset_t6::WorldRef) -> Vec<(u32, [[f32; 16]; 2])> {
    let mut banks: Vec<(u32, [[f32; 16]; 2])> = Vec::new();
    for (_, _, control, _, fog) in &world.fog_volumes {
        let bank = control & 0xff;
        if bank == 0 || banks.iter().any(|(b, _)| *b == bank) {
            continue;
        }
        banks.push((bank, fog_rows(T6Fog::World(*fog))));
    }
    banks
}

impl ZoneLane for T6Lane {
    fn load_world(
        &self,
        path: &Path,
        image: &ZoneImage,
        progress: &LoadProgress,
        _shared_surfaces: asset_model::SharedXModelSurfaces,
        material_seed: asset_material::MaterialCatalog,
        _common_film_visions: &mut std::collections::BTreeMap<
            String,
            Result<asset_world::FilmVision, asset_world::FilmVisionParseError>,
        >,
    ) -> LoadedWorld {
        let mut report = vec![
            format!("game: T6 ({})", path.display()),
            asset_census(path, image),
        ];
        let stage = progress.begin_scoped(StageId::MapAssets, "walk", None);
        let capture = match asset_t6::capture_image(path, image) {
            Ok(capture) => {
                stage.done();
                capture
            }
            Err(error) => {
                stage.fail();
                return LoadedWorld::with_gap(
                    WorldDrawPolicy::t6(),
                    format!("T6 zone walk: {error}"),
                );
            }
        };
        report.push(format!(
            "t6 capture: {} images, {} materials, {} technique sets, {} xmodels, {} unresolved refs",
            capture.images.len(),
            capture.materials.len(),
            capture.technique_sets.len(),
            capture.xmodels.len(),
            capture.unresolved_refs
        ));

        let map = path
            .file_stem()
            .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
        let (origin, yaw) = player_start(&capture, &map, &mut report);
        let scripts = super::t6_scripts::start_scripts(origin, yaw, map.starts_with("zm_"));

        // bo2mp: the map's breakable windows.
        let t6_glass = asset_world::build_t6_glass(&capture.glasses);
        report.push(match t6_glass.as_ref() {
            Some((g, _)) => format!(
                "t6 glass: {} windows read, {}, materials {:?}",
                capture.glasses.len(),
                g.report_line(),
                g.def_materials
            ),
            None => format!("t6 glass: {} windows read, none built", capture.glasses.len()),
        });
        let collision = match capture
            .clip
            .as_ref()
            .map(asset_world::build_t6_clip_collision)
        {
            Some(Ok(mut clip)) => {
                let (placed, with_tris) = t6_static_model_census(&capture);
                if let Some(Err(e)) =
                    capture.clip.as_ref().map(|c| asset_world::attach_t6_static_world(&mut clip, c, &capture.xmodels))
                {
                    report.push(format!("t6 static world gap: {e}"));
                }
                report.push(format!(
                    "t6 clip static models: {placed} placed, {with_tris} with collision triangles"
                ));
                if let Some((_, placements)) = t6_glass.as_ref() {
                    report.push(match asset_world::attach_t6_glass_brushes(&mut clip, placements) {
                        Ok(c) => format!(
                            "t6 glass clip: {} panes, {} brushes, {} leaf links, {} models missing, {} shared, contents {:#x}",
                            c.panes, c.brushes, c.leaf_links, c.missing_models, c.shared_brushes, c.contents
                        ),
                        Err(error) => format!("t6 glass clip gap: {error}"),
                    });
                }
                let m = asset_world::attach_t6_model_collision(
                    &mut clip,
                    capture
                        .xmodels
                        .iter()
                        .map(|x| (x.name.as_str(), x.collmap_brushes.as_slice())),
                );
                report.push(format!(
                    "t6 model collision: {} models, {} brushes, {} skipped",
                    m.models, m.brushes, m.skipped
                ));
                let world_brushes = clip.cmodels.first().map_or(0, |m| m.num_brushes);
                report.push(format!(
                    "t6 clip: {} brushes ({} in the world model), {} nodes, {} leaves, {} brush models, {} mesh verts, {} mesh tris, {} materials",
                    clip.brushes.len(),
                    world_brushes,
                    clip.nodes.len(),
                    clip.leaves.len(),
                    clip.cmodels.len(),
                    clip.t6.map_or(0, |m| m.verts.len()),
                    clip.t6.map_or(0, |m| m.tri_indices.len()),
                    clip.materials.len()
                ));
                Some(clip)
            }
            Some(Err(error)) => {
                report.push(format!("t6 clip gap: {error}"));
                None
            }
            None => {
                report.push("t6 clip gap: zone has no clipMap".to_owned());
                None
            }
        };

        let spawns = vec![asset_world::SpawnPoint {
            classname: "mp_dm_spawn".to_owned(),
            origin,
            angles: [0.0, yaw, 0.0],
            script_linkto: String::new(),
            script_destructable_area: String::new(),
        }];

        let Some(world) = capture.world.as_ref() else {
            let mut loaded = LoadedWorld::with_gap(WorldDrawPolicy::t6(), "T6 zone has no GfxWorld");
            loaded.scripts = scripts;
            loaded.collision = collision;
            loaded.spawns = spawns;
            loaded.report.extend(report);
            return loaded;
        };
        // Materials: every world surface's, linked into the lane's catalog
        // with its colour map decoded from the packs beside the zone.
        let stage = progress.begin_scoped(StageId::MapAssets, "materials", None);
        // BO2MP_HEADLESS=1: nothing is drawn, so no colour map is read or
        // decoded; the materials still link, without their images.
        let packs = match path.parent().filter(|_| !frame::Headless::requested()) {
            Some(dir) => match asset_t6::PackSet::open_dir(dir) {
                Ok(packs) => Some(packs),
                Err(error) => {
                    report.push(format!("t6 image packs: {error}"));
                    None
                }
            },
            None => None,
        };
        let mut materials = material_seed;
        materials.set_capture_ns(asset_core::AssetNamespace::T6);
        let sky_materials = capture
            .xmodels
            .iter()
            .filter(|m| m.name == world.sky_box_model)
            .flat_map(|m| m.lod0.iter())
            .filter_map(|srf| srf.material.map(|k| k.index));
        let model_materials = world
            .static_models
            .iter()
            .filter_map(|sm| sm.model)
            .flat_map(|k| capture.xmodels[k.index].lod0.iter())
            .filter_map(|srf| srf.material.map(|k| k.index))
            .chain(sky_materials);
        // codmp: the windows' own materials (no world surface names them).
        let glass_names: std::collections::HashSet<&str> = t6_glass
            .iter()
            .flat_map(|(g, _)| g.def_materials.iter())
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
        let linked = super::t6_materials::link_materials(
            &mut materials,
            &capture,
            packs.as_ref(),
            world
                .surfaces
                .iter()
                .filter_map(|s| s.material.map(|k| k.index))
                .chain(model_materials)
                .chain(glass_materials),
            &map,
        );
        stage.done();
        report.extend(linked.report);
        let surface_materials: Vec<Option<usize>> = world
            .surfaces
            .iter()
            .map(|s| s.material.and_then(|k| linked.local.get(&k.index).copied()))
            .collect();
        // Lighting: each lightmap's secondary image (the only one T6 keeps),
        // the world's sun, and the exposure of its first exposure volume
        // (exposure values are stops: the scale is 2^-exposure).
        let lightmaps: Vec<Option<asset_world::WorldLightmap>> = world
            .lightmaps
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
                .lightmaps
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
        report.push(grade_line);
        // The sky: its material's rotation and brightness constants.
        let sky_material = capture
            .xmodels
            .iter()
            .find(|m| m.name == world.sky_box_model)
            .and_then(|m| m.lod0.first())
            .and_then(|srf| srf.material)
            .map(|k| &capture.materials[k.index]);
        let constant = |name: &str| {
            sky_material
                .and_then(|m| m.constants.iter().find(|c| name.starts_with(&c.1)))
                .map(|c| c.2)
        };
        let rotation = constant("skyBoxRotation").unwrap_or([1.0, 0.0, -0.0, 1.0]);
        let brightness = constant("skyColorParm").map_or(1.0, |c| c[1]);
        let sky = [
            rotation[0],
            rotation[1],
            rotation[2],
            if rotation[3] < 0.0 {
                -brightness
            } else {
                brightness
            },
        ];
        let lighting = world.sun.map(|sun| asset_world::T6WorldLighting {
            sun: asset_world::WorldSunLight {
                direction: sun.dir,
                diffuse: [sun.color[0], sun.color[1], sun.color[2], 1.0],
                specular: [sun.color[0], sun.color[1], sun.color[2], 1.0],
            },
            exposure_scale: (-exposure).exp2(),
            sky,
        });
        report.push(format!(
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
                    .and_then(|k| capture.materials[k.index].technique_set)
                    .map(|k| {
                        asset_core::T6Layers::from_technique_set(
                            &capture.technique_sets[k.index].name,
                        )
                    })
                    .unwrap_or_default()
            })
            .collect();
        report.push(format!(
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
                let brush_model = e.get("model")?.strip_prefix('*')?.parse().ok()?;
                Some(asset_world::T6BrushPlacement {
                    brush_model,
                    origin: e.vec3("origin").unwrap_or_default(),
                    angles: e.vec3("angles").unwrap_or_default(),
                })
            })
            .collect();
        match asset_world::build_t6_world_draw(
            world,
            &surface_materials,
            &surface_layers,
            lightmaps,
            lighting,
            &brush_placements,
        ) {
            Ok(mut draw) => {
                report.push(format!(
                    "t6 placed submodels: {} with surfaces of {} script brushes",
                    draw.t6_brush_surfaces.len(),
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
                report.push(format!("t6 fog: {fog:?}"));
                draw.t6_lights = t6_light_table(&capture, fog);
                draw.t6_fog_banks = fog_banks(world);
                report.push(format!(
                    "t6 fog banks: {:?}",
                    draw.t6_fog_banks
                        .iter()
                        .map(|(b, rows)| (*b, rows[1][2]))
                        .collect::<Vec<_>>()
                ));
                // Reflection probes: their cube maps (one cube array) and
                // the lighting each was captured in.
                draw.t6_probes = world.reflection_probes.iter().map(|p| p.lighting_sh).collect();
                lighting_t6::install_t6_probe_origins(
                    world.reflection_probes.iter().map(|p| p.origin).collect(),
                );
                if let Some(line) =
                    super::t6_materials::link_reflection_probes(&mut materials, &capture, world, &map)
                {
                    report.push(line);
                }
                report.push(format!(
                    "t6 primary lights: {} (types {:?})",
                    draw.t6_lights.len(),
                    capture
                        .primary_lights
                        .iter()
                        .map(|p| p.ty)
                        .collect::<Vec<_>>()
                ));
                report.push(format!(
                    "world mesh: {} vertices, {} triangles, {} surfaces (fallback draw with colour maps)",
                    draw.stats.vertices, draw.stats.triangles, draw.stats.surfaces
                ));
                let (min, max) = (draw.stats.min, draw.stats.max);
                let props = build_props(&capture, world, &linked.local, collision.as_ref(), &mut report);
                // bo2zm M2: guns, arms, animations, effects and sound from
                // every zone the map loads.
                let stage = progress.begin_scoped(StageId::MapAssets, "weapons, effects, sound", None);
                let combat =
                    super::t6_m2::load_t6_combat(path, &capture, &mut materials, packs.as_ref());
                stage.done();
                report.extend(combat.report);
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
                            let len = f32::from(a.numframes) / a.framerate;
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
                    script_sound: asset_audio::MapScriptSoundFacts {
                        ambient_alias: combat.room_tone,
                        script: Some(format!("clientscripts/mp/{map}_amb.csc")),
                        ..Default::default()
                    },
                    ..Default::default()
                };
                // codmp: each window's own surfaces, where it stands (what
                // BO2 draws while it is not shattered).
                let t6_glass = t6_glass.map(|(mut g, placements)| {
                    g.panes = asset_world::build_t6_glass_draws(world, &capture.glasses, &placements);
                    report.push(asset_world::t6_glass_draw_report(&g.panes, &capture.glasses, &placements));
                    g
                });
                let mut scripts = scripts;
                scripts.t6 = Some(combat.scripts);
                LoadedWorld {
                    scripts,
                    world: PreparedWorld {
                        draw: Some(draw),
                        min,
                        max,
                        policy: WorldDrawPolicy::t6(),
                        static_model_meshes: props.0,
                        static_model_instances: props.1,
                        fx: combat.fx,
                        fx_models: combat.fx_models,
                        impact_fx: combat.impact_fx,
                        t6_glass,
                        ..Default::default()
                    },
                    materials,
                    collision,
                    spawns,
                    fpv_meshes: combat.fpv,
                    xanims: combat.xanims,
                    weapons: combat.weapons,
                    world_weapons: combat.world_weapons,
                    tracers: combat.tracers,
                    pen_table: combat.pen_table,
                    lochit_table: combat.lochit_table,
                    sound: Some(combat.sound),
                    facts,
                    report,
                    ..Default::default()
                }
            }
            Err(error) => {
                let mut loaded = LoadedWorld::with_gap(
                    WorldDrawPolicy::t6(),
                    format!("T6 world mesh: {error}"),
                );
                loaded.scripts = scripts;
                loaded.collision = collision;
                loaded.spawns = spawns;
                loaded.report.extend(report);
                loaded
            }
        }
    }

    fn load_common_mp(
        &self,
        path: &Path,
        image: &ZoneImage,
        _progress: &LoadProgress,
        _decode_color_maps: bool,
        _material_seed: asset_material::MaterialCatalog,
    ) -> CommonCensus {
        CommonCensus {
            report: vec![
                format!(
                    "Black Ops II common assets are not read yet (bo2zm M1): {}",
                    path.display()
                ),
                asset_census(path, image),
            ],
            ..Default::default()
        }
    }

    fn load_material_population(
        &self,
        path: &Path,
        image: &ZoneImage,
        _progress: &LoadProgress,
        material_seed: asset_material::MaterialCatalog,
    ) -> MaterialPopulation {
        MaterialPopulation {
            materials: material_seed,
            report: vec![
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
