//! Server-side record of one script model: the mesh it shows, the pose it holds (hidden parts, a scripted clip,
//! a spin), where it stands, and the bullet collision baked from all of that. Every pose change bumps a stamp;
//! the baked collision remembers the stamps it was made from and is rebaked only once they differ.

use std::sync::Arc;

use xmodel_runtime::{
    RigLayout, RigMeshEntry, RigPoseRecord, HiddenParts, KeptModelAbility,
    ModelPoseQuery, RebuildFault,
};

use super::posed_coll::{ServerModelCollision, build_coll_trace, matrix_from_angles};
use super::{SHOT_WORLD_FILTER, contents_match_mask};

/// Stamp every counter starts from on a newly made record.
const FIRST_STAMP: u32 = 1;

/// How a scripted clip on a script model plays.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScriptedModelAnimPlay {
    pub loops: bool,
    pub hz: f32,
}

#[derive(Clone, Debug)]
pub struct ServerModelState {
    pub active_mesh: String,
    pub model_version_stamp: u32,
    pub pose_rev: u32,
    pub can_do: Option<Arc<KeptModelAbility>>,

    pub bone_role_state: RigPoseRecord,
    pub pose_query: ModelPoseQuery,
    pub placement_matrix: glam::Mat4,
    pub built_model_stamp: Option<u32>,
    pub built_pose_stamp: Option<u32>,

    pub live_hitbox: Option<ServerModelCollision>,
    pub bake_failure: Option<RebuildFault>,

    pub scripted_anim_playback: Option<ScriptedModelAnimPlay>,

    pub apos: Option<entity_t6::Trajectory>,

    pub(crate) breakable: Option<crate::staged_props::BreakState>,
    /// Named capabilities for every mesh this record may switch to (husks, destructible stages).
    pub swap_capabilities: Vec<(String, Option<Arc<KeptModelAbility>>)>,
}

/// `words` with the hide flag of skeleton joint `joint` set or cleared. Joint 0 is the top bit of word 0.
fn with_joint_flag(mut words: [u32; HiddenParts::BIT_CHUNK_COUNT], joint: usize, hidden: bool) -> [u32; HiddenParts::BIT_CHUNK_COUNT] {
    let mask = 1u32 << (31 - joint % 32);
    let slot = &mut words[joint / 32];
    *slot = if hidden { *slot | mask } else { *slot & !mask };
    words
}

fn root_part(mesh: &str) -> RigMeshEntry {
    RigMeshEntry { model: mesh.to_owned(), host_index: None, mount_tag: None, no_hits: false, tag_offset: None }
}

/// A mesh hung on joint `tag` of the root part; it is drawn but never collides.
fn hung_part(mesh: &str, tag: &str, tag_offset: Option<xmodel_runtime::TagOffset>) -> RigMeshEntry {
    RigMeshEntry {
        model: mesh.to_owned(),
        host_index: Some(0),
        mount_tag: Some(tag.to_owned()),
        no_hits: true,
        tag_offset,
    }
}

impl ServerModelState {
    /// A record in bind pose at `placement_matrix`, with no collision baked yet.
    pub fn new_dirty(active_mesh: String, ability: Option<Arc<KeptModelAbility>>, placement_matrix: glam::Mat4) -> Self {
        let bone_role_state = RigPoseRecord::neutral_pose(active_mesh.clone(), FIRST_STAMP, FIRST_STAMP);
        Self {
            active_mesh,
            can_do: ability,
            placement_matrix,
            bone_role_state,
            model_version_stamp: FIRST_STAMP,
            pose_rev: FIRST_STAMP,
            pose_query: ModelPoseQuery::neutral_pose(),
            built_model_stamp: None,
            built_pose_stamp: None,
            live_hitbox: None,
            bake_failure: None,
            scripted_anim_playback: None,
            apos: None,
            breakable: None,
            swap_capabilities: Vec::new(),
        }
    }

    /// A record in bind pose placed at `origin`, turned by `angles`.
    pub fn at_pose(model: &str, ability: Option<Arc<KeptModelAbility>>, spot: [f32; 3], aim: [f32; 3]) -> Self {
        let placement = matrix_from_angles(glam::Vec3::from_array(spot), aim);
        Self::new_dirty(model.to_owned(), ability, placement)
    }

    /// Capability registered for swap target `model`, if one was loaded.
    pub fn swap_capability(&self, model: &str) -> Option<Arc<KeptModelAbility>> {
        let (_, found) = self.swap_capabilities.iter().find(|(name, _)| name == model)?;
        found.clone()
    }

    // Pose stamps.

    /// Advances the pose stamp and returns the new value.
    fn next_pose_stamp(&mut self) -> u32 {
        self.pose_rev = self.pose_rev.wrapping_add(1);
        self.pose_rev
    }

    /// Forgets the baked collision because the pose or placement it came from is gone.
    fn invalidate_collision(&mut self) {
        self.built_pose_stamp = None;
        self.live_hitbox = None;
    }

    /// Puts the record back in bind pose under a fresh pose stamp, keeping `hidden` parts hidden.
    fn reset_to_bind_pose(&mut self, hidden: HiddenParts) {
        let stamp = self.next_pose_stamp();
        let mut rest = RigPoseRecord::neutral_pose(self.active_mesh.clone(), self.model_version_stamp, stamp);
        rest.hidden_mask = hidden;
        self.bone_role_state = rest;
        self.pose_query = ModelPoseQuery { hidden_mask: hidden, ..ModelPoseQuery::neutral_pose() };
    }

    /// Applies a new hide-part mask. A mask equal to the current one is a no-op; otherwise the pose stamp advances
    /// and the baked collision is dropped.
    fn apply_hide_words(&mut self, words: [u32; HiddenParts::BIT_CHUNK_COUNT]) {
        let mask = HiddenParts::from_bit_chunks(words);
        if self.bone_role_state.hidden_mask == mask {
            return;
        }
        let stamp = self.next_pose_stamp();
        self.pose_query.hidden_mask = mask;
        self.bone_role_state.hidden_mask = mask;
        self.bone_role_state.pose_epoch = stamp;
        self.invalidate_collision();
    }

    fn current_hide_words(&self) -> [u32; HiddenParts::BIT_CHUNK_COUNT] {
        *self.bone_role_state.hidden_mask.words()
    }

    fn joint_where(&self, wanted: impl Fn(&str) -> bool) -> Option<usize> {
        let labels = &self.can_do.as_ref()?.pose.joint_labels;
        labels.iter().position(|label| wanted(label.as_str()))
    }

    // Mesh and attachments.

    /// Points the record at another mesh; everything baked for the previous one is discarded.
    pub fn set_model(&mut self, active_mesh: String, ability: Option<Arc<KeptModelAbility>>) {
        let stamp = self.model_version_stamp.wrapping_add(1);
        self.model_version_stamp = stamp;
        self.bone_role_state.composition = RigLayout::single(stamp, active_mesh.clone());
        self.active_mesh = active_mesh;
        self.can_do = ability;
        self.built_model_stamp = None;
        self.built_pose_stamp = None;
        self.live_hitbox = None;
        self.bake_failure = None;
    }

    /// Points the record at another mesh and starts it over in bind pose with no clip playing.
    pub fn replace_model(&mut self, mesh: &str, capability: Option<Arc<KeptModelAbility>>) {
        self.scripted_anim_playback = None;
        self.set_model(mesh.to_owned(), capability);
        let stamp = self.next_pose_stamp();
        self.bone_role_state = RigPoseRecord::neutral_pose(mesh.to_owned(), self.model_version_stamp, stamp);
        self.pose_query = ModelPoseQuery::neutral_pose();
    }

    /// Replaces the hung meshes, each given as `(mesh, joint tag)`. The composition revision advances only when the
    /// resulting list differs.
    pub fn set_attachments(&mut self, attachments: &[(&str, &str)]) {
        let at: Vec<_> = attachments.iter().map(|&(mesh, tag)| (mesh, tag, None)).collect();
        self.set_attachments_at(&at);
    }

    /// The same, each model optionally placed at an offset and rotation from its tag (the parts Black Ops II hangs on
    /// a weapon's world model).
    pub fn set_attachments_at(&mut self, attachments: &[(&str, &str, Option<xmodel_runtime::TagOffset>)]) {
        let mut wanted = Vec::with_capacity(attachments.len() + 1);
        wanted.push(root_part(&self.active_mesh));
        wanted.extend(attachments.iter().map(|&(mesh, tag, tag_offset)| hung_part(mesh, tag, tag_offset)));
        let held = &mut self.bone_role_state.composition;
        if held.models == wanted {
            return;
        }
        held.models = wanted;
        held.revision = held.revision.wrapping_add(1);
    }

    // Hidden parts.

    /// Hides every joint named in `tags` that the mesh has. Parts are only ever added here: a destructible piece
    /// does not come back during a round.
    pub fn concealed_bones(&mut self, tags: &[&str]) {
        if self.can_do.is_none() {
            return;
        }
        let words = tags
            .iter()
            .filter_map(|tag| self.joint_where(|label| label == *tag))
            .fold(self.current_hide_words(), |acc, joint| with_joint_flag(acc, joint, true));
        self.apply_hide_words(words);
    }

    /// Shows or hides joint `tag`, ignoring ASCII case. Returns false when the mesh has no such joint.
    pub fn set_tag_hidden(&mut self, tag: &str, hidden: bool) -> bool {
        match self.joint_where(|label| label.eq_ignore_ascii_case(tag)) {
            Some(joint) => {
                self.apply_hide_words(with_joint_flag(self.current_hide_words(), joint, hidden));
                true
            }
            None => false,
        }
    }

    pub fn bone_name(&self, bone: usize) -> Option<&str> {
        let labels = &self.can_do.as_ref()?.pose.joint_labels;
        labels.get(bone).map(|label| label.as_str())
    }

    // Placement and spin.

    /// Places the record; an unchanged placement keeps the baked collision.
    pub fn set_world_pose(&mut self, spot: [f32; 3], aim: [f32; 3]) {
        let target = matrix_from_angles(glam::Vec3::from_array(spot), aim);
        if target == self.placement_matrix {
            return;
        }
        self.placement_matrix = target;
        self.invalidate_collision();
    }

    pub fn start_scripted_spin(
        &mut self,
        pace: [f32; 3],
        spin_secs: f32,
        clock_ms: i32,
        live_angles: [f32; 3],
    ) {
        let start = match self.apos {
            Some(running) => running,
            None => entity_t6::Trajectory { base_point: live_angles, ..Default::default() },
        };
        let spin = crate::ent_slots::spin_rate_pose(&start, pace, spin_secs, clock_ms);
        self.apos = Some(spin);
        self.sample_scripted_motion_at(clock_ms);
    }

    /// Orients the record along its stored angular trajectory at `for_instant_millis` and returns the angles used.
    /// None when no trajectory is stored.
    pub fn sample_scripted_motion_at(&mut self, for_instant_millis: i32) -> Option<[f32; 3]> {
        match self.apos {
            Some(trajectory) => self.apply_trajectory_apos_at(trajectory, for_instant_millis),
            None => None,
        }
    }

    /// Orients the record along `apos` at `for_instant_millis`, keeping its origin. Always returns the angles.
    pub fn apply_trajectory_apos_at(&mut self, apos: entity_t6::Trajectory, for_instant_millis: i32) -> Option<[f32; 3]> {
        let turned_to = entity_t6::evaluate_trajectory(&apos, for_instant_millis);
        let stays_at = self.placement_matrix.w_axis.truncate();
        self.placement_matrix = matrix_from_angles(stays_at, turned_to);
        Some(turned_to)
    }

    // Scripted clip.

    /// Ends the scripted clip, if any, and returns to bind pose with the current hidden parts still hidden.
    pub fn clear_script_model_play_anim(&mut self) {
        if self.scripted_anim_playback.is_none() {
            return;
        }
        self.scripted_anim_playback = None;
        let keep_hidden = self.bone_role_state.hidden_mask;
        self.reset_to_bind_pose(keep_hidden);
        self.invalidate_collision();
    }

    /// Plays `clip` from time zero. The pose query goes back to neutral, hidden parts included.
    pub fn start_puppet_clip(&mut self, mag: &str, loops_back: bool, rate_hz: f32) {
        let stamp = self.next_pose_stamp();
        self.scripted_anim_playback = Some(ScriptedModelAnimPlay { loops: loops_back, hz: rate_hz });
        self.bone_role_state = RigPoseRecord::single_clip(
            self.active_mesh.clone(),
            mag.to_owned(),
            self.model_version_stamp,
            stamp,
            0.0,
        );
        self.pose_query = ModelPoseQuery::neutral_pose();
        self.invalidate_collision();
        self.bake_failure = None;
    }

    /// Advances the scripted clip by `dt_seconds` and returns its time afterwards. Returns 0 with nothing touched
    /// when no clip plays or the clip's tree is missing.
    pub fn step_scripted_model_anim(&mut self, step_secs: f32) -> f32 {
        let Some(ScriptedModelAnimPlay { loops: loops_back, hz: rate_hz }) = self.scripted_anim_playback else {
            return 0.0;
        };
        let Some(tree) = self.bone_role_state.tree.as_mut() else {
            return 0.0;
        };
        let Some(node) = tree.nodes.first_mut() else {
            return 0.0;
        };
        let clip = &mut node.state;
        let (from_time, from_cycles) = (clip.time, clip.laps);
        clip.prev_t = from_time;
        clip.prev_laps = from_cycles;
        // A step that makes no progress leaves the clip where it was.
        let (to_time, to_cycles) =
            xanim_t6::advance_leaf_time(from_time, from_cycles, clip.rate, rate_hz, step_secs, loops_back)
                .unwrap_or((from_time, from_cycles));
        clip.time = to_time;
        clip.laps = to_cycles;
        tree.clock_epoch = tree.clock_epoch.wrapping_add(1);
        let stamp = self.next_pose_stamp();
        self.bone_role_state.pose_epoch = stamp;
        self.invalidate_collision();
        to_time
    }

    // Collision.

    fn baked_for_current_pose(&self) -> bool {
        let model_ok = self.built_model_stamp == Some(self.model_version_stamp);
        model_ok && self.built_pose_stamp == Some(self.pose_rev)
    }

    fn store_bake(&mut self, collision: Option<ServerModelCollision>) {
        self.built_model_stamp = Some(self.model_version_stamp);
        self.built_pose_stamp = Some(self.pose_rev);
        self.live_hitbox = collision;
        self.bake_failure = None;
    }

    fn bake_from(&self, capability: &KeptModelAbility) -> Result<ServerModelCollision, RebuildFault> {
        let placed = self.placement_matrix;
        let mut bones = capability.mesh_hit_probe(&self.pose_query, placed, SHOT_WORLD_FILTER)?;
        // A hidden part stops nothing: a destructible's pieces (already blown off, or stages not yet reached), a
        // mannequin's shot-off head.
        let mask = self.pose_query.hidden_mask;
        bones.retain(|hit| !mask.get(usize::from(hit.bone)));
        let coll = build_coll_trace(capability, &self.pose_query, placed)?;
        Ok(ServerModelCollision { bones, coll, face_bits: capability.face_flags })
    }

    /// Bakes bullet collision for the present mesh and pose. Skips the work when the stored bake is still current.
    pub fn bake_collision_shape(&mut self) {
        if self.baked_for_current_pose() {
            return;
        }
        self.live_hitbox = None;
        let Some(capability) = self.can_do.clone() else {
            self.bake_failure = Some(RebuildFault::Skeleton("retained model capability unavailable".to_owned()));
            return;
        };
        // A mesh with no shot-blocking contents bakes to "nothing", which still counts as baked.
        if !contents_match_mask(capability.contents, SHOT_WORLD_FILTER) {
            self.store_bake(None);
            return;
        }
        match self.bake_from(&capability) {
            Err(fault) => self.bake_failure = Some(fault),
            Ok(baked) => self.store_bake(Some(baked)),
        }
    }

    // Joint tags.

    pub fn tag_world_matrix(&self, tag: &str) -> Option<glam::Mat4> {
        let joint = self.joint_where(|label| label == tag)?;
        let capability = self.can_do.as_ref()?;
        let joints = capability.pose(&self.pose_query, self.placement_matrix).ok()?;
        joints.get(joint).copied()
    }

    /// World position of joint `tag` and the unit direction its x axis points; +Z if that axis is degenerate.
    pub fn tag_world_pose(&self, tag: &str) -> Option<([f32; 3], [f32; 3])> {
        let world = self.tag_world_matrix(tag)?;
        let x_axis = world.x_axis.truncate();
        let facing = if x_axis.length_squared() > 1e-8 { x_axis.normalize() } else { glam::Vec3::Z };
        Some((world.w_axis.truncate().to_array(), facing.to_array()))
    }
}
