//! Models the server spawned during play (a script mover with a model of its
//! own) get an entity here; the map's own models are placed by the world.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use bevy::prelude::*;
use render_scene::LevelScriptModelInstance;

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
    mut cmds: Commands,
    presented: Option<Res<net::ShownWorldFrame>>,
    mut existing: Query<(Entity, &SpawnedSceneModel, &mut LevelScriptModelInstance)>,
) {
    let _slow = SlowGuard::new("sync_spawned_scene_models");
    let Some(snapshot) = presented.as_ref().and_then(|p| p.snapshot()) else {
        for (ent, _, _) in existing.iter() {
            cmds.entity(ent).despawn();
        }
        return;
    };
    let movers: HashMap<u32, &sim::MoverSlot> = snapshot
        .meta
        .script_movers
        .iter()
        .map(|mover| (mover.id.encode_for_net(), mover))
        .filter(|(wire, _)| is_spawned(*wire))
        .collect::<_>();

    let mut known = HashSet::new();
    for (ent, marker, mut instance) in existing.iter_mut() {
        known.insert(marker.source);
        match movers.get(&marker.source) {
            Some(mover) => instance.game_actor_number = u16::try_from(mover.state.slot_index).ok(),
            None => cmds.entity(ent).despawn(),
        }
    }

    for (owner, st) in &snapshot.meta.entity_dobjs {
        let Some(source) = owner.scripted_model().map(|id| id.encode_for_net()) else {
            continue;
        };
        if known.contains(&source) {
            continue;
        }
        let (Some(mover), Some(base)) = (movers.get(&source), st.composition.models.first())
        else {
            continue;
        };
        let origin = mover.state.lerp.pos.base_point;
        let transform = Transform::from_translation(Vec3::from_array(origin));
        cmds.spawn((
            SpawnedSceneModel { source },
            transform,
            Visibility::Hidden,
            LevelScriptModelInstance {
                id: asset_world::ScriptModelId::from_source_ordinal(source),
                server_owned_model_owner: Some(*owner),
                active_mesh: asset_world::LevelModelAssetId(base.model.clone()),
                place: transform,
                illum_origin: origin,
                puppet_state: st.clone(),
                metadata: asset_world::ScriptMeshDescriptor::default(),
                game_actor_number: u16::try_from(mover.state.slot_index).ok(),
            },
        ));
    }
}

mod tests {
    #![cfg(test)]
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
