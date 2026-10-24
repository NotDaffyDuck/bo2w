//! What the Materials phase hands to the draw side: the material generation
//! and the map's authored look (fog, sun, outdoor lookup, primary lights).

use std::collections::BTreeMap;
use std::sync::Arc;

use bevy::prelude::*;

use crate::assemble::drawsurf::{
    DrawMethodDfog, MapDirPrimaryLight, MapFrameFog, MapOutdoor, MapPrimaryLightTypes,
    MapPrimaryLights, MapSunEffects, MapT5SunParseExposure, MaterialFrameInputs,
    RuntimeImageId, admit_material_generation,
};
use crate::prepare::scene::world::WorldScene;

/// Publishes everything the draw side needs from the material phase.
pub(super) fn publish(commands: &mut Commands, scene: &WorldScene) {
    let generation = admit_material_generation(scene.runtime_material_catalog.clone());
    commands.insert_resource(render_scene::TessMaterials {
        catalog: Arc::clone(&generation.catalog),
        prepared: Arc::clone(&generation.prepared),
        material_images: Arc::new(Vec::new()),
    });
    commands.insert_resource(scene.map_xmodel_scene_assets.clone());
    commands.insert_resource(generation);
    commands.insert_resource(MaterialFrameInputs::default());

    publish_fog(commands, scene);
    publish_directional_light(commands, scene);
    publish_t5_terms(commands, scene);
    publish_outdoor(commands, scene);
    publish_sun_effects(commands, scene);
    publish_primary_lights(commands, scene);
    log_light_index_sources(scene);

    commands.insert_resource(DrawMethodDfog(false));
}

fn publish_fog(commands: &mut Commands, scene: &WorldScene) {
    let Some(fog) = scene.exp_fog else {
        commands.remove_resource::<MapFrameFog>();
        diag::warn!(
            World,
            "drawsurf createart fog: RED missing setExpFog — code constants 37/38/40/41/43 stay unproduced"
        );
        return;
    };
    diag::info!(
        World,
        "drawsurf createart fog: READY start={:.3} half={:.3} maxOpacity={:.3} sun={}",
        fog.start_dist,
        fog.halfway_dist,
        fog.max_opacity,
        fog.sun.is_some()
    );
    commands.insert_resource(MapFrameFog::new(fog));
}

fn publish_directional_light(commands: &mut Commands, scene: &WorldScene) {
    let Some(light) = scene.dir_primary_light else {
        commands.remove_resource::<MapDirPrimaryLight>();
        diag::warn!(
            World,
            "drawsurf DIR primary light: RED no is_sun GFX_LIGHT_TYPE_DIR — code constants 0/1/2 stay unproduced"
        );
        return;
    };
    let [dx, dy, dz] = light.direction;
    let [r, g, b] = light.color;
    diag::info!(
        World,
        "drawsurf DIR primary light: READY type={} dir=({dx:.3},{dy:.3},{dz:.3}) color=({r:.3},{g:.3},{b:.3})",
        light.light_type
    );
    commands.insert_resource(light);
}

fn publish_t5_terms(commands: &mut Commands, scene: &WorldScene) {
    if let Some(exposure) = scene.t5_sun_parse_exposure {
        diag::info!(
            World,
            "drawsurf T5 sunParse exposure: READY {exposure:.4} volumes={}",
            scene.t5_exposure_volume_count
        );
        commands.insert_resource(MapT5SunParseExposure { exposure });
    } else {
        commands.remove_resource::<MapT5SunParseExposure>();
    }
}

fn publish_outdoor(commands: &mut Commands, scene: &WorldScene) {
    commands.insert_resource(MapOutdoor {
        image: scene.outdoor_image.map(RuntimeImageId),
        lookup: scene.outdoor_lookup,
    });
    let m00 = f32::from_bits(scene.outdoor_lookup[0]);
    let m30 = f32::from_bits(scene.outdoor_lookup[12]);
    match scene.outdoor_image {
        Some(id) => diag::info!(
            World,
            "drawsurf outdoor: READY image={id} lookup_m00={m00:.6e} lookup_m30={m30:.4}"
        ),
        None => diag::warn!(
            World,
            "drawsurf outdoor: RED no $outdoor / GfxWorld outdoorImage — code texture 14 stays unproduced (lookup_m00={m00:.6e} lookup_m30={m30:.4})"
        ),
    }
}

fn publish_sun_effects(commands: &mut Commands, scene: &WorldScene) {
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

fn publish_primary_lights(commands: &mut Commands, scene: &WorldScene) {
    commands.insert_resource(MapPrimaryLightTypes {
        types: scene.primary_light_types.clone(),
    });
    commands.insert_resource(MapPrimaryLights {
        lights: scene.primary_light_pack.clone(),
        attenuation: scene.primary_light_attenuation.clone(),
        t5_falloff: scene.primary_light_t5_falloff.clone(),
        dynamic: scene.dynamic_light,
    });

    let lights = &scene.primary_light_pack;
    let with_width = lights
        .iter()
        .filter(|light| light.falloff_image_width.is_some())
        .count();
    let with_atten = scene
        .primary_light_attenuation
        .iter()
        .filter(|bind| bind.image.is_some())
        .count();
    let with_t5 = scene
        .primary_light_t5_falloff
        .iter()
        .filter(|pack| pack.attenuation.is_some())
        .count();
    diag::info!(
        World,
        "drawsurf primary lights: n={} falloff_width={with_width} atten_image={with_atten} t5_atten={with_t5} (tex 0xD only when atten_image is Some)",
        lights.len()
    );

    let def_name = |index: usize| {
        scene
            .primary_light_def_names
            .get(index)
            .and_then(|name| name.as_deref())
            .unwrap_or("-")
    };
    let missing_width = join_or_none(
        lights
            .iter()
            .enumerate()
            .filter(|(_, light)| light.falloff_image_width.is_none())
            .map(|(index, light)| format!("{index}:t{}:{}", light.light_type, def_name(index))),
    );
    let missing_atten = join_or_none(
        scene
            .primary_light_attenuation
            .iter()
            .enumerate()
            .filter(|(_, bind)| bind.image.is_none())
            .map(|(index, _)| {
                let light_type = lights.get(index).map_or(0, |light| light.light_type);
                format!("{index}:t{light_type}:{}", def_name(index))
            }),
    );
    diag::info!(
        World,
        "drawsurf primary lights missing falloff_width: {missing_width} (the def image width always yields 5; None is a retention gap)"
    );
    diag::info!(
        World,
        "drawsurf primary lights missing atten_image: {missing_atten}"
    );
}

fn join_or_none(parts: impl Iterator<Item = String>) -> String {
    let joined = parts.collect::<Vec<_>>().join(",");
    if joined.is_empty() {
        "none".to_owned()
    } else {
        joined
    }
}

/// Counts where the per-surface and per-model light and probe indices come
/// from, so a flat zero shows up in the log instead of as dark geometry.
fn log_light_index_sources(scene: &WorldScene) {
    let surface_lights = scene
        .cull
        .as_ref()
        .map_or(&[][..], |cull| cull.surface_primary_lights.as_slice());
    let models = || scene.static_model_instances.iter().flatten();
    diag::info!(
        World,
        "scene light index sources: surface nz={}/{} static model nz={}/{}",
        surface_lights.iter().filter(|&&index| index != 0).count(),
        surface_lights.len(),
        models().filter(|inst| inst.primary_light_index != 0).count(),
        scene.static_model_instances.len()
    );

    let (cell_lists, cells) = scene.cull.as_ref().map_or((0, 0), |cull| {
        (
            cull.dpvs.cell_reflection_probes.len(),
            cull.dpvs.cell_count,
        )
    });
    diag::info!(
        World,
        "reflectionProbeIndex sources: world_batches {} smodel_drawinst {} cell_lists={cell_lists}/{cells} origins={} (XModel/FPV/script Colour packs lightingInfo.hi from Assigned)",
        histogram_u8(scene.batches.iter().map(|batch| batch.reflection_probe_index)),
        histogram_u8(models().map(|inst| inst.reflection_probe_index)),
        scene.reflection_probe_origins.len(),
    );
}

/// "n=<count> zero=<how many were 0> {value:count ...}"
fn histogram_u8(values: impl Iterator<Item = u8>) -> String {
    let mut counts = BTreeMap::<u8, usize>::new();
    for value in values {
        *counts.entry(value).or_default() += 1;
    }
    let total: usize = counts.values().sum();
    let zero = counts.get(&0).copied().unwrap_or(0);
    let body = counts
        .iter()
        .map(|(value, count)| format!("{value}:{count}"))
        .collect::<Vec<_>>()
        .join(" ");
    format!("n={total} zero={zero} {{{body}}}")
}
