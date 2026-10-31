//! A script model's look for the other side.

use crate::sim_parts::SeatNo;

/// A model change for viewers on the other side (BO2 decides it per viewer): `model` shows to every viewer but the
/// owner whose team is not `team`; with no side (`TEAM_FREE`) it shows to all of them.
#[derive(Clone, Debug, PartialEq)]
pub struct EnemyModel {
    pub id: crate::ScriptedModel,
    pub mesh: String,
    /// Its side as a client team number (BO2's `team_t`).
    pub team: i32,
    /// Its owner's client number.
    pub owner: Option<u32>,
}

impl EnemyModel {
    /// Whether `viewer`, on `viewer_team`, keeps seeing the model's own look (they own it, or it is on their side).
    pub(super) fn shows_own_look_to(&self, viewer: SeatNo, viewer_team: i32) -> bool {
        let owns_it = self.owner == Some(viewer.0);
        let same_side = self.team != entity_t6::team::FREE && self.team == viewer_team;
        owns_it || same_side
    }
}

#[cfg(test)]
mod tests {
    use super::EnemyModel;
    use crate::player_ledger::{SeatTickNotes, TickLedgerNotes};
    use crate::{ServerModelSource, SeatNo, ScriptedModel};
    use entity_t6::team::{ALLIES, AXIS, FREE};

    const VIEWER: SeatNo = SeatNo(3);
    const FLIPPED: u32 = 7 ^ 0x8000_0000;

    /// Four script models, "friendly1".."friendly4", each with an enemy look; a viewer on `viewer_team`.
    fn stage(viewer_team: i32) -> TickLedgerNotes {
        let mut meta = TickLedgerNotes::default();
        meta.clients.push((VIEWER, SeatTickNotes { client_state_team: viewer_team, ..Default::default() }));
        for n in 1..=4u32 {
            meta.entity_dobjs.push((
                ServerModelSource::ScriptModel(ScriptedModel(n)),
                xmodel_runtime::RigPoseRecord::neutral_pose(format!("friendly{n}"), 7, 0),
            ));
        }
        let look = |n: u32, team: i32, owner: u32| EnemyModel {
            id: ScriptedModel(n),
            mesh: format!("enemy{n}"),
            team,
            owner: Some(owner),
        };
        meta.enemy_models = vec![
            look(1, AXIS, 3),   // the viewer's own, though on the other side
            look(2, ALLIES, 5), // the viewer's side
            look(3, AXIS, 5),   // the other side
            look(4, FREE, 5),   // no side, someone else's
        ];
        meta
    }

    fn looks(meta: &TickLedgerNotes) -> Vec<(String, u32)> {
        let first = |state: &xmodel_runtime::RigPoseRecord| {
            (state.composition.models[0].model.clone(), state.composition.revision)
        };
        meta.entity_dobjs.iter().map(|(_, state)| first(state)).collect()
    }

    #[test]
    fn the_other_side_sees_the_enemy_look_and_the_rows_are_used_up() {
        let mut meta = stage(ALLIES);
        meta.swap_enemy_models(VIEWER);
        let expected = [("friendly1", 7), ("friendly2", 7), ("enemy3", FLIPPED), ("enemy4", FLIPPED)];
        let expected: Vec<(String, u32)> = expected.iter().map(|(name, rev)| (name.to_string(), *rev)).collect();
        assert_eq!(looks(&meta), expected);
        assert!(meta.enemy_models.is_empty());
    }

    #[test]
    fn a_player_with_no_team_sees_every_look_but_their_own() {
        let mut meta = stage(FREE);
        meta.swap_enemy_models(VIEWER);
        let names: Vec<String> = looks(&meta).into_iter().map(|(name, _)| name).collect();
        assert_eq!(names, ["friendly1", "enemy2", "enemy3", "enemy4"]);
    }

    #[test]
    fn with_no_rows_nothing_changes() {
        let mut meta = stage(ALLIES);
        meta.enemy_models.clear();
        let before = looks(&meta);
        meta.swap_enemy_models(VIEWER);
        assert_eq!(looks(&meta), before);
    }
}
