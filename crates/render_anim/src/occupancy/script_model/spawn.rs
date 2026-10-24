//! Models the server spawned during play (a script mover with a model of its
//! own) get an entity here; the map's own models are placed by the world.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use bevy::prelude::*;
use render_scene::WorldScriptModelInstance;

use crate::occupancy::t6_body::SlowGuard;

/// Wire ids of script movers that came into being after the map loaded.
const SPAWNED_PRESENCE: Range<u32> = 0x4000_0000..0x8000_0000;

/// An entity standing for the spawned mover with this wire id.
#[derive(Component)]
pub(super) struct SpawnedSceneModel {
    source: u32,
}

fn is_spawned(wire_id: u32) -> bool {
    SPAWNED_PRESENCE.contains(&wire_id)
}

/// Keeps one entity per spawned mover that has a model: new ones appear,
/// gone ones go, and each keeps its gentity number.
pub(super) fn sync_spawned_scene_models(
    mut commands: Commands,
    presented: Option<Res<net::PresentedSnapshot>>,
    mut existing: Query<(Entity, &SpawnedSceneModel, &mut WorldScriptModelInstance)>,
) {
    let _slow = SlowGuard::new("sync_spawned_scene_models");
    let Some(snapshot) = presented.as_ref().and_then(|p| p.snapshot()) else {
        for (entity, _, _) in existing.iter() {
            commands.entity(entity).despawn();
        }
        return;
    };
    let movers: HashMap<u32, &sim::ScriptMoverGentity> = snapshot
        .meta
        .script_movers
        .iter()
        .map(|mover| (mover.id.to_wire(), mover))
        .filter(|(wire, _)| is_spawned(*wire))
        .collect();

    let mut known = HashSet::new();
    for (entity, marker, mut instance) in existing.iter_mut() {
        known.insert(marker.source);
        match movers.get(&marker.source) {
            Some(mover) => instance.gentity_number = u16::try_from(mover.state.number).ok(),
            None => commands.entity(entity).despawn(),
        }
    }

    for (owner, state) in &snapshot.meta.entity_dobjs {
        let Some(source) = owner.script_model().map(|id| id.to_wire()) else {
            continue;
        };
        if known.contains(&source) {
            continue;
        }
        let (Some(mover), Some(base)) = (movers.get(&source), state.composition.models.first())
        else {
            continue;
        };
        let origin = mover.state.lerp.pos.tr_base;
        let transform = Transform::from_translation(Vec3::from_array(origin));
        commands.spawn((
            SpawnedSceneModel { source },
            transform,
            Visibility::Hidden,
            WorldScriptModelInstance {
                id: asset_world::ScriptModelId::from_source_ordinal(source),
                authority_owner: Some(*owner),
                current_model: asset_world::MapXModelAssetKey(base.model.clone()),
                transform,
                lighting_origin: origin,
                dobj_state: state.clone(),
                metadata: asset_world::ScriptModelMetadata::default(),
                gentity_number: u16::try_from(mover.state.number).ok(),
            },
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawned_ids_are_the_second_quarter_of_the_id_space() {
        assert!(!is_spawned(0x3fff_ffff));
        assert!(is_spawned(0x4000_0000));
        assert!(is_spawned(0x7fff_ffff));
        assert!(!is_spawned(0x8000_0000));
        assert!(!is_spawned(7));
    }
}
