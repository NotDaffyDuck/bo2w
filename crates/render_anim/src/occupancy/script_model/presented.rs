//! Taking what the presented snapshot says about each model owner onto its
//! entity.

use std::collections::BTreeMap;

use bevy::prelude::*;
use render_scene::WorldScriptModelInstance;
use xmodel_runtime::DObjSemanticState;

use crate::occupancy::t6_body::SlowGuard;
use crate::occupancy::third_person::vehicle_view;

/// Each model owner's animation state in the presented snapshot (the first
/// one listed per owner).
pub(crate) fn presented_dobj_states(
    presented: &net::PresentedSnapshot,
) -> BTreeMap<sim::AuthorityModelOwner, &DObjSemanticState> {
    let mut by_owner = BTreeMap::new();
    if let Some(snapshot) = presented.snapshot() {
        for (owner, state) in &snapshot.meta.entity_dobjs {
            by_owner.entry(*owner).or_insert(state);
        }
    }
    by_owner
}

/// Shows each owner as its snapshot state says, and hides it when there is
/// none or it has no model. A vehicle the local player rides from its own
/// tag is hidden from them, as their view sits inside it.
pub(super) fn apply_presented_script_model_dobjs(
    presented: Option<Res<net::PresentedSnapshot>>,
    local: Option<Res<net::LocalPresentClient>>,
    mut owners: Query<(&mut WorldScriptModelInstance, &mut Visibility)>,
) {
    let _slow = SlowGuard::new("apply_presented_script_model_dobjs");
    let Some(presented) = presented.as_deref().filter(|p| p.snapshot().is_some()) else {
        return;
    };
    let by_owner = presented_dobj_states(presented);
    let ridden = local
        .as_deref()
        .and_then(|local| vehicle_view(presented, local.0))
        .filter(|view| view.first_person)
        .map(|view| view.model);
    for (mut instance, mut visibility) in owners.iter_mut() {
        let state = if ridden == Some(instance.id.source_ordinal()) {
            None
        } else {
            instance
                .authority_owner
                .and_then(|owner| by_owner.get(&owner).copied())
        };
        let shown = state.and_then(|state| Some((state, state.composition.models.first()?)));
        let Some((state, base)) = shown else {
            *visibility = Visibility::Hidden;
            continue;
        };
        if instance.current_model.0 != base.model {
            instance.current_model = asset_world::MapXModelAssetKey(base.model.clone());
        }
        if instance.dobj_state != *state {
            instance.dobj_state = state.clone();
        }
        *visibility = Visibility::Inherited;
    }
}
