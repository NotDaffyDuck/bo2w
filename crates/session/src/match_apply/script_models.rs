//! The models a map's scripts can move: which of them the simulation owns, what
//! it traces against for each, and where they start.

use std::collections::BTreeMap;
use std::sync::Arc;

use bevy::math::{EulerRot, Quat};
use render_frontend::prepare::scene::world::WorldScene;
use xmodel_runtime::RetainedModelCapability;

/// The simulation's name for a model the map placed: the asset side numbers
/// them by where the map's entity list had them, and so does the sim.
pub fn install_script_model_id(content: asset_world::ScriptModelId) -> sim::ScriptModelId {
    sim::ScriptModelId::from_authored_source_ordinal(content.source_ordinal())
}

/// A model that starts in the world: what it is, where, and which way it faces
/// (pitch, yaw, roll in degrees).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ModelSpawn {
    pub id: sim::ScriptModelId,
    pub origin: [f32; 3],
    pub angles: [f32; 3],
}

/// A brush model a script can move, with the place it stands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct BrushMover {
    pub id: sim::ScriptModelId,
    pub cmodel: u32,
    pub origin: [f32; 3],
    pub angles: [f32; 3],
}

/// Everything the simulation is told about the map's script-moved models.
#[derive(Default)]
pub(super) struct ModelInstall {
    pub capabilities: Vec<sim::EntityCollisionCapabilities>,
    pub brush_movers: Vec<BrushMover>,
    /// What each model is traced against, by model name; `None` for a model
    /// with no collision data.
    pub library: BTreeMap<String, Option<Arc<RetainedModelCapability>>>,
    /// Which sim entity owns each placed model, so the scene can leave out
    /// the drawing the sim now owns.
    pub owners: Vec<(asset_world::ScriptModelId, sim::AuthorityModelOwner)>,
    /// Models whose brush link could not be settled between several brushes.
    pub ambiguous_links: usize,
}

/// A map's placed model faces as pitch, yaw, roll in degrees; the asset side
/// keeps a rotation.
fn angles_of(rotation: Quat) -> [f32; 3] {
    let (yaw, pitch, roll) = rotation.to_euler(EulerRot::ZYX);
    [pitch, yaw, roll].map(f32::to_degrees)
}

pub(super) fn model_spawns(instances: &[asset_world::ScriptModelSceneInstance]) -> Vec<ModelSpawn> {
    instances
        .iter()
        .map(|instance| ModelSpawn {
            id: install_script_model_id(instance.id),
            origin: instance.transform.translation.to_array(),
            angles: angles_of(instance.transform.rotation),
        })
        .collect()
}

pub(super) fn spawn_movers(sim: &mut sim::SimWorld, spawns: &[ModelSpawn]) {
    for spawn in spawns {
        sim.spawn_script_mover(spawn.id, spawn.origin, spawn.angles)
            .expect("no free dynamic entity slot for a script model");
    }
}

/// Tell the scene which entity each of its models became, so the draw side can
/// follow the simulation.
pub(super) fn stamp_entity_numbers(scene: &mut WorldScene, sim: &sim::SimWorld) {
    for instance in &mut scene.script_model_instances {
        let number = sim.gentity_number(install_script_model_id(instance.id));
        instance.gentity_number = number.and_then(|n| u16::try_from(n).ok());
    }
}

fn capability_of(asset: &asset_world::MapXModelSceneAsset) -> Option<Arc<RetainedModelCapability>> {
    match asset {
        asset_world::MapXModelSceneAsset::T5(model) => model.retained_capability().map(Arc::new),
        asset_world::MapXModelSceneAsset::Unavailable { .. } => None,
    }
}

/// What a placed model starts out as in the simulation: an owner with a
/// drawable object that needs posing once.
fn placed_model(
    world: &assets::PreparedWorld,
    instance: &asset_world::ScriptModelSceneInstance,
    owner: sim::AuthorityModelOwner,
) -> sim::EntityCollisionCapabilities {
    let capability = world
        .map_xmodel_scene_assets
        .get(&instance.current_model)
        .and_then(capability_of);
    let mut dobj = sim::AuthorityDObjState::new_dirty(
        instance.current_model.0.clone(),
        capability,
        instance.transform.to_matrix(),
    );
    if let Some(definition) = &instance.metadata.t5_destructible {
        sim::t5_destructible::install(&mut dobj, definition.clone());
    }
    let hidden = instance.dobj_state.hide_part_bits;
    dobj.semantic_state.hide_part_bits = hidden;
    dobj.pose_request.hide_part_bits = hidden;
    sim::EntityCollisionCapabilities::current_tick(owner, Some(dobj), Vec::new())
}

/// A brush model has only its brushes to be solid with.
fn brush_model(
    brush: &asset_world::ScriptBrushModelPlacement,
) -> (BrushMover, sim::EntityCollisionCapabilities) {
    let id = install_script_model_id(asset_world::ScriptModelId::from_source_ordinal(
        brush.source_ordinal,
    ));
    let mover = BrushMover {
        id,
        cmodel: brush.cmodel_handle,
        origin: brush.origin,
        angles: brush.angles,
    };
    let linked = sim::LinkedBrushCollisionBrush {
        cmodel_handle: brush.cmodel_handle,
        origin: brush.origin,
        angles: brush.angles,
    };
    let capabilities = sim::EntityCollisionCapabilities::current_tick(
        sim::AuthorityModelOwner::ScriptModel(id),
        None,
        vec![linked],
    );
    (mover, capabilities)
}

pub(super) fn model_install(world: &assets::PreparedWorld) -> ModelInstall {
    let mut owners = Vec::new();
    let mut capabilities = Vec::new();
    let mut ambiguous_links = 0;
    for instance in &world.script_model_instances {
        let owner = sim::AuthorityModelOwner::ScriptModel(install_script_model_id(instance.id));
        owners.push((instance.id, owner));
        if matches!(
            instance.metadata.brush_link,
            asset_world::ScriptBrushModelLink::Ambiguous { .. }
        ) {
            ambiguous_links += 1;
        }
        capabilities.push(placed_model(world, instance, owner));
    }
    let mut brush_movers = Vec::new();
    for brush in &world.script_brush_models {
        let (mover, brush_capabilities) = brush_model(brush);
        brush_movers.push(mover);
        capabilities.push(brush_capabilities);
    }
    let library = world
        .map_xmodel_scene_assets
        .iter()
        .map(|(key, asset)| (key.0.clone(), capability_of(asset)))
        .collect();
    ModelInstall {
        capabilities,
        brush_movers,
        library,
        owners,
        ambiguous_links,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-3)
    }

    #[test]
    fn a_models_id_follows_its_place_in_the_map() {
        for ordinal in [0, 1, 7, 400] {
            let id = asset_world::ScriptModelId::from_source_ordinal(ordinal);
            assert_eq!(
                install_script_model_id(id),
                sim::ScriptModelId::from_authored_source_ordinal(ordinal)
            );
        }
    }

    #[test]
    fn no_rotation_faces_no_way() {
        assert!(near(angles_of(Quat::IDENTITY), [0.0; 3]));
    }

    #[test]
    fn a_turn_about_the_vertical_is_the_yaw() {
        let angles = angles_of(Quat::from_rotation_z(90f32.to_radians()));
        assert!(near(angles, [0.0, 90.0, 0.0]));
    }

    #[test]
    fn pitch_yaw_and_roll_come_out_in_that_order() {
        let rotation = Quat::from_euler(
            EulerRot::ZYX,
            30f32.to_radians(),
            20f32.to_radians(),
            10f32.to_radians(),
        );
        assert!(near(angles_of(rotation), [20.0, 30.0, 10.0]));
    }

    #[test]
    fn a_world_with_no_placed_models_installs_none() {
        let install = model_install(&assets::PreparedWorld::default());
        assert!(install.capabilities.is_empty());
        assert!(install.brush_movers.is_empty());
        assert!(install.library.is_empty());
        assert!(install.owners.is_empty());
        assert_eq!(install.ambiguous_links, 0);
    }

    #[test]
    fn no_instances_have_no_spawns() {
        assert!(model_spawns(&[]).is_empty());
    }
}
