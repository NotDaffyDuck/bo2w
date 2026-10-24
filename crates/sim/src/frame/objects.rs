//! Projectiles in flight and items lying on the ground.

use super::FrameWorld;
use super::rows::{DroppedItemRow, ProjectileRow};
use super::store::{find, find_mut, find_where, present_numbers, remove_row, spawn_row, visit_rows};
use crate::ProjectileState;
use crate::identities::ScriptModelId;
use crate::item::DroppedItem;

impl FrameWorld<'_> {
    /// The script models a missile passes through: whatever the map scripts say, plus the vehicle its owner rides.
    pub(crate) fn missile_collision_models(&self, id: crate::ProjectileId) -> Vec<ScriptModelId> {
        let mut models: Vec<ScriptModelId> = self
            .ecs
            .get_resource::<crate::script::Runtime>()
            .map(|runtime| runtime.missile_collision_models(id))
            .unwrap_or_default();
        let owner_of_missile = || find_where::<ProjectileRow>(self.ecs, |p| p.id == id).map(|p| p.owner.0);
        models.extend(crate::t6::missile_passes(self.ecs, owner_of_missile));
        models
    }

    pub(crate) fn projectile_by_number(&self, entnum: i32) -> Option<ProjectileState> {
        find::<ProjectileRow>(self.ecs, entnum).copied()
    }

    pub(crate) fn projectile_mut_by_number(&mut self, entnum: i32) -> Option<&mut ProjectileState> {
        find_mut::<ProjectileRow>(self.ecs, entnum)
    }

    pub(crate) fn push_projectile(&mut self, projectile: ProjectileState) {
        spawn_row::<ProjectileRow>(self.ecs, projectile);
    }

    pub(crate) fn remove_projectile_by_number(&mut self, entnum: i32) -> Option<ProjectileState> {
        remove_row::<ProjectileRow>(self.ecs, entnum)
    }

    pub(crate) fn visit_projectiles(&self, visit: impl FnMut(&ProjectileState)) {
        visit_rows::<ProjectileRow>(self.ecs, visit);
    }

    pub(crate) fn dropped_item_count(&self) -> usize {
        present_numbers::<DroppedItemRow>(self.ecs).len()
    }

    pub(crate) fn dropped_item_numbers_sorted(&self) -> Vec<i32> {
        present_numbers::<DroppedItemRow>(self.ecs)
    }

    pub(crate) fn dropped_item_by_number(&self, number: i32) -> Option<DroppedItem> {
        find::<DroppedItemRow>(self.ecs, number).copied()
    }

    pub(crate) fn dropped_item_mut_by_number(&mut self, number: i32) -> Option<&mut DroppedItem> {
        find_mut::<DroppedItemRow>(self.ecs, number)
    }

    pub(crate) fn push_dropped_item(&mut self, item: DroppedItem) {
        spawn_row::<DroppedItemRow>(self.ecs, item);
    }

    pub(crate) fn remove_dropped_item_by_number(&mut self, number: i32) -> Option<DroppedItem> {
        remove_row::<DroppedItemRow>(self.ecs, number)
    }
}
