//! Player bodies as shots see them: the skeletons kept between ticks, the hit boxes posed from them, the history
//! frames recorded each tick, the riot shield, and the bullet sweep through players and entities.

use std::collections::HashSet;

use super::state::AvatarModelSlot;
use super::{SeatNo, StepNo};
use crate::shot_trace::{
    ShotRayRequest, EntityCollisionMoment, EntityHitboxSnapshot, EntityRayShape,
    RewindFrame, RewindStage, HitboxShapeKind, BODY_BOX_UPPER, BODY_BOX_LOWER, PlayerHitboxPose, ShieldCover, RayResult,
    trace_shot_against_models as trace_through_models,
};
use crate::match_ids::LifeCount;
use crate::player_ledger::{PlayerSessionStage, PlayerMatchRecord};
use playerstate_t6::game::AvatarState;
use xmodel_runtime::{MountedModel, HitShapeBone, PosedSkeleton, ModelPoseQuery, LocalPose, PartSet, BodyAimInput};

/// Half the width of the arc a riot shield covers, in degrees.
const SHIELD_PROTECTION_DEGREES: f32 = 50.0;

/// Value stored in a skeleton slot's `persist` counter when the slot is carried over to the next tick.
const SLOT_PERSISTS: i64 = 1;

/// Matrix carrying a body from its own space to where the player stands, turned to his heading.
fn stance_matrix(ps: &AvatarState) -> glam::Mat4 {
    let heading = glam::Quat::from_rotation_z(ps.look_angles[1].to_radians());
    glam::Mat4::from_rotation_translation(heading, glam::Vec3::from_array(ps.origin))
}

/// A pose for a player standing upright, with the given per-bone boxes and no shield yet.
fn upright_pose(client: SeatNo, spot: [f32; 3], yaw: f32, life_no: LifeCount, bones: Vec<HitShapeBone>) -> PlayerHitboxPose {
    PlayerHitboxPose {
        client,
        origin: spot,
        yaw,
        life_sequence: life_no,
        lo: BODY_BOX_LOWER, hi: BODY_BOX_UPPER,
        joints: bones, hit_volume: HitboxShapeKind::Standing,
        shield: None,
        shield_tag: None,
    }
}

/// A body's boxes taken from the player's own space into the world, where he stands and faces.
pub(super) fn t6_world_bones(local: &[HitShapeBone], ps: &AvatarState) -> Vec<HitShapeBone> {
    let place = stance_matrix(ps);
    local
        .iter()
        .map(|bone| HitShapeBone {
            center: place.transform_point3(glam::Vec3::from_array(bone.center)).to_array(),
            axes: bone.axes.map(|axis| place.transform_vector3(glam::Vec3::from_array(axis)).to_array()),
            ..*bone
        })
        .collect()
}

/// What the body's controller needs of the player to bend the spine and lean.
pub(super) fn player_controller_input(ps: &AvatarState) -> BodyAimInput {
    let has_flag = |bit| ps.e_flags & bit != 0;
    let pitch = ps.look_angles[0];
    BodyAimInput { lying: has_flag(playerstate_t6::game::entity_flag_bits::LYING_FLAT),
        crouched: has_flag(playerstate_t6::game::entity_flag_bits::CROUCH_BIT),
        body_lean_ratio: game_math::lean_amount(ps.tilt_sideways),
        look_pitch_degrees: pitch }
}

/// Whether a recorded pose still describes this player: same life, same place.
fn pose_still_holds(pose: &PlayerHitboxPose, id: SeatNo, meta: &PlayerMatchRecord, ps: &AvatarState) -> bool {
    pose.client == id && pose.life_sequence == meta.life_sequence && pose.origin == ps.origin
}

impl super::state::MatchSimulation {
    // ---- poses ------------------------------------------------------------------------------------------------

    /// The hit boxes of every living player, and of the remote bodies a prediction world holds.
    pub(crate) fn alive_collision_poses(&self, player: impl Fn(SeatNo) -> Option<AvatarState>) -> Vec<PlayerHitboxPose> {
        self.collect_poses(player, &[], &mut None)
    }

    /// The poses again when they differ from the last recorded frame (somebody moved, joined or left), else none.
    pub(crate) fn player_poses_since_record(
        &self,
        player: impl Fn(SeatNo) -> Option<AvatarState>,
    ) -> Option<Vec<PlayerHitboxPose>> {
        let recorded = self.replay.history.latest_poses().map_or(&[][..], |(_, poses)| poses);
        let mut living = 0;
        for (id, meta) in &self.roster.members {
            if meta.lifecycle != PlayerSessionStage::Alive {
                continue;
            }
            let Some(ps) = player(*id) else { continue };
            living += 1;
            if !recorded.iter().any(|pose| pose_still_holds(pose, *id, meta, &ps)) {
                return Some(self.collect_poses(player, recorded, &mut None));
            }
        }
        let same_count = living + self.bodies.remote.len() == recorded.len();
        (!same_count).then(|| self.collect_poses(player, recorded, &mut None))
    }

    /// One pose per living player. A pose in `reuse` that still holds is kept, unless the body's boxes move with
    /// its animation (they are placed again every time). The first body that could not be built is put in
    /// `first_error`.
    fn collect_poses(
        &self,
        player: impl Fn(SeatNo) -> Option<AvatarState>,
        reuse: &[PlayerHitboxPose],
        first_error: &mut Option<String>,
    ) -> Vec<PlayerHitboxPose> {
        let mut poses = Vec::with_capacity(self.roster.members.len() + self.bodies.remote.len());
        for (id, meta) in &self.roster.members {
            if meta.lifecycle != PlayerSessionStage::Alive {
                continue;
            }
            let Some(ps) = player(*id) else { continue };
            let kept = meta.t6_hit_bones.is_none().then(|| reuse.iter().find(|pose| pose_still_holds(pose, *id, meta, &ps))).flatten();
            let mut pose = match kept {
                Some(pose) => pose.clone(),
                None => self.fresh_pose(*id, meta, &ps, first_error),
            };
            pose.shield = self.shield_cover(meta, &ps);
            poses.push(pose);
        }
        poses.extend(self.bodies.remote.iter().map(|remote| upright_pose(remote.client, remote.spot, 0.0, remote.life_sequence, Vec::new())));
        poses
    }

    fn fresh_pose(&self, id: SeatNo, info: &PlayerMatchRecord, ps: &AvatarState, first_error: &mut Option<String>) -> PlayerHitboxPose {
        let built = self.player_hitvol_bones(id, ps).unwrap_or_else(|error| {
            first_error.get_or_insert(error);
            Vec::new()
        });
        // A Black Ops II body brings its own boxes, posed in its own space; they go where he stands.
        let bones = match (&info.t6_hit_bones, built.is_empty()) {
            (Some(own), true) => t6_world_bones(own, ps),
            _ => built,
        };
        let mut pose = upright_pose(id, ps.origin, ps.look_angles[1], info.life_sequence, bones);
        pose.shield_tag = info.t6_shield_tag.as_ref().and_then(|tag| t6_world_bones(std::slice::from_ref(tag), ps).pop());
        pose
    }

    /// The arc the riot shield he bears covers, as he faces now.
    fn shield_cover(&self, meta: &PlayerMatchRecord, ps: &AvatarState) -> Option<ShieldCover> {
        let in_hands = meta.riot_shield?;
        let heading = ps.look_angles[1].to_radians();
        let side = if in_hands { 1.0 } else { -1.0 };
        Some(ShieldCover {
            facing: [heading.cos() * side, heading.sin() * side],
            cos_limit: SHIELD_PROTECTION_DEGREES.to_radians().cos(),
        })
    }

    /// The boxes of the body posed from its kept skeleton (a fresh one when it has none), in the world.
    fn player_hitvol_bones(&self, id: SeatNo, ps: &AvatarState) -> Result<Vec<HitShapeBone>, String> {
        let rig = self.collision_kit(id);
        let Some(body) = rig.body.as_ref() else { return Ok(Vec::new()) };
        let world = stance_matrix(ps);
        let request = ModelPoseQuery::neutral_pose();
        let stance = player_controller_input(ps);
        let driver = move |dobj: &PosedSkeleton, _: &PartSet, locals: &mut LocalPose| {
            xmodel_runtime::pose_body_aim(dobj, locals, stance);
        };
        let mut models = vec![(body.as_ref(), None)];
        if let (Some(top), Some(joint_tag)) = (rig.head.as_ref(), xmodel_runtime::tp_head_attach_tag(&body.pose.joint_labels)) {
            models.push((top.as_ref(), Some(MountedModel { host_model: 0, mount_tag: joint_tag.into() })));
        }
        let posed = match self.bodies.skeletons.get(&id.0) {
            Some(slot) => xmodel_runtime::hit_skeleton_with_aim(&slot.dobj, &models, &request, world, driver),
            None => xmodel_runtime::hit_shape_models_with_aim(&models, &request, world, driver),
        };
        posed.map_err(|error| format!("{error:?}"))
    }

    // ---- the riot shield --------------------------------------------------------------------------------------

    /// His Black Ops II hit boxes this tick (in his own space), or none.
    pub(crate) fn set_t6_hit_bones(
        &mut self,
        id: SeatNo,
        bones: Option<std::sync::Arc<Vec<HitShapeBone>>>,
        shield_tag: Option<HitShapeBone>,
    ) {
        let client = self.client_meta_mut(id);
        client.t6_hit_bones = bones;
        client.t6_shield_tag = shield_tag;
    }

    /// Whether he bears a riot shield, and if so whether it is in his hands (else on his back).
    pub(crate) fn set_riot_shield(&mut self, id: SeatNo, held: Option<bool>) {
        self.client_meta_mut(id).riot_shield = held;
    }

    /// How much of an explosion at `point` the riot shield he bears stops, 0 to 1. Flak Jacket stops none of it.
    /// Otherwise it goes by the angle between the forward of the tag the shield hangs on (in his hands or on his
    /// back) and the blast as seen from that tag. No tag, no cover.
    pub(crate) fn shield_blast_protection(&self, id: SeatNo, ps: &AvatarState, spot: [f32; 3]) -> f32 {
        let Some(meta) = self.client_meta(id) else { return 0.0 };
        if meta.riot_shield.is_none() || ps.has_perk_bit(playerstate_t6::perks::PERK_FLAKJACKET) {
            return 0.0;
        }
        let tag = meta.t6_shield_tag.as_ref().and_then(|tag| t6_world_bones(std::slice::from_ref(tag), ps).pop());
        let Some(tag) = tag else {
            return 0.0;
        };
        let (at, facing) = (tag.center, tag.axes[0]);
        let to_blast = [spot[0] - at[0], spot[1] - at[1], spot[2] - at[2]];
        let reach = (to_blast[0] * to_blast[0] + to_blast[1] * to_blast[1] + to_blast[2] * to_blast[2]).sqrt();
        let reach = if reach > 0.0 { reach } else { 1.0 };
        let cos = (facing[0] * to_blast[0] + facing[1] * to_blast[1] + facing[2] * to_blast[2]) / reach;
        // The arcs the game tunes at 30, 60, 120 and 180 degrees of full width.
        let stops = match cos {
            c if c > 0.966 => 0.4,
            c if c > 0.866 => 0.2,
            c if c > 0.5 => 0.1,
            _ => 0.0,
        };
        if std::env::var_os("BO2MP_SHIELDLOG").is_some() {
            diag::info!(
                Sim,
                "bo2mp shield: tag at {:?} forward {:?}, blast at {spot:?}, cos {cos:.3}, stops {stops}",
                at.map(|v| v.round()),
                facing.map(|v| (v * 100.0).round() / 100.0)
            );
        }
        stops
    }

    // ---- skeletons kept between ticks -------------------------------------------------------------------------

    /// Builds a skeleton for each living player who has none that fits his kit, and drops those of players gone.
    /// With `only` set (a prediction world), the others are left alone.
    fn refresh_player_dobjs(&mut self, restrict_to: Option<&[SeatNo]>) {
        let living: Vec<SeatNo> = self
            .roster
            .members
            .iter()
            .filter_map(|(seat, m)| (m.lifecycle == PlayerSessionStage::Alive).then_some(*seat))
            .collect();
        let mut keep = HashSet::new();
        for seat in living {
            keep.insert(seat.0);
            if restrict_to.is_some_and(|ids| !ids.contains(&seat)) {
                continue;
            }
            let rig = self.collision_kit(seat).clone();
            let Some(body) = rig.body.as_ref() else {
                self.bodies.skeletons.remove(&seat.0);
                continue;
            };
            let field = rig.reuse_key();
            if let Some(slot) = self.bodies.skeletons.get_mut(&seat.0)
                && xmodel_runtime::reuse_matches(slot.reuse_key, field)
            {
                slot.persist = SLOT_PERSISTS;
                continue;
            }
            let mut parts = vec![(&body.pose, None)];
            if let (Some(top), Some(joint_tag)) = (rig.head.as_ref(), xmodel_runtime::tp_head_attach_tag(&body.pose.joint_labels)) {
                parts.push((&top.pose, Some(MountedModel { host_model: 0, mount_tag: joint_tag.into() })));
            }
            if let Ok(dobj) = PosedSkeleton::build(&parts) {
                self.bodies.skeletons.insert(seat.0, AvatarModelSlot { dobj, reuse_key: field, persist: 0 });
            } else {
                self.bodies.skeletons.remove(&seat.0);
            }
        }
        self.bodies.skeletons.retain(|owner, _| keep.contains(owner));
    }

    // ---- history ----------------------------------------------------------------------------------------------

    /// Records the players as they stand after this tick's movement, for lag compensation to rewind to.
    pub(crate) fn record_collision_history(&mut self, tick: StepNo, _msec: i32,
        state_lookup: impl Fn(SeatNo) -> Option<AvatarState> + Copy,
        only_clients: Option<&[SeatNo]>) {
        self.refresh_player_dobjs(only_clients);
        let mut build_failure = None;
        let poses = self.collect_poses(state_lookup, &[], &mut build_failure);
        self.bodies.bake_failure = build_failure;
        self.replay.history.record_sample(RewindFrame { tick, phase: RewindStage::AfterPlayerMove, poses });
    }

    /// Records the collision owners as they stand after this tick.
    pub fn log_entity_hitboxes(&mut self, now: StepNo) {
        let owner_rows = self
            .space
            .caps
            .iter()
            .map(|owner| EntityRayShape {
                epoch: EntityCollisionMoment::PastFrame { frame: now },
                ..owner.trace_geom()
            })
            .collect();
        self.replay.entity_history.record_sample(EntityHitboxSnapshot { tick: now, phase: RewindStage::AfterPlayerMove, rows: owner_rows });
    }

    // ---- shots ------------------------------------------------------------------------------------------------

    /// A bullet through the world, the entity models on its way, and `poses` (the last recorded ones when none).
    pub fn trace_shot(&self, probe: ShotRayRequest, poses: Option<&[PlayerHitboxPose]>) -> RayResult {
        let content = &self.content;
        let in_reach: Vec<EntityRayShape> = self
            .space
            .caps
            .iter()
            .filter(|owner| owner.ray_may_hit(content.collision_submodel_set(), probe.from, probe.to))
            .map(|owner| owner.trace_geom())
            .collect();
        let recorded = self.replay.history.latest_poses().map_or(&[][..], |(_, poses)| poses);
        trace_through_models(
            content.collision_solids(),
            content.collision_tree(),
            content.collision_submodel_set(),
            content.collision_triangles(),
            poses.unwrap_or(recorded),
            &in_reach,
            &probe,
            &|pane| self.world_objects.pane_blocks_shots(pane as u32),
        )
    }

    /// What a bot's sensors see: the same sweep as [`Self::trace_shot`] against the last recorded players.
    pub fn sensor_trace(&self, query: ShotRayRequest) -> RayResult {
        self.trace_shot(query, None)
    }
}
