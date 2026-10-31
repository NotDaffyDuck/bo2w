//! Bullet holes and blood that stick to moving models (players, script
//! models) and ride along with the bone they landed on.
//!
//! Hits are recorded by [`queue`] while the world marks are made. Later in
//! the frame [`generate`] poses each struck model, cuts the decal out of its
//! rigidly skinned triangles in bone space, and draws every mark a model
//! carries at that bone's current pose.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use bevy::prelude::*;
use marks_t6::{
    DecalAllocationAsk, EffectStainStagingPoint, EffectStainStagingTri, GfxMarkContext,
    WorldDecalVertex,
};
use render_fx::{
    BoneSlot, BoundEntityMark, EntityMarkBoard, EntityMarkState, MarkPaint, MarkPlacement,
    PendingEntityMark, Refusals,
};

use crate::prepare::scene::world::LevelScriptModelInstance;
use crate::prepare::scene::world::LevelStage;

/// A bone whose scale strays further than this from 1 would warp a mark.
const UNIT_SCALE_SLACK: f32 = 0.0001;

fn place_point(pose: &Mat4, pt: [f32; 3]) -> [f32; 3] {
    pose.transform_point3(Vec3::from(pt)).into()
}

fn turn_vector(pose: &Mat4, dir_v: [f32; 3]) -> [f32; 3] {
    pose.transform_vector3(Vec3::from(dir_v)).into()
}

/// The host's most recent decal, if every part of it is known.
fn latest_hit(host: &fx::EffectSystemHost) -> Option<PendingEntityMark> {
    let target = host.last_stain_entity?;
    let placement = MarkPlacement {
        spot: host.last_stain_origin?,
        turn: fx_t6::runtime::impact_mark_axis(host.last_stain_axis?, host.last_stain_rotation?),
        reach_r: host.last_stain_size0?,
    };
    let paint = MarkPaint {
        shader: host.last_stain_mat0.as_deref()?.to_owned(),
        tint: host.last_stain_color?,
    };
    Some(PendingEntityMark {
        target,
        placement,
        paint,
    })
}

/// Record the host's latest decal as a hit on the entity it struck, when the
/// host puts marks on entities at all.
pub(crate) fn queue(host: &fx::EffectSystemHost, marks: &EntityMarkBoard) {
    if !host.stain_receivers.stains_on_ents {
        return;
    }
    if let Some(hit) = latest_hit(host) {
        marks.hold_marks().enqueue(hit);
    }
}

/// A mark stores which of the entity's models it sits on in a few bits of
/// its type byte; slots that do not fit cannot carry marks.
fn mark_model_slot(slot: usize) -> Option<u8> {
    let narrow = u8::try_from(slot).ok()?;
    (narrow <= marks_t6::MARK_MODEL_DOBJ_MODEL_MASK).then_some(narrow)
}

/// How a triangle is skinned, as far as marks care.
#[derive(Debug, PartialEq, Eq)]
enum Binding {
    /// No skin data for the leading corner: pass over it silently.
    NoSkin,
    /// Shared between bones or blended: a mark cannot follow it.
    Blended,
    /// All three corners belong wholly to this one bone.
    Bone(usize),
}

fn rigid_binding(skins: &[asset_model::CornerWeights], corners: [usize; 3]) -> Binding {
    let Some(lead) = skins.get(corners[0]) else {
        return Binding::NoSkin;
    };
    let bone = lead.joints[0];
    let fits_a_byte = usize::from(bone) <= usize::from(u8::MAX);
    let wholly_on_bone = |corner: usize| matches!(skins.get(corner), Some(w) if w.joints[0] == bone && w.weights == [1.0, 0.0, 0.0, 0.0]);
    if fits_a_byte && corners.into_iter().all(wholly_on_bone) {
        Binding::Bone(usize::from(bone))
    } else {
        Binding::Blended
    }
}

/// True when the bone pose has no scaling, so a mark keeps its shape on it.
fn keeps_shape(bone_pose: &Mat4) -> bool {
    let (scale, _, _) = bone_pose.to_scale_rotation_translation();
    scale.abs_diff_eq(Vec3::ONE, UNIT_SCALE_SLACK)
}

/// Three per-corner values of a triangle, or `None` if one is missing.
fn per_corner(values: &[[f32; 3]], corners: [usize; 3]) -> Option<[[f32; 3]; 3]> {
    let [a, b, c] = corners.map(|corner| values.get(corner).copied());
    Some([a?, b?, c?])
}

/// A struck model's skeleton and this frame's bone poses.
struct OwnerPose<'a> {
    rigs: Vec<&'a asset_model::MeshRig>,
    /// Where each shown model's bones start in `bones`.
    first_bone: Vec<usize>,
    bones: Vec<Mat4>,
}

/// Pose the owner, or give up (`None`) if it shows no models or any step of
/// building, animating or posing it fails.
fn pose_of<'a>(
    owner: &LevelScriptModelInstance,
    xf: &Transform,
    anims: Option<&assets::StagedXAnims>,
    roster: &'a asset_world::LevelModelPlacementRoster,
) -> Option<OwnerPose<'a>> {
    let puppet = &owner.puppet_state;
    if puppet.composition.models.is_empty() {
        return None;
    }
    let (specs, rigs) = crate::adapters::motion::script_prop::gather_shown_models(roster, puppet)?;
    let dobj = xmodel_runtime::PosedSkeleton::build(&specs).ok()?;
    let query = puppet
        .look_up_pose_query(|clip| {
            anims.and_then(|set| set.0.clip(asset_core::AssetCategoryScope::T6, clip))
        })
        .ok()?;
    let posed = xmodel_runtime::pose_dobj(&dobj, &query, xf.to_matrix()).ok()?;
    Some(OwnerPose {
        first_bone: dobj.models.iter().map(|shown| shown.first_joint).collect(),
        bones: dobj.skin_matrices(&posed),
        rigs,
    })
}

/// This frame's entity marks: land the new hits on the models they struck,
/// then add every mark still worn by a visible model to the mark mesh.
pub(crate) fn make_up(
    host: &mut fx::EffectSystemHost,
    scene: &LevelStage,
    marks: &EntityMarkBoard,
    owners: &Query<(&Visibility, &LevelScriptModelInstance, &Transform)>,
    animation_refs: Option<&assets::StagedXAnims>,
    models: &asset_world::LevelModelPlacementRoster,
) {
    let mut board = marks.hold_marks();
    let fresh_hits = board.drain_queue();
    let refusals_at_start = board.refusals();
    let Some(mut mesh) = host.last_stain_mesh.take() else {
        panic!("entity marks run after the world mark mesh is built");
    };
    let draw_worn = host.stain_receivers.stains && host.stain_receivers.stains_on_ents;

    for (vis_state, holder, transform) in owners.iter() {
        let Some(id) = holder.game_actor_number else {
            continue;
        };
        if matches!(vis_state, Visibility::Hidden) {
            continue;
        }
        let hits: Vec<&PendingEntityMark> =
            fresh_hits.iter().filter(|hit| hit.target == id).collect();
        if hits.is_empty() && board.bound_to(id).next().is_none() {
            continue;
        }
        let Some(pose) = pose_of(holder, transform, animation_refs, models) else {
            board.refuse_receivers(hits.len());
            continue;
        };
        for hit in hits {
            land_on_models(host, scene, &mut board, hit, holder, &pose, models);
        }
        if draw_worn {
            draw_worn_marks(host, &mut mesh, &board, holder, id, &pose.bones);
        }
    }

    report_refusals(refusals_at_start, board.refusals());
    let budget = &mesh.budget;
    host.stain_surfs = Some(budget.facet_sum);
    host.stain_verts = Some(budget.vertex_amount);
    host.stain_indices = Some(budget.index_tally);
    host.visual_stain_compact_tally = Some(mesh.vert_blob.len() as u32);
    host.last_stain_mesh = Some(mesh);
}

/// Try one hit against each model the owner shows.
fn land_on_models(
    host: &mut fx::EffectSystemHost,
    scene: &LevelStage,
    board: &mut EntityMarkState,
    hit: &PendingEntityMark,
    owner: &LevelScriptModelInstance,
    pose: &OwnerPose<'_>,
    roster: &asset_world::LevelModelPlacementRoster,
) {
    let composition = &owner.puppet_state.composition;
    for (slot, &rig) in pose.rigs.iter().enumerate() {
        let shown = pose.first_bone.get(slot).zip(composition.models.get(slot));
        let Some((&first_bone, desc)) = shown else {
            board.refuse_receivers(1);
            continue;
        };
        let target = MarkTarget {
            owner,
            rig,
            bones: &pose.bones,
            first_bone,
            model_name: desc.model.as_str(),
            rev: composition.revision,
        };
        let Some(model_slot) = mark_model_slot(slot) else {
            board.refuse_receivers(1);
            continue;
        };
        let context = GfxMarkContext::entity_model(hit.target, model_slot);
        let cut = target.cut(scene, board, hit, roster, context);
        target.attach(host, board, hit, cut);
    }
}

/// Redraw each mark the owner still wears, moved by its bone's pose. Stops
/// as soon as the mesh is full.
fn draw_worn_marks(
    host: &fx::EffectSystemHost,
    mesh: &mut fx::DecalMeshTally,
    board: &EntityMarkState,
    owner: &LevelScriptModelInstance,
    id: u16,
    bones: &[Mat4],
) {
    let puppet = &owner.puppet_state;
    let worn_now = &puppet.composition;
    let still_worn = |worn: &&BoundEntityMark| {
        worn.slot.edit_no == worn_now.revision
            && worn_now
                .models
                .iter()
                .any(|shown| shown.model == worn.slot.prop)
    };
    for worn in board.bound_to(id).filter(still_worn) {
        let bone = worn.slot.joint;
        if puppet.hidden_mask.get(bone) {
            continue;
        }
        let (Some(&pose), Some(&built)) = (bones.get(bone), host.marks.built_record(worn.handle))
        else {
            continue;
        };
        let carry = move |mut corner: EffectStainStagingPoint| {
            corner.spot = place_point(&pose, corner.spot);
            let turned = pose.transform_vector3(Vec3::from(corner.normal));
            corner.normal = turned.normalize().into();
            corner
        };
        let added = host.marks.add_mesh_decal(
            mesh,
            worn.handle,
            place_point(&pose, built.origin),
            turn_vector(&pose, built.uv_projection_axis),
            &carry,
        );
        if added.is_err() {
            break;
        }
    }
}

/// With `BO2MP_RENDER_LOG` set, say how many hits and triangles this frame
/// could not take a mark. Off by default: it fires on nearly every hit.
fn report_refusals(start: Refusals, end: Refusals) {
    static VERBOSE: OnceLock<bool> = OnceLock::new();
    if !*VERBOSE.get_or_init(|| std::env::var_os("BO2MP_RENDER_LOG").is_some()) {
        return;
    }
    let skipped_triangles = end.tri_total - start.tri_total;
    if skipped_triangles > 0 {
        diag::warn!(
            World,
            "entity marks: skipped {} triangles not bound rigidly to one bone",
            skipped_triangles
        );
    }
    let skipped_hits = end.receivers - start.receivers;
    if skipped_hits > 0 {
        diag::warn!(
            World,
            "entity marks: {} hits found no rigid posed model to land on",
            skipped_hits
        );
    }
}

/// One shown model of a struck entity.
struct MarkTarget<'a> {
    owner: &'a LevelScriptModelInstance,
    rig: &'a asset_model::MeshRig,
    bones: &'a [Mat4],
    first_bone: usize,
    model_name: &'a str,
    rev: u32,
}

impl MarkTarget<'_> {
    /// Index triples of one surface's triangles (none if its range is bad).
    fn surface_triangles(&self, surface: usize) -> impl Iterator<Item = [usize; 3]> + '_ {
        let rig = self.rig;
        let indices = rig
            .facet_index_ranges
            .get(surface)
            .and_then(|&(first, len)| rig.indices.get(first..first + len))
            .unwrap_or(&[]);
        indices
            .chunks_exact(3)
            .map(|tri| [tri[0], tri[1], tri[2]].map(|index| index as usize))
    }

    /// Clip the hit against every rigid, unhidden, markable triangle and
    /// gather the pieces per bone, in that bone's own space.
    fn cut(
        &self,
        scene: &LevelStage,
        board: &mut EntityMarkState,
        hit: &PendingEntityMark,
        roster: &asset_world::LevelModelPlacementRoster,
        context: GfxMarkContext,
    ) -> BTreeMap<usize, BoneDecal> {
        let asset_id = asset_world::LevelModelAssetId(self.model_name.to_owned());
        let hit_kinds = super::world_decal::live_material_by_name(scene, &hit.paint.shader)
            .and_then(|material| material.facet_type_bits);
        let MarkPlacement {
            spot: origin,
            turn: axis,
            reach_r: radius,
        } = hit.placement;
        let clip = marks_t6::mark_fragment_clip_planes(origin, axis, radius);
        let mut polygon = [WorldDecalVertex::ZERO; marks_t6::DECAL_CLIP_POLYGON_VERTEX_CEILING];
        let mut per_bone: BTreeMap<usize, BoneDecal> = BTreeMap::new();

        for surface in self.rig.facets_at_tier(0) {
            let is_rigid_surface = self.rig.bent.get(surface) == Some(&Some(false));
            if !is_rigid_surface {
                continue;
            }
            let surface_kind =
                super::world_decal::mark_receiver(scene, roster.facet_material(&asset_id, surface));
            let allowed = marks_t6::mark_allow(surface_kind, hit_kinds);
            if !marks_t6::mark_include_in_world_clip(allowed) {
                continue;
            }
            for corners in self.surface_triangles(surface) {
                let bone = match rigid_binding(&self.rig.vert_skin, corners) {
                    Binding::NoSkin => continue,
                    Binding::Blended => {
                        board.refuse_triangle();
                        continue;
                    }
                    Binding::Bone(local) => self.first_bone + local,
                };
                if self.owner.puppet_state.hidden_mask.get(bone) {
                    continue;
                }
                let Some(bone_pose) = self.bones.get(bone).copied() else {
                    continue;
                };
                if !keeps_shape(&bone_pose) {
                    board.refuse_triangle();
                    continue;
                }
                let Some(rest) = per_corner(&self.rig.positions, corners) else {
                    continue;
                };
                let [a, b, c] = rest.map(|corner| place_point(&bone_pose, corner));
                if marks_t6::mark_is_triangle_rejected(axis[0], a, b, c) {
                    continue;
                }
                let kept = marks_t6::mark_chop_world_triangle_points(&clip, a, b, c, &mut polygon)
                    as usize;
                if kept < 3 {
                    continue;
                }
                let Some(normals) = per_corner(&self.rig.surface_directions, corners) else {
                    continue;
                };
                let into_bone = bone_pose.inverse();
                polygon[..kept]
                    .iter_mut()
                    .for_each(|vertex| vertex.spot = place_point(&into_bone, vertex.spot));
                per_bone
                    .entry(bone)
                    .or_default()
                    .absorb(&polygon[..kept], normals, context);
            }
        }
        per_bone
    }

    /// Turn each bone's pieces into a live mark tied to that bone.
    fn attach(
        &self,
        fx_host: &mut fx::EffectSystemHost,
        board: &mut EntityMarkState,
        hit: &PendingEntityMark,
        per_bone: BTreeMap<usize, BoneDecal>,
    ) {
        for (bone, decal) in per_bone {
            let Some(lead_context) = decal.tris.first().map(|tri| tri.context) else {
                continue;
            };
            let into_bone = self.bones[bone].inverse();
            let material = 0;
            let ask = DecalAllocationAsk {
                any_stains: true,
                tri_tally: decal.tris.len() as u32,
                vertex_total_in_mark: decal.pts.len() as u32,
                origin: place_point(&into_bone, hit.placement.spot),
                radius: hit.placement.reach_r,
                uv_projection_axis: turn_vector(&into_bone, hit.placement.turn[1]),
                authored_tint: hit.paint.tint,
                material,
                anim_steps: fx_host.frame_tick_no,
                leading_triangle_mark_info: lead_context,
            };
            let claimed = fx_host
                .marks
                .claim_decal_for_scene_hook(ask, &decal.tris, &decal.pts);
            let Some(handle) = claimed else {
                continue;
            };
            fx_host.marks
                .assign_material_name(handle, Some(&hit.paint.shader));
            board.bind(BoundEntityMark {
                target: hit.target,
                slot: BoneSlot {
                    prop: self.model_name.to_owned(),
                    edit_no: self.rev,
                    joint: bone,
                },
                handle,
            });
        }
    }
}

/// Decal pieces cut for one bone, held in that bone's space.
#[derive(Default)]
struct BoneDecal {
    tris: Vec<EffectStainStagingTri>,
    pts: Vec<EffectStainStagingPoint>,
}

impl BoneDecal {
    /// Emit one clipped polygon into the pieces. The arrays are grown to
    /// capacity for the emitter and trimmed to what it filled; a polygon that
    /// does not fit leaves the pieces unchanged.
    fn absorb(
        &mut self,
        polygon: &[WorldDecalVertex],
        normals: [[f32; 3]; 3],
        context: GfxMarkContext,
    ) {
        let tri_cap = marks_t6::DECAL_FRAGMENT_TRIANGLE_CEILING;
        let point_cap = marks_t6::DECAL_FRAGMENT_VERTEX_CEILING;
        let before = (self.tris.len(), self.pts.len());
        self.tris
            .resize(tri_cap as usize, EffectStainStagingTri::ZERO);
        self.pts
            .resize(point_cap as usize, EffectStainStagingPoint::ZERO);
        let [n0, n1, n2] = normals;
        let no_uv = [0.0; 2];
        let (tris_used, points_used) = marks_t6::mark_emit_brush_fragment(
            before.0 as u32,
            before.1 as u32,
            tri_cap,
            point_cap,
            polygon,
            no_uv,
            no_uv,
            no_uv,
            n0,
            n1,
            n2,
            context,
            &mut self.tris,
            &mut self.pts,
        )
        .map_or(before, |(tris, points)| (tris as usize, points as usize));
        self.tris.truncate(tris_used);
        self.pts.truncate(points_used);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asset_model::CornerWeights;

    fn whole(joint: u16) -> CornerWeights {
        CornerWeights {
            joints: [joint; 4],
            weights: Vec4::X.to_array(),
            pull_u16: [u16::MAX, 0, 0, 0],
        }
    }

    fn up() -> [[f32; 3]; 3] {
        [[0.0, 0.0, 1.0]; 3]
    }

    #[test]
    fn model_slot_fits_only_up_to_the_mask() {
        let top = marks_t6::MARK_MODEL_DOBJ_MODEL_MASK;
        assert_eq!(mark_model_slot(0), Some(0));
        assert_eq!(mark_model_slot(usize::from(top)), Some(top));
        assert_eq!(mark_model_slot(usize::from(top) + 1), None);
        assert_eq!(mark_model_slot(usize::MAX), None);
    }

    #[test]
    fn one_bone_triangle_binds_to_it() {
        let skins = vec![whole(7); 3];
        assert_eq!(rigid_binding(&skins, [0, 1, 2]), Binding::Bone(7));
    }

    #[test]
    fn triangle_spanning_two_bones_is_blended() {
        let skins = [whole(7), whole(7), whole(8)];
        assert_eq!(rigid_binding(&skins, [0, 1, 2]), Binding::Blended);
    }

    #[test]
    fn half_weighted_corner_is_blended() {
        let mut split = whole(7);
        split.weights = [0.5, 0.5, 0.0, 0.0];
        let skins = [whole(7), split, whole(7)];
        assert_eq!(rigid_binding(&skins, [0, 1, 2]), Binding::Blended);
    }

    #[test]
    fn bones_beyond_a_byte_are_blended() {
        assert_eq!(
            rigid_binding(&vec![whole(256); 3], [0, 1, 2]),
            Binding::Blended
        );
        assert_eq!(
            rigid_binding(&vec![whole(255); 3], [0, 1, 2]),
            Binding::Bone(255)
        );
    }

    #[test]
    fn missing_lead_corner_is_no_skin() {
        assert_eq!(rigid_binding(&[whole(1)], [3, 0, 0]), Binding::NoSkin);
    }

    #[test]
    fn missing_trailing_corner_is_blended() {
        assert_eq!(rigid_binding(&[whole(1)], [0, 0, 5]), Binding::Blended);
    }

    #[test]
    fn scaled_bones_do_not_keep_shape() {
        assert!(keeps_shape(&Mat4::IDENTITY));
        assert!(keeps_shape(&Mat4::from_translation(Vec3::new(
            5.0, 6.0, 7.0
        ))));
        assert!(keeps_shape(&Mat4::from_rotation_z(1.0)));
        assert!(!keeps_shape(&Mat4::from_scale(Vec3::splat(1.01))));
        assert!(!keeps_shape(&Mat4::from_scale(Vec3::new(1.0, 1.0, 0.5))));
    }

    #[test]
    fn polygons_pile_up_while_they_fit() {
        let polygon = [WorldDecalVertex::ZERO; 3];
        let context = GfxMarkContext::entity_model(4, 1);
        let mut decal = BoneDecal::default();
        decal.absorb(&polygon, up(), context);
        let (tris, points) = (decal.tris.len(), decal.pts.len());
        assert!(tris > 0 && points >= 3);
        decal.absorb(&polygon, up(), context);
        assert!(decal.tris.len() > tris && decal.pts.len() > points);
    }

    #[test]
    fn full_decal_ignores_more_polygons() {
        let polygon = [WorldDecalVertex::ZERO; 3];
        let context = GfxMarkContext::entity_model(4, 1);
        let mut decal = BoneDecal::default();
        let rounds = marks_t6::DECAL_FRAGMENT_TRIANGLE_CEILING as usize + 8;
        (0..rounds).for_each(|_| decal.absorb(&polygon, up(), context));
        assert!(decal.tris.len() <= marks_t6::DECAL_FRAGMENT_TRIANGLE_CEILING as usize);
        assert!(decal.pts.len() <= marks_t6::DECAL_FRAGMENT_VERTEX_CEILING as usize);
        let held = (decal.tris.len(), decal.pts.len());
        decal.absorb(&polygon, up(), context);
        assert_eq!((decal.tris.len(), decal.pts.len()), held);
    }
}
