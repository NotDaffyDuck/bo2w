//! `hitvol`: what the authority holds for a bullet to hit.

use bevy::{ecs::message::MessageReader, ecs::system::Res};
use net::HostSimulationWorld;

use super::echo::PromptReplyWriter;
use crate::TypedOrder;
use sim::SimWorld;

pub(super) const HELP_BANNER: &[(&str, &str)] = &[(
    "hitvol",
    "hitvol — what the authority holds for a bullet to clip against: world tables, live player volumes, script-model clips, kit models",
)];

/// The report as lines of text: the world's tables, the players' volumes, the
/// entities' clips, then each kit model and each client's own volume.
pub(super) fn hitvol_report(world: &SimWorld) -> Vec<String> {
    let census = world.collision_census();
    let (map, players, entities) = (&census.world, &census.players, &census.entities);
    let mut lines: Vec<String> = vec![
        format!(
            "hitvol world: brushes {} leaves {} leafbrushes {} meshtris {} cmodels {} smodels {} (with tris {}) pen_table {}",
            map.brushes,
            map.bsp_leaves,
            map.brush_leaf_indices,
            map.mesh_tris,
            map.cmodels,
            map.prop_set,
            map.static_models_with_tris,
            map.wall_pierce_table_ready
        ),
        format!(
            "hitvol players: poses {} bones {} aabb-only {} bone-count {}..{} with-head-bone {}",
            players.poses,
            players.with_bones,
            players.aabb_only,
            players.min_bones,
            players.max_bones,
            players.with_head_bone
        ),
        format!(
            "hitvol entities: rows {} colltris {} boxes {} brush {} authored-no-collision {} not-bullet-solid {} no-dobj {} no-capability {} materialize-failed {} linked-brushes {}",
            entities.lines,
            entities.colltris,
            entities.boxes_only,
            entities.brush_only,
            entities.no_collision_authored,
            entities.not_bullet_solid,
            entities.no_dobj,
            entities.no_capability,
            entities.materialize_failed,
            entities.attached_solids
        ),
    ];
    if !entities.no_clip_sample.is_empty() {
        let unclipped = entities.no_clip_sample.join(", ");
        lines.push(format!("hitvol entities with no model clip: {unclipped}"));
    }
    if let Some(error) = &players.bake_failure {
        lines.push(format!("hitvol players: materialize error {error}"));
    }
    lines.extend(census.kits.iter().map(|kit| {
        format!(
            "hitvol kit `{}`: {} bones {} boxes {} collsurfs {} colltris {} lod {}",
            kit.key,
            kit.clip(),
            kit.bones,
            kit.bone_boxes,
            kit.collision_faces,
            kit.coll_tris,
            kit.collision_detail_tier
        )
    }));
    lines.extend(world.hitvol_dump().into_iter().map(|row| {
        format!(
            "hitvol client {:?}: geom {} bones {} pose {} body `{}` head `{}` controller {}",
            row.client.map(|client| client.0),
            row.shape_data,
            row.bones_total,
            row.pose_label,
            row.torso_model_key,
            row.face_model_key,
            row.driver
        )
    }));
    lines
}

pub(crate) fn route_hitvol_commands(
    mut commands: MessageReader<TypedOrder>,
    mut echo: PromptReplyWriter,
    authority: Option<Res<HostSimulationWorld>>,
) {
    for _ in commands.read().filter(|command| command.name == "hitvol") {
        match authority.as_deref() {
            Some(authority) => {
                for line in hitvol_report(&authority.0) {
                    echo.write(line);
                }
            }
            None => echo.write("hitvol: no authority world on this client"),
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::app::App;
    use crate::tool_cmds::hitvol::*;
    use crate::PromptOutputRow;
    use crate::PromptPanelOptions;
    use crate::PromptPanelStatus;
    use bevy::app::Update;

    #[test]
    fn without_an_authority_world_it_says_so() {
        let mut app = App::default();
        app.add_message::<TypedOrder>()
            .init_resource::<PromptPanelStatus>()
            .init_resource::<PromptPanelOptions>()
            .init_resource::<PromptOutputRow>()
            .add_systems(Update, route_hitvol_commands);
        for command in TypedOrder::parse_script("hitvol; bot add") {
            app.world_mut().write_message(command);
        }
        app.update();
        let expected = "hitvol: no authority world on this client";
        assert_eq!(app.world().resource::<PromptOutputRow>().0, expected);
        assert_eq!(app.world().resource::<PromptPanelStatus>().lines.len(), 1);
    }
}
