use crate::identities::{LifeSequence, ScriptModelId};
use crate::world::{ClientId, SimBrush, SimClipBsp, SimClipCmodels, SimClipMesh, Tick};
use trace_t6::{Trace, TraceHitType, surface_type_from_flags};
use weapon_t6::penetration::{
    ADVANCE_TRACE_FWD, ADVANCE_TRACE_REV, BulletPenFacts, CONTENTS_GLASS, MAX_EXTENDED_STEPS,
    MAX_PENETRATE_STEPS, PEN_THICKNESS_FLOOR, PenetrationDepthTable, REV_END_EPS,
    RIFLE_COLLATERAL_SCALE, SURF_TYPE_FLESH, advance_trace, depth_surface_type,
};

pub const CONTENTS_SOLID: u32 = 0x0000_0001;

pub const MASK_PLAYER_SOLID: u32 = 0x0281_0011;

pub const MASK_SHOT: u32 = 0x0280_6831;

pub const CONTENTS_BODY: u32 = 0x0200_0000;

pub const MASK_BULLET_WORLD: u32 = MASK_SHOT;

const LINK_BOUNDS_PAD: f32 = 1.0;

pub fn contents_match_mask(contents: Option<u32>, mask: u32) -> bool {
    match contents {
        Some(0) | None => true,
        Some(c) => c & mask != 0,
    }
}

pub const PLAYER_MINS: [f32; 3] = [-15.0, -15.0, 0.0];
pub const PLAYER_MAXS: [f32; 3] = [15.0, 15.0, 70.0];

pub const COLLISION_HISTORY_TICKS: usize = 32;

pub const LAGCOMP_MAX_REWIND_TICKS: u32 = 4;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ShotSampleProvenance {
    pub left: Tick,

    pub right: Tick,

    pub alpha: f32,
    pub quality: ShotSampleQuality,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShotSampleQuality {
    #[default]
    None,
    Exact,
    Interpolated,

    Held,
    Starved,
}

impl ShotSampleQuality {
    pub const fn claims_history(self) -> bool {
        matches!(self, Self::Exact | Self::Interpolated)
    }

    pub const fn dump_label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Exact => "exact",
            Self::Interpolated => "interp",
            Self::Held => "held",
            Self::Starved => "starved",
        }
    }
}

impl ShotSampleProvenance {
    pub const NO_CLAIM: Self = Self {
        left: Tick(0),
        right: Tick(0),
        alpha: 0.0,
        quality: ShotSampleQuality::None,
    };

    pub const fn requested_tick(self) -> Tick {
        self.left
    }

    pub fn is_valid_for(self, current: Tick) -> bool {
        self.quality.claims_history()
            && self.left.0 <= self.right.0
            && self.right.0 <= current.0
            && self.alpha.is_finite()
            && (0.0..=1.0).contains(&self.alpha)
    }
}

pub fn lagcomp_rewind_ticks(current: u32, last_acked: Option<u32>) -> u32 {
    let Some(acked) = last_acked else {
        return 0;
    };
    current.saturating_sub(acked).min(LAGCOMP_MAX_REWIND_TICKS)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AuthorityModelOwner {
    ScriptModel(ScriptModelId),
}

impl AuthorityModelOwner {
    pub const fn script_model(self) -> Option<ScriptModelId> {
        match self {
            Self::ScriptModel(id) => Some(id),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColliderId {
    World {
        surface_flags: u32,
        contents: u32,

        glass_encoded: u16,
    },
    Player {
        client: ClientId,

        life: LifeSequence,

        hitloc: u8,
    },

    EntityDObjBone {
        owner: AuthorityModelOwner,
        bone: u16,
        part_classification: u8,
        surface_flags: u32,
    },

    EntityLinkedBrush {
        owner: AuthorityModelOwner,
        cmodel_handle: u32,
        surface_flags: u32,
    },
}

pub type AuthorityDObjCollisionBone = xmodel_runtime::CollisionBone;

#[derive(Clone, Debug, PartialEq)]
pub struct AuthorityDObjCollision {
    pub bones: Vec<AuthorityDObjCollisionBone>,
    pub coll: Option<AuthorityDObjCollTrace>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AuthorityDObjCollTrace {
    pub model: std::sync::Arc<DObjColl>,
    pub world_from_model: glam::Mat4,
    pub bones: Vec<DObjCollBone>,
    pub hide_part_bits: [u32; xmodel_runtime::HidePartBits::WORDS],
}

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
    pub capability: Option<std::sync::Arc<xmodel_runtime::RetainedModelCapability>>,

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
    /// Capabilities for the models this script model can swap to: husks and
    /// the intermediate stages of a destructible.
    pub swap_capabilities: Vec<(
        String,
        Option<std::sync::Arc<xmodel_runtime::RetainedModelCapability>>,
    )>,
}

fn iw_angles_to_mat4(origin: glam::Vec3, angles: [f32; 3]) -> glam::Mat4 {
    let [pitch, yaw, roll] = angles.map(f32::to_radians);
    let rotation = glam::Quat::from_euler(glam::EulerRot::ZYX, yaw, pitch, roll);
    glam::Mat4::from_rotation_translation(rotation, origin)
}

/// bo2mp: a rigid transform as the animated collision trace keeps one: the
/// axis rows, then the origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RigidXform {
    pub axis: [[f32; 3]; 3],
    pub trans: [f32; 3],
}

impl RigidXform {
    pub const IDENTITY: Self = Self {
        axis: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        trans: [0.0, 0.0, 0.0],
    };

    fn inverse(&self) -> Self {
        let m = self.axis;
        let axis = [0, 1, 2].map(|i| [m[0][i], m[1][i], m[2][i]]);
        let trans = [0, 1, 2].map(|i| {
            -(axis[i][0] * self.trans[0] + axis[i][1] * self.trans[1] + axis[i][2] * self.trans[2])
        });
        Self { axis, trans }
    }

    /// `self` then `next`.
    fn then(&self, next: &Self) -> Self {
        let (a, b) = (self, next);
        let axis = [0, 1, 2].map(|i| [0, 1, 2].map(|j| a.axis[i][0] * b.axis[0][j] + a.axis[i][1] * b.axis[1][j] + a.axis[i][2] * b.axis[2][j]));
        let trans = [0, 1, 2]
            .map(|j| a.trans[0] * b.axis[0][j] + a.trans[1] * b.axis[1][j] + a.trans[2] * b.axis[2][j] + b.trans[j]);
        Self { axis, trans }
    }

    /// The point `p` taken back through this transform.
    fn untransform(&self, p: [f32; 3]) -> [f32; 3] {
        let d = [p[0] - self.trans[0], p[1] - self.trans[1], p[2] - self.trans[2]];
        self.axis.map(|r| d[0] * r[0] + d[1] * r[1] + d[2] * r[2])
    }
}

/// bo2mp: one bone of an animated model's collision: where it stands now
/// and in the bind pose its triangles were made in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DObjCollBone {
    pub posed: RigidXform,
    pub bind: RigidXform,
}

/// bo2mp: a surface of `DObjColl`: its run of `DObjColl::tris`.
#[derive(Clone, Debug, PartialEq)]
pub struct DObjCollSurf {
    pub first_tri: usize,
    pub tri_count: usize,
    pub mins: [f32; 3],
    pub maxs: [f32; 3],
    pub bone_idx: i32,
    pub contents: u32,
    pub surf_flags: u32,
}

/// bo2mp: an animated model's collision triangles, owned (BO2's
/// `XModelColl` borrows its model's tables).
#[derive(Clone, Debug, PartialEq)]
pub struct DObjColl {
    pub coll_lod: i32,
    pub contents: u32,
    pub tris: Vec<clipmap_t6::XModelCollTri>,
    pub surfs: Vec<DObjCollSurf>,
}

impl DObjColl {
    /// `XModelTraceLine` on the posed model: each surface not hidden takes
    /// the line from its bone's pose back to the bind pose, then traces its
    /// triangles. The bone of the last surface hit, or -1.
    fn trace_line_animated(
        &self,
        trace: &mut trace_t6::Trace,
        start: [f32; 3],
        end: [f32; 3],
        contentmask: u32,
        bones: &[DObjCollBone],
        hide: &[u32],
    ) -> i32 {
        if self.coll_lod < 0 {
            return -1;
        }
        let mut hit = -1;
        for surf in &self.surfs {
            if surf.contents & contentmask == 0 {
                continue;
            }
            let bone = usize::try_from(surf.bone_idx).unwrap_or(0);
            if hide.get(bone / 32).copied().unwrap_or(0) & (0x8000_0000 >> (bone % 32)) != 0 {
                continue;
            }
            let (s, e) = match bones.get(bone) {
                Some(b) if b.posed != b.bind => {
                    let to_bind = b.bind.inverse().then(&b.posed);
                    (to_bind.untransform(start), to_bind.untransform(end))
                }
                _ => (start, end),
            };
            let view = [clipmap_t6::XModelCollSurf {
                tris: self.tris.get(surf.first_tri..surf.first_tri + surf.tri_count).unwrap_or(&[]),
                mins: surf.mins,
                maxs: surf.maxs,
                bone_idx: surf.bone_idx,
                contents: surf.contents,
                surf_flags: surf.surf_flags,
            }];
            let coll = clipmap_t6::XModelColl {
                coll_lod: self.coll_lod,
                surfs: &view,
            };
            if coll.trace_line(trace, s, e, contentmask) >= 0 {
                hit = surf.bone_idx;
            }
        }
        hit
    }
}

fn rigid_from_mat4(m: glam::Mat4) -> RigidXform {
    let (_scale, quat, trans) = m.to_scale_rotation_translation();
    let rot = glam::Mat4::from_quat(quat);
    RigidXform {
        axis: [
            [rot.x_axis.x, rot.y_axis.x, rot.z_axis.x],
            [rot.x_axis.y, rot.y_axis.y, rot.z_axis.y],
            [rot.x_axis.z, rot.y_axis.z, rot.z_axis.z],
        ],
        trans: trans.to_array(),
    }
}

fn coll_trace_from_capability(
    capability: &xmodel_runtime::RetainedModelCapability,
    request: &xmodel_runtime::DObjPoseRequest,
    world_from_model: glam::Mat4,
) -> Result<Option<AuthorityDObjCollTrace>, xmodel_runtime::MaterializeError> {
    if capability.coll_lod < 0
        || capability.coll_surfs.is_empty()
        || !capability
            .coll_surfs
            .iter()
            .any(|surf| surf.contents & MASK_BULLET_WORLD != 0)
    {
        return Ok(None);
    }
    let posed = capability.pose(request, glam::Mat4::IDENTITY)?;
    let bind = capability.pose(
        &xmodel_runtime::DObjPoseRequest::bind_pose(),
        glam::Mat4::IDENTITY,
    )?;
    let n = posed.len().max(bind.len());
    let mut bones = Vec::with_capacity(n);
    for i in 0..n {
        bones.push(DObjCollBone {
            posed: posed
                .get(i)
                .copied()
                .map(rigid_from_mat4)
                .unwrap_or(RigidXform::IDENTITY),
            bind: bind
                .get(i)
                .copied()
                .map(rigid_from_mat4)
                .unwrap_or(RigidXform::IDENTITY),
        });
    }
    Ok(Some(AuthorityDObjCollTrace {
        model: std::sync::Arc::new(xmodel_coll_from_capability(capability)),
        world_from_model,
        bones,
        hide_part_bits: *request.hide_part_bits.words(),
    }))
}

fn xmodel_coll_from_capability(capability: &xmodel_runtime::RetainedModelCapability) -> DObjColl {
    let mut tris = Vec::new();
    let surfs = capability
        .coll_surfs
        .iter()
        .map(|surf| {
            let first_tri = tris.len();
            tris.extend(surf.tris.iter().map(|tri| clipmap_t6::XModelCollTri {
                plane: tri.plane,
                svec: tri.svec,
                tvec: tri.tvec,
            }));
            DObjCollSurf {
                first_tri,
                tri_count: surf.tris.len(),
                mins: [0, 1, 2].map(|i| surf.midpoint[i] - surf.half_size[i]),
                maxs: [0, 1, 2].map(|i| surf.midpoint[i] + surf.half_size[i]),
                bone_idx: i32::from(surf.bone),
                contents: surf.contents,
                surf_flags: surf.surf_flags,
            }
        })
        .collect();
    DObjColl {
        coll_lod: i32::from(capability.coll_lod),
        contents: capability.contents.unwrap_or(0),
        tris,
        surfs,
    }
}

fn world_point_to_entity(world_from_model: glam::Mat4, p: [f32; 3]) -> Option<[f32; 3]> {
    let inv = world_from_model.inverse();
    let local = inv.transform_point3(glam::Vec3::from_array(p));
    local.is_finite().then_some(local.to_array())
}

fn entity_normal_to_world(world_from_model: glam::Mat4, n: [f32; 3]) -> [f32; 3] {
    let (_, quat, _) = world_from_model.to_scale_rotation_translation();
    let world = quat * glam::Vec3::from_array(n);
    let len = world.length();
    if len <= 0.0 {
        return n;
    }
    (world / len).to_array()
}

impl AuthorityDObjState {
    pub fn new_dirty(
        current_model: String,
        capability: Option<std::sync::Arc<xmodel_runtime::RetainedModelCapability>>,
        world_from_model: glam::Mat4,
    ) -> Self {
        Self {
            current_model: current_model.clone(),
            model_revision: 1,
            pose_revision: 1,
            capability,
            semantic_state: xmodel_runtime::DObjSemanticState::bind_pose(
                current_model.clone(),
                1,
                1,
            ),
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

    pub fn swap_capability(
        &self,
        model: &str,
    ) -> Option<std::sync::Arc<xmodel_runtime::RetainedModelCapability>> {
        self.swap_capabilities
            .iter()
            .find(|(name, _)| name == model)
            .and_then(|(_, capability)| capability.clone())
    }

    pub fn at_pose(
        model: &str,
        capability: Option<std::sync::Arc<xmodel_runtime::RetainedModelCapability>>,
        origin: [f32; 3],
        angles: [f32; 3],
    ) -> Self {
        Self::new_dirty(
            model.to_owned(),
            capability,
            iw_angles_to_mat4(glam::Vec3::from_array(origin), angles),
        )
    }

    pub fn replace_model(
        &mut self,
        model: &str,
        capability: Option<std::sync::Arc<xmodel_runtime::RetainedModelCapability>>,
    ) {
        self.play_anim = None;
        self.set_model(model.to_owned(), capability);
        self.pose_revision = self.pose_revision.wrapping_add(1);
        self.semantic_state = xmodel_runtime::DObjSemanticState::bind_pose(
            model.to_owned(),
            self.model_revision,
            self.pose_revision,
        );
        self.pose_request = xmodel_runtime::DObjPoseRequest::bind_pose();
    }

    pub fn set_attachments(&mut self, attachments: &[(&str, &str)]) {
        let mut models = vec![xmodel_runtime::DObjModelDescriptor {
            model: self.current_model.clone(),
            parent_model: None,
            attach_tag: None,
            ignore_collision: false,
        }];
        models.extend(
            attachments
                .iter()
                .map(|(model, tag)| xmodel_runtime::DObjModelDescriptor {
                    model: (*model).to_owned(),
                    parent_model: Some(0),
                    attach_tag: Some((*tag).to_owned()),
                    ignore_collision: true,
                }),
        );
        if self.semantic_state.composition.models == models {
            return;
        }
        let composition = &mut self.semantic_state.composition;
        composition.revision = composition.revision.wrapping_add(1);
        composition.models = models;
    }

    /// Put the model a destructible state asks for on this dobj.
    pub fn set_stage_model(&mut self, model: &str) {
        if self.current_model == model {
            return;
        }
        self.play_anim = None;
        let capability = self.swap_capability(model);
        self.set_model(model.to_owned(), capability);
        self.semantic_state = xmodel_runtime::DObjSemanticState::bind_pose(
            model.to_owned(),
            self.model_revision,
            self.pose_revision,
        );
        self.pose_request = xmodel_runtime::DObjPoseRequest::bind_pose();
    }

    /// Hide the parts a destructible has launched. Parts never come back
    /// inside a round, so this only ever adds bits.
    pub fn hide_tags(&mut self, tags: &[&str]) {
        let Some(capability) = &self.capability else {
            return;
        };
        let mut words = *self.semantic_state.hide_part_bits.words();
        for tag in tags {
            let Some(bone) = capability
                .pose
                .bone_names
                .iter()
                .position(|name| name == *tag)
            else {
                continue;
            };
            words[bone / 32] |= 0x8000_0000 >> (bone % 32);
        }
        let hide = xmodel_runtime::HidePartBits::from_words(words);
        if hide == self.semantic_state.hide_part_bits {
            return;
        }
        self.semantic_state.hide_part_bits = hide;
        self.pose_request.hide_part_bits = hide;
        self.pose_revision = self.pose_revision.wrapping_add(1);
        self.semantic_state.pose_revision = self.pose_revision;
        self.current_collision = None;
        self.materialized_pose_revision = None;
    }

    pub fn set_world_pose(&mut self, origin: [f32; 3], angles: [f32; 3]) {
        let world_from_model = iw_angles_to_mat4(glam::Vec3::from_array(origin), angles);
        if world_from_model == self.world_from_model {
            return;
        }
        self.world_from_model = world_from_model;
        self.current_collision = None;
        self.materialized_pose_revision = None;
    }

    pub fn bone_name(&self, bone: usize) -> Option<&str> {
        self.capability
            .as_ref()?
            .pose
            .bone_names
            .get(bone)
            .map(String::as_str)
    }

    pub fn set_tag_hidden(&mut self, tag: &str, hidden: bool) -> bool {
        let Some(bone) = self.capability.as_ref().and_then(|capability| {
            capability
                .pose
                .bone_names
                .iter()
                .position(|name| name.eq_ignore_ascii_case(tag))
        }) else {
            return false;
        };
        let mut words = *self.semantic_state.hide_part_bits.words();
        let bit = 0x8000_0000 >> (bone % 32);
        if hidden {
            words[bone / 32] |= bit;
        } else {
            words[bone / 32] &= !bit;
        }
        let hide = xmodel_runtime::HidePartBits::from_words(words);
        if hide != self.semantic_state.hide_part_bits {
            self.semantic_state.hide_part_bits = hide;
            self.pose_request.hide_part_bits = hide;
            self.pose_revision = self.pose_revision.wrapping_add(1);
            self.semantic_state.pose_revision = self.pose_revision;
            self.current_collision = None;
            self.materialized_pose_revision = None;
        }
        true
    }

    pub fn clear_script_model_play_anim(&mut self) {
        if self.play_anim.take().is_none() {
            return;
        }
        self.pose_revision = self.pose_revision.wrapping_add(1);
        self.semantic_state = xmodel_runtime::DObjSemanticState {
            hide_part_bits: self.semantic_state.hide_part_bits,
            ..xmodel_runtime::DObjSemanticState::bind_pose(
                self.current_model.clone(),
                self.model_revision,
                self.pose_revision,
            )
        };
        self.pose_request = xmodel_runtime::DObjPoseRequest {
            hide_part_bits: self.semantic_state.hide_part_bits,
            ..xmodel_runtime::DObjPoseRequest::bind_pose()
        };
        self.current_collision = None;
        self.materialized_pose_revision = None;
    }

    pub fn set_model(
        &mut self,
        current_model: String,
        capability: Option<std::sync::Arc<xmodel_runtime::RetainedModelCapability>>,
    ) {
        self.model_revision = self.model_revision.wrapping_add(1);
        self.current_model = current_model.clone();
        self.semantic_state.composition =
            xmodel_runtime::DObjCompositionDescriptor::single(self.model_revision, current_model);
        self.capability = capability;
        self.current_collision = None;
        self.materialized_model_revision = None;
        self.materialized_pose_revision = None;
        self.materialize_error = None;
    }

    pub fn begin_script_model_play_anim(&mut self, clip: &str, looping: bool, frequency: f32) {
        self.pose_revision = self.pose_revision.wrapping_add(1);
        self.play_anim = Some(ScriptModelPlayAnim { looping, frequency });
        self.semantic_state = xmodel_runtime::DObjSemanticState::one_leaf(
            self.current_model.clone(),
            clip.to_owned(),
            self.model_revision,
            self.pose_revision,
            0.0,
        );
        self.pose_request = xmodel_runtime::DObjPoseRequest::bind_pose();
        self.current_collision = None;
        self.materialized_pose_revision = None;
        self.materialize_error = None;
    }

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
        // BO2 returns None when the step changes nothing: the leaf keeps its time.
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
        self.pose_revision = self.pose_revision.wrapping_add(1);
        self.semantic_state.pose_revision = self.pose_revision;
        self.current_collision = None;
        self.materialized_pose_revision = None;
        leaf.state.time
    }

    pub fn begin_script_mover_rotate_velocity(
        &mut self,
        speed: [f32; 3],
        total_time_seconds: f32,
        level_time_ms: i32,
        current_angles: [f32; 3],
    ) {
        let current = self.apos.unwrap_or(entity_t6::Trajectory {
            tr_base: current_angles,
            ..entity_t6::Trajectory::default()
        });
        self.apos = Some(crate::gentity::rotate_velocity_apos(
            &current,
            speed,
            total_time_seconds,
            level_time_ms,
        ));
        self.apply_apos_at(level_time_ms);
    }

    pub fn apply_apos_at(&mut self, at_time_ms: i32) -> Option<[f32; 3]> {
        let apos = self.apos?;
        self.apply_trajectory_apos_at(apos, at_time_ms)
    }

    pub fn apply_trajectory_apos_at(
        &mut self,
        apos: entity_t6::Trajectory,
        at_time_ms: i32,
    ) -> Option<[f32; 3]> {
        let angles = entity_t6::evaluate_trajectory(&apos, at_time_ms);
        let origin = self.world_from_model.w_axis.truncate();
        self.world_from_model = iw_angles_to_mat4(origin, angles);
        Some(angles)
    }

    pub fn materialize(&mut self) {
        if self.materialized_model_revision == Some(self.model_revision)
            && self.materialized_pose_revision == Some(self.pose_revision)
        {
            return;
        }
        self.current_collision = None;
        let Some(capability) = &self.capability else {
            self.materialize_error = Some(xmodel_runtime::MaterializeError::DObj(
                "retained model capability unavailable".to_owned(),
            ));
            return;
        };
        if !contents_match_mask(capability.contents, MASK_BULLET_WORLD) {
            self.current_collision = None;
            self.materialized_model_revision = Some(self.model_revision);
            self.materialized_pose_revision = Some(self.pose_revision);
            self.materialize_error = None;
            return;
        }
        match capability.geom_collision(
            &self.pose_request,
            self.world_from_model,
            MASK_BULLET_WORLD,
        ) {
            Ok(mut bones) => {
                // bo2mp: a destructible's hidden pieces (broken off, or a
                // stage not reached yet) stop nothing.
                if self.t5_destructible.is_some() {
                    bones.retain(|b| !self.pose_request.hide_part_bits.get(usize::from(b.bone)));
                }
                let coll = coll_trace_from_capability(
                    capability,
                    &self.pose_request,
                    self.world_from_model,
                );
                match coll {
                    Ok(coll) => {
                        self.current_collision = Some(AuthorityDObjCollision { bones, coll });
                        self.materialized_model_revision = Some(self.model_revision);
                        self.materialized_pose_revision = Some(self.pose_revision);
                        self.materialize_error = None;
                    }
                    Err(error) => {
                        self.materialize_error = Some(error);
                    }
                }
            }
            Err(error) => {
                self.materialize_error = Some(error);
            }
        }
    }

    pub fn tag_world_matrix(&self, tag: &str) -> Option<glam::Mat4> {
        let capability = self.capability.as_ref()?;
        let bone = capability
            .pose
            .bone_names
            .iter()
            .position(|name| name == tag)?;
        let posed = capability
            .pose(&self.pose_request, self.world_from_model)
            .ok()?;
        posed.get(bone).copied()
    }

    pub fn tag_world_pose(&self, tag: &str) -> Option<([f32; 3], [f32; 3])> {
        let matrix = self.tag_world_matrix(tag)?;
        let origin = matrix.w_axis.truncate().to_array();
        let forward = matrix.x_axis.truncate();
        let direction = if forward.length_squared() > 1e-8 {
            forward.normalize().to_array()
        } else {
            [0.0, 0.0, 1.0]
        };
        Some((origin, direction))
    }

    pub fn ensure_bounds_collision(&mut self) {
        let Some(capability) = self.capability.as_ref() else {
            return;
        };
        let Some(bone) = capability.bounds_collision_bone(self.world_from_model) else {
            return;
        };
        match &mut self.current_collision {
            Some(coll) => {
                let already = coll.bones.iter().any(|have| {
                    have.center == bone.center
                        && have.half_size == bone.half_size
                        && have.axes == bone.axes
                });
                if !already {
                    coll.bones.push(bone);
                }
            }
            None => {
                self.current_collision = Some(AuthorityDObjCollision {
                    bones: vec![bone],
                    coll: None,
                });
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LinkedBrushCollisionBrush {
    pub cmodel_handle: u32,
    pub origin: [f32; 3],
    pub angles: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityCollisionEpoch {
    CurrentTick,
    Historical { frame: Tick },
}

impl EntityCollisionEpoch {
    pub const fn digest_tag(self) -> u32 {
        match self {
            Self::CurrentTick => 1,
            Self::Historical { .. } => 2,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct EntityCollisionTraceGeom {
    pub owner: AuthorityModelOwner,
    pub epoch: EntityCollisionEpoch,
    pub collision: Option<AuthorityDObjCollision>,

    pub dobj_contents: Option<u32>,

    pub model_key: Option<String>,
    pub linked_brushes: Vec<LinkedBrushCollisionBrush>,
}

#[derive(Clone, Debug)]
pub struct EntityCollisionCapabilities {
    pub owner: AuthorityModelOwner,
    epoch: EntityCollisionEpoch,
    pub dobj: Option<AuthorityDObjState>,
    pub linked_brushes: Vec<LinkedBrushCollisionBrush>,
    pub hidden: bool,
    pub solid: bool,
    pub followed_pose: Option<([f32; 3], [f32; 3])>,
    /// bo2zm M3: made on the prediction side from a brush mover in the
    /// snapshot (`presence::adopt_brush_movers`); goes when its mover does.
    pub from_snapshot: bool,
}

impl EntityCollisionCapabilities {
    pub fn current_tick(
        owner: AuthorityModelOwner,
        dobj: Option<AuthorityDObjState>,
        linked_brushes: Vec<LinkedBrushCollisionBrush>,
    ) -> Self {
        Self {
            owner,
            epoch: EntityCollisionEpoch::CurrentTick,
            dobj,
            linked_brushes,
            hidden: false,
            solid: true,
            followed_pose: None,
            from_snapshot: false,
        }
    }

    pub const fn epoch(&self) -> EntityCollisionEpoch {
        self.epoch
    }

    pub fn solid_brushes(&self) -> &[LinkedBrushCollisionBrush] {
        if self.solid {
            &self.linked_brushes
        } else {
            &[]
        }
    }

    /// Must stay the bounds test `bullet_trace_filtered` applies to each geom.
    pub(crate) fn ray_may_hit(
        &self,
        cmodels: &SimClipCmodels,
        start: [f32; 3],
        end: [f32; 3],
    ) -> bool {
        if !self.solid {
            return false;
        }
        let collision = self
            .dobj
            .as_ref()
            .and_then(|state| state.current_collision.as_ref());
        entity_abs_aabb(&self.linked_brushes, collision, cmodels).is_some_and(|(mins, maxs)| {
            !matches!(ray_aabb_box(start, end, mins, maxs), RayAabb::Miss)
        })
    }

    pub fn trace_geom(&self) -> EntityCollisionTraceGeom {
        if !self.solid {
            return EntityCollisionTraceGeom {
                owner: self.owner,
                epoch: self.epoch,
                collision: None,
                dobj_contents: None,
                model_key: None,
                linked_brushes: Vec::new(),
            };
        }
        EntityCollisionTraceGeom {
            owner: self.owner,
            epoch: self.epoch,
            collision: self
                .dobj
                .as_ref()
                .and_then(|state| state.current_collision.clone()),
            dobj_contents: self.dobj.as_ref().and_then(|state| {
                state
                    .capability
                    .as_ref()
                    .and_then(|capability| capability.contents)
            }),
            model_key: self.dobj.as_ref().map(|state| state.current_model.clone()),
            linked_brushes: self.linked_brushes.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TraceInvalidReason {
    NonFiniteInput,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TraceOutcome {
    Miss {
        end: [f32; 3],
    },
    Hit {
        fraction: f32,
        end: [f32; 3],
        normal: [f32; 3],
        collider: ColliderId,
    },
    StartSolid {
        collider: Option<ColliderId>,
        end: [f32; 3],
    },
    Invalid {
        reason: TraceInvalidReason,
    },
}

pub type BulletHitKind = ColliderId;

#[derive(Clone, Copy, Debug, PartialEq)]
struct TraceCandidate {
    fraction: f32,
    endpos: [f32; 3],
    normal: [f32; 3],
    collider: ColliderId,
    startsolid: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BulletPath {
    #[default]
    Extended,
    Penetrate,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BulletTraceSegment {
    pub start: [f32; 3],
    pub end: [f32; 3],
    /// Impact surface normal; exit effects carry the forward bullet direction.
    pub normal: [f32; 3],
    pub surf_type: u8,

    pub surface_flags: u32,
    pub penetrated: bool,

    pub thickness: f32,

    pub damage_mult: f32,
    pub path: BulletPath,
    pub collider: Option<ColliderId>,

    pub startsolid: bool,

    pub glass_encoded: u16,

    pub hit_type: i32,

    pub hit_id: u16,
}

impl BulletTraceSegment {
    pub fn surf_name(self) -> &'static str {
        // bo2mp: BO2's 32 names; past the end is "default".
        weapon_t6::surface::T6_SURFACE_TYPE_NAMES
            .get(self.surf_type as usize)
            .copied()
            .unwrap_or("default")
    }
}

#[derive(Clone, Copy, Debug)]
pub struct BulletTraceQuery {
    pub start: [f32; 3],
    pub end: [f32; 3],
    pub mask: u32,
    pub ignore: Option<ClientId>,

    pub ignore_hit: Option<ClientId>,
    pub ignore_model: Option<ScriptModelId>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HitVolumeKind {
    #[default]
    Standing,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlayerCollisionPose {
    pub client: ClientId,
    pub origin: [f32; 3],
    pub mins: [f32; 3],
    pub maxs: [f32; 3],
    pub life_sequence: LifeSequence,
    pub hit_volume: HitVolumeKind,
    pub bones: Vec<AuthorityDObjCollisionBone>,
    /// The riot shield he bears (Black Ops II), if any.
    pub shield: Option<ShieldCover>,
    /// The tag that shield hangs on, where it stood (Black Ops II's
    /// G_GetShieldTagMatrix: its place and axes), if he bears one.
    pub shield_tag: Option<xmodel_runtime::CollisionBone>,
}

/// The hit location of a bullet the riot shield stopped ("riotshield" to
/// BO2's scripts).
pub const HITLOC_SHIELD: u8 = weapon_t6::hitloc::HITLOC_RIOTSHIELD;
/// The surface a riot shield is (its impact effects; nothing goes through):
/// Black Ops II's "riotshield" (MY_SURFTYPEINDEX). the legacy list has it at 29,
/// which in Black Ops II's is "player".
pub const SURF_TYPE_RIOTSHIELD: u32 = 31;
/// The sky surface flag.
pub const SURF_SKY: u32 = 0x4;
/// Bullet_FirePenetrate: a bullet goes this deep into a sky brush, whatever the table says.
pub const SKY_PENETRATION_DEPTH: f32 = 100.0;

/// A riot shield on a player (Black Ops II), for when the shield's model
/// gives no boxes of its own to shoot. Black Ops II's bullets meet the
/// shield as its own part of his body (a box on the model in his hand, or on
/// his back, that outranks the body's parts); this stands in for that box
/// only when it is missing: a bullet from within the angle of the way the
/// shield faces — his forward for the shield in his hands, his back for the
/// one stowed there — is taken for a hit on the shield.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShieldCover {
    /// The way the shield faces (flat, unit length).
    pub facing: [f32; 2],
    /// The cosine of the protection angle.
    pub cos_limit: f32,
}

impl ShieldCover {
    /// True when something travelling along `dir` comes at him from the
    /// side the shield faces, within its angle.
    pub fn stops(&self, dir: [f32; 3]) -> bool {
        let (x, y) = (-dir[0], -dir[1]);
        let len = (x * x + y * y).sqrt();
        len > 1e-6 && (x * self.facing[0] + y * self.facing[1]) / len >= self.cos_limit
    }
}

/// True when the shield he bears has boxes of its own among his bones.
fn has_shield_box(pose: &PlayerCollisionPose) -> bool {
    pose.bones.iter().any(|b| b.part_classification == HITLOC_SHIELD)
}

/// With no box of its own, a hit on a shield-bearing player from the side
/// his shield faces is taken for a hit on the shield. (A pose with the
/// shield's boxes needs none: the bullet meets them or the body directly.)
fn shield_hits(candidates: &mut [TraceCandidate], players: &[PlayerCollisionPose], query: &BulletTraceQuery) {
    let dir = [
        query.end[0] - query.start[0],
        query.end[1] - query.start[1],
        query.end[2] - query.start[2],
    ];
    for c in candidates.iter_mut() {
        if let ColliderId::Player { client, life, .. } = c.collider
            && players
                .iter()
                .find(|p| p.client == client && p.life_sequence == life)
                .filter(|p| !has_shield_box(p))
                .and_then(|p| p.shield)
                .is_some_and(|s| s.stops(dir))
        {
            c.collider = ColliderId::Player {
                client,
                life,
                hitloc: HITLOC_SHIELD,
            };
        }
    }
}

fn is_shield(collider: ColliderId) -> bool {
    matches!(collider, ColliderId::Player { hitloc: HITLOC_SHIELD, .. })
}

/// A hit on a sky surface.
pub(crate) fn is_sky(collider: ColliderId) -> bool {
    collider_surface_flags(collider) & SURF_SKY != 0
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HistoryPhase {
    #[default]
    PostMovement,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HistoryFrame {
    pub tick: Tick,
    pub phase: HistoryPhase,
    pub poses: Vec<PlayerCollisionPose>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CurrentAuthorityReason {
    NoSampleClaim,

    SampleAtShot,

    HistoryUnavailable { requested: Tick },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryClampReason {
    MaxRewindWindow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryRefusalReason {
    MissingFrame,

    SampleAfterShot,

    SampleMalformed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistorySampleVerdict {
    CurrentAuthority {
        tick: Tick,
        reason: CurrentAuthorityReason,
    },
    Exact {
        requested: Tick,
        frame: Tick,
        phase: HistoryPhase,
    },
    Clamped {
        requested: Tick,
        used: Tick,
        frame: Tick,
        phase: HistoryPhase,
        reason: HistoryClampReason,
    },
    Refused {
        requested: Tick,
        reason: HistoryRefusalReason,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct HistorySample {
    pub poses: Vec<PlayerCollisionPose>,
    pub verdict: HistorySampleVerdict,
}

#[derive(Clone, Debug)]
pub struct CollisionHistory {
    frames: Vec<HistoryFrame>,
    capacity: usize,
}

impl Default for CollisionHistory {
    fn default() -> Self {
        Self::with_capacity(COLLISION_HISTORY_TICKS)
    }
}

impl CollisionHistory {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            frames: Vec::new(),
            capacity: capacity.max(1),
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn push_frame(&mut self, frame: HistoryFrame) {
        if let Some(last) = self.frames.last_mut()
            && last.tick == frame.tick
            && last.phase == frame.phase
        {
            *last = frame;
            return;
        }
        self.frames.push(frame);
        while self.frames.len() > self.capacity {
            self.frames.remove(0);
        }
    }

    pub fn push(&mut self, tick: Tick, poses: Vec<PlayerCollisionPose>) {
        self.push_frame(HistoryFrame {
            tick,
            phase: HistoryPhase::PostMovement,
            poses,
        });
    }

    pub fn frame_at(&self, tick: Tick) -> Option<&HistoryFrame> {
        self.frames.iter().rev().find(|f| f.tick == tick)
    }

    pub fn latest(&self) -> Option<&HistoryFrame> {
        self.frames.last()
    }

    pub fn latest_poses(&self) -> Option<(Tick, &[PlayerCollisionPose])> {
        self.latest().map(|f| (f.tick, f.poses.as_slice()))
    }

    pub fn clear(&mut self) {
        self.frames.clear();
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct EntityCollisionFrame {
    pub tick: Tick,
    pub phase: HistoryPhase,
    pub rows: Vec<EntityCollisionTraceGeom>,
}

#[derive(Clone, Debug)]
pub struct EntityCollisionHistory {
    frames: Vec<EntityCollisionFrame>,
    capacity: usize,
}

impl Default for EntityCollisionHistory {
    fn default() -> Self {
        Self::with_capacity(COLLISION_HISTORY_TICKS)
    }
}

impl EntityCollisionHistory {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            frames: Vec::new(),
            capacity: capacity.max(1),
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn push_frame(&mut self, frame: EntityCollisionFrame) {
        if let Some(last) = self.frames.last_mut()
            && last.tick == frame.tick
            && last.phase == frame.phase
        {
            *last = frame;
            return;
        }
        self.frames.push(frame);
        while self.frames.len() > self.capacity {
            self.frames.remove(0);
        }
    }

    pub fn frame_at(&self, tick: Tick) -> Option<&EntityCollisionFrame> {
        self.frames.iter().rev().find(|f| f.tick == tick)
    }

    pub fn latest(&self) -> Option<&EntityCollisionFrame> {
        self.frames.last()
    }

    pub fn clear(&mut self) {
        self.frames.clear();
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct EntityCollisionSample {
    pub rows: Vec<EntityCollisionTraceGeom>,
    pub verdict: HistorySampleVerdict,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LagcompQuery {
    pub players: HistorySample,
    pub entities: EntityCollisionSample,
}

pub fn bullet_trace(
    brushes: &[SimBrush],
    players: &[PlayerCollisionPose],
    query: &BulletTraceQuery,
    glass_is_solid: &dyn Fn(u16) -> bool,
) -> TraceOutcome {
    bullet_trace_with_entity_models(
        brushes,
        &SimClipBsp::default(),
        &SimClipCmodels::default(),
        &SimClipMesh::default(),
        players,
        &[],
        query,
        glass_is_solid,
    )
}

pub fn bullet_trace_with_entity_models(
    brushes: &[SimBrush],
    bsp: &SimClipBsp,
    cmodels: &SimClipCmodels,
    mesh: &SimClipMesh,
    players: &[PlayerCollisionPose],
    script_models: &[EntityCollisionTraceGeom],
    query: &BulletTraceQuery,
    glass_is_solid: &dyn Fn(u16) -> bool,
) -> TraceOutcome {
    bullet_trace_filtered(
        brushes,
        bsp,
        cmodels,
        mesh,
        players,
        script_models,
        query,
        glass_is_solid,
        false,
    )
}

enum DObjGeometryVerdict {
    Hit(TraceCandidate),
    Missed,
    Unavailable,
}

pub(crate) fn coll_tris_clip_available(coll: Option<&AuthorityDObjCollTrace>, mask: u32) -> bool {
    coll.is_some_and(|coll| {
        coll.model.coll_lod >= 0
            && coll
                .model
                .surfs
                .iter()
                .any(|surf| surf.contents & mask != 0 && surf.tri_count != 0)
    })
}

fn trace_dobj_coll_tris(
    coll: Option<&AuthorityDObjCollTrace>,
    owner: AuthorityModelOwner,
    query: &BulletTraceQuery,
) -> DObjGeometryVerdict {
    let Some(coll) = coll else {
        return DObjGeometryVerdict::Unavailable;
    };
    if !coll_tris_clip_available(Some(coll), query.mask) {
        return DObjGeometryVerdict::Unavailable;
    }
    let (Some(local_start), Some(local_end)) = (
        world_point_to_entity(coll.world_from_model, query.start),
        world_point_to_entity(coll.world_from_model, query.end),
    ) else {
        return DObjGeometryVerdict::Unavailable;
    };
    let mut tr = trace_t6::Trace {
        fraction: 1.0,
        ..trace_t6::Trace::default()
    };
    let bone = coll.model.trace_line_animated(
        &mut tr,
        local_start,
        local_end,
        query.mask,
        &coll.bones,
        &coll.hide_part_bits,
    );
    if bone < 0 {
        return DObjGeometryVerdict::Missed;
    }
    let Ok(bone) = u16::try_from(bone) else {
        return DObjGeometryVerdict::Missed;
    };
    DObjGeometryVerdict::Hit(TraceCandidate {
        fraction: tr.fraction,
        endpos: [
            query.start[0] + (query.end[0] - query.start[0]) * tr.fraction,
            query.start[1] + (query.end[1] - query.start[1]) * tr.fraction,
            query.start[2] + (query.end[2] - query.start[2]) * tr.fraction,
        ],
        normal: entity_normal_to_world(coll.world_from_model, tr.normal),
        collider: ColliderId::EntityDObjBone {
            owner,
            bone,
            part_classification: 0,
            surface_flags: tr.sflags,
        },
        startsolid: false,
    })
}

fn bullet_trace_filtered(
    brushes: &[SimBrush],
    bsp: &SimClipBsp,
    cmodels: &SimClipCmodels,
    mesh: &SimClipMesh,
    players: &[PlayerCollisionPose],
    script_models: &[EntityCollisionTraceGeom],
    query: &BulletTraceQuery,
    glass_is_solid: &dyn Fn(u16) -> bool,
    locational: bool,
) -> TraceOutcome {
    if !is_finite_vec3(query.start) || !is_finite_vec3(query.end) {
        return TraceOutcome::Invalid {
            reason: TraceInvalidReason::NonFiniteInput,
        };
    }

    let mut candidates: Vec<TraceCandidate> = Vec::new();

    let (world, glass_encoded) = trace_world(
        brushes,
        bsp,
        mesh,
        query.start,
        query.end,
        query.mask,
        glass_is_solid,
    );
    let world_collider = ColliderId::World {
        surface_flags: world.sflags,
        contents: world.cflags,
        glass_encoded,
    };
    if world.startsolid || world.allsolid {
        return TraceOutcome::StartSolid {
            collider: Some(world_collider),
            end: query.start,
        };
    }
    if world.fraction < 1.0 {
        candidates.push(TraceCandidate {
            fraction: world.fraction,
            endpos: world.end_pos(query.start, query.end),
            normal: world.normal,
            collider: world_collider,
            startsolid: false,
        });
    }

    let entity_end = if world.fraction < 1.0 {
        world.end_pos(query.start, query.end)
    } else {
        query.end
    };
    for pose in players {
        if query.ignore == Some(pose.client)
            || query.ignore_hit == Some(pose.client)
            || query.mask & CONTENTS_BODY == 0
        {
            continue;
        }
        if !pose.bones.is_empty() {
            if let Some((mins, maxs)) = pose_bones_aabb(pose) {
                if matches!(
                    ray_aabb_box(query.start, entity_end, mins, maxs),
                    RayAabb::Miss
                ) {
                    continue;
                }
            }
            for bone in &pose.bones {
                let collider = ColliderId::Player {
                    client: pose.client,
                    life: pose.life_sequence,
                    hitloc: bone.part_classification,
                };
                match ray_obb(query.start, query.end, bone, collider, Some(pose.origin)) {
                    RayAabb::StartSolid => {
                        return TraceOutcome::StartSolid {
                            collider: Some(collider),
                            end: query.start,
                        };
                    }
                    RayAabb::Hit(c) => candidates.push(c),
                    RayAabb::Miss => {}
                }
            }
            continue;
        }
        match ray_aabb(query.start, query.end, pose) {
            RayAabb::StartSolid => {
                return TraceOutcome::StartSolid {
                    collider: Some(ColliderId::Player {
                        client: pose.client,
                        life: pose.life_sequence,
                        hitloc: 0,
                    }),
                    end: query.start,
                };
            }
            RayAabb::Hit(c) => candidates.push(c),
            RayAabb::Miss => {}
        }
    }

    for geom in script_models {
        if query.ignore_model.is_some() && geom.owner.script_model() == query.ignore_model {
            continue;
        }
        if let Some((mins, maxs)) = geom_abs_aabb(geom, cmodels) {
            if matches!(
                ray_aabb_box(query.start, query.end, mins, maxs),
                RayAabb::Miss
            ) {
                continue;
            }
        } else {
            continue;
        }
        for brush in &geom.linked_brushes {
            // bo2mp: a gun's bullet traces a model's triangles alone (BO2's
            // SV_TracePointToEntity takes DObjGeomTraceline for a script
            // model, svFlags 4, and never its collmap): a car's walk brushes
            // stop no bullet, so one goes in through a broken window.
            if locational
                && geom.model_key.is_some()
                && cmodels
                    .model_cmodels
                    .values()
                    .any(|&handle| handle == brush.cmodel_handle)
            {
                continue;
            }
            let Some(map) = bsp.t6 else {
                continue;
            };
            let hit = crate::world::t6_transformed_trace(
                map,
                query.start,
                query.end,
                [0.0; 3],
                [0.0; 3],
                crate::world::T6Model::Brush(brush.cmodel_handle),
                query.mask,
                brush.origin,
                brush.angles,
            );
            let collider = ColliderId::EntityLinkedBrush {
                owner: geom.owner,
                cmodel_handle: brush.cmodel_handle,
                surface_flags: hit.sflags,
            };
            if hit.startsolid || hit.allsolid {
                return TraceOutcome::StartSolid {
                    collider: Some(collider),
                    end: query.start,
                };
            }
            if hit.fraction < 1.0 {
                candidates.push(TraceCandidate {
                    fraction: hit.fraction,
                    endpos: hit.end_pos(query.start, query.end),
                    normal: hit.normal,
                    collider,
                    startsolid: false,
                });
            }
        }
        let Some(dobj_geom) = geom.collision.as_ref() else {
            continue;
        };
        if !contents_match_mask(geom.dobj_contents, query.mask) {
            continue;
        }
        match trace_dobj_coll_tris(dobj_geom.coll.as_ref(), geom.owner, query) {
            DObjGeometryVerdict::Hit(candidate) => {
                candidates.push(candidate);
                continue;
            }
            DObjGeometryVerdict::Missed => continue,
            DObjGeometryVerdict::Unavailable => {}
        }
        for bone in &dobj_geom.bones {
            let collider = ColliderId::EntityDObjBone {
                owner: geom.owner,
                bone: bone.bone,
                part_classification: bone.part_classification,
                surface_flags: 0,
            };
            let origin = dobj_geom
                .coll
                .as_ref()
                .map(|coll| coll.world_from_model.w_axis.truncate().to_array());
            match ray_obb(query.start, query.end, bone, collider, origin) {
                RayAabb::StartSolid => {
                    return TraceOutcome::StartSolid {
                        collider: Some(collider),
                        end: query.start,
                    };
                }
                RayAabb::Hit(candidate) => candidates.push(candidate),
                RayAabb::Miss => {}
            }
        }
    }

    shield_hits(&mut candidates, players, query);
    let outcome = select_first_hit(query, &mut candidates);
    if shield_log_on() {
        log_shield_holders(players, query, entity_end, &candidates, &outcome);
    }
    outcome
}

/// `BO2MP_DAMAGE_LOG`: why a penetrating bullet stopped early (test aid).
fn pen_stop_log(reason: &str, at: [f32; 3]) {
    if crate::combat::damage_log() {
        diag::info!(
            Sim,
            "bo2mp damage: bullet stopped ({reason}) at ({:.1} {:.1} {:.1})",
            at[0],
            at[1],
            at[2]
        );
    }
}

/// BO2MP_HITBOXLOG=1: each ray that starts inside a hit box.
fn hitbox_log_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("BO2MP_HITBOXLOG").is_some())
}

fn shield_log_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("BO2MP_SHIELDLOG").is_some())
}

/// `BO2MP_SHIELDLOG=1`: for each bullet that reaches a player bearing a
/// riot shield, whether it hit the shield or a part of his body (and which),
/// or went by him.
fn log_shield_holders(
    players: &[PlayerCollisionPose],
    query: &BulletTraceQuery,
    reach: [f32; 3],
    candidates: &[TraceCandidate],
    outcome: &TraceOutcome,
) {
    let name_of = |hitloc: u8| -> &'static str {
        if hitloc == HITLOC_SHIELD {
            "riotshield"
        } else {
            crate::t6::T6_HITLOC_NAMES.get(usize::from(hitloc)).copied().unwrap_or("none")
        }
    };
    for pose in players {
        let shield_boxes = pose.bones.iter().filter(|b| b.part_classification == HITLOC_SHIELD).count();
        if (pose.shield.is_none() && shield_boxes == 0)
            || query.ignore == Some(pose.client)
            || query.ignore_hit == Some(pose.client)
            || query.mask & CONTENTS_BODY == 0
        {
            continue;
        }
        let (mins, maxs) = pose_bones_aabb(pose).unwrap_or_else(|| {
            (
                [
                    pose.origin[0] + pose.mins[0],
                    pose.origin[1] + pose.mins[1],
                    pose.origin[2] + pose.mins[2],
                ],
                [
                    pose.origin[0] + pose.maxs[0],
                    pose.origin[1] + pose.maxs[1],
                    pose.origin[2] + pose.maxs[2],
                ],
            )
        });
        if matches!(ray_aabb_box(query.start, reach, mins, maxs), RayAabb::Miss) {
            continue;
        }
        let mut crossed: Vec<(f32, &'static str)> = candidates
            .iter()
            .filter_map(|c| match c.collider {
                ColliderId::Player { client, life, hitloc } if client == pose.client && life == pose.life_sequence => {
                    Some((c.fraction, name_of(hitloc)))
                }
                _ => None,
            })
            .collect();
        crossed.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        let crossed: Vec<String> = crossed.iter().map(|(f, n)| format!("{n}@{f:.3}")).collect();
        let verdict = match outcome {
            TraceOutcome::Hit {
                collider: ColliderId::Player { client, hitloc, .. },
                fraction,
                ..
            } if *client == pose.client => {
                if *hitloc == HITLOC_SHIELD {
                    format!("hit SHIELD at {fraction:.3}")
                } else {
                    format!("hit BODY {} at {fraction:.3}", name_of(*hitloc))
                }
            }
            TraceOutcome::Hit { fraction, .. } => format!("blocked by something else at {fraction:.3}"),
            TraceOutcome::StartSolid { .. } => "started inside something".to_string(),
            _ => "MISSED him".to_string(),
        };
        diag::info!(
            Sim,
            "bo2mp shield: holder {}: bullet {verdict}; his boxes on its line (nearest first) [{}]; shield boxes {shield_boxes}; cone fallback {}",
            pose.client.0,
            crossed.join(", "),
            if shield_boxes == 0 && pose.shield.is_some() { "in use" } else { "off" }
        );
    }
}

pub fn bullet_trace_segments(
    brushes: &[SimBrush],
    players: &[PlayerCollisionPose],
    query: &BulletTraceQuery,
    pen: BulletPenFacts,
    table: &PenetrationDepthTable,
    glass_is_solid: &dyn Fn(u16) -> bool,
) -> (Vec<BulletTraceSegment>, Option<ColliderId>) {
    bullet_trace_segments_with_entity_models(
        brushes,
        &SimClipBsp::default(),
        &SimClipCmodels::default(),
        &SimClipMesh::default(),
        players,
        &[],
        query,
        pen,
        table,
        glass_is_solid,
    )
}

pub fn bullet_trace_segments_with_entity_models(
    brushes: &[SimBrush],
    bsp: &SimClipBsp,
    cmodels: &SimClipCmodels,
    mesh: &SimClipMesh,
    players: &[PlayerCollisionPose],
    script_models: &[EntityCollisionTraceGeom],
    query: &BulletTraceQuery,
    pen: BulletPenFacts,
    table: &PenetrationDepthTable,
    glass_is_solid: &dyn Fn(u16) -> bool,
) -> (Vec<BulletTraceSegment>, Option<ColliderId>) {
    bullet_trace_segments_filtered(
        brushes,
        bsp,
        cmodels,
        mesh,
        players,
        script_models,
        query,
        pen,
        table,
        glass_is_solid,
        None,
    )
}

pub(crate) fn bullet_trace_segments_filtered(
    brushes: &[SimBrush],
    bsp: &SimClipBsp,
    cmodels: &SimClipCmodels,
    mesh: &SimClipMesh,
    players: &[PlayerCollisionPose],
    script_models: &[EntityCollisionTraceGeom],
    query: &BulletTraceQuery,
    pen: BulletPenFacts,
    table: &PenetrationDepthTable,
    glass_is_solid: &dyn Fn(u16) -> bool,
    on_glass_hit: Option<&dyn Fn(u16, [f32; 3])>,
) -> (Vec<BulletTraceSegment>, Option<ColliderId>) {
    if !is_finite_vec3(query.start) || !is_finite_vec3(query.end) {
        return (Vec::new(), None);
    }
    let dir = [
        query.end[0] - query.start[0],
        query.end[1] - query.start[1],
        query.end[2] - query.start[2],
    ];
    let max_len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
    if max_len <= 0.0 {
        return (Vec::new(), None);
    }
    let ray_dir = [dir[0] / max_len, dir[1] / max_len, dir[2] / max_len];
    let world = TraceWorld {
        brushes,
        bsp,
        mesh,
        cmodels,
        players,
        script_models,
        glass_is_solid,
        on_glass_hit,
    };
    if pen.pen_gate() {
        fire_penetrate(&world, query, ray_dir, pen, table)
    } else {
        fire_extended(&world, query, ray_dir, pen)
    }
}

struct TraceWorld<'a> {
    brushes: &'a [SimBrush],
    bsp: &'a SimClipBsp,
    mesh: &'a SimClipMesh,
    cmodels: &'a SimClipCmodels,
    players: &'a [PlayerCollisionPose],
    script_models: &'a [EntityCollisionTraceGeom],

    glass_is_solid: &'a dyn Fn(u16) -> bool,
    on_glass_hit: Option<&'a dyn Fn(u16, [f32; 3])>,
}

struct HitGeom {
    end: [f32; 3],
    normal: [f32; 3],
    collider: ColliderId,
}

fn fire_extended(
    world: &TraceWorld<'_>,
    query: &BulletTraceQuery,
    dir: [f32; 3],
    pen: BulletPenFacts,
) -> (Vec<BulletTraceSegment>, Option<ColliderId>) {
    let mut segments = Vec::new();
    let mut start = query.start;
    let mut ignore_hit = query.ignore_hit;
    let mut multiplier = 1.0_f32;
    let mut terminal = None;
    let mut seg_start = query.start;
    for _ in 0..MAX_EXTENDED_STEPS {
        let outcome = trace_from(world, query, start, ignore_hit, query.ignore_model);
        match decode_hit(outcome) {
            Decode::StopMiss { end } => {
                segments.push(make_segment(
                    seg_start,
                    end,
                    [0.0; 3],
                    None,
                    false,
                    0.0,
                    multiplier,
                    BulletPath::Extended,
                    false,
                ));
                break;
            }
            Decode::StopSolid { end, collider } => {
                segments.push(make_segment(
                    seg_start,
                    end,
                    [0.0; 3],
                    collider,
                    false,
                    0.0,
                    multiplier,
                    BulletPath::Extended,
                    true,
                ));
                if collider.is_some() {
                    terminal = collider;
                }
                break;
            }
            Decode::Hit(hit) => {
                let contents = collider_contents(hit.collider);
                let glass_contents = contents & CONTENTS_GLASS != 0;
                if glass_contents {
                    if is_open_pane(world, hit.collider) {
                        let Some(next) =
                            advance_trace(hit.end, hit.normal, dir, true, ADVANCE_TRACE_FWD)
                        else {
                            break;
                        };
                        start = next;
                        continue;
                    }
                    note_glass_hit(world, hit.collider, hit.end);
                }
                segments.push(make_segment_from_hit(
                    seg_start,
                    &hit,
                    glass_contents || is_player(hit.collider),
                    0.0,
                    multiplier,
                    BulletPath::Extended,
                    false,
                ));
                terminal = Some(hit.collider);
                if glass_contents {
                    let Some(next) =
                        advance_trace(hit.end, hit.normal, dir, true, ADVANCE_TRACE_FWD)
                    else {
                        break;
                    };
                    start = next;
                    seg_start = hit.end;
                    continue;
                }
                if !is_damage_collider(hit.collider) {
                    break;
                }
                ignore_hit = player_id(hit.collider).or(ignore_hit);
                if !pen.rifle_bullet || !is_player(hit.collider) || is_shield(hit.collider) {
                    break;
                }
                multiplier *= RIFLE_COLLATERAL_SCALE;
                let Some(next) = advance_trace(hit.end, hit.normal, dir, false, 0.0) else {
                    break;
                };
                start = next;
                seg_start = hit.end;
            }
            Decode::Invalid => break,
        }
    }
    (segments, terminal)
}

fn fire_penetrate(
    world: &TraceWorld<'_>,
    query: &BulletTraceQuery,
    dir: [f32; 3],
    pen: BulletPenFacts,
    table: &PenetrationDepthTable,
) -> (Vec<BulletTraceSegment>, Option<ColliderId>) {
    let mut segments = Vec::new();
    let mut start = query.start;
    let mut ignore_hit = query.ignore_hit;
    let mut multiplier = 1.0_f32;
    let mut last_surf = 0u32;
    let mut seg_start = query.start;

    let mut open_skips = 0u32;
    let (first, first_startsolid) = loop {
        match decode_hit(trace_from(world, query, start, ignore_hit, query.ignore_model)) {
            Decode::StopMiss { end } => {
                segments.push(make_segment(
                    seg_start,
                    end,
                    [0.0; 3],
                    None,
                    false,
                    0.0,
                    multiplier,
                    BulletPath::Penetrate,
                    false,
                ));
                return (segments, None);
            }
            Decode::StopSolid { collider, .. } => {
                let Some(collider) = collider else {
                    segments.push(make_segment(
                        seg_start,
                        query.start,
                        [0.0; 3],
                        None,
                        false,
                        0.0,
                        multiplier,
                        BulletPath::Penetrate,
                        true,
                    ));
                    return (segments, None);
                };
                break (startsolid_as_hit(collider, query.start, dir), true);
            }
            Decode::Hit(hit) if is_open_pane(world, hit.collider) => {
                let Some(next) = advance_trace(hit.end, hit.normal, dir, true, ADVANCE_TRACE_FWD)
                else {
                    return (segments, None);
                };
                open_skips = open_skips.saturating_add(1);
                if open_skips as usize >= MAX_EXTENDED_STEPS {
                    return (segments, None);
                }
                start = next;
            }
            Decode::Hit(hit) => break (hit, false),
            Decode::Invalid => return (segments, None),
        }
    };
    last_surf = depth_surf(first.collider, last_surf);
    segments.push(make_segment_from_hit(
        seg_start,
        &first,
        true,
        0.0,
        multiplier,
        BulletPath::Penetrate,
        first_startsolid,
    ));
    note_glass_hit(world, first.collider, first.end);
    let mut terminal = Some(first.collider);
    let mut last_hit = first;

    for _ in 0..MAX_PENETRATE_STEPS {
        // A riot shield stops the bullet.
        if is_shield(last_hit.collider) {
            pen_stop_log("riot shield", last_hit.end);
            break;
        }
        let entry_depth = if is_sky(last_hit.collider) {
            SKY_PENETRATION_DEPTH
        } else {
            table.depth(pen.penetrate_type, last_surf)
        };
        let mut max_depth = entry_depth * pen.penetrate_multiplier;
        if max_depth <= 0.0 {
            pen_stop_log("no depth for this surface", last_hit.end);
            break;
        }
        let hit_world = is_world(last_hit.collider);
        let Some(next_start) = advance_trace(
            last_hit.end,
            last_hit.normal,
            dir,
            hit_world,
            ADVANCE_TRACE_FWD,
        ) else {
            break;
        };
        ignore_hit = player_id(last_hit.collider).or(ignore_hit);
        let ignore_model = model_id(last_hit.collider).or(query.ignore_model);
        start = next_start;
        seg_start = last_hit.end;
        let entry_pos = last_hit.end;
        let fwd = decode_hit(trace_from(world, query, start, ignore_hit, ignore_model));
        let (trace_hit, fwd_hit) = match fwd {
            Decode::Hit(hit) => (true, Some(hit)),
            Decode::StopMiss { end } => {
                segments.push(make_segment(
                    seg_start,
                    end,
                    [0.0; 3],
                    None,
                    false,
                    0.0,
                    multiplier,
                    BulletPath::Penetrate,
                    false,
                ));
                break;
            }
            Decode::StopSolid { collider, .. } => {
                let Some(exit) = walk_out_of_solid(
                    world,
                    query,
                    start,
                    dir,
                    ignore_hit,
                    ignore_model,
                    (max_depth + PEN_THICKNESS_FLOOR).max(2.0),
                ) else {
                    pen_stop_log("started inside a solid it could not leave", start);
                    break;
                };
                let mut thickness = segment_length(entry_pos, exit);
                if thickness < PEN_THICKNESS_FLOOR {
                    thickness = PEN_THICKNESS_FLOOR;
                }
                multiplier -= thickness / max_depth;
                if multiplier <= 0.0 {
                    break;
                }
                last_hit.end = exit;
                last_hit.normal = [-dir[0], -dir[1], -dir[2]];
                if collider.is_some_and(is_damage_collider) {
                    terminal = collider;
                }
                continue;
            }
            Decode::Invalid => {
                pen_stop_log("invalid trace", start);
                break;
            }
        };
        if let Some(hit) = &fwd_hit {
            if is_open_pane(world, hit.collider) {
                let Some(next) = advance_trace(hit.end, hit.normal, dir, true, ADVANCE_TRACE_FWD)
                else {
                    break;
                };
                last_hit.end = next;
                last_hit.normal = hit.normal;
                continue;
            }
            note_glass_hit(world, hit.collider, hit.end);
        }

        let rev_dir = [-dir[0], -dir[1], -dir[2]];

        let mut rev_start = fwd_hit.as_ref().map(|h| h.end).unwrap_or(query.end);
        let rev_end = vec3_mad(entry_pos, REV_END_EPS, rev_dir);
        if let Some(hit) = &fwd_hit {
            if let Some(nudged) = advance_trace(
                hit.end,
                scale3(hit.normal, -1.0),
                rev_dir,
                is_world(hit.collider),
                ADVANCE_TRACE_REV,
            ) {
                rev_start = nudged;
            }
        }

        let rev_ignore = fwd_hit.as_ref().and_then(|h| player_id(h.collider));
        let rev_ignore_model = match &fwd_hit {
            Some(h) => model_id(h.collider).or(query.ignore_model),
            None => ignore_model,
        };
        let rev_q = BulletTraceQuery {
            start: rev_start,
            end: rev_end,
            mask: query.mask,
            ignore: query.ignore,
            ignore_hit: rev_ignore,
            ignore_model: rev_ignore_model,
        };
        let rev = bullet_trace_filtered(
            world.brushes,
            world.bsp,
            world.cmodels,
            world.mesh,
            world.players,
            world.script_models,
            &rev_q,
            world.glass_is_solid,
            true,
        );
        let (rev_hit_pos, rev_surf, rev_hit, rev_sky) = match rev {
            TraceOutcome::Hit { end, collider, .. } => {
                (Some(end), depth_surf(collider, 0), true, is_sky(collider))
            }
            TraceOutcome::StartSolid { end, collider, .. } => (
                Some(end),
                collider.map(depth_surf_new).unwrap_or(0),
                true,
                collider.is_some_and(is_sky),
            ),
            _ => (None, 0, false, false),
        };

        let mut thickness = 0.0_f32;
        if rev_hit {
            thickness = if let Some(pos) = rev_hit_pos {
                segment_length(entry_pos, pos)
            } else {
                segment_length(rev_end, rev_start)
            };
            if thickness < PEN_THICKNESS_FLOOR {
                thickness = PEN_THICKNESS_FLOOR;
            }
            if rev_hit {
                let exit_depth =
                    table.depth(pen.penetrate_type, rev_surf) * pen.penetrate_multiplier;
                max_depth = if exit_depth < max_depth {
                    exit_depth
                } else {
                    max_depth
                };
                // Leaving through a sky face is always 100 deep.
                if rev_sky {
                    max_depth = SKY_PENETRATION_DEPTH;
                } else if max_depth <= 0.0 {
                    break;
                }
            }
            multiplier -= thickness / max_depth;
            if multiplier <= 0.0 {
                break;
            }
        } else if !trace_hit {
            break;
        }

        if let Some(pos) = rev_hit_pos {
            let mut exit_seg = make_segment(
                entry_pos,
                pos,
                dir,
                Some(last_hit.collider),
                true,
                thickness,
                multiplier,
                BulletPath::Penetrate,
                false,
            );
            exit_seg.surface_flags |= fx_t6::runtime::FX_IMPACT_EXIT_SURFACE_FLAG;
            segments.push(exit_seg);
        }

        if let Some(hit) = fwd_hit {
            last_surf = depth_surf(hit.collider, last_surf);
            segments.push(make_segment_from_hit(
                seg_start,
                &hit,
                true,
                thickness,
                multiplier,
                BulletPath::Penetrate,
                false,
            ));
            terminal = Some(hit.collider);
            last_hit = hit;
        } else {
            break;
        }
    }
    (segments, terminal)
}

enum Decode {
    Hit(HitGeom),
    StopMiss {
        end: [f32; 3],
    },
    StopSolid {
        end: [f32; 3],
        collider: Option<ColliderId>,
    },
    Invalid,
}

fn decode_hit(outcome: TraceOutcome) -> Decode {
    match outcome {
        TraceOutcome::Miss { end } => Decode::StopMiss { end },
        TraceOutcome::StartSolid { end, collider, .. } => Decode::StopSolid { end, collider },
        TraceOutcome::Hit {
            end,
            normal,
            collider,
            ..
        } => Decode::Hit(HitGeom {
            end,
            normal,
            collider,
        }),
        TraceOutcome::Invalid { .. } => Decode::Invalid,
    }
}

fn trace_from(
    world: &TraceWorld<'_>,
    query: &BulletTraceQuery,
    start: [f32; 3],
    ignore_hit: Option<ClientId>,
    ignore_model: Option<ScriptModelId>,
) -> TraceOutcome {
    bullet_trace_filtered(
        world.brushes,
        world.bsp,
        world.cmodels,
        world.mesh,
        world.players,
        world.script_models,
        &BulletTraceQuery {
            start,
            end: query.end,
            mask: query.mask,
            ignore: query.ignore,
            ignore_hit,
            ignore_model,
        },
        world.glass_is_solid,
        true,
    )
}

fn make_segment(
    start: [f32; 3],
    end: [f32; 3],
    normal: [f32; 3],
    collider: Option<ColliderId>,
    penetrated: bool,
    thickness: f32,
    damage_mult: f32,
    path: BulletPath,
    startsolid: bool,
) -> BulletTraceSegment {
    let (hit_type, hit_id) = collider
        .map(|c| collider_hit_kind(c, startsolid))
        .unwrap_or((0, 0));
    BulletTraceSegment {
        start,
        end,
        normal,
        surf_type: collider.map(surf_type_for).unwrap_or(0),
        surface_flags: collider.map(collider_surface_flags).unwrap_or(0),
        penetrated,
        thickness,
        damage_mult,
        path,
        collider,
        startsolid,
        glass_encoded: collider.map(collider_glass_encoded).unwrap_or(0),
        hit_type,
        hit_id,
    }
}

fn make_segment_from_hit(
    start: [f32; 3],
    hit: &HitGeom,
    penetrated: bool,
    thickness: f32,
    damage_mult: f32,
    path: BulletPath,
    startsolid: bool,
) -> BulletTraceSegment {
    make_segment(
        start,
        hit.end,
        hit.normal,
        Some(hit.collider),
        penetrated,
        thickness,
        damage_mult,
        path,
        startsolid,
    )
}

fn surf_type_for(collider: ColliderId) -> u8 {
    match collider {
        ColliderId::World { surface_flags, .. } => surface_type_from_flags(surface_flags),
        ColliderId::Player { hitloc: HITLOC_SHIELD, .. } => SURF_TYPE_RIOTSHIELD as u8,
        ColliderId::Player { .. } => SURF_TYPE_FLESH as u8,
        ColliderId::EntityDObjBone { surface_flags, .. }
        | ColliderId::EntityLinkedBrush { surface_flags, .. } => {
            surface_type_from_flags(surface_flags)
        }
    }
}

fn depth_surf(collider: ColliderId, last: u32) -> u32 {
    depth_surface_type(collider_surface_flags(collider), last)
}

fn depth_surf_new(collider: ColliderId) -> u32 {
    depth_surf(collider, 0)
}

fn collider_surface_flags(collider: ColliderId) -> u32 {
    match collider {
        ColliderId::World { surface_flags, .. } => surface_flags,
        ColliderId::Player { hitloc: HITLOC_SHIELD, .. } => SURF_TYPE_RIOTSHIELD << 20,
        ColliderId::Player { .. } => SURF_TYPE_FLESH << 20,
        ColliderId::EntityDObjBone { surface_flags, .. }
        | ColliderId::EntityLinkedBrush { surface_flags, .. } => surface_flags,
    }
}

fn collider_contents(collider: ColliderId) -> u32 {
    match collider {
        ColliderId::World { contents, .. } => contents,
        _ => 0,
    }
}

fn collider_glass_encoded(collider: ColliderId) -> u16 {
    match collider {
        ColliderId::World { glass_encoded, .. } => glass_encoded,
        _ => 0,
    }
}

fn pane_id(collider: ColliderId) -> Option<u16> {
    let encoded = collider_glass_encoded(collider);
    (encoded != 0).then(|| encoded - 1)
}

fn is_open_pane(world: &TraceWorld<'_>, collider: ColliderId) -> bool {
    pane_id(collider).is_some_and(|piece| !(world.glass_is_solid)(piece))
}

fn note_glass_hit(world: &TraceWorld<'_>, collider: ColliderId, end: [f32; 3]) {
    let Some(piece) = pane_id(collider) else {
        return;
    };
    if !(world.glass_is_solid)(piece) {
        return;
    }
    if let Some(cb) = world.on_glass_hit {
        cb(piece, end);
    }
}

/// A bullet segment's hit kind (BO2's `TraceHitType`, as a number) and id.
const HITTYPE_ENTITY: i32 = TraceHitType::Entity as i32;
const HITTYPE_GLASS: i32 = TraceHitType::Glass as i32;

fn collider_hit_kind(collider: ColliderId, startsolid: bool) -> (i32, u16) {
    match collider {
        ColliderId::World { glass_encoded, .. } if !startsolid && glass_encoded != 0 => {
            (HITTYPE_GLASS, glass_encoded)
        }
        ColliderId::World { .. } => (HITTYPE_ENTITY, trace_t6::ENTITYNUM_WORLD),
        ColliderId::Player { .. }
        | ColliderId::EntityDObjBone { .. }
        | ColliderId::EntityLinkedBrush { .. } => (HITTYPE_ENTITY, 0),
    }
}

fn is_world(collider: ColliderId) -> bool {
    matches!(collider, ColliderId::World { .. })
}

fn is_player(collider: ColliderId) -> bool {
    matches!(collider, ColliderId::Player { .. })
}

fn is_damage_collider(collider: ColliderId) -> bool {
    matches!(
        collider,
        ColliderId::Player { .. }
            | ColliderId::EntityDObjBone { .. }
            | ColliderId::EntityLinkedBrush { .. }
    )
}

fn player_id(collider: ColliderId) -> Option<ClientId> {
    match collider {
        ColliderId::Player { client, .. } => Some(client),
        _ => None,
    }
}

/// The script model a hit belongs to. BO2's `BG_AdvanceTrace` sets
/// `ignoreEntIndex` to the hit entity, so the next trace skips it.
fn model_id(collider: ColliderId) -> Option<ScriptModelId> {
    match collider {
        ColliderId::EntityDObjBone { owner, .. } | ColliderId::EntityLinkedBrush { owner, .. } => {
            owner.script_model()
        }
        _ => None,
    }
}

fn segment_length(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    let dz = b[2] - a[2];
    (dx * dx + dy * dy + dz * dz).sqrt()
}

fn vec3_mad(a: [f32; 3], s: f32, b: [f32; 3]) -> [f32; 3] {
    [a[0] + s * b[0], a[1] + s * b[1], a[2] + s * b[2]]
}

fn startsolid_as_hit(collider: ColliderId, start: [f32; 3], dir: [f32; 3]) -> HitGeom {
    HitGeom {
        end: start,
        normal: [-dir[0], -dir[1], -dir[2]],
        collider,
    }
}

fn walk_out_of_solid(
    world: &TraceWorld<'_>,
    query: &BulletTraceQuery,
    start: [f32; 3],
    dir: [f32; 3],
    ignore_hit: Option<ClientId>,
    ignore_model: Option<ScriptModelId>,
    limit: f32,
) -> Option<[f32; 3]> {
    let step = 0.25_f32;
    let mut traveled = step;
    while traveled <= limit {
        let probe = vec3_mad(start, traveled, dir);
        match trace_from(world, query, probe, ignore_hit, ignore_model) {
            TraceOutcome::StartSolid { .. } => {}
            TraceOutcome::Invalid { .. } => return None,
            _ => return Some(probe),
        }
        traveled += step;
    }
    None
}

/// Brush traces back off the hit plane by this many inches (the legacy trace).
const SURFACE_CLIP_EPSILON: f32 = 0.125;

fn ray_length(query: &BulletTraceQuery) -> f32 {
    let dx = query.end[0] - query.start[0];
    let dy = query.end[1] - query.start[1];
    let dz = query.end[2] - query.start[2];
    (dx * dx + dy * dy + dz * dz).sqrt()
}

fn flush_with_world(query: &BulletTraceQuery, a: &TraceCandidate, b: &TraceCandidate) -> bool {
    let world_vs_prop = (is_world(a.collider) && is_damage_collider(b.collider))
        || (is_world(b.collider) && is_damage_collider(a.collider));
    if !world_vs_prop {
        return false;
    }
    let len = ray_length(query);
    (a.fraction * len - b.fraction * len).abs() <= SURFACE_CLIP_EPSILON
}

fn select_first_hit(query: &BulletTraceQuery, candidates: &mut [TraceCandidate]) -> TraceOutcome {
    if candidates.is_empty() {
        return TraceOutcome::Miss { end: query.end };
    }
    candidates.sort_by(|a, b| {
        if flush_with_world(query, a, b) {
            return hit_order_key(a).cmp(&hit_order_key(b));
        }
        match a
            .fraction
            .partial_cmp(&b.fraction)
            .unwrap_or(std::cmp::Ordering::Equal)
        {
            std::cmp::Ordering::Equal => hit_order_key(a).cmp(&hit_order_key(b)),
            o => o,
        }
    });
    let best = &candidates[0];
    if best.startsolid {
        return TraceOutcome::StartSolid {
            collider: Some(best.collider),
            end: query.start,
        };
    }
    TraceOutcome::Hit {
        fraction: best.fraction,
        end: best.endpos,
        normal: best.normal,
        collider: best.collider,
    }
}

fn hit_order_key(hit: &TraceCandidate) -> (u8, u32) {
    match hit.collider {
        ColliderId::Player { client, .. } => (0, client.0),
        ColliderId::EntityDObjBone { owner, bone, .. } => {
            (1, owner_order_key(owner).wrapping_add(u32::from(bone)))
        }
        ColliderId::EntityLinkedBrush { owner, .. } => (2, owner_order_key(owner)),
        ColliderId::World { .. } => (3, 0),
    }
}

fn owner_order_key(owner: AuthorityModelOwner) -> u32 {
    match owner {
        AuthorityModelOwner::ScriptModel(id) => id.0,
    }
}

fn is_finite_vec3(v: [f32; 3]) -> bool {
    v.iter().all(|c| c.is_finite())
}

pub fn glass_piece_from_hit(hit_type: i32, hit_id: u16) -> Option<u32> {
    (hit_type == HITTYPE_GLASS && hit_id != 0).then(|| u32::from(hit_id) - 1)
}

fn trace_world(
    brushes: &[SimBrush],
    bsp: &SimClipBsp,
    mesh: &SimClipMesh,
    start: [f32; 3],
    end: [f32; 3],
    mask: u32,
    glass_is_solid: &dyn Fn(u16) -> bool,
) -> (Trace, u16) {
    let mut hit = crate::world::clip_trace(
        brushes,
        bsp,
        mesh,
        start,
        end,
        [0.0; 3],
        [0.0; 3],
        mask,
        glass_is_solid,
    );
    if let Some(statics) = mesh.t6_static.filter(|_| hit.fraction != 0.0) {
        crate::world::t6_static_trace(statics, start, end, mask, &mut hit);
    }
    let encoded = trace_t6::trace_get_glass_hit_id(&hit).unwrap_or(0);
    (hit, encoded)
}

enum RayAabb {
    Miss,
    StartSolid,
    Hit(TraceCandidate),
}

fn ray_aabb(start: [f32; 3], end: [f32; 3], pose: &PlayerCollisionPose) -> RayAabb {
    let mins = [
        pose.origin[0] + pose.mins[0],
        pose.origin[1] + pose.mins[1],
        pose.origin[2] + pose.mins[2],
    ];
    let maxs = [
        pose.origin[0] + pose.maxs[0],
        pose.origin[1] + pose.maxs[1],
        pose.origin[2] + pose.maxs[2],
    ];
    match ray_aabb_box(start, end, mins, maxs) {
        RayAabb::Miss => RayAabb::Miss,
        RayAabb::StartSolid => RayAabb::StartSolid,
        RayAabb::Hit(mut c) => {
            c.collider = ColliderId::Player {
                client: pose.client,
                life: pose.life_sequence,
                hitloc: 0,
            };
            RayAabb::Hit(c)
        }
    }
}

fn ray_aabb_box(start: [f32; 3], end: [f32; 3], mins: [f32; 3], maxs: [f32; 3]) -> RayAabb {
    let inside = (0..3).all(|i| start[i] >= mins[i] && start[i] <= maxs[i]);
    if inside {
        return RayAabb::StartSolid;
    }

    let dir = [end[0] - start[0], end[1] - start[1], end[2] - start[2]];

    let mut tmin = 0.0_f32;
    let mut tmax = 1.0_f32;
    let mut normal = [0.0_f32; 3];

    for axis in 0..3 {
        if dir[axis].abs() < 1e-8 {
            if start[axis] < mins[axis] || start[axis] > maxs[axis] {
                return RayAabb::Miss;
            }
            continue;
        }
        let inv = 1.0 / dir[axis];
        let mut t1 = (mins[axis] - start[axis]) * inv;
        let mut t2 = (maxs[axis] - start[axis]) * inv;
        let mut n = [0.0_f32; 3];
        n[axis] = if inv < 0.0 { 1.0 } else { -1.0 };
        if t1 > t2 {
            core::mem::swap(&mut t1, &mut t2);
        }
        if t1 > tmin {
            tmin = t1;
            normal = n;
        }
        tmax = tmax.min(t2);
        if tmin > tmax {
            return RayAabb::Miss;
        }
    }

    if tmin < 0.0 || tmin > 1.0 {
        return RayAabb::Miss;
    }

    RayAabb::Hit(TraceCandidate {
        fraction: tmin,
        endpos: [
            start[0] + dir[0] * tmin,
            start[1] + dir[1] * tmin,
            start[2] + dir[2] * tmin,
        ],
        normal,
        collider: ColliderId::World {
            surface_flags: 0,
            contents: 0,
            glass_encoded: 0,
        },
        startsolid: false,
    })
}

fn expand_aabb(mins: &mut [f32; 3], maxs: &mut [f32; 3], pmin: [f32; 3], pmax: [f32; 3]) {
    for i in 0..3 {
        mins[i] = mins[i].min(pmin[i]);
        maxs[i] = maxs[i].max(pmax[i]);
    }
}

fn pad_aabb(mins: [f32; 3], maxs: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    (
        [
            mins[0] - LINK_BOUNDS_PAD,
            mins[1] - LINK_BOUNDS_PAD,
            mins[2] - LINK_BOUNDS_PAD,
        ],
        [
            maxs[0] + LINK_BOUNDS_PAD,
            maxs[1] + LINK_BOUNDS_PAD,
            maxs[2] + LINK_BOUNDS_PAD,
        ],
    )
}

fn obb_world_aabb(bone: &AuthorityDObjCollisionBone) -> ([f32; 3], [f32; 3]) {
    let mut extent = [0.0_f32; 3];
    for world_axis in 0..3 {
        extent[world_axis] = bone.axes[0][world_axis].abs() * bone.half_size[0]
            + bone.axes[1][world_axis].abs() * bone.half_size[1]
            + bone.axes[2][world_axis].abs() * bone.half_size[2];
    }
    (
        [
            bone.center[0] - extent[0],
            bone.center[1] - extent[1],
            bone.center[2] - extent[2],
        ],
        [
            bone.center[0] + extent[0],
            bone.center[1] + extent[1],
            bone.center[2] + extent[2],
        ],
    )
}

fn pose_bones_aabb(pose: &PlayerCollisionPose) -> Option<([f32; 3], [f32; 3])> {
    let mut mins = [f32::MAX; 3];
    let mut maxs = [f32::MIN; 3];
    let mut any = false;
    for bone in &pose.bones {
        let (bmin, bmax) = obb_world_aabb(bone);
        expand_aabb(&mut mins, &mut maxs, bmin, bmax);
        any = true;
    }
    any.then(|| pad_aabb(mins, maxs))
}

fn cmodel_world_aabb(
    cmodel: &asset_world::ClipCmodel,
    origin: [f32; 3],
    angles: [f32; 3],
) -> ([f32; 3], [f32; 3]) {
    if angles[0] == 0.0 && angles[1] == 0.0 && angles[2] == 0.0 {
        return pad_aabb(
            [
                origin[0] + cmodel.mins[0],
                origin[1] + cmodel.mins[1],
                origin[2] + cmodel.mins[2],
            ],
            [
                origin[0] + cmodel.maxs[0],
                origin[1] + cmodel.maxs[1],
                origin[2] + cmodel.maxs[2],
            ],
        );
    }
    let mat = iw_angles_to_mat4(glam::Vec3::from_array(origin), angles);
    let mut mins = [f32::MAX; 3];
    let mut maxs = [f32::MIN; 3];
    for &x in &[cmodel.mins[0], cmodel.maxs[0]] {
        for &y in &[cmodel.mins[1], cmodel.maxs[1]] {
            for &z in &[cmodel.mins[2], cmodel.maxs[2]] {
                let p = mat.transform_point3(glam::Vec3::new(x, y, z));
                expand_aabb(&mut mins, &mut maxs, p.to_array(), p.to_array());
            }
        }
    }
    pad_aabb(mins, maxs)
}

fn geom_abs_aabb(
    geom: &EntityCollisionTraceGeom,
    cmodels: &SimClipCmodels,
) -> Option<([f32; 3], [f32; 3])> {
    entity_abs_aabb(&geom.linked_brushes, geom.collision.as_ref(), cmodels)
}

fn entity_abs_aabb(
    linked_brushes: &[LinkedBrushCollisionBrush],
    collision: Option<&AuthorityDObjCollision>,
    cmodels: &SimClipCmodels,
) -> Option<([f32; 3], [f32; 3])> {
    let mut mins = [f32::MAX; 3];
    let mut maxs = [f32::MIN; 3];
    let mut any = false;
    for brush in linked_brushes {
        let Some(cmodel) = cmodels.models.get(brush.cmodel_handle as usize)
        else {
            continue;
        };
        let (bmin, bmax) = cmodel_world_aabb(cmodel, brush.origin, brush.angles);
        expand_aabb(&mut mins, &mut maxs, bmin, bmax);
        any = true;
    }
    if let Some(dobj) = collision {
        for bone in &dobj.bones {
            let (bmin, bmax) = obb_world_aabb(bone);
            expand_aabb(&mut mins, &mut maxs, bmin, bmax);
            any = true;
        }
    }
    any.then_some((mins, maxs))
}

fn ray_obb(
    start: [f32; 3],
    end: [f32; 3],
    bone: &AuthorityDObjCollisionBone,
    collider: ColliderId,
    origin: Option<[f32; 3]>,
) -> RayAabb {
    let relative = |point: [f32; 3]| {
        let delta = [
            point[0] - bone.center[0],
            point[1] - bone.center[1],
            point[2] - bone.center[2],
        ];
        bone.axes.map(|axis| dot3(delta, axis))
    };
    let local_start = relative(start);
    let local_end = relative(end);
    let inside = |local: [f32; 3]| (0..3).all(|axis| local[axis].abs() <= bone.half_size[axis]);
    if inside(local_start) {
        // bo2mp window 5: Black Ops II's hit-box ray test never starts solid. A ray that starts
        // inside a hit box and leaves it gets no hit from that box; one that stays inside while
        // moving (level) in toward the body's middle hits at once, facing level out from the middle.
        let out = origin.map(|o| [start[0] - o[0], start[1] - o[1]]);
        let inward = out.is_some_and(|out| {
            (end[0] - start[0]) * out[0] + (end[1] - start[1]) * out[1] < 0.0
        });
        let verdict = if inside(local_end) && inward {
            let out = out.unwrap_or([0.0; 2]);
            let len = (out[0] * out[0] + out[1] * out[1]).sqrt();
            let normal = if len > 0.0 {
                [out[0] / len, out[1] / len, 0.0]
            } else {
                [0.0; 3]
            };
            RayAabb::Hit(TraceCandidate {
                fraction: 0.0,
                endpos: start,
                normal,
                collider,
                startsolid: false,
            })
        } else {
            RayAabb::Miss
        };
        if hitbox_log_on() {
            diag::info!(
                Sim,
                "hitbox: ray starts inside {collider:?} at {start:?}: {}",
                if matches!(verdict, RayAabb::Hit(_)) { "hit at once" } else { "no hit from this box" }
            );
        }
        return verdict;
    }
    let direction = [
        local_end[0] - local_start[0],
        local_end[1] - local_start[1],
        local_end[2] - local_start[2],
    ];
    let mut tmin = 0.0_f32;
    let mut tmax = 1.0_f32;
    let mut normal = [0.0; 3];
    for axis in 0..3 {
        if direction[axis].abs() < 1e-8 {
            if local_start[axis].abs() > bone.half_size[axis] {
                return RayAabb::Miss;
            }
            continue;
        }
        let inverse = direction[axis].recip();
        let mut near = (-bone.half_size[axis] - local_start[axis]) * inverse;
        let mut far = (bone.half_size[axis] - local_start[axis]) * inverse;
        let sign = if inverse < 0.0 { 1.0 } else { -1.0 };
        if near > far {
            core::mem::swap(&mut near, &mut far);
        }
        if near > tmin {
            tmin = near;
            normal = scale3(bone.axes[axis], sign);
        }
        tmax = tmax.min(far);
        if tmin > tmax {
            return RayAabb::Miss;
        }
    }
    if !(0.0..=1.0).contains(&tmin) {
        return RayAabb::Miss;
    }
    RayAabb::Hit(TraceCandidate {
        fraction: tmin,
        endpos: [
            start[0] + (end[0] - start[0]) * tmin,
            start[1] + (end[1] - start[1]) * tmin,
            start[2] + (end[2] - start[2]) * tmin,
        ],
        normal,
        collider,
        startsolid: false,
    })
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn scale3(value: [f32; 3], scale: f32) -> [f32; 3] {
    [value[0] * scale, value[1] * scale, value[2] * scale]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoverageSupport {
    Supported,
    Locked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CollisionCoverageRow {
    pub id: &'static str,
    pub support: CoverageSupport,
    pub note: &'static str,
}

pub const COLLISION_COVERAGE: &[CollisionCoverageRow] = &[
    CollisionCoverageRow {
        id: "walked_brush_planes",
        support: CoverageSupport::Supported,
        note: "convex brush sweeps via BO2's CM_BoxTrace (clipmap_t6)",
    },
    CollisionCoverageRow {
        id: "player_aabb",
        support: CoverageSupport::Supported,
        note: "standing AABB hit volume; stance variants Locked",
    },
    CollisionCoverageRow {
        id: "leaf_brush_bsp",
        support: CoverageSupport::Locked,
        note: "leafbrush BSP culling not walked",
    },
    CollisionCoverageRow {
        id: "patches_terrain",
        support: CoverageSupport::Supported,
        note: "clipmap CollisionAabbTree / tri soup via trace_brush_and_mesh (same tables as pmove clip_trace); GfxStaticModel is static_model_collision",
    },
    CollisionCoverageRow {
        id: "script_model_xbone_obb",
        support: CoverageSupport::Supported,
        note: "posed collSurf Bounds as abs-AABB cull; a model with captured collTris (collLod>=0, MASK_SHOT surfs) is clipped by them alone, hit or miss; fixture/bounds OBB clip only where no collTris were captured; XModel contents of 0 is unspecified",
    },
    CollisionCoverageRow {
        id: "static_model_collision",
        support: CoverageSupport::Supported,
        note: "linear walk of captured static-model collision triangles; world sectors open",
    },
    CollisionCoverageRow {
        id: "dynamic_entities",
        support: CoverageSupport::Locked,
        note: "dynent sweeps absent",
    },
    CollisionCoverageRow {
        id: "stance_hit_locations",
        support: CoverageSupport::Locked,
        note: "crouch/prone/hitloc volumes not differentiated",
    },
    CollisionCoverageRow {
        id: "glass_destructibles",
        support: CoverageSupport::Supported,
        note: "MASK_SHOT includes CONTENTS_GLASS; mid-trace on_glass_hit updates solidity before the next hop; glass tess apply is presentation",
    },
    CollisionCoverageRow {
        id: "mask_shot_material_surface",
        support: CoverageSupport::Locked,
        note: "material surface table is not modelled",
    },
    CollisionCoverageRow {
        id: "projectile_sweep",
        support: CoverageSupport::Supported,
        note: "the missile world trace is the same zero-extent brush∪mesh ray as hitscan; plantable capsule hull and TR_STATIONARY rest stay open",
    },
];

#[cfg(test)]
mod shield_tests {
    use super::*;
    use crate::identities::LifeSequence;

    fn pose(client: u32, shield: Option<ShieldCover>) -> PlayerCollisionPose {
        PlayerCollisionPose {
            client: ClientId(client),
            origin: [0.0, 0.0, 0.0],
            mins: [-15.0, -15.0, 0.0],
            maxs: [15.0, 15.0, 72.0],
            life_sequence: LifeSequence(1),
            hit_volume: HitVolumeKind::Standing,
            bones: Vec::new(),
            shield,
            shield_tag: None,
        }
    }

    fn shoot(players: &[PlayerCollisionPose], from_x: f32) -> TraceOutcome {
        let query = BulletTraceQuery {
            start: [from_x, 0.0, 40.0],
            end: [-from_x, 0.0, 40.0],
            mask: MASK_SHOT,
            ignore: None,
            ignore_hit: None,
            ignore_model: None,
        };
        bullet_trace(&[], players, &query, &|_| false)
    }

    fn hitloc(out: TraceOutcome) -> Option<u8> {
        match out {
            TraceOutcome::Hit { collider: ColliderId::Player { hitloc, .. }, .. } => Some(hitloc),
            _ => None,
        }
    }

    // Shield faces +x and covers 60 degrees either side.
    const FRONT: ShieldCover = ShieldCover { facing: [1.0, 0.0], cos_limit: 0.5 };

    #[test]
    fn a_shot_from_the_front_hits_the_shield() {
        assert_eq!(hitloc(shoot(&[pose(3, Some(FRONT))], 200.0)), Some(HITLOC_SHIELD));
    }

    #[test]
    fn a_shot_from_behind_hits_the_body() {
        let h = hitloc(shoot(&[pose(3, Some(FRONT))], -200.0));
        assert!(h.is_some() && h != Some(HITLOC_SHIELD), "{h:?}");
    }

    #[test]
    fn no_shield_means_no_shield_hit() {
        let h = hitloc(shoot(&[pose(3, None)], 200.0));
        assert!(h.is_some() && h != Some(HITLOC_SHIELD), "{h:?}");
    }

    #[test]
    fn a_shot_from_the_side_misses_the_shield() {
        let side = ShieldCover { facing: [0.0, 1.0], cos_limit: 0.5 };
        let h = hitloc(shoot(&[pose(3, Some(side))], 200.0));
        assert!(h.is_some() && h != Some(HITLOC_SHIELD), "{h:?}");
    }
}

#[cfg(test)]
mod sky_tests {
    use super::*;

    fn aabb(min: [f32; 3], max: [f32; 3], surf: u32) -> SimBrush {
        SimBrush {
            planes: vec![
                [-1.0, 0.0, 0.0, -min[0]],
                [1.0, 0.0, 0.0, max[0]],
                [0.0, -1.0, 0.0, -min[1]],
                [0.0, 1.0, 0.0, max[1]],
                [0.0, 0.0, -1.0, -min[2]],
                [0.0, 0.0, 1.0, max[2]],
            ],
            contents: 0x1,
            plane_surface_flags: vec![surf; 6],
            glass_encoded: 0,
            verts: Vec::new(),
            hit_surface_flags: surf,
        }
    }

    /// The furthest x a bullet along +x reaches through a 50-thick brush at x 100..150 with a
    /// wall at x 300, when the table gives every surface no depth.
    fn reach(first_surf: u32) -> f32 {
        let brushes = [
            aabb([100.0, -50.0, -50.0], [150.0, 50.0, 50.0], first_surf),
            aabb([300.0, -50.0, -50.0], [320.0, 50.0, 50.0], 2 << 20),
        ];
        let query = BulletTraceQuery {
            start: [0.0, 0.0, 0.0],
            end: [1000.0, 0.0, 0.0],
            mask: MASK_SHOT,
            ignore: None,
            ignore_hit: None,
            ignore_model: None,
        };
        let pen = BulletPenFacts {
            penetrate_type: 1,
            penetrate_multiplier: 1.0,
            rifle_bullet: false,
            impact_explode: false,
        };
        let table = PenetrationDepthTable::empty();
        let (segments, _) = bullet_trace_segments(&brushes, &[], &query, pen, &table, &|_| false);
        segments.iter().map(|s| s.end[0]).fold(0.0, f32::max)
    }

    #[test]
    fn a_bullet_goes_through_a_sky_brush_the_table_gives_no_depth() {
        let x = reach(SURF_SKY);
        assert!((x - 300.0).abs() < 1.0, "{x}");
    }

    #[test]
    fn the_same_brush_without_sky_stops_the_bullet() {
        let x = reach(2 << 20);
        assert!((x - 100.0).abs() < 1.0, "{x}");
    }
}
