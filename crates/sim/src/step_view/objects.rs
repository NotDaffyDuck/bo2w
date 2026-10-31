//! Projectiles in flight and items lying on the ground.

use super::StepWorldView;
use super::rows::{DroppedItemRow, ProjectileRow};
use super::store::{find, find_mut, find_where, present_numbers, remove_row, spawn_row, visit_rows};
use crate::FlyingProjectile;
use crate::match_ids::ScriptedModel;
use crate::floor_guns::LootOnGround;

impl<'frame> StepWorldView<'frame> {
    /// The script models a missile passes through: whatever the map scripts say, plus the vehicle its owner rides.
    pub(crate) fn missile_collision_models(&self, id: crate::ThrownId) -> Vec<ScriptedModel> {
        let mut models: Vec<ScriptedModel> = self
            .ecs
            .get_resource::<crate::gsc_host::Runtime>()
            .map(|runtime| runtime.missile_collision_models(id))
            .unwrap_or_default();
        let owner_of_missile = || find_where::<ProjectileRow>(self.ecs, |p| p.id == id).map(|p| p.owner.0);
        models.extend(crate::zombie_rt::missile_passes(self.ecs, owner_of_missile));
        models
    }

    pub(crate) fn projectile_in_slot(&self, slot: i32) -> Option<FlyingProjectile> {
        find::<ProjectileRow>(self.ecs, slot).copied()
    }

    pub(crate) fn projectile_in_slot_mut(&mut self, slot: i32) -> Option<&mut FlyingProjectile> {
        find_mut::<ProjectileRow>(self.ecs, slot)
    }

    pub(crate) fn add_shell(&mut self, shot: FlyingProjectile) {
        spawn_row::<ProjectileRow>(self.ecs, shot);
    }

    pub(crate) fn drop_projectile_slot(&mut self, slot: i32) -> Option<FlyingProjectile> {
        remove_row::<ProjectileRow>(self.ecs, slot)
    }

    pub(crate) fn visit_projectiles(&self, visit: impl FnMut(&FlyingProjectile)) {
        visit_rows::<ProjectileRow>(self.ecs, visit);
    }

    pub(crate) fn dropped_item_total(&self) -> usize {
        present_numbers::<DroppedItemRow>(self.ecs).len()
    }

    pub(crate) fn ordered_loot_slots(&self) -> Vec<i32> {
        present_numbers::<DroppedItemRow>(self.ecs)
    }

    pub(crate) fn loot_at_slot(&self, slot: i32) -> Option<LootOnGround> {
        find::<DroppedItemRow>(self.ecs, slot).copied()
    }

    pub(crate) fn loot_at_slot_mut(&mut self, slot: i32) -> Option<&mut LootOnGround> {
        find_mut::<DroppedItemRow>(self.ecs, slot)
    }

    pub(crate) fn add_loot_row(&mut self, dropped: LootOnGround) {
        spawn_row::<DroppedItemRow>(self.ecs, dropped);
    }

    pub(crate) fn take_loot_at_slot(&mut self, slot: i32) -> Option<LootOnGround> {
        remove_row::<DroppedItemRow>(self.ecs, slot)
    }
}
