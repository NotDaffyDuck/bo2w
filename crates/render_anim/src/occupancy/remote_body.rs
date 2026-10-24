use crate::anim::remote_body::{
    PreparedRemoteKits, RemoteBodySkinnedItem, RemoteBodySkinnedQueue, occupy_lod_byte,
    occupy_remote_kit_dobj,
};
use crate::anim::scene_submission::{AnimDObjSceneSkels, AnimDObjSceneSubmission, AnimSceneSubmit};
use crate::gaps::{RenderGap, RenderGapCause, RenderPresentationGaps};
use crate::radius_lighting_box_half;
use crate::{
    RemoteBodyDrawPlan, append_remote_body_cpu_blob, finish_remote_body_draw_plan,
    install_body_packed_session, push_remote_body_cpu_draw, take_body_packed_session,
    topology_fingerprint,
};
use asset_model::ModelSkel;
use assets::{PreparedBodies, PreparedWeapons, PreparedWorldWeapons};
use bevy::prelude::*;
use frame::{ModelLightingSeated, ViewSubject, WorkerCmdSet};
use net::{
    CEntity, CEntityRuntime, ClientSet, LocalPresentClient, PlayerDrawGate,
    PresentedPublished, PresentedSnapshot, remote_body_submits, remote_pose_sample,
};
use render_scene::WorldModelLightingAtlas;
use render_scene::WorldScriptModelInstance;
use render_scene::{FlyCamera, FpvLens};
use render_scene::{LodRampArgs, smodel_camera_lod};
use render_scene::{
    ModelLightingOwner, ModelLightingRequest, ModelLightingRequests, ResolvedModelLighting,
    ResolvedModelLightingTable,
};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

#[derive(Component, Debug, Clone, Copy)]
#[require(RemoteFxBolts)]
pub struct RemotePlayer {
    pub ffa_team: Option<u8>,

    pub client_state_team: i32,
}

#[derive(Component, Default)]
pub struct RemoteFxBolts {
    pub flash: Option<fx::FxBoltTarget>,

    pub brass: Option<fx::FxBoltTarget>,

    pub knife: Option<fx::FxBoltTarget>,

    pub laser: Option<fx::FxBoltTarget>,
}

#[derive(Resource, Default)]
struct RemoteBodyLightingBinds {
    by_client: HashMap<u32, ModelLightingRequest>,
}

pub fn register_remote_body_systems(app: &mut App) {
    app.init_resource::<RenderPresentationGaps>()
        .init_resource::<RemoteBodyDrawPlan>()
        .init_resource::<RemoteBodySkinnedQueue>()
        .init_resource::<RemoteBodyLightingBinds>()
        .init_resource::<PreparedRemoteKits>()
        .add_systems(Update, prepare_remote_kits.in_set(ClientSet::Load))
        .init_resource::<crate::anim::dobj_pose::HostDObjPoseFrame>()
        .init_resource::<crate::anim::dobj_pose::PosedPlayerFrame>()
        .add_systems(
            Update,
            sync_remote_bodies
                .after(PresentedPublished)
                .before(occupy_remote_scene_ents)
                .in_set(render_scene::GfxSceneAdd),
        )
        .add_systems(
            Update,
            occupy_remote_scene_ents
                .after(sync_remote_bodies)
                .after(PresentedPublished)
                .after(crate::occupancy::fpv_present::occupy_fpv_scene)
                .in_set(render_scene::GfxSceneAdd)
                .in_set(AnimSceneSubmit),
        )
        .add_systems(
            Update,
            (
                begin_remote_body_tess
                    .after(sync_remote_bodies)
                    .before(pose_remote_bodies),
                pose_remote_bodies
                    .after(sync_remote_bodies)
                    .after(begin_remote_body_tess)
                    .after(crate::anim::dobj_pose::begin_dobj_pose_frame)
                    .after(WorkerCmdSet::CellStatic)
                    .after(WorkerCmdSet::CellSceneEnt)
                    .after(WorkerCmdSet::DpvsEnt),
            )
                .after(PresentedPublished)
                .in_set(WorkerCmdSet::SkinModel),
        )
        .add_systems(
            Update,
            crate::anim::dobj_pose::begin_dobj_pose_frame
                .after(PresentedPublished)
                .in_set(ClientSet::Present),
        )
        .add_systems(
            Update,
            enqueue_remote_body_lighting
                .after(WorkerCmdSet::SkinModel)
                .after(crate::occupancy::script_model::ScriptModelSkinSet)
                .before(WorkerCmdSet::CellDynModel)
                .after(PresentedPublished)
                .in_set(ClientSet::Present),
        )
        .add_systems(
            Update,
            (
                submit_remote_bodies.after(ModelLightingSeated),
                finish_body_draw_plan.after(submit_remote_bodies),
            )
                .in_set(WorkerCmdSet::AddSceneEnt),
        );
}

fn prepare_remote_kits(
    bodies: Option<Res<PreparedBodies>>,
    weapons: Option<Res<PreparedWeapons>>,
    world_weapons: Option<Res<PreparedWorldWeapons>>,
    mut kits: ResMut<PreparedRemoteKits>,
) {
    let (Some(bodies), Some(weapons), Some(world)) = (bodies, weapons, world_weapons) else {
        return;
    };
    if kits.owned_by(&bodies, &weapons, &world) {
        return;
    }
    let started = std::time::Instant::now();
    *kits = PreparedRemoteKits::prepare(&bodies, &weapons, &world);
    diag::info!(
        World,
        "remote kits: prepared for {} weapons in {:.1}ms",
        weapons.0.len(),
        started.elapsed().as_secs_f64() * 1000.0
    );
}

fn finish_body_draw_plan(mut plan: ResMut<RemoteBodyDrawPlan>) {
    finish_remote_body_draw_plan(&mut plan);
}

fn begin_remote_body_tess() {}

fn occupy_remote_scene_ents(
    mut scene_skels: ResMut<AnimDObjSceneSkels>,
    mut scene_submissions: MessageWriter<AnimDObjSceneSubmission>,
    lod_skinned: Res<render_scene::LodRampSkinnedDvar>,
    cameras: Query<&GlobalTransform, With<Camera3d>>,
    local: Res<LocalPresentClient>,
    presented: Res<PresentedSnapshot>,
    subject: Option<Res<ViewSubject>>,
    bodies: Option<Res<PreparedBodies>>,
    weapons: Option<Res<PreparedWeapons>>,
    world_weapons: Option<Res<PreparedWorldWeapons>>,
    prepared_kits: Res<PreparedRemoteKits>,
    remotes: Query<
        (&CEntity, &CEntityRuntime, &Transform),
        (
            Without<WorldScriptModelInstance>,
            Without<FpvLens>,
            Without<FlyCamera>,
        ),
    >,
) {
    let Some(bodies) = bodies.as_deref() else {
        return;
    };
    if let Some(world) = world_weapons.as_deref() {
        scene_skels.seed(bodies, world);
    }
    let live_kits = match (weapons.as_deref(), world_weapons.as_deref()) {
        (Some(weapons), Some(world)) if prepared_kits.owned_by(bodies, weapons, world) => {
            Some((&*prepared_kits, world))
        }
        _ => None,
    };
    let kits = bodies.0.kits();
    if kits.allies.is_none() && kits.axis.is_none() {
        return;
    }
    let eye = cameras.single().ok().map(|xf| xf.translation());
    let in_killcam = subject.as_ref().is_some_and(|s| s.in_killcam());
    let eyes = match subject.as_deref() {
        Some(ViewSubject::Seat {
            focus: Some(focus), ..
        }) => i32::try_from(*focus).unwrap_or(0),
        _ => i32::try_from(local.0.0).unwrap_or(0),
    };
    let gate = PlayerDrawGate {
        eyes_entity_num: eyes,
        other_flags: presented
            .player(local.0)
            .map(|ps| ps.other_flags)
            .unwrap_or(0),
        rendering_third_person: crate::occupancy::third_person::presented_is_third_person(
            &presented, local.0, in_killcam,
        ),
    };
    for (identity, runtime, transform) in &remotes {
        if !remote_body_submits(identity.number(), runtime, gate) {
            continue;
        }
        let Some(client) = identity.client() else {
            continue;
        };
        let meta = presented
            .snapshot()
            .and_then(|snap| snap.meta.for_client(client));
        let ffa_team = meta.and_then(|m| m.ffa_team);
        let client_state_team = meta.map(|m| m.client_state_team).unwrap_or(0);
        let axis = asset_model::kit_assignment_is_axis(client_state_team, ffa_team);
        let held = remote_pose_sample(runtime).weapon;
        let (kit_models, radius, hide_part_bits) = match live_kits {
            Some((kits, world)) => {
                let Some(kit) = kits.get(axis, held) else {
                    continue;
                };
                let Some(models) = kit.models(bodies, world) else {
                    continue;
                };
                (models, kit.radius, kit.hide_part_bits)
            }
            None => {
                let Some((models, radius)) = occupy_remote_kit_dobj(
                    bodies,
                    weapons.as_deref(),
                    world_weapons.as_deref(),
                    axis,
                    held,
                    true,
                ) else {
                    continue;
                };
                let bits = crate::anim::remote_body::kit_hide_part_bits(&models);
                (models, radius, bits)
            }
        };
        let origin = transform.translation.to_array();
        let quat = [
            transform.rotation.x,
            transform.rotation.y,
            transform.rotation.z,
            transform.rotation.w,
        ];
        let entnum = u32::from(identity.number());

        let ramp = lod_skinned.args();
        let lod_bytes: Vec<i8> = kit_models
            .iter()
            .map(|model| occupy_lod_byte(skel_camera_lod(model.skel, origin, eye, ramp)))
            .collect();
        let models: Vec<_> = kit_models
            .iter()
            .enumerate()
            .map(|(i, model)| {
                scene_skels.model(
                    model.name,
                    model.skel,
                    lod_bytes.get(i).copied().unwrap_or(-1),
                )
            })
            .collect();
        scene_submissions.write(AnimDObjSceneSubmission {
            render_fx_flags: 0,
            has_tree: true,
            origin,
            lighting_origin: origin,
            radius,
            entnum,
            quat: Some(quat),
            occupy_model_n: u8::try_from(models.len()).unwrap_or(u8::MAX),
            models,
            hide_part_bits,
            store_skin: true,
        });
    }
}

fn sync_remote_bodies(
    mut commands: Commands,
    local: Res<LocalPresentClient>,
    presented: Res<PresentedSnapshot>,
    subject: Option<Res<ViewSubject>>,
    bodies: Option<Res<PreparedBodies>>,
    existing: Query<(Entity, &CEntity, &CEntityRuntime, Option<&RemotePlayer>)>,
) {
    let _slow = crate::occupancy::t6_body::SlowGuard::new("sync_remote_bodies");
    let Some(bodies) = bodies else {
        return;
    };

    let kits = bodies.0.kits();
    if kits.allies.is_none() && kits.axis.is_none() {
        return;
    }

    let in_killcam = subject.as_ref().is_some_and(|s| s.in_killcam());
    let eyes = match subject.as_deref() {
        Some(ViewSubject::Seat {
            focus: Some(focus), ..
        }) => i32::try_from(*focus).unwrap_or(0),
        _ => i32::try_from(local.0.0).unwrap_or(0),
    };
    let gate = PlayerDrawGate {
        eyes_entity_num: eyes,
        other_flags: presented
            .player(local.0)
            .map(|ps| ps.other_flags)
            .unwrap_or(0),
        rendering_third_person: crate::occupancy::third_person::presented_is_third_person(
            &presented, local.0, in_killcam,
        ),
    };

    for (entity, identity, runtime, remote) in &existing {
        let Some(client) = identity.client() else {
            if remote.is_some() {
                commands
                    .entity(entity)
                    .remove::<(RemotePlayer, RemoteFxBolts)>();
            }
            continue;
        };
        if !remote_body_submits(identity.number(), runtime, gate) {
            if remote.is_some() {
                commands
                    .entity(entity)
                    .remove::<(RemotePlayer, RemoteFxBolts)>();
            }
            continue;
        }
        let meta = presented
            .snapshot()
            .and_then(|snap| snap.meta.for_client(client));
        let ffa_team = meta.and_then(|m| m.ffa_team);
        let client_state_team = meta.map(|m| m.client_state_team).unwrap_or(0);
        let Some(kit) = kits.kit(asset_model::kit_assignment_is_axis(client_state_team, ffa_team))
        else {
            if remote.is_some() {
                commands
                    .entity(entity)
                    .remove::<(RemotePlayer, RemoteFxBolts)>();
            }
            continue;
        };
        let Some(body_entry) = bodies.0.get(&kit.body) else {
            if remote.is_some() {
                commands
                    .entity(entity)
                    .remove::<(RemotePlayer, RemoteFxBolts)>();
            }
            continue;
        };
        if body_entry.skel.positions.is_empty() || body_entry.skel.bones.is_empty() {
            if remote.is_some() {
                commands
                    .entity(entity)
                    .remove::<(RemotePlayer, RemoteFxBolts)>();
            }
            continue;
        }

        let origin = runtime.origin;
        let yaw = runtime.angles[1];
        let pose = Transform {
            translation: Vec3::from_array(origin),
            rotation: Quat::from_rotation_z(yaw.to_radians()),
            scale: Vec3::ONE,
        };
        let marker = RemotePlayer {
            ffa_team,
            client_state_team,
        };
        if remote.is_some() {
            commands.entity(entity).insert((pose, marker));
        } else {
            attach_remote_root(&mut commands, entity, pose, marker);
        }
    }
}

/// MW2's animation tree and playeranim script posed these bodies; BO2 bodies are
/// posed by t6_body, so this only clears last frame's queue and bolts.
fn pose_remote_bodies(
    mut submit: ResMut<RemoteBodySkinnedQueue>,
    mut roots: Query<
        &mut RemoteFxBolts,
        (
            With<RemotePlayer>,
            Without<WorldScriptModelInstance>,
            Without<render_scene::FpvLens>,
            Without<render_scene::FlyCamera>,
        ),
    >,
) {
    submit.clear();
    for mut bolts in &mut roots {
        *bolts = RemoteFxBolts::default();
    }
}

fn skel_camera_lod(
    skel: &ModelSkel,
    origin: [f32; 3],
    eye: Option<Vec3>,
    ramp: LodRampArgs,
) -> Option<u8> {
    smodel_camera_lod(skel.lod, origin, 1.0, eye, ramp)
}

fn enqueue_remote_body_lighting(
    submit: ResMut<RemoteBodySkinnedQueue>,
    mut binds: ResMut<RemoteBodyLightingBinds>,
    atpoint: Res<render_scene::DynAtPointLookup>,
    mut lighting_requests: ResMut<ModelLightingRequests>,
) {
    binds.by_client.clear();
    if submit.is_empty() {
        return;
    }
    for item in submit.iter() {
        let box_half = item.radius.map(radius_lighting_box_half);
        let lookup_fallback = atpoint.fallback(item.origin, box_half);
        let client = u16::try_from(item.client).unwrap_or(u16::MAX);
        binds.by_client.insert(
            item.client,
            lighting_requests.request(ModelLightingRequest {
                owner: ModelLightingOwner::RemoteClient(client),
                origin: item.origin,
                lookup_fallback,
            }),
        );
    }
}

fn submit_remote_bodies(
    mut plan: ResMut<RemoteBodyDrawPlan>,
    gaps: Res<RenderPresentationGaps>,
    mut submit: ResMut<RemoteBodySkinnedQueue>,
    mut binds: ResMut<RemoteBodyLightingBinds>,
    resolved: Res<ResolvedModelLightingTable>,
    atlas: Option<Res<WorldModelLightingAtlas>>,
    tess: Option<Res<render_scene::TessMaterials>>,
    model_materials: Res<crate::anim::model_materials::PreparedModelMaterials>,
    mut last_catalog: Local<Option<Arc<render_material::RuntimeMaterialCatalog>>>,
) {
    let items = submit.take();
    let binds = std::mem::take(&mut binds.by_client);
    if items.is_empty() {
        if plan.decoded_n != 0 || plan.last_packed_id.is_some() {
            plan.clear_geometry();
        }
        *last_catalog = None;
        return;
    }
    if atlas.is_none() {
        plan.clear_geometry();
        *last_catalog = None;
        gaps.raise(RenderGapCause::RemoteBodyLightingAllocFailed);
        return;
    }
    let Some(tess) = tess else {
        plan.clear_geometry();
        *last_catalog = None;
        gaps.raise(RenderGapCause::RemoteBodyLightingAllocFailed);
        return;
    };
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
        let seated_light = binds
            .get(&item.client)
            .and_then(|request| resolved.get(request.owner));
        let Some((handle, scene_light, probe)) = (match seated_light {
            Some(ResolvedModelLighting::Seated {
                handle,
                scene_light_index,
                reflection_probe_index,
                ..
            }) => Some((handle, scene_light_index, reflection_probe_index)),
            Some(ResolvedModelLighting::Failed) | None => {
                last_cause = Some(RenderGapCause::RemoteBodyLightingAllocFailed);
                None
            }
        }) else {
            continue;
        };
        seated.push((item, handle, scene_light, probe));
    }
    if seated.is_empty() {
        if plan.decoded_n != 0 || plan.last_packed_id.is_some() {
            plan.clear_geometry();
        }
        *last_catalog = None;
        gaps.raise(last_cause.unwrap_or(RenderGapCause::RemoteBodySubmitMissing));
        return;
    }

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
    let vert_n: usize = seated
        .iter()
        .map(|(item, _, _, _)| item.geom.decoded_n)
        .sum();
    let idx_n: usize = seated
        .iter()
        .map(|(item, _, _, _)| item.geom.indices.len())
        .sum();
    plan.indices.reserve(idx_n);
    let mut session = take_body_packed_session(&mut plan);
    session.reserve_packed(vert_n);
    let mut mat_by_name: HashMap<String, u32> = HashMap::new();
    let mut submitted_verts = 0usize;
    let mut any_missing_material = false;
    for (item, handle, scene_light, probe) in seated {
        let before = plan.decoded_n;
        let appended = append_remote_body_cpu_blob(
            &mut plan,
            &mut session,
            &item.geom.packed,
            &item.geom.indices,
            item.geom.decoded_n,
        );
        let index_base = appended.map(|(_, index_base)| index_base);
        for surface in item.geom.surfaces.iter() {
            let Some(name) = surface.name.as_deref() else {
                any_missing_material = true;
                last_cause = Some(RenderGapCause::RemoteBodyMaterialMissing {
                    name: String::new(),
                });
                continue;
            };
            let mat_idx = if let Some(&idx) = mat_by_name.get(name) {
                idx
            } else {
                let Some(material) = model_materials.material(&tess.catalog, name).cloned() else {
                    any_missing_material = true;
                    last_cause = Some(RenderGapCause::RemoteBodyMaterialMissing {
                        name: name.to_owned(),
                    });
                    continue;
                };
                let idx = plan.materials.len() as u32;
                plan.materials.push(material);
                mat_by_name.insert(name.to_owned(), idx);
                idx
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
                    mat_idx,
                    Some(item.client),
                );
            }
        }
        if plan.decoded_n > before {
            submitted_verts += plan.decoded_n - before;
        }
    }
    install_body_packed_session(&mut plan, session);
    if submitted_verts > 0 && !any_missing_material {
        plan.revisions.bump_surfaces();
        plan.revisions.bump_draws();
        let topology = topology_fingerprint(&plan.indices, &plan.surface_ranges, plan.decoded_n);
        if plan.revisions.topology != topology {
            plan.revisions.topology = topology;
        }
        plan.revision = plan.revision.wrapping_add(1);
        plan.last_packed_id = Some(packed_id);
        plan.last_draw_id = Some(draw_id);
        gaps.clear(RenderGap::RemoteBodyAnimation);
        return;
    }
    gaps.raise(last_cause.unwrap_or(RenderGapCause::RemoteBodySubmitMissing));
}

fn seated_packed_id(seated: &[(RemoteBodySkinnedItem, u32, u8, u8)]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    seated.len().hash(&mut hasher);
    for (item, _, _, _) in seated {
        item.client.hash(&mut hasher);
        item.geometry_revision.hash(&mut hasher);
        item.geom.decoded_n.hash(&mut hasher);
    }
    hasher.finish()
}

fn seated_draw_id(seated: &[(RemoteBodySkinnedItem, u32, u8, u8)]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    seated.len().hash(&mut hasher);
    for (item, handle, scene_light, probe) in seated {
        item.client.hash(&mut hasher);
        for lane in item.world_from_local.to_cols_array() {
            lane.to_bits().hash(&mut hasher);
        }
        handle.hash(&mut hasher);
        scene_light.hash(&mut hasher);
        probe.hash(&mut hasher);
    }
    hasher.finish()
}

fn seated_clients_match(
    plan: &RemoteBodyDrawPlan,
    seated: &[(RemoteBodySkinnedItem, u32, u8, u8)],
) -> bool {
    let mut live: Vec<u32> = seated.iter().map(|(item, _, _, _)| item.client).collect();
    let mut drawn: Vec<u32> = plan.draws.iter().filter_map(|d| d.scene_entnum).collect();
    live.sort_unstable();
    live.dedup();
    drawn.sort_unstable();
    drawn.dedup();
    live == drawn
}

fn refresh_remote_body_draws(
    plan: &mut RemoteBodyDrawPlan,
    seated: &[(RemoteBodySkinnedItem, u32, u8, u8)],
) {
    for draw in &mut plan.draws {
        let Some(ent) = draw.scene_entnum else {
            continue;
        };
        let Some((_, handle, scene_light, probe, world)) =
            seated
                .iter()
                .find_map(|(item, handle, scene_light, probe)| {
                    (item.client == ent).then_some((
                        item,
                        *handle,
                        *scene_light,
                        *probe,
                        item.world_from_local,
                    ))
                })
        else {
            continue;
        };
        draw.world_from_local = world;
        draw.lighting_handle = handle;
        draw.scene_light_index = scene_light;
        draw.reflection_probe_index = probe;
    }
}

fn attach_remote_root(
    commands: &mut Commands,
    entity: Entity,
    transform: Transform,
    remote: RemotePlayer,
) {
    commands
        .entity(entity)
        .insert((remote, transform, Visibility::default()));
}
