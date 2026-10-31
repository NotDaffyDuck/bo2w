//! Taking what the presented snapshot says about each model owner onto its
//! entity.

use std::collections::BTreeMap;

use bevy::prelude::*;
use render_scene::LevelScriptModelInstance;
use xmodel_runtime::RigPoseRecord;

use crate::occupancy::t6_body::SlowGuard;
use crate::occupancy::other_view::vehicle_view;

/// Each model owner's animation state in the presented snapshot (the first
/// one listed per owner).
pub(crate) fn presented_dobj_states(
    presented: &net::ShownWorldFrame,
) -> BTreeMap<sim::ServerModelSource, &RigPoseRecord> {
    let listed = presented
        .snapshot()
        .into_iter()
        .flat_map(|snapshot| snapshot.meta.entity_dobjs.iter());
    let mut lighting_per_owner = BTreeMap::new();
    for (owner, state) in listed {
        lighting_per_owner.entry(*owner).or_insert(state);
    }
    lighting_per_owner
}

/// Shows each owner as its snapshot state says, and hides it when there is
/// none or it has no model. A vehicle the local player rides from its own
/// tag is hidden from them, as their view sits inside it.
pub(super) fn apply_shown_script_model_dobjs(
    presented: Option<Res<net::ShownWorldFrame>>,
    local: Option<Res<net::HostViewClient>>,
    mut owners_in: Query<(&mut Visibility, &mut LevelScriptModelInstance)>,
) {
    let _slow = SlowGuard::new("apply_presented_script_model_dobjs");
    let Some(presented) = presented.as_deref().filter(|p| p.snapshot().is_some()) else {
        return;
    };
    let lighting_per_owner = presented_dobj_states(presented);
    let ridden = local
        .as_deref()
        .and_then(|local| vehicle_view(presented, local.0))
        .filter(|view| view.first_person)
        .map(|view| view.xmodel);
    for (mut shown_vis, mut instance) in &mut owners_in {
        let own_ride = ridden == Some(instance.id.origin_sequence_number());
        let state = instance
            .server_owned_model_owner
            .filter(|_| !own_ride)
            .and_then(|owner| lighting_per_owner.get(&owner).copied());
        let Some((state, base)) =
            state.and_then(|state| Some((state, state.composition.models.first()?)))
        else {
            *shown_vis = Visibility::Hidden;
            continue;
        };
        *shown_vis = Visibility::Inherited;
        if instance.puppet_state != *state {
            instance.puppet_state = state.clone();
        }
        if base.model != instance.active_mesh.0 {
            instance.active_mesh = asset_world::LevelModelAssetId(base.model.clone());
        }
    }
}
