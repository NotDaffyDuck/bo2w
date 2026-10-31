use super::*;

/// Identifies the geometry of a batch: which clients, at which revision.
pub(super) fn seated_packed_id(seated: &[SeatedBody]) -> u64 {
    let mut digest_in = std::collections::hash_map::DefaultHasher::new();
    seated.len().hash(&mut digest_in);
    seated.iter().for_each(|body| {
        let item = &body.item;
        (item.client_no, item.mesh_rev, item.shape_data.decoded_tally).hash(&mut digest_in);
    });
    digest_in.finish()
}

/// Identifies where a batch's bodies stand and how they are lit.
pub(super) fn seated_draw_id(seated: &[SeatedBody]) -> u64 {
    let mut digest_v = std::collections::hash_map::DefaultHasher::new();
    seated.len().hash(&mut digest_v);
    for body in seated {
        let transform_bits = body.item.scene_from_local.to_cols_array().map(f32::to_bits);
        body.item.client_no.hash(&mut digest_v);
        transform_bits.hash(&mut digest_v);
        (body.hnd, body.stage_light, body.probe).hash(&mut digest_v);
    }
    digest_v.finish()
}

/// True when both lists name the same set of clients, ignoring order and
/// repeats.
pub(super) fn same_clients(
    live: impl IntoIterator<Item = u32>,
    drawn_in: impl IntoIterator<Item = u32>,
) -> bool {
    use std::collections::BTreeSet;
    let live: BTreeSet<u32> = live.into_iter().collect();
    let drawn_in: BTreeSet<u32> = drawn_in.into_iter().collect();
    live == drawn_in
}

pub(super) fn seated_clients_match(plan: &OtherBodyPaintPlan, seated: &[SeatedBody]) -> bool {
    same_clients(
        seated.iter().map(|body| body.item.client_no),
        plan.draws.iter().filter_map(|draw| draw.stage_actor_num),
    )
}

/// Re-aims the drawn surfaces at their bodies' new places and lights.
pub(super) fn refresh_remote_body_draws(plan: &mut OtherBodyPaintPlan, seated: &[SeatedBody]) {
    for draw_in in plan.draws.iter_mut() {
        let sitting = draw_in.stage_actor_num.and_then(|who| {
            let mut bodies = seated.iter();
            bodies.find(|body| body.item.client_no == who)
        });
        if let Some(body) = sitting {
            draw_in.env_probe_index = body.probe;
            draw_in.stage_light_index = body.stage_light;
            draw_in.illum_handle = body.hnd;
            draw_in.model_to_world = body.item.scene_from_local;
        }
    }
}
