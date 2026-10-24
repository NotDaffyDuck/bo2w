//! What the `hitvol` console commands print: a count of what the collision world holds, and one line per player
//! body that shots can hit.

use super::bodies::player_controller_input;
use super::state::SimState;
use super::ClientId;
use crate::bullet_collision::PlayerCollisionPose;
use crate::collision_census::{
    CollisionCensus, ModelCollisionCensus, WorldClipCensus, entity_clip_census, player_clip_census,
};
use playerstate_t6::game::PlayerState;

/// `part_classification` of the bone boxes the dump reports on.
const PART_HEAD: u8 = 2;
const PART_PELVIS: u8 = 5;

/// One player body as shots see it. A client of `None` stands for "no body is recorded yet".
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HitvolDumpRow {
    pub client: Option<ClientId>,
    pub bone_count: u32,
    /// "bones", "aabb" (no bone boxes, the plain player box) or "none".
    pub geom: &'static str,
    pub body_key: String,
    pub head_key: String,
    /// How the skeleton is posed: "bind", or "none" without a body model or a player state.
    pub pose_kind: &'static str,
    /// The first bone box's center, the pelvis box's half size and center, the head box's center.
    pub first_center: Option<[f32; 3]>,
    pub pelvis_half: Option<[f32; 3]>,
    pub pelvis_center: Option<[f32; 3]>,
    pub head_center: Option<[f32; 3]>,
    /// "standing", "prone" or "none".
    pub controller: &'static str,
    pub controller_tags: Option<i64>,
    pub pitch: Option<f32>,
    pub e_flags: Option<u32>,
    pub pm_flags: Option<u32>,
    pub leanf: Option<f32>,
    pub movetype: Option<i64>,
    pub pen_table_loaded: bool,
    pub error: Option<String>,
}

/// What the body's controller (the spine bend and lean) read from the player.
struct ControllerFacts {
    kind: &'static str,
    tags: i64,
}

impl SimState {
    /// Counts of what the collision world, the recorded players, the entity owners and the body kits hold.
    pub fn collision_census(&self) -> CollisionCensus {
        let bsp = self.clip_bsp();
        let t6 = bsp.t6;
        let statics = self.clip_mesh().t6_static;
        let world = WorldClipCensus {
            brushes: self.clip_brushes().len() as u32,
            bsp_nodes: t6.map_or(0, |map| map.nodes.len()) as u32,
            bsp_leaves: t6.map_or(0, |map| map.leafs.len()) as u32,
            leafbrushes: bsp.leafbrushes.len() as u32,
            mesh_tris: t6.map_or(0, |map| map.tri_indices.len()) as u32,
            cmodels: self.clip_cmodels().models.len() as u32,
            static_models: statics.map_or(0, |w| w.models.len()) as u32,
            static_models_with_tris: statics.map_or(0, |w| {
                w.models.iter().filter(|m| m.xmodel.is_some_and(|x| x.surfs.iter().any(|s| !s.tris.is_empty()))).count()
            }) as u32,
            pen_table_loaded: self.content.pen_table_loaded(),
        };
        let recorded = self.replay.history.latest_poses().map_or(&[][..], |(_, poses)| poses);
        let players = player_clip_census(recorded, self.bodies.materialize_error.clone());
        let entities = entity_clip_census(&self.space.caps, crate::bullet_collision::MASK_BULLET_WORLD);
        let kits = self
            .content
            .player_kits()
            .iter()
            .flat_map(|kit| {
                [
                    ModelCollisionCensus::of(&kit.body_key, kit.body.as_deref()),
                    ModelCollisionCensus::of(&kit.head_key, kit.head.as_deref()),
                ]
            })
            .collect();
        CollisionCensus { world, players, entities, kits }
    }

    /// A row for each body in the last recorded frame; one row of blanks when nothing is recorded.
    pub fn hitvol_dump(&self, player: impl Fn(ClientId) -> Option<PlayerState>) -> Vec<HitvolDumpRow> {
        let recorded = self.replay.history.latest_poses().map(|(_, poses)| poses).filter(|poses| !poses.is_empty());
        match recorded {
            Some(poses) => poses.iter().map(|pose| self.dump_row(pose, player(pose.client).as_ref())).collect(),
            None => {
                let kit = &self.content.player_kits()[0];
                vec![HitvolDumpRow {
                    geom: "none",
                    body_key: kit.body_key.clone(),
                    head_key: kit.head_key.clone(),
                    pose_kind: self.player_body_pose_kind(),
                    controller: "none",
                    pen_table_loaded: self.content.pen_table_loaded(),
                    error: self.bodies.materialize_error.clone(),
                    ..HitvolDumpRow::default()
                }]
            }
        }
    }

    fn dump_row(&self, pose: &PlayerCollisionPose, ps: Option<&PlayerState>) -> HitvolDumpRow {
        let kit = self.collision_kit(pose.client);
        let bone_of = |class: u8| pose.bones.iter().find(|bone| bone.part_classification == class);
        let (pelvis, head) = (bone_of(PART_PELVIS), bone_of(PART_HEAD));
        let facts = ps.map(|ps| self.controller_facts(pose.client, ps));
        HitvolDumpRow {
            client: Some(pose.client),
            bone_count: pose.bones.len() as u32,
            geom: if pose.bones.is_empty() { "aabb" } else { "bones" },
            body_key: kit.body_key.clone(),
            head_key: kit.head_key.clone(),
            pose_kind: if ps.is_some() { self.player_body_pose_kind() } else { "none" },
            first_center: pose.bones.first().map(|bone| bone.center),
            pelvis_half: pelvis.map(|bone| bone.half_size),
            pelvis_center: pelvis.map(|bone| bone.center),
            head_center: head.map(|bone| bone.center),
            controller: facts.as_ref().map_or("none", |f| f.kind),
            controller_tags: facts.as_ref().map(|f| f.tags),
            pitch: ps.map(|ps| ps.viewangles[0]),
            e_flags: ps.map(|ps| ps.e_flags),
            pm_flags: ps.map(|ps| ps.pm_flags),
            leanf: ps.map(|ps| ps.leanf),
            movetype: self.last_anim_movetype(pose.client).map(i64::from),
            pen_table_loaded: self.content.pen_table_loaded(),
            error: self.bodies.materialize_error.clone(),
        }
    }

    /// Which way the controller bends the body, and how many of its tags the body model has.
    fn controller_facts(&self, client: ClientId, ps: &PlayerState) -> ControllerFacts {
        let tags = self.collision_kit(client).body.as_ref().map_or(0, |body| {
            let has_tag = |tag: &&&str| body.pose.bone_names.iter().any(|name| name == **tag);
            xmodel_runtime::PLAYER_CONTROLLER_TAGS.iter().filter(has_tag).count() as i64
        });
        ControllerFacts { kind: if player_controller_input(ps).prone { "prone" } else { "standing" }, tags }
    }
}
