//! What the `hitvol` console commands print: a count of what the collision world holds, and one line per player
//! body that shots can hit.

use super::bodies::player_controller_input;
use super::SeatNo;
use crate::shot_trace::PlayerHitboxPose;
use crate::clip_tally::{
    CollisionCensus, ModelCollisionCensus, WorldClipCensus, entity_clip_census, player_clip_census,
};
use playerstate_t6::game::AvatarState;

/// `body_part_category` of the bone boxes the dump reports on.
const PART_HEAD: u8 = 2;
/// BO2 `torso_lower` (6); 5 is `torso_mid`.
const PART_PELVIS: u8 = 6;
/// Text shown in a dump column that has nothing to report.
const NO_DATA: &str = "none";

/// One player body as shots see it. A client of `None` stands for "no body is recorded yet".
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HitboxReportRow {
    pub client: Option<super::SeatNo>,
    pub bones_total: usize,
    /// "bones", "aabb" (no bone boxes, the plain player box) or "none".
    pub shape_data: &'static str,
    pub torso_model_key: String,
    pub face_model_key: String,
    /// How the skeleton is posed: "bind", or "none" without a body model or a player state.
    pub pose_label: &'static str,
    /// The first bone box's center, the pelvis box's half size and center, the head box's center.
    pub first_center: Option<[f32; 3]>,
    pub pelvis_half: Option<[f32; 3]>,
    pub pelvis_center: Option<[f32; 3]>,
    pub head_center: Option<[f32; 3]>,
    /// "standing", "prone" or "none".
    pub driver: &'static str,
    pub controller_tags: Option<i64>,
    pub view_pitch: Option<f32>,
    pub entity_flags: Option<u32>,
    pub move_flags: Option<u32>,
    pub lean: Option<f32>,
    pub move_type: Option<i64>,
    pub pen_table_ready: bool,
    pub bake_failure: Option<String>,
}

/// What the body's controller (the spine bend and lean) read from the player.
struct ControllerFacts {
    stance: &'static str,
    tags: i64,
}

impl super::state::MatchSimulation {
    /// Counts of what the collision world, the recorded players, the entity owners and the body kits hold.
    pub fn collision_census(&self) -> CollisionCensus {
        let bsp = self.collision_tree();
        let t6 = bsp.t6;
        let statics = self.collision_triangles().t6_static;
        let world = WorldClipCensus {
            brushes: self.collision_solids().len() as u32,
            bsp_nodes: t6.map_or(0, |map| map.nodes.len()) as u32,
            bsp_leaves: t6.map_or(0, |map| map.leafs.len()) as u32,
            brush_leaf_indices: bsp.brush_leaf_indices.len() as u32,
            mesh_tris: t6.map_or(0, |map| map.tri_indices.len()) as u32,
            cmodels: self.collision_submodel_set().models.len() as u32,
            prop_set: statics.map_or(0, |w| w.models.len()) as u32,
            static_models_with_tris: statics.map_or(0, |w| {
                w.models.iter().filter(|m| m.xmodel.is_some_and(|x| x.surfs.iter().any(|s| !s.triangle_list.is_empty()))).count()
            }) as u32,
            wall_pierce_table_ready: self.content.wall_pierce_table_ready(),
        };
        let recorded = self.replay.history.latest_poses().map_or(&[][..], |(_, poses)| poses);
        let players = player_clip_census(recorded, self.bodies.bake_failure.clone());
        let entities = entity_clip_census(&self.space.caps, crate::shot_trace::SHOT_WORLD_FILTER);
        let kits = self
            .content
            .outfit_hitboxes()
            .iter()
            .flat_map(|kit| {
                [
                    ModelCollisionCensus::of(&kit.torso_model_key, kit.body.as_deref()),
                    ModelCollisionCensus::of(&kit.face_model_key, kit.head.as_deref()),
                ]
            })
            .collect();
        CollisionCensus { world, players, entities, kits }
    }

    /// A row for each body in the last recorded frame; one row of blanks when nothing is recorded.
    pub fn hitvol_dump(&self, player: impl Fn(SeatNo) -> Option<AvatarState>) -> Vec<HitboxReportRow> {
        let recorded = self.replay.history.latest_poses().map(|(_, poses)| poses).filter(|poses| !poses.is_empty());
        match recorded {
            Some(poses) => poses.iter().map(|pose| self.dump_row(pose, player(pose.client).as_ref())).collect(),
            None => {
                let kit = &self.content.outfit_hitboxes()[0];
                let pose_label = self.body_pose_label();
                vec![HitboxReportRow {
                    shape_data: NO_DATA,
                    torso_model_key: kit.torso_model_key.to_owned(),
                    face_model_key: kit.face_model_key.to_owned(),
                    pose_label,
                    driver: NO_DATA,
                    pen_table_ready: self.content.wall_pierce_table_ready(),
                    bake_failure: self.bodies.bake_failure.clone(),
                    ..HitboxReportRow::default()
                }]
            }
        }
    }

    fn dump_row(&self, pose: &PlayerHitboxPose, ps: Option<&AvatarState>) -> HitboxReportRow {
        let kit = Self::collision_kit(self, pose.client);
        let bone_of = |class: u8| pose.joints.iter().find(|bone| bone.body_part_category == class);
        let (pelvis, head) = (bone_of(PART_PELVIS), bone_of(PART_HEAD));
        let facts = ps.map(|ps| self.controller_facts(pose.client, ps));
        let row = HitboxReportRow {
            client: Option::from(pose.client),
            bones_total: pose.joints.len(),
            shape_data: if pose.joints.is_empty() { "aabb" } else { "bones" },
            torso_model_key: kit.torso_model_key.to_owned(),
            face_model_key: kit.face_model_key.to_owned(),
            pose_label: if ps.is_some() { self.body_pose_label() } else { NO_DATA },
            first_center: pose.joints.first().map(|bone| bone.center),
            pelvis_half: pelvis.map(|bone| bone.half_ext),
            pelvis_center: pelvis.map(|bone| bone.center),
            head_center: head.map(|bone| bone.center),
            driver: facts.as_ref().map_or(NO_DATA, |f| f.stance),
            controller_tags: facts.as_ref().map(|f| f.tags),
            view_pitch: ps.map(|ps| ps.look_angles[0]),
            entity_flags: ps.map(|ps| ps.e_flags),
            move_flags: ps.map(|ps| ps.movement_state_bits),
            lean: ps.map(|ps| ps.tilt_sideways),
            move_type: self.last_anim_movetype(pose.client).map(i64::from),
            pen_table_ready: self.content.wall_pierce_table_ready(),
            bake_failure: self.bodies.bake_failure.clone(),
        };
        row
    }

    /// Which way the controller bends the body, and how many of its tags the body model has.
    fn controller_facts(&self, who: SeatNo, ps: &AvatarState) -> ControllerFacts {
        let tags = self.collision_kit(who).body.as_ref().map_or(0, |corpse| {
            let has_tag = |tag: &&&str| corpse.pose.joint_labels.iter().any(|name| name == **tag);
            xmodel_runtime::SPINE_AIM_BONES.iter().filter(has_tag).count() as i64
        });
        let stance = if player_controller_input(ps).lying { "prone" } else { "standing" };
        ControllerFacts { stance, tags }
    }
}
