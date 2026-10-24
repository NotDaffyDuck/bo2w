//! Putting a loaded world into the ECS: the static, script and dynamic model
//! entities, the model-lighting atlas, and the camera rig.

use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;
use bevy::render::render_resource::TextureUsages;
use bevy::render::view::Msaa;
use hud_t6::fov::{CG_FOV_DEFAULT, vertical_fov_deg};
use lighting_t6::ModelLightingAtlasDims;

use crate::assemble::drawsurf::{RuntimeImageHandles, RuntimeLightmapHandles};
use crate::prepare::scene::camera::{FlyCamera, FpvLens, transform_from_iw_view};
use crate::prepare::scene::cull::{
    DynEntModelEntity, ScriptModelEntity, ScriptModelGameObject, StaticModelEntity,
};
use crate::prepare::scene::model_lighting_atlas::WorldModelLightingAtlas;
use crate::prepare::scene::model_lighting_cache::WorldModelLightingCache;
use crate::prepare::scene::smodel_geom_cache::WorldStaticModelCache;
use crate::prepare::scene::smodel_lighting::{
    WorldSmodelLighting, prepare_smodel_lighting, slot_has_lighting_sample, slot_is_lit_candidate,
    surfaces_take_model_lighting,
};
use crate::prepare::scene::world::{
    SMODEL_LIGHTING_MAX_CLIENT_VIEWS, WorldDynEntInstance, WorldScene, WorldScriptModelInstance,
    WorldStaticModelInstance,
};

pub(crate) const CAMERA_NEAR_INCHES: f32 = 2.0;

/// Fly speed of the intermission camera, in inches a second.
const INTERMISSION_FLY_SPEED: f32 = 320.0;

pub(crate) fn camera_far_inches(radius: f32) -> f32 {
    (radius * 8.0).max(CAMERA_NEAR_INCHES * 100.0)
}

/// What the static-model pass found out about the world's slots.
#[derive(Default)]
struct SlotTally {
    unresolved: usize,
    lit: usize,
    sample_without_technique: usize,
    technique_without_sample: usize,
}

pub fn place(
    commands: &mut Commands,
    scene: &mut WorldScene,
    images: &mut Assets<Image>,
    exact_material_handles: Vec<Option<Handle<Image>>>,
    reflection_probe_handles: Vec<Option<Handle<Image>>>,
    lightmap_handles: Vec<Option<RuntimeLightmapHandles>>,
) {
    // The batches have been tessellated by now; nothing past here reads them.
    drop(std::mem::take(&mut scene.batches));

    let static_instances = std::mem::take(&mut scene.static_model_instances);
    let scene_assets = std::mem::take(&mut scene.map_xmodel_scene_assets);
    let script_instances = std::mem::take(&mut scene.script_model_instances);
    let dyn_ent_instances = std::mem::take(&mut scene.dyn_ent_instances);

    // A Black Ops II world lights its models itself (the fallback pass); the
    // atlas only has to exist for the first-person rig to build.
    let atlas = (scene.light_grid.is_some() || !scene.t6_lights.is_empty())
        .then(|| lighting_t6::model_lighting_atlas_dims(SMODEL_LIGHTING_MAX_CLIENT_VIEWS))
        .flatten()
        .map(|dims| WorldModelLightingAtlas::new(images, dims));

    if !scene.static_model_meshes.is_empty() {
        place_static_models(commands, scene, &static_instances, atlas.as_ref());
    }
    scene.static_model_instances = static_instances;

    place_script_models(commands, script_instances, &scene_assets);
    place_dyn_ents(commands, dyn_ent_instances, &scene_assets, scene.dyn_ent_brush_n);
    commands.insert_resource(scene_assets);

    if let Some(atlas) = atlas {
        scene.model_lighting_image = Some(atlas.image.clone());
        scene.model_lighting_dims = Some(atlas.dims);
        commands.insert_resource(RuntimeImageHandles::from_pools(
            scene.runtime_material_catalog.generation_id,
            exact_material_handles,
            scene.exact_material_names.clone(),
            reflection_probe_handles,
            lightmap_handles,
            Some(atlas.image.clone()),
        ));
        diag::info!(
            World,
            "drawsurf typed image registry: model_lighting=READY code_texture=3 sampler=0xe2 volume=D3"
        );
        commands.insert_resource(WorldModelLightingCache::new(atlas.dims));
        commands.insert_resource(atlas);
    }

    spawn_camera_rig(commands, scene);
}

/// One hidden parent entity per resolvable static-model slot, plus the
/// lighting table and geometry cache for them.
fn place_static_models(
    commands: &mut Commands,
    scene: &mut WorldScene,
    instances: &[Option<WorldStaticModelInstance>],
    atlas: Option<&WorldModelLightingAtlas>,
) {
    for model in &scene.static_model_meshes {
        diag::info!(
            World,
            "static model mesh: {} ({} surfaces)",
            model.name,
            model.lod_surfaces.iter().map(Vec::len).sum::<usize>()
        );
    }
    scene.smodel_mesh_names = scene
        .static_model_meshes
        .iter()
        .map(|model| model.name.clone())
        .collect();
    let authored_materials: Vec<Vec<Option<assets::MaterialIndex>>> = scene
        .static_model_meshes
        .iter()
        .map(|model| model.surface_materials().collect())
        .collect();

    let slot_count = instances.len();
    let mut lighting = atlas.and_then(|atlas| {
        prepare_smodel_lighting(atlas.dims, &scene.smodel_lighting_samples, slot_count)
    });

    let mut entities = vec![None; slot_count];
    let mut cull_dists = vec![0; slot_count];
    let mut tally = SlotTally::default();

    for (slot, instance) in instances.iter().enumerate() {
        let Some(instance) = instance else {
            tally.unresolved += 1;
            continue;
        };
        let materials = match authored_materials.get(instance.mesh) {
            Some(row) if !row.is_empty() => row,
            _ => {
                tally.unresolved += 1;
                continue;
            }
        };
        let wants_atlas = surfaces_take_model_lighting(
            materials.iter().copied(),
            &scene.runtime_material_catalog,
        );
        let has_sample = lighting
            .as_ref()
            .is_some_and(|table| slot_has_lighting_sample(table, slot));
        tally.sample_without_technique += usize::from(has_sample && !wants_atlas);
        tally.technique_without_sample += usize::from(wants_atlas && !has_sample);

        let lit = lighting
            .as_ref()
            .is_some_and(|table| slot_is_lit_candidate(table, slot, wants_atlas));
        let parent = commands
            .spawn((instance.transform, Visibility::Hidden, StaticModelEntity))
            .id();
        if lit {
            let Some(table) = lighting.as_mut() else {
                tally.unresolved += 1;
                commands.entity(parent).despawn();
                continue;
            };
            table.parents[slot] = Some(parent);
            tally.lit += 1;
        }
        entities[slot] = Some(parent);
        cull_dists[slot] = instance.cull_dist;
    }

    match lighting {
        Some(mut table) => {
            table.spawn_lit_n = tally.lit as u32;
            table.spawn_sample_without_technique = tally.sample_without_technique as u32;
            log_lighting_summary(scene, atlas.map(|a| a.dims), &tally);
            commands.insert_resource::<WorldSmodelLighting>(table);
        }
        None if !scene.smodel_lighting_samples.is_empty() => {
            let dims = lighting_t6::model_lighting_atlas_dims(SMODEL_LIGHTING_MAX_CLIENT_VIEWS);
            diag::info!(
                World,
                "smodel lighting samples: resolved={} concurrent_entry_limit={:?} \
                 (BO2MP_SMODEL_LIGHTING disabled — unlit gap)",
                scene.smodel_lighting_samples.len(),
                dims.map(|dims| dims.smodel_entry_limit)
            );
        }
        None => {}
    }

    commands.insert_resource(WorldStaticModelCache::new(slot_count));
    diag::info!(
        World,
        "static models: {slot_count} authored slots ({} unresolved); tess left for assemble",
        tally.unresolved
    );
    if let Some(cull) = scene.cull.as_mut() {
        cull.static_model_entities = entities;
        cull.static_model_cull_dists = cull_dists;
        cull.smodel_vis.resize(slot_count, 0);
    }
}

fn log_lighting_summary(
    scene: &WorldScene,
    dims: Option<ModelLightingAtlasDims>,
    tally: &SlotTally,
) {
    diag::info!(
        World,
        "smodel lighting: atlas={}x{}x4 entry_limit={} resolved_samples={} \
         lit_candidate_slots={} (technique+sample; sample_without_technique={} \
         technique_without_sample={}; retained path — handled in immediates, no Mesh3d)",
        lighting_t6::MODEL_LIGHTING_ATLAS_WIDTH,
        dims.map_or(0, |dims| dims.image_height),
        dims.map_or(0, |dims| dims.smodel_entry_limit),
        scene.smodel_lighting_samples.len(),
        tally.lit,
        tally.sample_without_technique,
        tally.technique_without_sample,
    );
}

fn place_script_models(
    commands: &mut Commands,
    instances: Vec<WorldScriptModelInstance>,
    scene_assets: &asset_world::MapXModelSceneCatalog,
) {
    let (mut ready, mut unavailable, mut tagged) = (0usize, 0usize, 0usize);
    for instance in instances {
        if matches!(
            scene_assets.get(&instance.current_model),
            Some(asset_world::MapXModelSceneAsset::T5(_))
        ) {
            ready += 1;
        } else {
            unavailable += 1;
        }
        let gameobject = instance.metadata.gameobject.clone();
        let tag = (!gameobject.is_empty()).then_some(ScriptModelGameObject(gameobject));
        tagged += usize::from(tag.is_some());
        let mut entity = commands.spawn((
            instance.transform,
            Visibility::Inherited,
            ScriptModelEntity,
            instance,
        ));
        if let Some(tag) = tag {
            entity.insert(tag);
        }
    }
    diag::info!(
        World,
        "script models: ready={ready} unavailable={unavailable} gameobject-tagged={tagged} script_smodel_placements=0"
    );
}

fn place_dyn_ents(
    commands: &mut Commands,
    instances: Vec<WorldDynEntInstance>,
    scene_assets: &asset_world::MapXModelSceneCatalog,
    brushes_undrawn: usize,
) {
    let (mut ready, mut missing) = (0usize, 0usize);
    for instance in instances {
        if matches!(
            scene_assets.get(&instance.current_model),
            Some(asset_world::MapXModelSceneAsset::T5(_))
        ) {
            ready += 1;
        } else {
            missing += 1;
        }
        commands.spawn((
            instance.transform,
            Visibility::Inherited,
            DynEntModelEntity,
            instance,
        ));
    }
    diag::info!(
        World,
        "dynents: spawned_ready={ready} catalog_miss={missing} brushes_undrawn={brushes_undrawn}"
    );
}

/// The fly camera host (above the map's middle, or at the intermission view)
/// with the first-person lens as its child.
fn spawn_camera_rig(commands: &mut Commands, scene: &WorldScene) {
    let transform = match scene.intermission_view {
        Some(view) => transform_from_iw_view(view),
        None => {
            let radius = scene.radius;
            let eye = scene.center + Vec3::new(radius * 0.15, -radius * 0.45, radius * 0.06);
            let target = scene.center + Vec3::new(0.0, radius * 0.2, 0.0);
            Transform::from_translation(eye).looking_at(target, Vec3::Z)
        }
    };
    let (yaw, pitch, _) = transform.rotation.to_euler(EulerRot::ZYX);
    let speed = if scene.intermission_view.is_some() {
        INTERMISSION_FLY_SPEED
    } else {
        scene.radius * 0.25
    };

    let host = commands
        .spawn((
            transform,
            FlyCamera { yaw, pitch, speed },
            audio::AmbientListener,
            Visibility::Inherited,
        ))
        .id();
    let lens = commands
        .spawn((
            FpvLens,
            Camera3d {
                depth_texture_usages: (TextureUsages::RENDER_ATTACHMENT
                    | TextureUsages::TEXTURE_BINDING)
                    .into(),
                ..default()
            },
            CompositingSpace::Srgb,
            Tonemapping::None,
            Msaa::Off,
            Transform::IDENTITY,
            Projection::Perspective(PerspectiveProjection {
                fov: vertical_fov_deg(CG_FOV_DEFAULT).to_radians(),
                near: CAMERA_NEAR_INCHES,
                far: camera_far_inches(scene.radius),
                ..default()
            }),
        ))
        .id();
    commands.entity(host).add_child(lens);
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use asset_core::AssetNamespace;
    use bevy::asset::RenderAssetUsages;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::render::render_resource::PrimitiveTopology;
    use render_material::{
        MaterialAssetId, MaterialGenerationId, RemapResolution, RuntimeMaterial,
        RuntimeMaterialCatalog, RuntimeTechniqueSetId, TECHNIQUE_SLOT_COUNT,
    };

    use render_scene::WorldCameraPose;
    use super::*;
    use crate::prepare::scene::world::{
        WorldSmodelLightingSample, WorldStaticModelMesh, WorldStaticModelSurface,
    };

    fn near(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-3
    }

    fn material(id: u16, takes_model_lighting: bool) -> RuntimeMaterial {
        RuntimeMaterial {
            asset_id: MaterialAssetId(id),
            name: format!("m{id}"),
            namespace: AssetNamespace::T6,
            technique_set: String::new(),
            baked_draw_surf: None,
            local_technique_set: RuntimeTechniqueSetId(0),
            remap: RemapResolution::SelfSet,
            state_bits_entry: None::<[u8; TECHNIQUE_SLOT_COUNT]>,
            state_bits_table: Vec::new(),
            camera_region: 0,
            sort_key: 0,
            info_game_flags: 0,
            state_flags: 0,
            surface_type_bits: None,
            unlit: false,
            uses_model_lighting_const: takes_model_lighting,
            square_color_map: false,
            shadow_only: false,
            t6_draw: None,
            cull_mode: None,
            uv_anim_bits: [0; 4],
            falloff_parms_bits: [0; 4],
            falloff_begin_bits: [0; 4],
            falloff_end_bits: [0; 4],
            env_map_parms_bits: [0; 4],
            textures: Vec::new(),
            constants: Vec::new(),
        }
    }

    fn surface(material: Option<assets::MaterialIndex>) -> WorldStaticModelSurface {
        WorldStaticModelSurface {
            mesh: Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default()),
            material,
            packed_vertices: Vec::new(),
            xsurface_plus_1: None,
            xsurface_base_index: 0,
            xsurface_vert_offset: 0,
            collision: asset_world::XSurfaceCollisionPayload::Unavailable {
                source_layout: "test",
            },
        }
    }

    fn mesh(name: &str, surfaces: Vec<WorldStaticModelSurface>) -> WorldStaticModelMesh {
        WorldStaticModelMesh {
            name: name.to_owned(),
            lod_surfaces: [surfaces, Vec::new(), Vec::new(), Vec::new()],
            lod_smc: None,
            lod: None,
        }
    }

    fn instance(mesh: usize, x: f32) -> WorldStaticModelInstance {
        WorldStaticModelInstance {
            mesh,
            transform: Transform::from_xyz(x, 0.0, 0.0),
            origin: [x, 0.0, 0.0],
            axis: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            scale: 1.0,
            cull_dist: 700,
            reflection_probe_index: 0,
            primary_light_index: 0,
            flags: 0,
            packed_lighting: None,
            t6_primary: [0, 0],
            t6_light: 0,
        }
    }

    fn sample(slot: usize) -> WorldSmodelLightingSample {
        WorldSmodelLightingSample {
            authored_slot: slot,
            lighting_origin: [0.0; 3],
            tile_rgba: [7; 256],
            packed_lighting: [7, 0, 0, 0xff],
        }
    }

    fn lit_scene(takes_model_lighting: bool) -> WorldScene {
        WorldScene {
            runtime_material_catalog: Arc::new(RuntimeMaterialCatalog {
                generation_id: MaterialGenerationId(4),
                materials: vec![material(0, takes_model_lighting)],
                ..RuntimeMaterialCatalog::default()
            }),
            t6_lights: Arc::new(vec![[0.0; 16]]),
            static_model_meshes: vec![mesh(
                "rock",
                vec![surface(Some(assets::MaterialIndex::from_order(0)))],
            )],
            static_model_instances: vec![Some(instance(0, 10.0))],
            smodel_lighting_samples: vec![sample(0)],
            ..WorldScene::default()
        }
    }

    fn place_scene(scene: WorldScene) -> World {
        let mut world = World::new();
        world.init_resource::<Assets<Image>>();
        world.insert_resource(scene);
        world
            .run_system_once(
                |mut commands: Commands,
                 mut scene: ResMut<WorldScene>,
                 mut images: ResMut<Assets<Image>>| {
                    place(&mut commands, &mut scene, &mut images, Vec::new(), Vec::new(), Vec::new());
                },
            )
            .unwrap();
        world
    }

    fn lighting_switched_on() -> bool {
        !matches!(
            std::env::var("BO2MP_SMODEL_LIGHTING").as_deref(),
            Ok("0" | "false" | "off" | "no")
        )
    }

    fn count<C: Component>(world: &mut World) -> usize {
        world.query_filtered::<Entity, With<C>>().iter(world).count()
    }

    #[test]
    fn the_far_plane_is_eight_radii_out_but_never_nearer_than_a_hundred_near_planes() {
        assert_eq!(camera_far_inches(1000.0), 8000.0);
        assert_eq!(camera_far_inches(25.0), 200.0);
        assert_eq!(camera_far_inches(0.0), CAMERA_NEAR_INCHES * 100.0);
    }

    #[test]
    fn an_empty_world_still_gets_a_camera_and_no_lighting_atlas() {
        let mut world = place_scene(WorldScene::default());
        assert_eq!(count::<FlyCamera>(&mut world), 1);
        assert_eq!(count::<FpvLens>(&mut world), 1);
        assert!(world.get_resource::<WorldModelLightingAtlas>().is_none());
        assert!(world.get_resource::<WorldModelLightingCache>().is_none());
        assert!(world.get_resource::<WorldSmodelLighting>().is_none());
        assert!(world.get_resource::<WorldStaticModelCache>().is_none());
        let scene = world.resource::<WorldScene>();
        assert!(scene.model_lighting_image.is_none() && scene.model_lighting_dims.is_none());
    }

    #[test]
    fn the_camera_starts_above_the_middle_of_the_map_looking_at_it() {
        let scene = WorldScene {
            center: Vec3::new(100.0, 200.0, 50.0),
            radius: 1000.0,
            ..WorldScene::default()
        };
        let mut world = place_scene(scene);
        let (host, fly) = world
            .query::<(&Transform, &FlyCamera)>()
            .single(&world)
            .map(|(t, f)| (*t, (f.speed, f.yaw, f.pitch)))
            .unwrap();
        assert!(near(host.translation, Vec3::new(250.0, -250.0, 110.0)));
        assert_eq!(fly.0, 250.0);
        // It faces the point a fifth of a radius past the middle.
        let target = Vec3::new(100.0, 400.0, 50.0);
        assert!(near(*host.forward(), (target - host.translation).normalize()));
    }

    #[test]
    fn the_intermission_view_sets_the_cameras_pose_and_fly_speed() {
        let pose = WorldCameraPose {
            origin: [10.0, 20.0, 30.0],
            angles: [5.0, 90.0, 0.0],
        };
        let scene = WorldScene {
            intermission_view: Some(pose),
            radius: 4000.0,
            ..WorldScene::default()
        };
        let mut world = place_scene(scene);
        let (host, speed) = world
            .query::<(&Transform, &FlyCamera)>()
            .single(&world)
            .map(|(t, f)| (*t, f.speed))
            .unwrap();
        let expected = transform_from_iw_view(pose);
        assert!(near(host.translation, expected.translation));
        assert!(host.rotation.angle_between(expected.rotation) < 1e-4);
        assert_eq!(speed, INTERMISSION_FLY_SPEED);
    }

    #[test]
    fn the_lens_is_the_hosts_child_with_planes_that_fit_the_map() {
        let scene = WorldScene {
            radius: 1000.0,
            ..WorldScene::default()
        };
        let mut world = place_scene(scene);
        let host = world
            .query_filtered::<Entity, With<FlyCamera>>()
            .single(&world)
            .unwrap();
        let (lens_parent, projection) = world
            .query_filtered::<(&ChildOf, &Projection), With<FpvLens>>()
            .single(&world)
            .map(|(parent, projection)| (parent.parent(), projection.clone()))
            .unwrap();
        assert_eq!(lens_parent, host);
        let Projection::Perspective(perspective) = projection else {
            panic!("the lens is not a perspective camera");
        };
        assert_eq!(perspective.near, CAMERA_NEAR_INCHES);
        assert_eq!(perspective.far, 8000.0);
    }

    #[test]
    fn the_camera_listens_for_ambient_sound() {
        let mut world = place_scene(WorldScene::default());
        assert_eq!(count::<audio::AmbientListener>(&mut world), 1);
    }

    #[test]
    fn placing_hands_back_the_instances_and_drops_what_tessellation_used() {
        let scene = WorldScene {
            static_model_instances: vec![None, None],
            ..WorldScene::default()
        };
        let world = place_scene(scene);
        let scene = world.resource::<WorldScene>();
        assert_eq!(scene.static_model_instances.len(), 2);
        assert!(scene.batches.is_empty());
        assert!(scene.script_model_instances.is_empty());
        assert!(scene.dyn_ent_instances.is_empty());
    }

    #[test]
    fn a_world_with_lights_gets_a_model_lighting_atlas_and_its_registry() {
        let scene = WorldScene {
            runtime_material_catalog: Arc::new(RuntimeMaterialCatalog {
                generation_id: MaterialGenerationId(4),
                ..RuntimeMaterialCatalog::default()
            }),
            t6_lights: Arc::new(vec![[0.0; 16]]),
            exact_material_names: vec!["a".to_owned()],
            ..WorldScene::default()
        };
        let world = place_scene(scene);
        let atlas = world.resource::<WorldModelLightingAtlas>();
        let scene = world.resource::<WorldScene>();
        assert_eq!(scene.model_lighting_image.as_ref().map(Handle::id), Some(atlas.image.id()));
        assert_eq!(scene.model_lighting_dims.map(|d| d.image_height), Some(atlas.dims.image_height));
        assert!(world.contains_resource::<WorldModelLightingCache>());

        let handles = world.resource::<RuntimeImageHandles>();
        assert_eq!(handles.generation_id, MaterialGenerationId(4));
        assert_eq!(handles.material_names, ["a"]);
        assert_eq!(handles.model_lighting.as_ref().map(Handle::id), Some(atlas.image.id()));
    }

    #[test]
    fn slots_that_resolve_to_no_model_spawn_nothing_and_are_counted_unresolved() {
        let scene = WorldScene {
            static_model_meshes: vec![mesh("empty", Vec::new()), mesh("rock", vec![surface(None)])],
            // No instance, a model with no surfaces, a model out of range.
            static_model_instances: vec![None, Some(instance(0, 1.0)), Some(instance(9, 2.0))],
            ..WorldScene::default()
        };
        let mut world = place_scene(scene);
        assert_eq!(count::<StaticModelEntity>(&mut world), 0);
        // The geometry cache is still sized for every authored slot.
        let cache = world.resource::<WorldStaticModelCache>();
        assert_eq!(cache.cache_index.len(), 3);
    }

    #[test]
    fn a_resolved_slot_is_a_hidden_entity_at_its_transform() {
        let scene = WorldScene {
            static_model_meshes: vec![mesh("rock", vec![surface(None)]), mesh("tree", vec![surface(None)])],
            static_model_instances: vec![Some(instance(1, 33.0)), None],
            ..WorldScene::default()
        };
        let mut world = place_scene(scene);
        assert_eq!(count::<StaticModelEntity>(&mut world), 1);
        let (transform, visibility) = world
            .query_filtered::<(&Transform, &Visibility), With<StaticModelEntity>>()
            .single(&world)
            .map(|(t, v)| (*t, *v))
            .unwrap();
        assert_eq!(transform.translation, Vec3::new(33.0, 0.0, 0.0));
        assert_eq!(visibility, Visibility::Hidden);
        assert_eq!(
            world.resource::<WorldScene>().smodel_mesh_names,
            ["rock", "tree"]
        );
    }

    #[test]
    fn a_sampled_slot_whose_material_takes_model_lighting_is_lit_from_the_start() {
        if !lighting_switched_on() {
            return;
        }
        let mut world = place_scene(lit_scene(true));
        let lighting = world.resource::<WorldSmodelLighting>();
        assert_eq!(lighting.spawn_lit_n, 1);
        assert_eq!(lighting.spawn_sample_without_technique, 0);
        let parent = lighting.parents[0].expect("lit slot has its entity");
        assert!(world.get::<StaticModelEntity>(parent).is_some());
    }

    #[test]
    fn a_sample_whose_material_does_not_take_model_lighting_is_counted_but_not_lit() {
        if !lighting_switched_on() {
            return;
        }
        let mut world = place_scene(lit_scene(false));
        let lighting = world.resource::<WorldSmodelLighting>();
        assert_eq!(lighting.spawn_lit_n, 0);
        assert_eq!(lighting.spawn_sample_without_technique, 1);
        assert!(lighting.parents[0].is_none());
        // The model is still placed, just not lit through the atlas.
        assert_eq!(count::<StaticModelEntity>(&mut world), 1);
    }

    fn script_instance(ordinal: u32, gameobject: &str) -> WorldScriptModelInstance {
        WorldScriptModelInstance {
            id: asset_world::ScriptModelId::from_source_ordinal(ordinal),
            authority_owner: None,
            current_model: asset_world::MapXModelAssetKey("model".to_owned()),
            transform: Transform::from_xyz(ordinal as f32, 0.0, 0.0),
            lighting_origin: [0.0; 3],
            dobj_state: xmodel_runtime::DObjSemanticState::bind_pose("model".to_owned(), 1, 1),
            metadata: asset_world::ScriptModelMetadata {
                gameobject: gameobject.to_owned(),
                ..asset_world::ScriptModelMetadata::default()
            },
            gentity_number: None,
        }
    }

    #[test]
    fn each_script_model_gets_an_entity_and_only_game_objects_are_tagged() {
        let mut world = World::new();
        world
            .run_system_once(|mut commands: Commands| {
                place_script_models(
                    &mut commands,
                    vec![script_instance(0, ""), script_instance(1, "flag"), script_instance(2, "")],
                    &asset_world::MapXModelSceneCatalog::default(),
                );
            })
            .unwrap();
        assert_eq!(count::<ScriptModelEntity>(&mut world), 3);
        let tags: Vec<_> = world
            .query::<&ScriptModelGameObject>()
            .iter(&world)
            .map(|tag| tag.0.clone())
            .collect();
        assert_eq!(tags, ["flag"]);
        let mut moved: Vec<_> = world
            .query_filtered::<&Transform, With<ScriptModelEntity>>()
            .iter(&world)
            .map(|t| t.translation.x)
            .collect();
        moved.sort_by(f32::total_cmp);
        assert_eq!(moved, [0.0, 1.0, 2.0]);
    }

    #[test]
    fn each_dynamic_entity_gets_an_entity_of_its_own() {
        let mut world = World::new();
        world
            .run_system_once(|mut commands: Commands| {
                let instances = (0..4u16)
                    .map(|index| WorldDynEntInstance {
                        index,
                        ty: asset_world::DynEntType::Clutter,
                        current_model: asset_world::MapXModelAssetKey("crate".to_owned()),
                        transform: Transform::from_xyz(f32::from(index), 0.0, 0.0),
                        lighting_origin: [0.0; 3],
                        phys_preset: None,
                        health: 10,
                        destroy_fx: None,
                        dead: false,
                    })
                    .collect();
                place_dyn_ents(
                    &mut commands,
                    instances,
                    &asset_world::MapXModelSceneCatalog::default(),
                    2,
                );
            })
            .unwrap();
        assert_eq!(count::<DynEntModelEntity>(&mut world), 4);
        let indices: Vec<_> = world
            .query::<&WorldDynEntInstance>()
            .iter(&world)
            .map(|dyn_ent| dyn_ent.index)
            .collect();
        assert_eq!(indices, [0, 1, 2, 3]);
    }
}
