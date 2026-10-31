//! The models a map's scripts can move: which of them the simulation owns, what
//! it traces against for each, and where they start.

use std::collections::BTreeMap;
use std::sync::Arc;

use render_frontend::prepare::scene::world::LevelStage;
use xmodel_runtime::KeptModelAbility;
use bevy::math::{Quat, EulerRot};

/// The simulation's name for a model the map placed: the asset side numbers
/// them by where the map's entity list had them, and so does the sim.
pub fn scripted_prop_id(content: asset_world::ScriptModelId) -> sim::ScriptModelId {
    sim::ScriptModelId::from_authored_source_ordinal(content.origin_sequence_number())
}

/// A model that starts in the world: what it is, where, and which way it faces
/// (pitch, yaw, roll in degrees).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ModelSpawn {
    pub id: sim::ScriptModelId,
    pub at: [f32; 3],
    pub turn: [f32; 3],
}

/// A brush model a script can move, with the place it stands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct BrushMover {
    pub id: sim::ScriptModelId,
    pub collision_geometry: u32,
    pub at: [f32; 3],
    pub turn: [f32; 3],
}

/// Everything the simulation is told about the map's script-moved models.
#[derive(Default)]
pub(super) struct ModelInstall {
    pub abilities: Vec<sim::EntityHitAbilities>,
    pub brush_movers: Vec<BrushMover>,
    /// What each model is traced against, by model name; `None` for a model
    /// with no collision data.
    pub library: BTreeMap<String, Option<Arc<KeptModelAbility>>>,
    /// Which sim entity owns each placed model, so the scene can leave out
    /// the drawing the sim now owns.
    pub owners: Vec<(asset_world::ScriptModelId, sim::ServerModelSource)>,
    /// Models whose brush link could not be settled between several brushes.
    pub ambiguous_links: usize,
}

/// A map's placed model faces as pitch, yaw, roll in degrees; the asset side
/// keeps a rotation.
fn angles_of(rotation: Quat) -> [f32; 3] {
    let (yaw, pitch, roll) = rotation.to_euler(EulerRot::ZYX);
    [pitch, yaw, roll].map(f32::to_degrees)
}

pub(super) fn model_spawns(instances: &[asset_world::ScriptedPropPlacement]) -> Vec<ModelSpawn> {
    instances
        .iter()
        .map(|instance| ModelSpawn {
            id: scripted_prop_id(instance.handle),
            at: instance.transform.translation.to_array(),
            turn: angles_of(instance.transform.rotation),
        })
        .collect()
}

pub(super) fn spawn_movers(sim: &mut sim::SimWorld, spawns: &[ModelSpawn]) {
    for one in spawns {
        sim.create_scripted_mover(one.id, one.at, one.turn)
            .expect("no free dynamic entity slot for a script model");
    }
}

/// Tell the scene which entity each of its models became, so the draw side can
/// follow the simulation.
pub(super) fn stamp_entity_numbers(stage: &mut LevelStage, running: &sim::SimWorld) {
    for prop in &mut stage.scripted_prop_placements {
        let number = running.game_actor_number(scripted_prop_id(prop.id));
        prop.game_actor_number = number.and_then(|n| u16::try_from(n).ok());
    }
}

fn capability_of(asset: &asset_world::LevelModelPlacementAsset) -> Option<Arc<KeptModelAbility>> {
    match asset {
        asset_world::LevelModelPlacementAsset::T5(model) => model.retained_capability().map(Arc::new),
        asset_world::LevelModelPlacementAsset::Unavailable { .. } => None,
    }
}

/// What a placed model starts out as in the simulation: an owner with a
/// drawable object that needs posing once.
fn placed_model(
    world: &assets::StagedLevel,
    prop: &asset_world::ScriptedPropPlacement,
    owner: sim::ServerModelSource,
) -> sim::EntityHitAbilities {
    let ability = world
        .map_figure_stage_assets
        .get(&prop.active_mesh)
        .and_then(capability_of);
    let mut held = sim::ServerModelState::new_dirty(
        prop.active_mesh.0.clone(),
        ability,
        prop.transform.to_matrix(),
    );
    if let Some(spec) = &prop.metadata.t5_destructible {
        sim::staged_props::install(&mut held, spec.clone());
    }
    let hidden = prop.puppet_state.hidden_mask;
    held.bone_role_state.hidden_mask = hidden;
    held.pose_query.hidden_mask = hidden;
    sim::EntityHitAbilities::now_tick(owner, Some(held), vec![])
}

/// A brush model has only its brushes to be solid with.
fn solid_model(
    part: &asset_world::ScriptSolidPlacementInfo,
) -> (BrushMover, sim::EntityHitAbilities) {
    let id = scripted_prop_id(asset_world::ScriptModelId::from_source_ordinal(
        part.origin_sequence_number,
    ));
    let mover = BrushMover {
        id,
        collision_geometry: part.collision_model_ref,
        at: part.origin,
        turn: part.angles,
    };
    let (at, turn) = (part.origin, part.angles);
    let linked = sim::AttachedSolidShape {
        collision_model_ref: part.collision_model_ref,
        origin: at,
        turn,
    };
    let capabilities = sim::EntityHitAbilities::now_tick(
        sim::ServerModelSource::ScriptModel(id),
        None,
        vec![linked],
    );
    (mover, capabilities)
}

pub(super) fn model_install(world: &assets::StagedLevel) -> ModelInstall {
    let mut owners = Vec::new();
    let mut capabilities = Vec::new();
    let mut ambiguous_links = 0;
    for instance in &world.scripted_prop_placements {
        let owner = sim::ServerModelSource::ScriptModel(scripted_prop_id(instance.handle));
        owners.push((instance.handle, owner));
        if matches!(
            instance.metadata.brush_link,
            asset_world::ScriptSolidBinding::Ambiguous { .. }
        ) {
            ambiguous_links += 1;
        }
        capabilities.push(placed_model(world, instance, owner));
    }
    let mut brush_movers = Vec::new();
    for brush in &world.script_solid_models {
        let (mover, brush_capabilities) = solid_model(brush);
        brush_movers.push(mover);
        capabilities.push(brush_capabilities);
    }
    let library = world
        .map_figure_stage_assets
        .iter()
        .map(|(handle, asset)| (handle.0.clone(), capability_of(asset)))
        .collect();
    ModelInstall {
        abilities: capabilities,
        brush_movers,
        library,
        owners,
        ambiguous_links,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::math::{Quat, EulerRot};

    fn near(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-3)
    }

    #[test]
    fn a_models_id_follows_its_place_in_the_map() {
        for ordinal in [0, 1, 7, 400] {
            let id = asset_world::ScriptModelId::from_source_ordinal(ordinal);
            assert_eq!(
                scripted_prop_id(id),
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
        let install = model_install(&assets::StagedLevel::default());
        assert!(install.abilities.is_empty());
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
