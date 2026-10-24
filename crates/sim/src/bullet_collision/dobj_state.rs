//! The state of one script model's display object: which model it is, how it is posed and animated, and the
//! collision built from that (rebuilt only when the model or pose has moved on).

use std::sync::Arc;

use super::posed_coll::{
    AuthorityDObjCollision, build_coll_trace, matrix_from_angles,
};
use super::{MASK_BULLET_WORLD, contents_match_mask};

type Capability = Arc<xmodel_runtime::RetainedModelCapability>;

/// An animation a script model is playing with `playanim`-style calls.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScriptModelPlayAnim {
    pub looping: bool,
    pub frequency: f32,
}

#[derive(Clone, Debug)]
pub struct AuthorityDObjState {
    pub current_model: String,
    pub model_revision: u32,
    pub pose_revision: u32,
    pub capability: Option<Capability>,

    pub semantic_state: xmodel_runtime::DObjSemanticState,
    pub pose_request: xmodel_runtime::DObjPoseRequest,
    pub world_from_model: glam::Mat4,
    pub materialized_model_revision: Option<u32>,
    pub materialized_pose_revision: Option<u32>,

    pub current_collision: Option<AuthorityDObjCollision>,
    pub materialize_error: Option<xmodel_runtime::MaterializeError>,

    pub play_anim: Option<ScriptModelPlayAnim>,

    pub apos: Option<entity_t6::Trajectory>,

    pub(crate) t5_destructible: Option<crate::t5_destructible::State>,
    /// Capabilities of the models this one can turn into (husks, and the stages of a destructible), by name.
    pub swap_capabilities: Vec<(String, Option<Capability>)>,
}

/// The bit that marks `bone` in a hide-part word array, and the word it lives in.
fn hide_slot(bone: usize) -> (usize, u32) {
    (bone / 32, 0x8000_0000 >> (bone % 32))
}

impl AuthorityDObjState {
    /// A fresh display object in its bind pose whose collision has not been built yet.
    pub fn new_dirty(current_model: String, capability: Option<Capability>, world_from_model: glam::Mat4) -> Self {
        Self {
            semantic_state: xmodel_runtime::DObjSemanticState::bind_pose(current_model.clone(), 1, 1),
            current_model,
            model_revision: 1,
            pose_revision: 1,
            capability,
            pose_request: xmodel_runtime::DObjPoseRequest::bind_pose(),
            world_from_model,
            materialized_model_revision: None,
            materialized_pose_revision: None,
            current_collision: None,
            materialize_error: None,
            play_anim: None,
            apos: None,
            t5_destructible: None,
            swap_capabilities: Vec::new(),
        }
    }

    /// A fresh display object standing at `origin` with `angles`.
    pub fn at_pose(model: &str, capability: Option<Capability>, origin: [f32; 3], angles: [f32; 3]) -> Self {
        Self::new_dirty(model.to_owned(), capability, matrix_from_angles(glam::Vec3::from_array(origin), angles))
    }

    /// The capability of one of the models this one can swap to.
    pub fn swap_capability(&self, model: &str) -> Option<Capability> {
        self.swap_capabilities.iter().find(|(name, _)| name == model).and_then(|(_, capability)| capability.clone())
    }

    // ---- keeping the pose and the built collision in step ----

    fn bump_pose_revision(&mut self) {
        self.pose_revision = self.pose_revision.wrapping_add(1);
    }

    /// The pose or position changed: the collision built from it is no longer good.
    fn drop_collision(&mut self) {
        self.current_collision = None;
        self.materialized_pose_revision = None;
    }

    /// Stores new hide-part bits. Returns whether anything changed; if so the pose moves on and the collision is
    /// dropped.
    fn store_hidden_parts(&mut self, words: [u32; xmodel_runtime::HidePartBits::WORDS]) -> bool {
        let hide = xmodel_runtime::HidePartBits::from_words(words);
        if hide == self.semantic_state.hide_part_bits {
            return false;
        }
        self.semantic_state.hide_part_bits = hide;
        self.pose_request.hide_part_bits = hide;
        self.bump_pose_revision();
        self.semantic_state.pose_revision = self.pose_revision;
        self.drop_collision();
        true
    }

    fn bone_index(&self, matches: impl Fn(&str) -> bool) -> Option<usize> {
        self.capability.as_ref()?.pose.bone_names.iter().position(|name| matches(name))
    }

    // ---- model and attachments ----

    /// Swaps the model, clearing everything built for the old one.
    pub fn set_model(&mut self, current_model: String, capability: Option<Capability>) {
        self.model_revision = self.model_revision.wrapping_add(1);
        self.semantic_state.composition =
            xmodel_runtime::DObjCompositionDescriptor::single(self.model_revision, current_model.clone());
        self.current_model = current_model;
        self.capability = capability;
        self.current_collision = None;
        self.materialized_model_revision = None;
        self.materialized_pose_revision = None;
        self.materialize_error = None;
    }

    /// Swaps the model and resets the pose and animation to the bind pose.
    pub fn replace_model(&mut self, model: &str, capability: Option<Capability>) {
        self.play_anim = None;
        self.set_model(model.to_owned(), capability);
        self.bump_pose_revision();
        self.semantic_state =
            xmodel_runtime::DObjSemanticState::bind_pose(model.to_owned(), self.model_revision, self.pose_revision);
        self.pose_request = xmodel_runtime::DObjPoseRequest::bind_pose();
    }

    /// Sets the models attached to this one (`(model, tag)` pairs). Attached models never take part in collision.
    pub fn set_attachments(&mut self, attachments: &[(&str, &str)]) {
        let base = xmodel_runtime::DObjModelDescriptor {
            model: self.current_model.clone(),
            parent_model: None,
            attach_tag: None,
            ignore_collision: false,
        };
        let attached = attachments.iter().map(|(model, tag)| xmodel_runtime::DObjModelDescriptor {
            model: (*model).to_owned(),
            parent_model: Some(0),
            attach_tag: Some((*tag).to_owned()),
            ignore_collision: true,
        });
        let models: Vec<_> = std::iter::once(base).chain(attached).collect();
        let composition = &mut self.semantic_state.composition;
        if composition.models != models {
            composition.revision = composition.revision.wrapping_add(1);
            composition.models = models;
        }
    }

    // ---- hidden parts ----

    /// Hides the named parts. A destructible only ever loses parts within a round, so this only adds bits.
    pub fn hide_tags(&mut self, tags: &[&str]) {
        if self.capability.is_none() {
            return;
        }
        let mut words = *self.semantic_state.hide_part_bits.words();
        for tag in tags {
            if let Some(bone) = self.bone_index(|name| name == *tag) {
                let (word, bit) = hide_slot(bone);
                words[word] |= bit;
            }
        }
        self.store_hidden_parts(words);
    }

    /// Hides or shows one part by name (case does not matter). False if the model has no such part.
    pub fn set_tag_hidden(&mut self, tag: &str, hidden: bool) -> bool {
        let Some(bone) = self.bone_index(|name| name.eq_ignore_ascii_case(tag)) else {
            return false;
        };
        let mut words = *self.semantic_state.hide_part_bits.words();
        let (word, bit) = hide_slot(bone);
        if hidden {
            words[word] |= bit;
        } else {
            words[word] &= !bit;
        }
        self.store_hidden_parts(words);
        true
    }

    pub fn bone_name(&self, bone: usize) -> Option<&str> {
        self.capability.as_ref()?.pose.bone_names.get(bone).map(String::as_str)
    }

    // ---- placement ----

    /// Moves the model. Nothing is rebuilt if it was already there.
    pub fn set_world_pose(&mut self, origin: [f32; 3], angles: [f32; 3]) {
        let placed = matrix_from_angles(glam::Vec3::from_array(origin), angles);
        if placed != self.world_from_model {
            self.world_from_model = placed;
            self.drop_collision();
        }
    }

    pub fn begin_script_mover_rotate_velocity(
        &mut self,
        speed: [f32; 3],
        total_time_seconds: f32,
        level_time_ms: i32,
        current_angles: [f32; 3],
    ) {
        let from = self
            .apos
            .unwrap_or(entity_t6::Trajectory { tr_base: current_angles, ..entity_t6::Trajectory::default() });
        self.apos = Some(crate::gentity::rotate_velocity_apos(&from, speed, total_time_seconds, level_time_ms));
        self.apply_apos_at(level_time_ms);
    }

    /// Turns the model to where its angle trajectory has it at `at_time_ms`; returns those angles.
    pub fn apply_apos_at(&mut self, at_time_ms: i32) -> Option<[f32; 3]> {
        let apos = self.apos?;
        self.apply_trajectory_apos_at(apos, at_time_ms)
    }

    pub fn apply_trajectory_apos_at(&mut self, apos: entity_t6::Trajectory, at_time_ms: i32) -> Option<[f32; 3]> {
        let angles = entity_t6::evaluate_trajectory(&apos, at_time_ms);
        let origin = self.world_from_model.w_axis.truncate();
        self.world_from_model = matrix_from_angles(origin, angles);
        Some(angles)
    }

    // ---- animation ----

    /// Stops a playing animation and goes back to the bind pose (keeping hidden parts hidden).
    pub fn clear_script_model_play_anim(&mut self) {
        if self.play_anim.take().is_none() {
            return;
        }
        self.bump_pose_revision();
        let hide_part_bits = self.semantic_state.hide_part_bits;
        self.semantic_state = xmodel_runtime::DObjSemanticState {
            hide_part_bits,
            ..xmodel_runtime::DObjSemanticState::bind_pose(
                self.current_model.clone(),
                self.model_revision,
                self.pose_revision,
            )
        };
        self.pose_request = xmodel_runtime::DObjPoseRequest { hide_part_bits, ..xmodel_runtime::DObjPoseRequest::bind_pose() };
        self.drop_collision();
    }

    /// Starts playing `clip` from the start.
    pub fn begin_script_model_play_anim(&mut self, clip: &str, looping: bool, frequency: f32) {
        self.bump_pose_revision();
        self.play_anim = Some(ScriptModelPlayAnim { looping, frequency });
        self.semantic_state = xmodel_runtime::DObjSemanticState::one_leaf(
            self.current_model.clone(),
            clip.to_owned(),
            self.model_revision,
            self.pose_revision,
            0.0,
        );
        self.pose_request = xmodel_runtime::DObjPoseRequest::bind_pose();
        self.drop_collision();
        self.materialize_error = None;
    }

    /// Steps the playing animation by `dt_seconds`. Returns the clip's new time, or 0 if nothing is playing.
    pub fn advance_script_model_play_anim(&mut self, dt_seconds: f32) -> f32 {
        let Some(play) = self.play_anim else {
            return 0.0;
        };
        let Some(tree) = self.semantic_state.tree.as_mut() else {
            return 0.0;
        };
        let Some(leaf) = tree.nodes.first_mut() else {
            return 0.0;
        };
        leaf.state.old_time = leaf.state.time;
        leaf.state.old_cycle_count = leaf.state.cycle_count;
        // When the step changes nothing the leaf keeps the time it had.
        let (time, cycle) = xanim_t6::advance_leaf_time(
            leaf.state.old_time,
            leaf.state.cycle_count,
            leaf.state.rate,
            play.frequency,
            dt_seconds,
            play.looping,
        )
        .unwrap_or((leaf.state.old_time, leaf.state.cycle_count));
        leaf.state.time = time;
        leaf.state.cycle_count = cycle;
        tree.state_revision = tree.state_revision.wrapping_add(1);
        self.bump_pose_revision();
        self.semantic_state.pose_revision = self.pose_revision;
        self.drop_collision();
        time
    }

    // ---- collision ----

    fn collision_is_current(&self) -> bool {
        self.materialized_model_revision == Some(self.model_revision)
            && self.materialized_pose_revision == Some(self.pose_revision)
    }

    fn record_materialized(&mut self, collision: Option<AuthorityDObjCollision>) {
        self.current_collision = collision;
        self.materialized_model_revision = Some(self.model_revision);
        self.materialized_pose_revision = Some(self.pose_revision);
        self.materialize_error = None;
    }

    fn build_collision(
        &self,
        capability: &xmodel_runtime::RetainedModelCapability,
    ) -> Result<AuthorityDObjCollision, xmodel_runtime::MaterializeError> {
        let mut bones = capability.geom_collision(&self.pose_request, self.world_from_model, MASK_BULLET_WORLD)?;
        if self.t5_destructible.is_some() {
            // A destructible's hidden pieces (already launched, or a stage not reached yet) stop nothing.
            let hidden = &self.pose_request.hide_part_bits;
            bones.retain(|bone| !hidden.get(usize::from(bone.bone)));
        }
        let coll = build_coll_trace(capability, &self.pose_request, self.world_from_model)?;
        Ok(AuthorityDObjCollision { bones, coll })
    }

    /// Builds the collision for the current model and pose, unless it is already built.
    pub fn materialize(&mut self) {
        if self.collision_is_current() {
            return;
        }
        self.current_collision = None;
        let Some(capability) = self.capability.clone() else {
            self.materialize_error =
                Some(xmodel_runtime::MaterializeError::DObj("retained model capability unavailable".to_owned()));
            return;
        };
        if !contents_match_mask(capability.contents, MASK_BULLET_WORLD) {
            // Nothing on this model stops a bullet; remember that it was looked at.
            self.record_materialized(None);
            return;
        }
        match self.build_collision(&capability) {
            Ok(collision) => self.record_materialized(Some(collision)),
            Err(error) => self.materialize_error = Some(error),
        }
    }

    // ---- tags ----

    pub fn tag_world_matrix(&self, tag: &str) -> Option<glam::Mat4> {
        let bone = self.bone_index(|name| name == tag)?;
        let posed = self.capability.as_ref()?.pose(&self.pose_request, self.world_from_model).ok()?;
        posed.get(bone).copied()
    }

    /// A tag's world origin and its forward direction (straight up if the tag has no usable direction).
    pub fn tag_world_pose(&self, tag: &str) -> Option<([f32; 3], [f32; 3])> {
        let matrix = self.tag_world_matrix(tag)?;
        let forward = matrix.x_axis.truncate();
        let direction = if forward.length_squared() > 1e-8 { forward.normalize().to_array() } else { [0.0, 0.0, 1.0] };
        Some((matrix.w_axis.truncate().to_array(), direction))
    }
}
