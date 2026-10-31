use super::*;

/// Remote bodies are posed by the third-person body pass, so all that is left to do
/// here is to drop last frame's skinned queue and effect bolts.
pub(super) fn clear_remote_body_frame(
    mut queue: ResMut<BodySkinJobs>,
    mut roots: Query<&mut OtherBodyEffectTags, (With<OtherPlayerBody>, Without<LevelScriptModelInstance>, Without<FirstPersonLens>, Without<FreeRoamCamera>)>,
) {
    queue.clear();
    roots
        .iter_mut()
        .for_each(|mut bolts| *bolts = OtherBodyEffectTags::default());
}

pub(super) fn skel_camera_lod(
    mdl: &MeshRig,
    pos: [f32; 3],
    view_eye: Option<Vec3>,
    ramp: DetailRampArgs,
) -> Option<u8> {
    prop_camera_detail(mdl.lod, pos, 1.0, view_eye, ramp)
}

pub(super) fn queue_body_lighting(
    queue: ResMut<BodySkinJobs>,
    mut claims: ResMut<BodyLightBinds>,
    at_spot: Res<LooseAtSpotLookup>,
    mut asks: ResMut<ModelIllumRequests>,
) {
    claims.per_client.retain(|_, _| false);
    for body_in in queue.iter() {
        let half_box = body_in.reach.map(illum_query_half_size);
        let ticket = asks.queue(ModelIllumRequest {
            lookup_backup: at_spot.fallback(body_in.spot, half_box),
            origin: body_in.spot,
            owner: ModelIllumOwner::OtherPlayerSlot(u16::try_from(body_in.client_no).unwrap_or(u16::MAX)),
        });
        claims.per_client.insert(body_in.client_no, ticket);
    }
}

/// Empties the batch's geometry unless it is already empty.
pub(super) fn clear_if_populated(draws: &mut OtherBodyPaintPlan) {
    if draws.decoded_tally != 0 || draws.prev_pack_id.is_some() {
        draws.drop_mesh();
    }
}

/// The queued bodies that have a light seated, and the gap cause to report when some
/// had none.
fn seat_bodies(
    queued: Vec<BodySkinJob>,
    tickets: &HashMap<u32, ModelIllumRequest>,
    resolved: &ResolvedModelIllumTable,
) -> (Vec<SeatedBody>, Option<Cause>) {
    let mut left_dark = false;
    let seated: Vec<SeatedBody> = queued
        .into_iter()
        .filter_map(|pickup| {
            let found_in = tickets
                .get(&pickup.client_no)
                .and_then(|ticket| resolved.answer_for(ticket.owner));
            match found_in {
                Some(ResolvedModelIllum::Seated {
                    handle,
                    stage_light_index,
                    env_probe_index,
                    ..
                }) => Some(SeatedBody {
                    probe: env_probe_index,
                    stage_light: stage_light_index,
                    hnd: handle,
                    item: pickup,
                }),
                _ => {
                    left_dark = true;
                    None
                }
            }
        })
        .collect::<_>();
    let cause = left_dark.then_some(Cause::BodyLightNoSlot);
    (seated, cause)
}

/// The draw slot of the material a surface names, adding the material to the batch the
/// first time it is seen. None when the catalog has no such material.
fn surface_material_index(
    batch: &mut OtherBodyPaintPlan,
    known: &mut HashMap<String, u32>,
    name: &str,
    tess: &StitchMaterials,
    model_materials: &PreparedModelMaterials,
) -> Option<u32> {
    if let Some(&bay) = known.get(name) {
        return Some(bay);
    }
    let material = model_materials.mat(&tess.catalog, name).cloned()?;
    let bay = batch.mat_list.len() as u32;
    batch.mat_list.push(material);
    known.insert(name.to_owned(), bay);
    Some(bay)
}

/// Writes the geometry and draws of `seated` into `draws`. Returns how many vertices
/// went in, and the cause when a surface had no usable material.
fn lay_out_bodies(
    draws: &mut OtherBodyPaintPlan,
    seated: Vec<SeatedBody>,
    tess: &StitchMaterials,
    model_materials: &PreparedModelMaterials,
) -> (usize, Option<Cause>) {
    draws.drop_mesh();
    let index_total: usize = seated.iter().map(|body| body.item.shape_data.index_list.len()).sum();
    draws.indices.reserve(index_total);
    let mut session_in = end_body_packed_session(draws);
    session_in.reserve_pack(seated.iter().map(|body| body.item.shape_data.decoded_tally).sum());
    let mut known_slots = HashMap::new();
    let mut vertices_in = 0usize;
    let mut material_cause = None;
    for SeatedBody { item, hnd: handle, stage_light, probe } in seated {
        let decoded_before = draws.decoded_tally;
        let shape_data = &item.shape_data;
        let blob_base = add_other_body_software_mesh(
            draws,
            &mut session_in,
            &shape_data.packed_rows,
            &shape_data.index_list,
            shape_data.decoded_tally,
        )
        .map(|(_, base)| base);
        for surface in &shape_data.surfaces {
            let slot = match surface.name.as_deref() {
                None => Err(String::new()),
                Some(name) => {
                    surface_material_index(draws, &mut known_slots, name, tess, model_materials)
                        .ok_or_else(|| name.to_owned())
                }
            };
            let slot = match slot {
                Ok(slot) => slot,
                Err(name) => {
                    material_cause = Some(Cause::RemoteBodyMaterialMissing { name });
                    continue;
                }
            };
            if let Some(base) = blob_base {
                let head_index = base.saturating_add(surface.first_triangle_slot);
                let place = item.scene_from_local;
                let count = surface.idx_count;
                queue_other_body_cpu_paint(
                    draws, head_index, count, place, handle, stage_light, probe, slot,
                    Some(item.client_no),
                );
            }
        }
        vertices_in += draws.decoded_tally.saturating_sub(decoded_before);
    }
    begin_body_packed_session(draws, session_in);
    (vertices_in, material_cause)
}

pub(super) fn submit_remote_bodies(
    mut batch: ResMut<OtherBodyPaintPlan>,
    gaps: Res<ShortfallLedger>,
    mut queue: ResMut<BodySkinJobs>,
    mut light_binds: ResMut<BodyLightBinds>,
    lit_table: Res<ResolvedModelIllumTable>,
    atlas: Option<Res<LevelModelIllumAtlas>>,
    materials_opt: Option<Res<StitchMaterials>>,
    model_materials: Res<PreparedModelMaterials>,
    mut last_catalog: Local<Option<Arc<render_material::LiveMaterialLedger>>>,
) {
    let queued = queue.hand_over();
    let tickets = std::mem::take(&mut light_binds.per_client);
    let settled = PresentationShortfall::BodyAnim;
    if queued.is_empty() {
        clear_if_populated(&mut batch);
        *last_catalog = None;
        return;
    }
    let tess = match materials_opt {
        Some(tess) if atlas.is_some() => tess,
        _ => {
            batch.drop_mesh();
            *last_catalog = None;
            let cause_in = Cause::BodyLightNoSlot;
            gaps.open_gap(cause_in);
            return;
        }
    };
    // A new material table makes every material index in the batch stale.
    let same_catalog = last_catalog
        .as_ref()
        .is_some_and(|old| Arc::ptr_eq(old, &tess.catalog));
    if !same_catalog {
        batch.drop_mesh();
        *last_catalog = Some(Arc::clone(&tess.catalog));
    }

    let (seated, unlit) = seat_bodies(queued, &tickets, &lit_table);
    if seated.first().is_none() {
        clear_if_populated(&mut batch);
        *last_catalog = None;
        gaps.open_gap(unlit.unwrap_or(Cause::BodySubmitAbsent));
        return;
    }

    // The same bodies as in the last batch: only where they stand and how they are
    // lit can have moved.
    let (packed_id, draw_id) = (seated_packed_id(&seated), seated_draw_id(&seated));
    if batch.prev_pack_id == Some(packed_id) && seated_clients_match(&batch, &seated) {
        if batch.prev_draw_id != Some(draw_id) {
            refresh_remote_body_draws(&mut batch, &seated);
            batch.revisions.advance_drawlist_stamp();
            batch.plan_rev = batch.plan_rev.wrapping_add(1);
            batch.prev_draw_id = Some(draw_id);
        }
        gaps.close_gap(settled);
        return;
    }

    let (vertices_in, material_cause) = lay_out_bodies(&mut batch, seated, &tess, &model_materials);
    if vertices_in == 0 || material_cause.is_some() {
        let cause_in = material_cause.or(unlit);
        gaps.open_gap(cause_in.unwrap_or(Cause::BodySubmitAbsent));
        return;
    }
    batch.revisions.bump_surfaces();
    batch.revisions.advance_drawlist_stamp();
    batch.revisions.topology = ix_layout_digest(&batch.indices, &batch.facet_ranges, batch.decoded_tally);
    batch.plan_rev = batch.plan_rev.wrapping_add(1);
    batch.prev_pack_id = Some(packed_id);
    batch.prev_draw_id = Some(draw_id);
    gaps.close_gap(settled);
}
