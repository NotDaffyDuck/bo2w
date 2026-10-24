use super::*;

/// Remote bodies are posed by the third-person body pass, so the only thing
/// left here is to empty last frame's skinned queue and effect bolts.
pub(super) fn clear_remote_body_frame(
    mut submit: ResMut<RemoteBodySkinnedQueue>,
    mut roots: Query<&mut RemoteFxBolts, (With<RemotePlayer>, RemoteQueryFilter)>,
) {
    submit.clear();
    for mut bolts in &mut roots {
        *bolts = RemoteFxBolts::default();
    }
}

pub(super) fn skel_camera_lod(
    skel: &ModelSkel,
    origin: [f32; 3],
    eye: Option<Vec3>,
    ramp: LodRampArgs,
) -> Option<u8> {
    smodel_camera_lod(skel.lod, origin, 1.0, eye, ramp)
}

pub(super) fn enqueue_remote_body_lighting(
    submit: ResMut<RemoteBodySkinnedQueue>,
    mut binds: ResMut<RemoteBodyLightingBinds>,
    atpoint: Res<DynAtPointLookup>,
    mut lighting_requests: ResMut<ModelLightingRequests>,
) {
    binds.by_client.clear();
    for item in submit.iter() {
        let lookup_fallback =
            atpoint.fallback(item.origin, item.radius.map(radius_lighting_box_half));
        let request = lighting_requests.request(ModelLightingRequest {
            owner: ModelLightingOwner::RemoteClient(u16::try_from(item.client).unwrap_or(u16::MAX)),
            origin: item.origin,
            lookup_fallback,
        });
        binds.by_client.insert(item.client, request);
    }
}

/// Empties the batch's geometry unless it is already empty.
pub(super) fn clear_if_populated(plan: &mut RemoteBodyDrawPlan) {
    if plan.decoded_n != 0 || plan.last_packed_id.is_some() {
        plan.clear_geometry();
    }
}

pub(super) fn submit_remote_bodies(
    mut plan: ResMut<RemoteBodyDrawPlan>,
    gaps: Res<RenderPresentationGaps>,
    mut submit: ResMut<RemoteBodySkinnedQueue>,
    mut binds: ResMut<RemoteBodyLightingBinds>,
    resolved: Res<ResolvedModelLightingTable>,
    atlas: Option<Res<WorldModelLightingAtlas>>,
    tess: Option<Res<TessMaterials>>,
    model_materials: Res<PreparedModelMaterials>,
    mut last_catalog: Local<Option<Arc<render_material::RuntimeMaterialCatalog>>>,
) {
    let items = submit.take();
    let binds = std::mem::take(&mut binds.by_client);
    if items.is_empty() {
        clear_if_populated(&mut plan);
        *last_catalog = None;
        return;
    }
    let tess = match (atlas.is_some(), tess) {
        (true, Some(tess)) => tess,
        _ => {
            plan.clear_geometry();
            *last_catalog = None;
            gaps.raise(RenderGapCause::RemoteBodyLightingAllocFailed);
            return;
        }
    };
    // New material tables invalidate every material index in the batch.
    if !last_catalog
        .as_ref()
        .is_some_and(|old| Arc::ptr_eq(old, &tess.catalog))
    {
        plan.clear_geometry();
        *last_catalog = Some(Arc::clone(&tess.catalog));
    }

    let mut last_cause = None;
    let mut seated = Vec::with_capacity(items.len());
    for item in items {
        let light = binds
            .get(&item.client)
            .and_then(|request| resolved.get(request.owner));
        match light {
            Some(ResolvedModelLighting::Seated {
                handle,
                scene_light_index,
                reflection_probe_index,
                ..
            }) => seated.push(SeatedBody {
                item,
                handle,
                scene_light: scene_light_index,
                probe: reflection_probe_index,
            }),
            Some(ResolvedModelLighting::Failed) | None => {
                last_cause = Some(RenderGapCause::RemoteBodyLightingAllocFailed);
            }
        }
    }
    if seated.is_empty() {
        clear_if_populated(&mut plan);
        *last_catalog = None;
        gaps.raise(last_cause.unwrap_or(RenderGapCause::RemoteBodySubmitMissing));
        return;
    }

    // Same bodies as the last batch: only where they stand and how they are
    // lit can have changed.
    let packed_id = seated_packed_id(&seated);
    let draw_id = seated_draw_id(&seated);
    if plan.last_packed_id == Some(packed_id) && seated_clients_match(&plan, &seated) {
        if plan.last_draw_id != Some(draw_id) {
            refresh_remote_body_draws(&mut plan, &seated);
            plan.revisions.bump_draws();
            plan.revision = plan.revision.wrapping_add(1);
            plan.last_draw_id = Some(draw_id);
        }
        gaps.clear(RenderGap::RemoteBodyAnimation);
        return;
    }

    plan.clear_geometry();
    plan.indices
        .reserve(seated.iter().map(|body| body.item.geom.indices.len()).sum());
    let mut session = take_body_packed_session(&mut plan);
    session.reserve_packed(seated.iter().map(|body| body.item.geom.decoded_n).sum());
    let mut material_slots: HashMap<String, u32> = HashMap::new();
    let mut submitted_verts = 0usize;
    let mut material_missing = false;
    for body in seated {
        let SeatedBody {
            item,
            handle,
            scene_light,
            probe,
        } = body;
        let verts_before = plan.decoded_n;
        let index_base = append_remote_body_cpu_blob(
            &mut plan,
            &mut session,
            &item.geom.packed,
            &item.geom.indices,
            item.geom.decoded_n,
        )
        .map(|(_, index_base)| index_base);
        for surface in item.geom.surfaces.iter() {
            let Some(name) = surface.name.as_deref() else {
                material_missing = true;
                last_cause = Some(RenderGapCause::RemoteBodyMaterialMissing {
                    name: String::new(),
                });
                continue;
            };
            let slot = match material_slots.get(name) {
                Some(&slot) => slot,
                None => {
                    let Some(material) = model_materials.material(&tess.catalog, name).cloned()
                    else {
                        material_missing = true;
                        last_cause = Some(RenderGapCause::RemoteBodyMaterialMissing {
                            name: name.to_owned(),
                        });
                        continue;
                    };
                    let slot = plan.materials.len() as u32;
                    plan.materials.push(material);
                    material_slots.insert(name.to_owned(), slot);
                    slot
                }
            };
            if let Some(index_base) = index_base {
                push_remote_body_cpu_draw(
                    &mut plan,
                    index_base.saturating_add(surface.index_start),
                    surface.index_count,
                    item.world_from_local,
                    handle,
                    scene_light,
                    probe,
                    slot,
                    Some(item.client),
                );
            }
        }
        submitted_verts += plan.decoded_n.saturating_sub(verts_before);
    }
    install_body_packed_session(&mut plan, session);
    if submitted_verts == 0 || material_missing {
        gaps.raise(last_cause.unwrap_or(RenderGapCause::RemoteBodySubmitMissing));
        return;
    }
    plan.revisions.bump_surfaces();
    plan.revisions.bump_draws();
    let topology = topology_fingerprint(&plan.indices, &plan.surface_ranges, plan.decoded_n);
    plan.revisions.topology = topology;
    plan.revision = plan.revision.wrapping_add(1);
    plan.last_packed_id = Some(packed_id);
    plan.last_draw_id = Some(draw_id);
    gaps.clear(RenderGap::RemoteBodyAnimation);
}
