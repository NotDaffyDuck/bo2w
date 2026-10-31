use super::*;
use render_scene::DetailRampArgs;

/// What every loose piece is posed against in one frame.
struct PoseFrame<'a> {
    catalog: &'a asset_world::LevelModelPlacementRoster,
    prepared: &'a PreparedModelMaterials,
    view_eye: Option<Vec3>,
    ramp: DetailRampArgs,
}

impl PoseFrame<'_> {
    /// Poses a piece's model once in its bind pose, at the given camera level
    /// of detail.
    fn pose_asset(
        &self,
        inst: &LevelDebrisInstance,
        bone_rig: &asset_model::MeshRig,
        lod: Option<u8>,
    ) -> Option<DynEntPosedModel> {
        let model = &inst.active_mesh.0;
        let dobj = self.prepared.stage_puppet(model)?;
        let bind = xmodel_runtime::RigPoseRecord::neutral_pose(model.clone(), 1, 1);
        let request = bind.look_up_pose_query(|_| None).ok()?;
        let view = DetailLevelWindow {
            ramp: self.ramp,
            eye_pos: self.view_eye,
            spot: inst.illum_origin,
        };
        let posed = pose_script_skeleton_and_materials(
            Some(self.catalog),
            &[bone_rig],
            dobj,
            &request,
            Some(view),
            &[],
        );
        let (drawn, authored_materials) = posed?;
        Some(DynEntPosedModel {
            lod,
            drawn,
            authored_materials,
            mesh_asset_name: inst.active_mesh.clone(),
        })
    }

    /// The owner record for one piece, posing its model first when no piece
    /// has needed that model at this level of detail yet. `None` leaves the
    /// piece out of the frame.
    fn place(
        &self,
        product: &mut DynEntPosed,
        piece: Entity,
        pieces: (&LevelDebrisInstance, &Transform, &Visibility),
    ) -> Option<DynEntPosedHolder> {
        let (inst, transform, visibility) = pieces;
        let bone_rig = match self.catalog.get(&inst.active_mesh) {
            Some(asset_world::LevelModelPlacementAsset::T5(bone_rig)) => bone_rig.as_ref(),
            _ => return None,
        };
        let lod = prop_camera_detail(bone_rig.lod, inst.illum_origin, 1.0, self.view_eye, self.ramp);
        if let (None, Some(_)) = (lod, self.view_eye) {
            return None;
        }
        let asset_at = if let Some(known) = product.draw_asset_slot(&inst.active_mesh, lod) {
            known
        } else {
            let fresh = self.pose_asset(inst, bone_rig, lod)?;
            product.posed_assets.push(fresh);
            product.posed_assets.len() - 1
        };
        Some(DynEntPosedHolder {
            model_name: inst.active_mesh.0.clone(),
            piece,
            asset_at,
            concealed: *visibility == Visibility::Hidden,
            reach: bone_rig.radius,
            world_mat: transform.to_matrix(),
            lit_sample_point: inst.illum_origin,
        })
    }
}

pub(super) fn place_dyn_ent_models(
    mut product: ResMut<DynEntPosed>,
    prepared: Res<PreparedModelMaterials>,
    detail_ramp: Res<render_scene::DetailRampSkinnedTunable>,
    pieces: Query<(Entity, (&'static LevelDebrisInstance, &'static Transform, &'static Visibility))>,
    cameras: Query<(&Projection, &GlobalTransform, &Camera), With<FirstPersonLens>>,
    scene_cat: Option<Res<asset_world::LevelModelPlacementRoster>>,
) {
    let _slow = SlowGuard::new("pose_dyn_ents");
    product.posed_owners.clear();
    let scene_changed = scene_cat.as_ref().is_some_and(|scene| scene.is_changed());
    let Some(scene_cat) = scene_cat.as_deref() else {
        product.posed_assets.clear();
        return;
    };
    if scene_changed {
        product.posed_assets.clear();
    }
    let frame = PoseFrame {
        catalog: scene_cat,
        prepared: &prepared,
        view_eye: cameras.single().ok().map(|(_, lens, _)| lens.translation()),
        ramp: detail_ramp.args(),
    };
    let live = pieces.iter().filter(|(_, (inst, _, _))| !inst.gone);
    for (piece, parts) in live {
        if let Some(owner) = frame.place(&mut product, piece, parts) {
            product.posed_owners.push(owner);
        }
    }
}

/// What `push_dyn_ent_draws` reads and does not change.
#[derive(bevy::ecs::system::SystemParam)]
pub(super) struct DrawSources<'w> {
    cat: Option<Res<'w, asset_world::LevelModelPlacementRoster>>,
    light_atlas: Option<Res<'w, LevelModelIllumAtlas>>,
    tess: Option<Res<'w, StitchMaterials>>,
    facts: Res<'w, LevelPresentFacts>,
    product: Res<'w, DynEntPosed>,
    model_materials: Res<'w, PreparedModelMaterials>,
    at_point: Res<'w, LooseAtSpotLookup>,
}

/// The rows and owners being collected for the frame; kept between frames so
/// their storage is reused.
#[derive(Default)]
pub(super) struct DrawScratch {
    rows: Vec<FigureFacetDraw>,
    owners: Vec<ModelOwnerPaint>,
}

/// Empties the plan but lets its revision counters run on.
fn restart_plan(plan: &mut DynEntPaintPlan) {
    let DynEntPaintPlan {
        plan_rev,
        generation: epoch,
        ..
    } = *plan;
    *plan = DynEntPaintPlan {
        plan_rev,
        generation: epoch,
        ..Default::default()
    };
}

/// The surfaces of a posed asset in the plan: already planned, packed earlier
/// this frame, or packed now. `None` when none of its surfaces has a material.
fn surfaces_for(
    plan_in: &mut DynEntPaintPlan,
    packed: &mut [Option<Vec<(u32, u32)>>],
    (asset_at, posed): (usize, &DynEntPosedModel),
    material_of: impl Fn(assets::SurfaceMaterialId) -> Option<PropPassMaterial>,
) -> Option<Vec<(u32, u32)>> {
    if let Some(planned) = plan_in.asset_for(&posed.mesh_asset_name, posed.lod) {
        return Some(planned.surf_pairs.clone());
    }
    if let Some(done) = &packed[asset_at] {
        return Some(done.clone());
    }
    let mats: Vec<Option<PropPassMaterial>> = posed
        .authored_materials
        .iter()
        .take(posed.drawn.len())
        .map(|authored| (*authored).and_then(&material_of))
        .collect::<_>();
    if mats.iter().all(Option::is_none) {
        return None;
    }
    let done = append_debris_asset(
        plan_in,
        posed.mesh_asset_name.clone(),
        posed.lod,
        &posed.drawn,
        &mats,
    );
    packed[asset_at] = Some(done.clone());
    Some(done)
}

pub(super) fn push_dyn_ent_draws(
    sources: DrawSources,
    mut plan_v: ResMut<DynEntPaintPlan>,
    mut light_requests: ResMut<ModelIllumRequests>,
    mut scratch_in: Local<DrawScratch>,
    mut seen_generation: Local<Option<render_material::MaterialEpochId>>,
) {
    let _slow = SlowGuard::new("append_dynent_draws");
    let DrawSources {
        cat: catalog,
        light_atlas,
        tess,
        facts,
        product,
        model_materials,
        at_point,
    } = sources;
    let ready = match (catalog.as_ref(), light_atlas.as_ref(), tess.as_ref()) {
        (Some(catalog), Some(sheet), Some(tess)) if facts.spawned => Some((catalog, sheet, tess)),
        _ => None,
    };
    let Some((catalog, light_atlas, tessellation)) = ready else {
        plan_v.publish_no_rows();
        return;
    };
    let generation = tessellation.catalog.revision_stamp;
    let inputs_changed = *seen_generation != Some(generation)
        || catalog.is_changed()
        || light_atlas.is_changed()
        || tessellation.is_changed();
    if inputs_changed {
        restart_plan(&mut plan_v);
        *seen_generation = Some(generation);
    }
    let tess_catalog = std::sync::Arc::clone(&tessellation.catalog);
    let material_of = |index| model_materials.authored(&tess_catalog, index).cloned();
    let DrawScratch { rows: entries, owners } = &mut *scratch_in;
    entries.clear();
    owners.clear();

    let mut packed_in = vec![None::<Vec<(u32, u32)>>; product.posed_assets.len()];
    for owner in &product.posed_owners {
        let posed = &product.posed_assets[owner.asset_at];
        let found = surfaces_for(&mut plan_v, &mut packed_in, (owner.asset_at, posed), material_of);
        let Some(surfaces) = found.filter(|list| !list.is_empty()) else {
            continue;
        };
        let queued_illum = if owner.concealed {
            None
        } else {
            let light_box = owner.reach.map(illum_query_half_size);
            Some(light_requests.queue(ModelIllumRequest {
                origin: owner.lit_sample_point,
                lookup_backup: at_point.fallback(owner.lit_sample_point, light_box),
                owner: ModelIllumOwner::DynamicProp(owner.piece),
            }))
        };
        let caster_bound = owner.reach.map(|reach| XModelCasterBound {
            reach: reach.max(1.0),
            origin: owner.world_mat.w_axis.truncate().to_array(),
        });
        let drawn_so_far = u16::try_from(owners.len()).unwrap_or(u16::MAX);
        let object_tag = PAINT_ID_FIRST_PROP.saturating_add(drawn_so_far);
        owners.push(ModelOwnerPaint {
            xmodel: owner.model_name.clone(),
            object_tag,
        });
        let colour_refusal = owner.concealed.then_some(ModelTintRejection::CameraViewVolume);
        for (surf, material) in surfaces {
            entries.push(FigureFacetDraw {
                is_scoped_view: false,
                stage_actor_num: None,
                compact_illum: None,
                object_tag,
                surface: surf,
                material,
                caster_bound,
                colour_refusal,
                queued_illum,
                illum_handle: 0,
                stage_light_index: 0,
                env_probe_index: 0,
                map_from_self: owner.world_mat,
            });
        }
    }
    if !entries.is_empty() {
        let plan_v = &mut *plan_v;
        let fingerprint =
            ix_layout_digest(&plan_v.indices, &plan_v.facet_ranges, plan_v.verts.len());
        plan_v.plan_rev = bump_plan_mesh_rev(&mut plan_v.revisions, plan_v.plan_rev, fingerprint);
    }
    plan_v.publish_frame_rows(entries, owners);
}
