//! What the Materials phase hands to the draw side: the material generation
//! and the map's authored look (fog, sun, outdoor lookup, primary lights).

use std::collections::BTreeMap;
use std::sync::Arc;

use bevy::prelude::*;

use crate::{
    DrawMethodDepthHaze, MapDirKeyLight, MapFrameHaze, MapOpenair, MapKeyLightTypes,
    MapKeyLights, MapSunEffects, MapT5DaylightParseExposure, MaterialFrameFeed,
    LiveImageId, admit_material_generation,
};
use crate::prepare::scene::world::LevelStage;

/// Publishes everything the draw side needs from the material phase.
pub(super) fn publish(commands: &mut Commands, scene: &LevelStage) {
    let epoch = admit_material_generation(scene.live_material_ledger.clone());
    commands.insert_resource(render_scene::StitchMaterials {
        catalog: Arc::clone(&epoch.catalog),
        prepared: Arc::clone(&epoch.prepared),
        shader_textures: Arc::new(Vec::new()),
    });
    commands.insert_resource(scene.map_figure_stage_assets.clone());
    commands.insert_resource(epoch);
    commands.insert_resource(MaterialFrameFeed::default());

    publish_fog(commands, scene);
    publish_directional_light(commands, scene);
    publish_t5_terms(commands, scene);
    publish_outdoor(commands, scene);
    publish_sun_effects(commands, scene);
    publish_primary_lights(commands, scene);
    log_light_index_sources(scene);

    commands.insert_resource(DrawMethodDepthHaze(false));
}

fn publish_fog(commands: &mut Commands, scene: &LevelStage) {
    let Some(haze) = scene.exp_haze else {
        commands.remove_resource::<MapFrameHaze>();
        diag::warn!(World, "drawsurf createart fog: RED missing setExpFog — code constants 37/38/40/41/43 stay unproduced");
        return;
    };
    let (start, half, peak, sunlit) = (haze.near_edge, haze.half_density_at, haze.opacity_cap, haze.sun.is_some());
    diag::info!(World, "drawsurf createart fog: READY start={start:.3} half={half:.3} maxOpacity={peak:.3} sun={sunlit}");
    commands.insert_resource(MapFrameHaze::new(haze));
}

fn publish_directional_light(commands: &mut Commands, scene: &LevelStage) {
    let Some(key) = scene.dir_key_light else {
        commands.remove_resource::<MapDirKeyLight>();
        diag::warn!(World, "drawsurf DIR primary light: RED no is_sun GFX_LIGHT_TYPE_DIR — code constants 0/1/2 stay unproduced");
        return;
    };
    let ([dx, dy, dz], [r, g, b], kind) = (key.direction, key.color, key.light_kind);
    diag::info!(
        World,
        "drawsurf DIR primary light: READY type={kind} dir=({dx:.3},{dy:.3},{dz:.3}) color=({r:.3},{g:.3},{b:.3})"
    );
    commands.insert_resource(key);
}

fn publish_t5_terms(commands: &mut Commands, scene: &LevelStage) {
    if let Some(exposure) = scene.classic_daylight_parse_exposure {
        let volumes = scene.classic_exposure_volume_tally;
        let saturation = scene.t6_vision_saturation;
        diag::info!(World, "drawsurf T5 sunParse exposure: READY {exposure:.4} volumes={volumes} vision saturation={saturation:?}");
        commands.insert_resource(MapT5DaylightParseExposure {
            exposure,
            saturation: saturation.unwrap_or(0.853516),
        });
    } else {
        commands.remove_resource::<MapT5DaylightParseExposure>();
    }
}

fn publish_outdoor(commands: &mut Commands, scene: &LevelStage) {
    commands.insert_resource(MapOpenair {
        image: scene.openair_image.map(LiveImageId),
        find_in: scene.openair_lookup,
    });
    let m00 = f32::from_bits(scene.openair_lookup[0]);
    let m30 = f32::from_bits(scene.openair_lookup[12]);
    if let Some(id) = scene.openair_image {
        diag::info!(World, "drawsurf outdoor: READY image={id} lookup_m00={m00:.6e} lookup_m30={m30:.4}");
    } else {
        diag::warn!(
            World,
            "drawsurf outdoor: RED no $outdoor / GfxWorld outdoorImage — code texture 14 stays unproduced (lookup_m00={m00:.6e} lookup_m30={m30:.4})"
        );
    }
}

fn publish_sun_effects(commands: &mut Commands, scene: &LevelStage) {
    commands.insert_resource(MapSunEffects {
        def: scene.sun_effects,
    });
    let Some(sun) = scene.sun_effects else {
        diag::info!(World, "sun effects: off (no authored sun record)");
        return;
    };
    diag::info!(
        World,
        "sun effects: READY sprite={} flare={} sprite_image={:?} flare_image={:?} blind={} glare={} dir=({:.3},{:.3},{:.3})",
        id_or_dash(sun.sprite_material),
        id_or_dash(sun.flare_material),
        sun.sprite_image,
        sun.flare_image,
        sun.blind_max_darken,
        sun.glare_max_lighten,
        sun.direction[0],
        sun.direction[1],
        sun.direction[2]
    );
}

fn id_or_dash<T: ToString>(id: Option<T>) -> String {
    id.map_or_else(|| "-".to_owned(), |id| id.to_string())
}

fn publish_primary_lights(commands: &mut Commands, scene: &LevelStage) {
    commands.insert_resource(MapKeyLightTypes {
        types: scene.key_light_types.clone(),
    });
    commands.insert_resource(MapKeyLights {
        lights: scene.key_light_pack.clone(),
        attenuation: scene.key_light_attenuation.clone(),
        classic_falloff: scene.key_light_classic_falloff.clone(),
        dynamic: scene.dynamic_light,
    });

    let lights = &scene.key_light_pack;
    let with_width = lights.iter().filter(|light| light.falloff_map_width.is_some()).count();
    let with_atten = scene.key_light_attenuation.iter().filter(|slot| slot.image.is_some()).count();
    let with_t5 = scene.key_light_classic_falloff.iter().filter(|falloff| falloff.attenuation.is_some()).count();
    diag::info!(
        World,
        "drawsurf primary lights: n={} falloff_width={with_width} atten_image={with_atten} t5_atten={with_t5} (tex 0xD only when atten_image is Some)",
        lights.len()
    );

    let definition_name = |index: usize| scene.key_light_def_names.get(index).and_then(Option::as_deref).unwrap_or("-");
    let missing_width = join_or_none(
        lights
            .iter()
            .enumerate()
            .filter(|entry| entry.1.falloff_map_width.is_none())
            .map(|(at, lamp)| format!("{at}:t{}:{}", lamp.kind, definition_name(at))),
    );
    let unbound = scene.key_light_attenuation.iter().enumerate().filter_map(|(index, slot)| slot.image.is_none().then_some(index));
    let missing_atten = join_or_none(unbound.map(|index| {
        let light_type = lights.get(index).map_or(0, |light| light.kind);
        format!("{index}:t{light_type}:{}", definition_name(index))
    }));
    diag::info!(
        World,
        "drawsurf primary lights without a falloff_width: {missing_width} (the def image width is always 5, so None means it was not kept)"
    );
    diag::info!(
        World,
        "drawsurf primary lights missing atten_image: {missing_atten}"
    );
}

fn join_or_none(parts: impl Iterator<Item = String>) -> String {
    let joined = parts.collect::<Vec<_>>().join(",");
    if joined.is_empty() {
        String::from("none")
    } else {
        joined
    }
}

/// Counts where the per-surface and per-model light and probe indices come
/// from, so a flat zero shows up in the log instead of as dark geometry.
fn log_light_index_sources(scene: &LevelStage) {
    let surface_lights = scene
        .culling
        .as_ref()
        .map_or(Default::default(), |cull| cull.facet_key_lights.as_slice());
    let models = || scene.prop_instances.iter().flatten();
    diag::info!(
        World,
        "scene light index sources: surface nz={}/{} static model nz={}/{}",
        surface_lights.iter().filter(|&&index| index != 0).count(),
        surface_lights.len(),
        models().filter(|inst| inst.key_light_index != 0).count(),
        scene.prop_instances.len()
    );

    let (cell_lists, cells) = scene.culling.as_ref().map_or((0, 0), |cull| {
        (
            cull.dpvs.room_env_probes.len(),
            cull.dpvs.cells_n,
        )
    });
    diag::info!(
        World,
        "reflectionProbeIndex sources: world_batches {} smodel_drawinst {} cell_lists={cell_lists}/{cells} origins={} (XModel/FPV/script Colour packs lightingInfo.hi from Assigned)",
        histogram_u8(scene.batches.iter().map(|batch| batch.env_probe_index)),
        histogram_u8(models().map(|inst| inst.env_probe_index)),
        scene.env_probe_origins.len(),
    );
}

/// "n=<count> zero=<how many were 0> {value:count ...}"
fn histogram_u8(values: impl Iterator<Item = u8>) -> String {
    let mut counts: BTreeMap<u8, usize> = Default::default();
    values.for_each(|seen| *counts.entry(seen).or_default() += 1);
    let total: usize = counts.values().sum();
    let zero = counts.get(&0).copied().unwrap_or(0);
    let body = counts
        .iter()
        .map(|(value, count)| format!("{value}:{count}"))
        .collect::<Vec<_>>()
        .join(" ");
    format!("n={total} zero={zero} {{{body}}}")
}
