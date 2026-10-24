//! Player bodies as shots see them: the skeletons kept between ticks, the hit boxes posed from them, the history
//! frames recorded each tick, the riot shield, and the bullet sweep through players and entities.

use std::collections::HashSet;

use super::state::{PlayerDobjSlot, SimState};
use super::{ClientId, Tick};
use crate::bullet_collision::{
    BulletTraceQuery, EntityCollisionCapabilities, EntityCollisionEpoch, EntityCollisionFrame, EntityCollisionTraceGeom,
    HistoryFrame, HistoryPhase, HitVolumeKind, PLAYER_MAXS, PLAYER_MINS, PlayerCollisionPose, ShieldCover, TraceOutcome,
};
use crate::match_state::{ClientLifecycle, ClientMatchState};
use playerstate_t6::game::PlayerState;
use xmodel_runtime::{Attach, CollisionBone, DObj, DObjPoseRequest, LocalPose, PartBits, PlayerControllerInput};

/// Half the width of the arc a riot shield covers, in degrees.
const SHIELD_PROTECTION_DEGREES: f32 = 50.0;

/// A body's boxes taken from the player's own space into the world, where he stands and faces.
pub(super) fn t6_world_bones(local: &[CollisionBone], ps: &PlayerState) -> Vec<CollisionBone> {
    let place = glam::Mat4::from_rotation_translation(
        glam::Quat::from_rotation_z(ps.viewangles[1].to_radians()),
        glam::Vec3::from_array(ps.origin),
    );
    local
        .iter()
        .map(|bone| CollisionBone {
            center: place.transform_point3(glam::Vec3::from_array(bone.center)).to_array(),
            axes: bone.axes.map(|axis| place.transform_vector3(glam::Vec3::from_array(axis)).to_array()),
            ..*bone
        })
        .collect()
}

/// What the body's controller needs of the player to bend the spine and lean.
pub(super) fn player_controller_input(ps: &PlayerState) -> PlayerControllerInput {
    PlayerControllerInput {
        view_pitch_deg: ps.viewangles[0],
        prone: ps.e_flags & playerstate_t6::game::eflags::PRONE != 0,
        crouch: ps.e_flags & playerstate_t6::game::eflags::DUCK != 0,
        lean_frac: game_math::get_lean_fraction(ps.leanf),
    }
}

/// Whether a recorded pose still describes this player: same life, same place.
fn pose_still_holds(pose: &PlayerCollisionPose, id: ClientId, meta: &ClientMatchState, ps: &PlayerState) -> bool {
    pose.client == id && pose.life_sequence == meta.life_sequence && pose.origin == ps.origin
}

impl SimState {
    // ---- poses ------------------------------------------------------------------------------------------------

    /// The hit boxes of every living player, and of the remote bodies a prediction world holds.
    pub(crate) fn alive_collision_poses(&self, player: impl Fn(ClientId) -> Option<PlayerState>) -> Vec<PlayerCollisionPose> {
        self.collect_poses(player, &[], &mut None)
    }

    /// The poses again when they differ from the last recorded frame (somebody moved, joined or left), else none.
    pub(crate) fn player_poses_since_record(
        &self,
        player: impl Fn(ClientId) -> Option<PlayerState>,
    ) -> Option<Vec<PlayerCollisionPose>> {
        let recorded = self.replay.history.latest_poses().map_or(&[][..], |(_, poses)| poses);
        let mut living = 0;
        for (id, meta) in &self.roster.members {
            if meta.lifecycle != ClientLifecycle::Alive {
                continue;
            }
            let Some(ps) = player(*id) else {
                continue;
            };
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
        player: impl Fn(ClientId) -> Option<PlayerState>,
        reuse: &[PlayerCollisionPose],
        first_error: &mut Option<String>,
    ) -> Vec<PlayerCollisionPose> {
        let mut poses = Vec::new();
        for (id, meta) in &self.roster.members {
            if meta.lifecycle != ClientLifecycle::Alive {
                continue;
            }
            let Some(ps) = player(*id) else {
                continue;
            };
            let kept = meta.t6_hit_bones.is_none().then(|| reuse.iter().find(|pose| pose_still_holds(pose, *id, meta, &ps))).flatten();
            let mut pose = match kept {
                Some(pose) => pose.clone(),
                None => self.fresh_pose(*id, meta, &ps, first_error),
            };
            pose.shield = self.shield_cover(meta, &ps);
            poses.push(pose);
        }
        poses.extend(self.bodies.remote.iter().map(|body| PlayerCollisionPose {
            client: body.client,
            origin: body.origin,
            mins: PLAYER_MINS,
            maxs: PLAYER_MAXS,
            life_sequence: body.life_sequence,
            hit_volume: HitVolumeKind::Standing,
            bones: Vec::new(),
            shield: None,
            shield_tag: None,
        }));
        poses
    }

    fn fresh_pose(&self, id: ClientId, meta: &ClientMatchState, ps: &PlayerState, first_error: &mut Option<String>) -> PlayerCollisionPose {
        let built = self.player_hitvol_bones(id, ps).unwrap_or_else(|error| {
            first_error.get_or_insert(error);
            Vec::new()
        });
        // A Black Ops II body brings its own boxes, posed in its own space; they go where he stands.
        let bones = match (&meta.t6_hit_bones, built.is_empty()) {
            (Some(own), true) => t6_world_bones(own, ps),
            _ => built,
        };
        PlayerCollisionPose {
            client: id,
            origin: ps.origin,
            mins: PLAYER_MINS,
            maxs: PLAYER_MAXS,
            life_sequence: meta.life_sequence,
            hit_volume: HitVolumeKind::Standing,
            bones,
            shield: None,
            shield_tag: meta.t6_shield_tag.as_ref().and_then(|tag| t6_world_bones(std::slice::from_ref(tag), ps).pop()),
        }
    }

    /// The arc the riot shield he bears covers, as he faces now.
    fn shield_cover(&self, meta: &ClientMatchState, ps: &PlayerState) -> Option<ShieldCover> {
        let in_hands = meta.riot_shield?;
        let yaw = ps.viewangles[1].to_radians();
        let side = if in_hands { 1.0 } else { -1.0 };
        Some(ShieldCover {
            facing: [yaw.cos() * side, yaw.sin() * side],
            cos_limit: SHIELD_PROTECTION_DEGREES.to_radians().cos(),
        })
    }

    /// The boxes of the body posed from its kept skeleton (a fresh one when it has none), in the world.
    fn player_hitvol_bones(&self, id: ClientId, ps: &PlayerState) -> Result<Vec<CollisionBone>, String> {
        let kit = self.collision_kit(id);
        let Some(body) = kit.body.as_ref() else {
            return Ok(Vec::new());
        };
        let world = glam::Mat4::from_rotation_translation(
            glam::Quat::from_rotation_z(ps.viewangles[1].to_radians()),
            glam::Vec3::from_array(ps.origin),
        );
        let request = DObjPoseRequest::bind_pose();
        let input = player_controller_input(ps);
        let controller = move |dobj: &DObj, _: &PartBits, locals: &mut LocalPose| {
            xmodel_runtime::apply_player_controller(dobj, locals, input);
        };
        let mut models = vec![(body.as_ref(), None)];
        if let (Some(head), Some(tag)) = (kit.head.as_ref(), xmodel_runtime::tp_head_attach_tag(&body.pose.bone_names)) {
            models.push((head.as_ref(), Some(Attach { parent_model: 0, tag: tag.into() })));
        }
        let posed = match self.bodies.dobjs.get(&id.0) {
            Some(slot) => xmodel_runtime::collision_dobj_with_controller(&slot.dobj, &models, &request, world, controller),
            None => xmodel_runtime::collision_models_with_controller(&models, &request, world, controller),
        };
        posed.map_err(|error| format!("{error:?}"))
    }

    // ---- the riot shield --------------------------------------------------------------------------------------

    /// His Black Ops II hit boxes this tick (in his own space), or none.
    pub(crate) fn set_t6_hit_bones(
        &mut self,
        id: ClientId,
        bones: Option<std::sync::Arc<Vec<CollisionBone>>>,
        shield_tag: Option<CollisionBone>,
    ) {
        let meta = self.client_meta_mut(id);
        meta.t6_hit_bones = bones;
        meta.t6_shield_tag = shield_tag;
    }

    /// Whether he bears a riot shield, and if so whether it is in his hands (else on his back).
    pub(crate) fn set_riot_shield(&mut self, id: ClientId, held: Option<bool>) {
        self.client_meta_mut(id).riot_shield = held;
    }

    /// How much of an explosion at `point` the riot shield he bears stops, 0 to 1. Flak Jacket stops none of it.
    /// Otherwise it goes by the angle between the forward of the tag the shield hangs on (in his hands or on his
    /// back) and the blast as seen from that tag. No tag, no cover.
    pub(crate) fn shield_blast_protection(&self, id: ClientId, ps: &PlayerState, point: [f32; 3]) -> f32 {
        let Some(meta) = self.client_meta(id) else {
            return 0.0;
        };
        if meta.riot_shield.is_none() || ps.perks[0] & playerstate_t6::perks::PERK_FLAKJACKET != 0 {
            return 0.0;
        }
        let tag = meta.t6_shield_tag.as_ref().and_then(|tag| t6_world_bones(std::slice::from_ref(tag), ps).pop());
        let Some(tag) = tag else {
            return 0.0;
        };
        let (at, facing) = (tag.center, tag.axes[0]);
        let to_blast = [point[0] - at[0], point[1] - at[1], point[2] - at[2]];
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
                "bo2mp shield: tag at {:?} forward {:?}, blast at {point:?}, cos {cos:.3}, stops {stops}",
                at.map(|v| v.round()),
                facing.map(|v| (v * 100.0).round() / 100.0)
            );
        }
        stops
    }

    // ---- skeletons kept between ticks -------------------------------------------------------------------------

    /// Builds a skeleton for each living player who has none that fits his kit, and drops those of players gone.
    /// With `only` set (a prediction world), the others are left alone.
    fn tick_player_dobjs(&mut self, only: Option<&[ClientId]>) {
        let living: Vec<ClientId> = self
            .roster
            .members
            .iter()
            .filter(|(_, meta)| meta.lifecycle == ClientLifecycle::Alive)
            .map(|(id, _)| *id)
            .collect();
        let mut keep = HashSet::new();
        for id in living {
            keep.insert(id.0);
            if only.is_some_and(|ids| !ids.contains(&id)) {
                continue;
            }
            let kit = self.collision_kit(id).clone();
            let Some(body) = kit.body.as_ref() else {
                self.bodies.dobjs.remove(&id.0);
                continue;
            };
            let key = kit.reuse_key();
            if let Some(slot) = self.bodies.dobjs.get_mut(&id.0)
                && xmodel_runtime::reuse_matches(slot.reuse_key, key)
            {
                slot.persist = 1;
                continue;
            }
            let mut parts = vec![(&body.pose, None)];
            if let (Some(head), Some(tag)) = (kit.head.as_ref(), xmodel_runtime::tp_head_attach_tag(&body.pose.bone_names)) {
                parts.push((&head.pose, Some(Attach { parent_model: 0, tag: tag.into() })));
            }
            match DObj::build(&parts) {
                Ok(dobj) => {
                    self.bodies.dobjs.insert(id.0, PlayerDobjSlot { dobj, reuse_key: key, persist: 0 });
                }
                Err(_) => {
                    self.bodies.dobjs.remove(&id.0);
                }
            }
        }
        self.bodies.dobjs.retain(|owner, _| keep.contains(owner));
    }

    // ---- history ----------------------------------------------------------------------------------------------

    /// Records the players as they stand after this tick's movement, for lag compensation to rewind to.
    pub(crate) fn record_collision_history(
        &mut self,
        tick: Tick,
        _msec: i32,
        player: impl Fn(ClientId) -> Option<PlayerState> + Copy,
        hitbox_cmds: Option<&[ClientId]>,
    ) {
        self.tick_player_dobjs(hitbox_cmds);
        let mut first_error = None;
        let poses = self.collect_poses(player, &[], &mut first_error);
        self.bodies.materialize_error = first_error;
        self.replay.history.push_frame(HistoryFrame { tick, phase: HistoryPhase::PostMovement, poses });
    }

    /// Records the collision owners as they stand after this tick.
    pub fn record_entity_collision_history(&mut self, tick: Tick) {
        let rows = self
            .space
            .caps
            .iter()
            .map(|owner| {
                let mut row = owner.trace_geom();
                row.epoch = EntityCollisionEpoch::Historical { frame: tick };
                row
            })
            .collect();
        self.replay.entity_history.push_frame(EntityCollisionFrame { tick, phase: HistoryPhase::PostMovement, rows });
    }

    // ---- shots ------------------------------------------------------------------------------------------------

    /// A bullet through the world, the entity models on its way, and `poses` (the last recorded ones when none).
    pub fn bullet_trace(&self, query: BulletTraceQuery, poses: Option<&[PlayerCollisionPose]>) -> TraceOutcome {
        let content = &self.content;
        let in_reach: Vec<EntityCollisionTraceGeom> = self
            .space
            .caps
            .iter()
            .filter(|owner| owner.ray_may_hit(content.clip_cmodels(), query.start, query.end))
            .map(EntityCollisionCapabilities::trace_geom)
            .collect();
        let recorded = self.replay.history.latest_poses().map_or(&[][..], |(_, poses)| poses);
        crate::bullet_collision::bullet_trace_with_entity_models(
            content.clip_brushes(),
            content.clip_bsp(),
            content.clip_cmodels(),
            content.clip_mesh(),
            poses.unwrap_or(recorded),
            &in_reach,
            &query,
            &|piece| self.world_objects.glass_is_solid(piece as u32),
        )
    }

    /// What a bot's sensors see: the same sweep as [`Self::bullet_trace`] against the last recorded players.
    pub fn sensor_trace(&self, query: BulletTraceQuery) -> TraceOutcome {
        self.bullet_trace(query, None)
    }
}
