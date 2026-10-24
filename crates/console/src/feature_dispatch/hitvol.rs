//! `hitvol`: what the authority holds for a bullet to hit.

use bevy::prelude::*;
use net::AuthorityWorld;
use sim::SimWorld;

use super::echo::ConsoleEcho;
use crate::ConsoleCommand;

pub(super) const USAGE: &[(&str, &str)] = &[(
    "hitvol",
    "hitvol — what the authority holds for a bullet to clip against: world tables, live player volumes, script-model clips, kit models",
)];

/// The report as lines of text: the world's tables, the players' volumes, the
/// entities' clips, then each kit model and each client's own volume.
pub(super) fn hitvol_report(world: &SimWorld) -> Vec<String> {
    let census = world.collision_census();
    let (map, players, entities) = (&census.world, &census.players, &census.entities);
    let mut lines = vec![
        format!(
            "hitvol world: brushes {} leaves {} leafbrushes {} meshtris {} cmodels {} smodels {} (with tris {}) pen_table {}",
            map.brushes,
            map.bsp_leaves,
            map.leafbrushes,
            map.mesh_tris,
            map.cmodels,
            map.static_models,
            map.static_models_with_tris,
            map.pen_table_loaded
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
            entities.rows,
            entities.colltris,
            entities.boxes_only,
            entities.brush_only,
            entities.no_collision_authored,
            entities.not_bullet_solid,
            entities.no_dobj,
            entities.no_capability,
            entities.materialize_failed,
            entities.linked_brushes
        ),
    ];
    if !entities.no_clip_sample.is_empty() {
        lines.push(format!(
            "hitvol entities with no model clip: {}",
            entities.no_clip_sample.join(", ")
        ));
    }
    if let Some(error) = &players.materialize_error {
        lines.push(format!("hitvol players: materialize error {error}"));
    }
    lines.extend(census.kits.iter().map(|kit| {
        format!(
            "hitvol kit `{}`: {} bones {} boxes {} collsurfs {} colltris {} lod {}",
            kit.key,
            kit.clip(),
            kit.bones,
            kit.bone_boxes,
            kit.coll_surfs,
            kit.coll_tris,
            kit.coll_lod
        )
    }));
    lines.extend(world.hitvol_dump().into_iter().map(|row| {
        format!(
            "hitvol client {:?}: geom {} bones {} pose {} body `{}` head `{}` controller {}",
            row.client.map(|client| client.0),
            row.geom,
            row.bone_count,
            row.pose_kind,
            row.body_key,
            row.head_key,
            row.controller
        )
    }));
    lines
}

pub(crate) fn route_hitvol_commands(
    mut commands: MessageReader<ConsoleCommand>,
    mut echo: ConsoleEcho,
    authority: Option<Res<AuthorityWorld>>,
) {
    for _ in commands.read().filter(|command| command.name == "hitvol") {
        match authority.as_deref() {
            Some(authority) => hitvol_report(&authority.0)
                .into_iter()
                .for_each(|line| echo.write(line)),
            None => echo.write("hitvol: no authority world on this client"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ConsoleLine, ConsoleSettings, ConsoleState};

    #[test]
    fn without_an_authority_world_it_says_so() {
        let mut app = App::new();
        app.add_message::<ConsoleCommand>()
            .init_resource::<ConsoleState>()
            .init_resource::<ConsoleSettings>()
            .init_resource::<ConsoleLine>()
            .add_systems(Update, route_hitvol_commands);
        for command in ConsoleCommand::parse_script("hitvol; bot add") {
            app.world_mut().write_message(command);
        }
        app.update();
        assert_eq!(
            app.world().resource::<ConsoleLine>().0,
            "hitvol: no authority world on this client"
        );
        assert_eq!(app.world().resource::<ConsoleState>().log.len(), 1);
    }
}
