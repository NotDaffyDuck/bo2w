use super::*;

/// Identifies the geometry of a batch: which clients, at which revision.
pub(super) fn seated_packed_id(seated: &[SeatedBody]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    seated.len().hash(&mut hasher);
    for body in seated {
        body.item.client.hash(&mut hasher);
        body.item.geometry_revision.hash(&mut hasher);
        body.item.geom.decoded_n.hash(&mut hasher);
    }
    hasher.finish()
}

/// Identifies where a batch's bodies stand and how they are lit.
pub(super) fn seated_draw_id(seated: &[SeatedBody]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    seated.len().hash(&mut hasher);
    for body in seated {
        body.item.client.hash(&mut hasher);
        for lane in body.item.world_from_local.to_cols_array() {
            lane.to_bits().hash(&mut hasher);
        }
        body.handle.hash(&mut hasher);
        body.scene_light.hash(&mut hasher);
        body.probe.hash(&mut hasher);
    }
    hasher.finish()
}

/// True when both lists name the same set of clients, ignoring order and
/// repeats.
pub(super) fn same_clients(
    live: impl IntoIterator<Item = u32>,
    drawn: impl IntoIterator<Item = u32>,
) -> bool {
    let sorted_set = |ids: Vec<u32>| {
        let mut ids = ids;
        ids.sort_unstable();
        ids.dedup();
        ids
    };
    sorted_set(live.into_iter().collect()) == sorted_set(drawn.into_iter().collect())
}

pub(super) fn seated_clients_match(plan: &RemoteBodyDrawPlan, seated: &[SeatedBody]) -> bool {
    same_clients(
        seated.iter().map(|body| body.item.client),
        plan.draws.iter().filter_map(|draw| draw.scene_entnum),
    )
}

/// Re-aims the drawn surfaces at their bodies' new places and lights.
pub(super) fn refresh_remote_body_draws(plan: &mut RemoteBodyDrawPlan, seated: &[SeatedBody]) {
    for draw in &mut plan.draws {
        let Some(body) = draw
            .scene_entnum
            .and_then(|client| seated.iter().find(|body| body.item.client == client))
        else {
            continue;
        };
        draw.world_from_local = body.item.world_from_local;
        draw.lighting_handle = body.handle;
        draw.scene_light_index = body.scene_light;
        draw.reflection_probe_index = body.probe;
    }
}
