//! Hands a finished level over to the ECS: hidden parents for the static
//! props, entities for script models and debris, the model-lighting atlas
//! with its image registry, and the fly camera carrying the player's lens.

use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;
use bevy::render::render_resource::TextureUsages;
use bevy::render::view::Msaa;
use hud_t6::fov::{STOCK_VIEW_CONE_DEGREES, vertical_fov_deg};
use lighting_t6::ModelIllumAtlasDims;

use asset_world::{LevelModelAssetId, LevelModelPlacementAsset, LevelModelPlacementRoster};

use crate::assemble::draw_items::{LiveImageHandles, LiveLightbakeHandles};
use super::camera::FirstPersonLens;
use super::camera::FreeRoamCamera;
use super::camera::transform_from_game_view;
use super::cull::{
    DynamicPropModelEntity, PropEntity, ScriptModelActorItem, ScriptedPropEntity,
};
use crate::prepare::scene::model_illum_cache::LevelModelIllumCache;
use crate::prepare::scene::model_illum_atlas::LevelModelIllumAtlas;
use crate::prepare::scene::prop_geom_memo::LevelPropCache;
use crate::prepare::scene::prop_light_slots::{
    LevelPropIllum, facets_take_model_illum, prepare_prop_illum, slot_has_lighting_sample,
    seat_is_lit_candidate,
};
use crate::{
    LevelDebrisInstance, FixedPropPlacement, LevelScriptModelInstance, LevelStage,
    PROP_ILLUM_MAX_CLIENT_VIEWS,
};

/// Distance from the eye to the lens's near plane.
pub(crate) const LENS_NEAR: f32 = 2.0;

/// How fast the camera cruises when it starts at the scoreboard view.
const SCOREBOARD_CRUISE: f32 = 320.0;

/// The far plane sits eight map radii away, and never closer than a hundred
/// near-plane distances so a tiny map still gets usable depth.
pub(crate) fn lens_far_plane(map_radius: f32) -> f32 {
    let floor = LENS_NEAR * 100.0;
    f32::max(map_radius * 8.0, floor)
}

/// Counts gathered while the static props are given entities.
#[derive(Default)]
struct PropCounts {
    unresolved: usize,
    lit: usize,
    sample_without_technique: usize,
    technique_without_sample: usize,
}

/// The texture handles the image registry is built from.
pub struct RegistryHandles {
    pub precise_materials: Vec<Option<Handle<Image>>>,
    pub env_probes: Vec<Option<Handle<Image>>>,
    pub lightbakes: Vec<Option<LiveLightbakeHandles>>,
}

/// Spawns everything a loaded level needs in the world. The tessellated
/// batches are released first because nothing after this reads them.
pub fn populate_world(
    commands: &mut Commands,
    scene: &mut LevelStage,
    gpu_images: &mut Assets<Image>,
    handles: RegistryHandles,
) {
    scene.batches = Vec::new();

    let props = std::mem::take(&mut scene.prop_instances);
    let roster = std::mem::take(&mut scene.map_figure_stage_assets);
    let scripted = std::mem::take(&mut scene.scripted_prop_placements);
    let debris = std::mem::take(&mut scene.debris_instances);

    // Black Ops II lights its models in the fallback pass; the atlas is only
    // needed so the first-person rig has something to bind.
    let level_is_lit = scene.light_grid.is_some() || !scene.t6_lights.is_empty();
    let atlas = if level_is_lit {
        lighting_t6::illum_atlas_dims(PROP_ILLUM_MAX_CLIENT_VIEWS)
            .map(|dims| LevelModelIllumAtlas::new(gpu_images, dims))
    } else {
        None
    };

    if !scene.prop_meshes.is_empty() {
        spawn_props(commands, scene, &props, atlas.as_ref());
    }
    scene.prop_instances = props;

    spawn_scripted_models(commands, scripted, &roster);
    spawn_debris(commands, debris, &roster, scene.debris_solid_tally);
    commands.insert_resource(roster);

    if let Some(atlas) = atlas {
        install_lighting_atlas(commands, scene, atlas, handles);
    }

    spawn_camera_rig(commands, scene);
}

/// Publishes the atlas: on the level, in the image registry, and as the
/// resources the lighting cache and patch upload read.
fn install_lighting_atlas(
    commands: &mut Commands,
    scene: &mut LevelStage,
    lit_atlas: LevelModelIllumAtlas,
    handles: RegistryHandles,
) {
    let atlas_image = lit_atlas.image.clone();
    scene.model_illum_image = Some(atlas_image.clone());
    scene.model_illum_dims = Some(lit_atlas.dims);

    let registry = LiveImageHandles::build_from_stores(
        scene.live_material_ledger.revision_stamp,
        handles.precise_materials,
        scene.precise_material_names.clone(),
        handles.env_probes,
        handles.lightbakes,
        Some(atlas_image),
    );
    commands.insert_resource(registry);
    diag::info!(
        World,
        "drawsurf image registry typed: model lighting READY, code texture 3, sampler 0xe2, volume D3"
    );
    commands.insert_resource(LevelModelIllumCache::new(lit_atlas.dims));
    commands.insert_resource(lit_atlas);
}

/// Gives every static-prop slot that names a real model a hidden parent
/// entity, and builds the lighting table and geometry cache that go with them.
fn spawn_props(
    commands: &mut Commands,
    scene: &mut LevelStage,
    slots: &[Option<FixedPropPlacement>],
    atlas: Option<&LevelModelIllumAtlas>,
) {
    let mut mesh_names = Vec::with_capacity(scene.prop_meshes.len());
    let mut materials_by_mesh: Vec<Vec<Option<assets::SurfaceMaterialId>>> =
        Vec::with_capacity(scene.prop_meshes.len());
    for prop_mesh in &scene.prop_meshes {
        let facet_total: usize = prop_mesh.detail_facets.iter().map(Vec::len).sum();
        diag::info!(World, "static model mesh: {} with {facet_total} surfaces", prop_mesh.name);
        mesh_names.push(prop_mesh.name.clone());
        materials_by_mesh.push(prop_mesh.facet_materials().collect());
    }
    scene.prop_mesh_names = mesh_names;

    let slot_count = slots.len();
    let mut illum = match atlas {
        Some(atlas) => prepare_prop_illum(atlas.dims, &scene.prop_illum_samples, slot_count),
        None => None,
    };
    let mut parents: Vec<Option<Entity>> = std::iter::repeat_n(None, slot_count).collect();
    let mut fade_out: Vec<u16> = std::iter::repeat_n(0, slot_count).collect();
    let mut counts = PropCounts::default();

    for (slot, placement) in slots.iter().enumerate() {
        let resolved = placement.as_ref().and_then(|placement| {
            materials_by_mesh
                .get(placement.mesh)
                .filter(|facets| !facets.is_empty())
                .map(|facets| (placement, facets))
        });
        let Some((placement, facets)) = resolved else {
            counts.unresolved += 1;
            continue;
        };

        let wants_atlas =
            facets_take_model_illum(facets.iter().copied(), &scene.live_material_ledger);
        let sampled = illum
            .as_ref()
            .is_some_and(|table| slot_has_lighting_sample(table, slot));
        if sampled != wants_atlas {
            if sampled {
                counts.sample_without_technique += 1;
            } else {
                counts.technique_without_sample += 1;
            }
        }

        let hidden = Visibility::Hidden;
        let holder = commands.spawn((placement.transform, hidden, PropEntity)).id();
        let lit_table = illum
            .as_mut()
            .filter(|table| seat_is_lit_candidate(table, slot, wants_atlas));
        if let Some(table) = lit_table {
            table.parent_list[slot] = Some(holder);
            counts.lit += 1;
        }
        parents[slot] = Some(holder);
        fade_out[slot] = placement.hide_dist;
    }

    if let Some(mut table) = illum {
        table.spawn_lit_tally = counts.lit as u32;
        table.spawn_sample_without_method = counts.sample_without_technique as u32;
        report_prop_lighting(scene, atlas.map(|atlas| atlas.dims), &counts);
        commands.insert_resource::<LevelPropIllum>(table);
    } else if !scene.prop_illum_samples.is_empty() {
        let entry_limit = lighting_t6::illum_atlas_dims(PROP_ILLUM_MAX_CLIENT_VIEWS)
            .map(|dims| dims.prop_entry_limit);
        diag::info!(
            World,
            "smodel lighting samples: resolved={} concurrent_entry_limit={entry_limit:?}, \
             left unlit because BO2MP_SMODEL_LIGHTING is off",
            scene.prop_illum_samples.len()
        );
    }

    commands.insert_resource(LevelPropCache::new(slot_count));
    diag::info!(
        World,
        "static models: {slot_count} authored slots, {} with no model; assemble does the tessellation",
        counts.unresolved
    );
    if let Some(hide) = &mut scene.culling {
        hide.prop_entities = parents;
        hide.prop_hide_dists = fade_out;
        hide.prop_vis.resize(slot_count, 0);
    }
}

fn report_prop_lighting(scene: &LevelStage, dims: Option<ModelIllumAtlasDims>, counts: &PropCounts) {
    let (height, entry_limit) = dims.map_or((0, 0), |dims| (dims.rows_tall, dims.prop_entry_limit));
    diag::info!(
        World,
        "smodel lighting: atlas={}x{height}x4 entry_limit={entry_limit} resolved_samples={} \
         lit_candidate_slots={} sample_without_technique={} technique_without_sample={}; \
         drawn by the retained immediates path, not as Mesh3d",
        lighting_t6::MODEL_LIGHT_ATLAS_TEXELS_ACROSS,
        scene.prop_illum_samples.len(),
        counts.lit,
        counts.sample_without_technique,
        counts.technique_without_sample,
    );
}

/// True when the roster already holds a drawable model under this id.
fn roster_can_draw(roster: &LevelModelPlacementRoster, id: &LevelModelAssetId) -> bool {
    matches!(roster.get(id), Some(LevelModelPlacementAsset::T5(_)))
}

/// One entity per script model; the ones a game object refers to also carry
/// that object's name.
fn spawn_scripted_models(
    commands: &mut Commands,
    placements: Vec<LevelScriptModelInstance>,
    roster: &LevelModelPlacementRoster,
) {
    let total = placements.len();
    let mut drawable = 0usize;
    let mut named = 0usize;
    for placement in placements {
        drawable += usize::from(roster_can_draw(roster, &placement.active_mesh));
        let owner = &placement.metadata.script_entity;
        let actor = (!owner.is_empty()).then(|| ScriptModelActorItem(owner.clone()));
        let pose = placement.place;
        let mut spawned = commands.spawn((pose, Visibility::default(), ScriptedPropEntity, placement));
        if let Some(actor) = actor {
            named += 1;
            spawned.insert(actor);
        }
    }
    let missing = total - drawable;
    diag::info!(
        World,
        "script models: ready={drawable} unavailable={missing} gameobject-tagged={named} script_smodel_placements=0"
    );
}

/// One entity per dynamic prop, whether or not its model is loaded yet.
fn spawn_debris(
    commands: &mut Commands,
    pieces: Vec<LevelDebrisInstance>,
    roster: &LevelModelPlacementRoster,
    brushes_undrawn: usize,
) {
    let total = pieces.len();
    let mut drawable = 0usize;
    for piece in pieces {
        drawable += usize::from(roster_can_draw(roster, &piece.active_mesh));
        let pose = piece.place;
        commands.spawn((pose, Visibility::default(), DynamicPropModelEntity, piece));
    }
    let catalog_miss = total - drawable;
    diag::info!(
        World,
        "dynents: spawned_ready={drawable} catalog_miss={catalog_miss} brushes_undrawn={brushes_undrawn}"
    );
}

/// Where the camera starts when the level has no scoreboard view: up and
/// off to one side of the middle, looking a little past it.
fn overview_pose(middle: Vec3, map_radius: f32) -> Transform {
    let offset = Vec3::new(0.15, -0.45, 0.06) * map_radius;
    let focus = middle + Vec3::Y * (map_radius * 0.2);
    Transform::from_translation(middle + offset).looking_at(focus, Vec3::Z)
}

/// Spawns the fly-camera host and hangs the first-person lens under it.
fn spawn_camera_rig(commands: &mut Commands, scene: &LevelStage) {
    let (start, cruise) = match scene.scoreboard_eye {
        Some(view) => (transform_from_game_view(view), SCOREBOARD_CRUISE),
        None => (overview_pose(scene.mid, scene.rad), scene.rad / 4.0),
    };
    let (yaw, pitch, _roll) = start.rotation.to_euler(EulerRot::ZYX);
    let host = commands
        .spawn((
            start,
            FreeRoamCamera { turn_deg: yaw, tilt: pitch, pace: cruise },
            audio::BackgroundHearer,
            Visibility::Inherited,
        ))
        .id();

    let depth_use = TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING;
    let mut body = Camera3d::default();
    body.depth_texture_usages = depth_use.into();
    let tonemap = Tonemapping::None;
    let output = (CompositingSpace::Srgb, tonemap, Msaa::Off);
    let frustum = PerspectiveProjection {
        fov: vertical_fov_deg(STOCK_VIEW_CONE_DEGREES).to_radians(),
        near: LENS_NEAR,
        far: lens_far_plane(scene.rad),
        ..default()
    };
    let lens = commands
        .spawn((FirstPersonLens, body, output, Transform::default(), Projection::Perspective(frustum)))
        .id();
    commands.entity(host).add_child(lens);
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use asset_core::AssetCategoryScope;
    use bevy::asset::RenderAssetUsages;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::render::render_resource::PrimitiveTopology;
    use render_material::{
        MaterialAssetKey, MaterialEpochId, MaterialRemapOutcome, LiveMaterial,
        LiveMaterialLedger, LiveTechniqueBundleKey, RENDER_RECIPE_SLOT_TOTAL,
    };

    use render_scene::WorldEyePlacement;
    use super::*;
    use crate::{
        StaticPropLightSample, LevelPropMesh, LevelPropFacet,
    };

    fn near(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-3
    }

    fn material(id: u16, takes_model_lighting: bool) -> LiveMaterial {
        let cull_mode = None;
        let sort_key = 0;
        LiveMaterial {
            asset_id: MaterialAssetKey(id),
            name: format!("m{id}"),
            namespace: AssetCategoryScope::T6,
            method_set: String::new(),
            baked_facet_draw: None,
            local_technique_set: LiveTechniqueBundleKey(0),
            remap: MaterialRemapOutcome::SelfSet,
            state_row_for: None::<[u8; RENDER_RECIPE_SLOT_TOTAL]>,
            state_rows: Vec::new(),
            camera_zone: 0,
            draw_sort_key: sort_key,
            game_flag_info: 0,
            condition_bits: 0,
            facet_type_bits: None,
            unlit: false,
            uses_model_illum_const: takes_model_lighting,
            colour_map_is_square: false,
            occluder_exclusive: false,
            t6_draw: None,
            cull_mode,
            uv_scroll_bits: [0; 4],
            attenuation_terms_raw: [0; 4],
            attenuation_start_raw: [0; 4],
            attenuation_stop_raw: [0; 4],
            reflection_map_param_bits: [0; 4],
            sheet_bits: [0; 5],
            textures: Vec::new(),
            constants: Vec::new(),
        }
    }

    fn surface(material: Option<assets::SurfaceMaterialId>) -> LevelPropFacet {
        LevelPropFacet {
            mesh_h: Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default()),
            material,
            compact_vertices: Vec::new(),
            figure_part_plus_1: None,
            figure_part_base_index: 0,
            figure_part_vertex_offset: 0,
            collision: asset_world::XSurfaceCollisionPayload::Unavailable {
                origin_layout: "test",
            },
        }
    }

    fn mesh(name: &str, surfaces: Vec<LevelPropFacet>) -> LevelPropMesh {
        LevelPropMesh {
            name: name.to_owned(),
            detail_facets: [surfaces, vec![], vec![], vec![]],
            detail_prop_cache: None,
            lod: None,
        }
    }

    fn instance(mesh: usize, x: f32) -> FixedPropPlacement {
        FixedPropPlacement {
            mesh,
            transform: Transform::from_xyz(x, 0.0, 0.0),
            origin: [x, 0.0, 0.0],
            axis: Mat3::IDENTITY.to_cols_array_2d(),
            scale: 1.0,
            hide_dist: 700,
            env_probe_index: 0,
            key_light_index: 0,
            flags: 0,
            compact_illum: None,
            t6_primary: [0, 0],
            t6_light: 0,
        }
    }

    fn sample(slot: usize) -> StaticPropLightSample {
        StaticPropLightSample {
            designer_slot: slot,
            illum_origin: [0.0; 3],
            tile_px: [7; 256],
            compact_illum: [7, 0, 0, 0xff],
        }
    }

    fn lit_scene(takes_model_lighting: bool) -> LevelStage {
        LevelStage {
            live_material_ledger: Arc::new(LiveMaterialLedger {
                revision_stamp: MaterialEpochId(4),
                materials: vec![material(0, takes_model_lighting)],
                ..LiveMaterialLedger::default()
            }),
            t6_lights: Arc::new(vec![[0.0; 16]]),
            prop_meshes: vec![mesh(
                "rock",
                vec![surface(Some(assets::SurfaceMaterialId::from_sequence(0)))],
            )],
            prop_instances: vec![Some(instance(0, 10.0))],
            prop_illum_samples: vec![sample(0)],
            ..LevelStage::default()
        }
    }

    fn place_scene(scene: LevelStage) -> World {
        let mut world = World::new();
        world.init_resource::<Assets<Image>>();
        world.insert_resource(scene);
        world
            .run_system_once(
                |mut commands: Commands,
                 mut scene: ResMut<LevelStage>,
                 mut gpu_images: ResMut<Assets<Image>>| {
                    populate_world(&mut commands, &mut scene, &mut gpu_images, no_handles());
                },
            )
            .unwrap();
        world
    }

    fn no_handles() -> RegistryHandles {
        RegistryHandles { precise_materials: Vec::new(), env_probes: Vec::new(), lightbakes: Vec::new() }
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
        assert_eq!(lens_far_plane(1000.0), 8000.0);
        assert_eq!(lens_far_plane(25.0), 200.0);
        assert_eq!(lens_far_plane(0.0), LENS_NEAR * 100.0);
    }

    #[test]
    fn an_empty_world_still_gets_a_camera_and_no_lighting_atlas() {
        let mut world = place_scene(LevelStage::default());
        assert_eq!(count::<FreeRoamCamera>(&mut world), 1);
        assert_eq!(count::<FirstPersonLens>(&mut world), 1);
        assert!(world.get_resource::<LevelModelIllumAtlas>().is_none());
        assert!(world.get_resource::<LevelModelIllumCache>().is_none());
        assert!(world.get_resource::<LevelPropIllum>().is_none());
        assert!(world.get_resource::<LevelPropCache>().is_none());
        let scene = world.resource::<LevelStage>();
        assert!(scene.model_illum_image.is_none() && scene.model_illum_dims.is_none());
    }

    #[test]
    fn the_camera_starts_above_the_middle_of_the_map_looking_at_it() {
        let scene = LevelStage {
            mid: Vec3::new(100.0, 200.0, 50.0),
            rad: 1000.0,
            ..LevelStage::default()
        };
        let mut world = place_scene(scene);
        let (host, fly) = world
            .query::<(&Transform, &FreeRoamCamera)>()
            .single(&world)
            .map(|(t, f)| (*t, (f.pace, f.turn_deg, f.tilt)))
            .unwrap();
        assert!(near(host.translation, Vec3::new(250.0, -250.0, 110.0)));
        assert_eq!(fly.0, 250.0);
        // It faces the point a fifth of a radius past the middle.
        let target = Vec3::new(100.0, 400.0, 50.0);
        assert!(near(*host.forward(), (target - host.translation).normalize()));
    }

    #[test]
    fn the_intermission_view_sets_the_cameras_pose_and_fly_speed() {
        let pose = WorldEyePlacement {
            origin: [10.0, 20.0, 30.0],
            turn: [5.0, 90.0, 0.0],
        };
        let scene = LevelStage {
            scoreboard_eye: Some(pose),
            rad: 4000.0,
            ..LevelStage::default()
        };
        let mut world = place_scene(scene);
        let (host, speed) = world
            .query::<(&Transform, &FreeRoamCamera)>()
            .single(&world)
            .map(|(t, f)| (*t, f.pace))
            .unwrap();
        let expected = transform_from_game_view(pose);
        assert!(near(host.translation, expected.translation));
        assert!(host.rotation.angle_between(expected.rotation) < 1e-4);
        assert_eq!(speed, SCOREBOARD_CRUISE);
    }

    #[test]
    fn the_lens_is_the_hosts_child_with_planes_that_fit_the_map() {
        let scene = LevelStage {
            rad: 1000.0,
            ..LevelStage::default()
        };
        let mut world = place_scene(scene);
        let host = world
            .query_filtered::<Entity, With<FreeRoamCamera>>()
            .single(&world)
            .unwrap();
        let (lens_parent, projection) = world
            .query_filtered::<(&ChildOf, &Projection), With<FirstPersonLens>>()
            .single(&world)
            .map(|(parent, proj_m)| (parent.parent(), proj_m.clone()))
            .unwrap();
        assert_eq!(lens_parent, host);
        let Projection::Perspective(perspective) = projection else {
            panic!("the lens is not a perspective camera");
        };
        assert_eq!(perspective.near, LENS_NEAR);
        assert_eq!(perspective.far, 8000.0);
    }

    #[test]
    fn the_camera_listens_for_ambient_sound() {
        let mut world = place_scene(LevelStage::default());
        assert_eq!(count::<audio::BackgroundHearer>(&mut world), 1);
    }

    #[test]
    fn placing_hands_back_the_instances_and_drops_what_tessellation_used() {
        let scene = LevelStage {
            prop_instances: vec![None, None],
            ..LevelStage::default()
        };
        let world = place_scene(scene);
        let scene = world.resource::<LevelStage>();
        assert_eq!(scene.prop_instances.len(), 2);
        assert!(scene.batches.is_empty());
        assert!(scene.scripted_prop_placements.is_empty());
        assert!(scene.debris_instances.is_empty());
    }

    #[test]
    fn a_world_with_lights_gets_a_model_lighting_atlas_and_its_registry() {
        let scene = LevelStage {
            live_material_ledger: Arc::new(LiveMaterialLedger {
                revision_stamp: MaterialEpochId(4),
                ..LiveMaterialLedger::default()
            }),
            t6_lights: Arc::new(vec![[0.0; 16]]),
            precise_material_names: vec!["a".to_owned()],
            ..LevelStage::default()
        };
        let world = place_scene(scene);
        let atlas = world.resource::<LevelModelIllumAtlas>();
        let scene = world.resource::<LevelStage>();
        assert_eq!(scene.model_illum_image.as_ref().map(Handle::id), Some(atlas.image.id()));
        assert_eq!(scene.model_illum_dims.map(|d| d.rows_tall), Some(atlas.dims.rows_tall));
        assert!(world.contains_resource::<LevelModelIllumCache>());

        let handles = world.resource::<LiveImageHandles>();
        assert_eq!(handles.revision_stamp, MaterialEpochId(4));
        assert_eq!(handles.surface_titles, ["a"]);
        assert_eq!(handles.model_light.as_ref().map(Handle::id), Some(atlas.image.id()));
    }

    #[test]
    fn slots_that_resolve_to_no_model_spawn_nothing_and_are_counted_unresolved() {
        let scene = LevelStage {
            prop_meshes: vec![mesh("empty", vec![]), mesh("rock", vec![surface(None)])],
            // No instance, a model with no surfaces, a model out of range.
            prop_instances: vec![None, Some(instance(0, 1.0)), Some(instance(9, 2.0))],
            ..LevelStage::default()
        };
        let mut world = place_scene(scene);
        assert_eq!(count::<PropEntity>(&mut world), 0);
        // The geometry cache is still sized for every authored slot.
        let cache = world.resource::<LevelPropCache>();
        assert_eq!(cache.cache_slot_no.len(), 3);
    }

    #[test]
    fn a_resolved_slot_is_a_hidden_entity_at_its_transform() {
        let scene = LevelStage {
            prop_meshes: vec![mesh("rock", vec![surface(None)]), mesh("tree", vec![surface(None)])],
            prop_instances: vec![Some(instance(1, 33.0)), None],
            ..LevelStage::default()
        };
        let mut world = place_scene(scene);
        assert_eq!(count::<PropEntity>(&mut world), 1);
        let (transform, visibility) = world
            .query_filtered::<(&Transform, &Visibility), With<PropEntity>>()
            .single(&world)
            .map(|(t, v)| (*t, *v))
            .unwrap();
        assert_eq!(transform.translation, Vec3::new(33.0, 0.0, 0.0));
        assert_eq!(visibility, Visibility::Hidden);
        assert_eq!(
            world.resource::<LevelStage>().prop_mesh_names,
            ["rock", "tree"]
        );
    }

    #[test]
    fn a_sampled_slot_whose_material_takes_model_lighting_is_lit_from_the_start() {
        if !lighting_switched_on() {
            return;
        }
        let world = place_scene(lit_scene(true));
        let lighting = world.resource::<LevelPropIllum>();
        assert_eq!(lighting.spawn_lit_tally, 1);
        assert_eq!(lighting.spawn_sample_without_method, 0);
        let parent = lighting.parent_list[0].expect("lit slot has its entity");
        assert!(world.get::<PropEntity>(parent).is_some());
    }

    #[test]
    fn a_sample_whose_material_does_not_take_model_lighting_is_counted_but_not_lit() {
        if !lighting_switched_on() {
            return;
        }
        let mut world = place_scene(lit_scene(false));
        let lighting = world.resource::<LevelPropIllum>();
        assert_eq!(lighting.spawn_lit_tally, 0);
        assert_eq!(lighting.spawn_sample_without_method, 1);
        assert!(lighting.parent_list[0].is_none());
        // The model is still placed, just not lit through the atlas.
        assert_eq!(count::<PropEntity>(&mut world), 1);
    }

    fn script_instance(ordinal: u32, script_entity: &str) -> LevelScriptModelInstance {
        LevelScriptModelInstance {
            id: asset_world::ScriptModelId::from_source_ordinal(ordinal),
            server_owned_model_owner: None,
            active_mesh: asset_world::LevelModelAssetId("model".to_owned()),
            place: Transform::from_xyz(ordinal as f32, 0.0, 0.0),
            illum_origin: [0.0; 3],
            puppet_state: xmodel_runtime::RigPoseRecord::neutral_pose("model".to_owned(), 1, 1),
            metadata: asset_world::ScriptMeshDescriptor {
                script_entity: script_entity.to_owned(),
                ..asset_world::ScriptMeshDescriptor::default()
            },
            game_actor_number: None,
        }
    }

    #[test]
    fn each_script_model_gets_an_entity_and_only_game_objects_are_tagged() {
        let mut world = World::new();
        world
            .run_system_once(|mut commands: Commands| {
                spawn_scripted_models(
                    &mut commands,
                    vec![script_instance(0, ""), script_instance(1, "flag"), script_instance(2, "")],
                    &asset_world::LevelModelPlacementRoster::default(),
                );
            })
            .unwrap();
        assert_eq!(count::<ScriptedPropEntity>(&mut world), 3);
        let tags: Vec<_> = world
            .query::<&ScriptModelActorItem>()
            .iter(&world)
            .map(|tag| tag.0.clone())
            .collect();
        assert_eq!(tags, ["flag"]);
        let mut moved: Vec<_> = world
            .query_filtered::<&Transform, With<ScriptedPropEntity>>()
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
                let dead = false;
                let instances = (0..4u16)
                    .map(|index| LevelDebrisInstance {
                        index,
                        ty: asset_world::MovableObjectKind::Clutter,
                        active_mesh: asset_world::LevelModelAssetId("crate".to_owned()),
                        place: Transform::from_xyz(f32::from(index), 0.0, 0.0),
                        illum_origin: [0.0; 3],
                        physics_template: None,
                        hp: 10,
                        destroy_effect: None,
                        gone: dead,
                    })
                    .collect();
                spawn_debris(
                    &mut commands,
                    instances,
                    &asset_world::LevelModelPlacementRoster::default(),
                    2,
                );
            })
            .unwrap();
        assert_eq!(count::<DynamicPropModelEntity>(&mut world), 4);
        let indices: Vec<_> = world
            .query::<&LevelDebrisInstance>()
            .iter(&world)
            .map(|debris_piece| debris_piece.index)
            .collect();
        assert_eq!(indices, [0, 1, 2, 3]);
    }
}
