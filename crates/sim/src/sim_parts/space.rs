//! Where things are: the area grid players and entities are linked into, the entity kernel and its frame phases,
//! the collision owners, the model library, and missiles that are in their last moments.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::{SOLID_PLAYER_BODY, SeatNo, StepNo};
use crate::sector_links::{AreaBox, AreaWorld};
use crate::shot_trace::EntityHitAbilities;
use crate::gear::FlyingProjectile;
use crate::ent_slots::{EntitySlotShortage, EntitySlotArena, EntHandle, EntityBehaviorClass, PASS_STAGE_SEQUENCE, EntityPassStage};
use crate::match_ids::ScriptedModel;

/// The slot a player takes in the area grid: its client number.
pub(super) fn area_entity_num(id: SeatNo) -> usize {
    let slot = id.0 as usize;
    assert!(slot < clipmap_t6::LINKED_ENTITIES, "player entity index does not fit the 1024-entry collision world");
    slot
}

impl super::state::MatchSimulation {
    // ---- the area grid ----------------------------------------------------------------------------------------

    /// Builds an empty area grid over the first collision model of the loaded map, or none when there is no map.
    pub(super) fn reset_area_world(&mut self) {
        let Some(root) = self.content.collision_submodel_set().models.first() else {
            self.space.area = None;
            return;
        };
        let bounds = AreaBox { mins: root.lo, maxs: root.hi };
        assert!(bounds.is_valid(), "area query: world cmodel Bounds are invalid");
        self.space.area = Some(AreaWorld::new(bounds));
    }

    /// Links player `id` at `origin` with its box, grown one unit each way. False when there is no grid.
    pub(crate) fn link_player_area(&mut self, id: SeatNo, spot: [f32; 3], lo: [f32; 3], hi: [f32; 3]) -> bool {
        let Some(area) = self.space.area.as_mut() else {
            return false;
        };
        let slot = area_entity_num(id);
        let bounds = AreaBox {
            mins: core::array::from_fn(|k| spot[k] + lo[k] - 1.0),
            maxs: core::array::from_fn(|k| spot[k] + hi[k] + 1.0),
        };
        assert!(bounds.is_valid(), "player link Bounds are invalid");
        area.link(slot, SOLID_PLAYER_BODY, bounds);
        true
    }

    pub(crate) fn translate_player_area(
        &mut self,
        client: SeatNo,
        shift: [f32; 3],
    ) -> bool {
        let Some(area) = self.space.area.as_mut() else {
            return false;
        };
        assert!(shift.iter().all(|d| d.is_finite()), "collision link rejected translated player Bounds");
        area.translate(area_entity_num(client), shift)
    }

    pub(crate) fn release_seat_area(
        &mut self,
        client: SeatNo,
    ) -> bool {
        self.space.area.as_mut().is_some_and(|area| area.unlink(area_entity_num(client)))
    }

    /// Forgets everything held for a client that left.
    pub(crate) fn drop_seat_links(&mut self, leaver: SeatNo) {
        Self::release_seat_area(self, leaver);
        self.roster.members.retain(|member| member.0 != leaver);
        self.bodies.skeletons.remove(&leaver.0);
        self.replay.sample.remove(&leaver);
        self.replay.commands.retain(|key, _| key.0 != leaver);
        self.roster.walking.remove(&leaver);
        self.roster.view_change.remove(&leaver);
        self.roster.movetype.remove(&leaver);
    }

    /// The linked entities whose box meets `bounds` and whose contents meet `mask`, at most `room` of them.
    pub(crate) fn area_entity_candidates(&self, bounds: AreaBox, mask: u32, room: usize) -> Vec<u16> {
        let Some(area) = self.space.area.as_ref() else {
            panic!("area query needs an initialized world cmodel Bounds root");
        };
        area.query(bounds, mask, room)
    }

    pub(crate) fn seat_box(&self, id: SeatNo) -> Option<AreaBox> {
        let slot = usize::try_from(id.0).ok()?;
        self.space.area.as_ref()?.bounds(slot)
    }

    // ---- the entity kernel ------------------------------------------------------------------------------------

    #[must_use]
    pub fn entity_kernel(&self) -> &EntitySlotArena {
        &self.space.kernel
    }

    pub(crate) fn entity_kernel_mut(&mut self) -> &mut EntitySlotArena {
        &mut self.space.kernel
    }

    /// Starts a frame of the kernel at `tick` and puts it in its first phase.
    pub(crate) fn begin_entity_frame(
        &mut self,
        tick: StepNo,
    ) {
        self.space.kernel.open_tick(crate::bodies::level_time_ms(tick));
        self.space.phases.clear();
        self.begin_pass_stage(EntityPassStage::TimerStepResult);
    }

    /// Moves the kernel to `phase`, which must be the next one in the fixed order.
    pub(crate) fn begin_pass_stage(&mut self, stage: EntityPassStage) {
        let due = PASS_STAGE_SEQUENCE.get(self.space.phases.len()).copied();
        assert_eq!(due, Some(stage), "stages ran out of their planned order");
        self.space.phases.push(stage);
    }

    pub(crate) fn allocate_dynamic_entity(&mut self, kind: EntityBehaviorClass) -> Result<EntHandle, EntitySlotShortage> {
        let thing = self.space.kernel.claim_slot(kind)?;
        self.space.kernel.mark_linked(thing, true).expect("newly allocated dynamic entity must resolve");
        Ok(thing)
    }

    pub(crate) fn free_dynamic_entity_number(
        &mut self,
        slot: i32,
    ) {
        let kernel = &mut self.space.kernel;
        let Ok(entity) = kernel.ref_for(slot) else {
            panic!("dynamic store pointed at an unused or out-of-range entity slot {slot}");
        };
        if kernel.release_slot(entity).is_err() {
            panic!("entity slot {slot} changed generation while its typed store still held it");
        }
    }

    // ---- missiles that just ended -----------------------------------------------------------------------------

    /// Lets go of the missiles whose last-moment events have run their time out.
    pub(crate) fn expire_dying_missiles(
        &mut self,
        step: StepNo,
    ) {
        debug_assert_eq!(self.space.kernel.level_time_ms(), crate::bodies::level_time_ms(step));
        let expired = self.space.kernel.drop_old_events();
        let gone: Vec<i32> = expired.into_iter().map(|entity| entity.slot()).collect();
        self.space.dying_missiles.retain(|state| !gone.contains(&state.slot_index));
    }

    /// Keeps a missile that hit something visible to clients for a moment more, so they see how it ended.
    pub(crate) fn mark_shell_dying(
        &mut self,
        step: StepNo,
        missile: FlyingProjectile,
    ) {
        let FlyingProjectile { ent_no: entnum, gun: weapon, at: pos, turn_path: apos, spawn_moment, .. } = missile;
        if entnum == playerstate_t6::game::NO_ENTITY_SLOT {
            return;
        }
        let kernel = &mut self.space.kernel;
        let Ok(entity) = kernel.ref_for(entnum) else {
            panic!("the finished missile no longer owns entity slot {entnum}");
        };
        let retained = kernel.mark_transient_event(entity, crate::bodies::level_time_ms(step));
        assert!(retained.is_ok(), "missile entity {entnum} changed generation before its end event was kept");
        let state = crate::ent_slots::shell_state_at(entnum, weapon, pos, apos, spawn_moment);
        self.space.dying_missiles.push(state);
    }

    // ---- collision owners -------------------------------------------------------------------------------------

    /// Replaces the collision owners (kept sorted by owner) and hashes the content again.
    pub fn install_entity_collision_capabilities(&mut self, mut owners: Vec<EntityHitAbilities>) {
        owners.sort_by_key(|row| row.owner);
        self.space.caps = owners;
        self.refresh_digest();
    }

    #[must_use]
    pub fn entity_hit_support(
        &self,
    ) -> &[EntityHitAbilities] {
        &self.space.caps
    }

    pub(crate) fn entity_collision_capabilities_mut(&mut self) -> &mut [EntityHitAbilities] {
        &mut self.space.caps
    }

    pub(crate) fn collision_owner_mut(&mut self, id: ScriptedModel) -> Option<&mut EntityHitAbilities> {
        let owner = crate::ServerModelSource::ScriptModel(id);
        let at = self.space.caps.binary_search_by_key(&owner, |row| row.owner).ok()?;
        self.space.caps.get_mut(at)
    }

    pub(crate) fn insert_collision_owner(&mut self, row: EntityHitAbilities) {
        match self.space.caps.binary_search_by_key(&row.owner, |have| have.owner) {
            Ok(at) => self.space.caps[at] = row,
            Err(at) => self.space.caps.insert(at, row),
        }
    }

    pub(crate) fn remove_collision_owner(&mut self, id: ScriptedModel) {
        let owner = crate::ServerModelSource::ScriptModel(id);
        self.space.caps.retain(|row| row.owner != owner);
    }

    // ---- the model library ------------------------------------------------------------------------------------

    pub fn install_model_library(&mut self, models: BTreeMap<String, Option<Arc<xmodel_runtime::KeptModelAbility>>>) {
        self.space.models = Arc::new(models);
    }

    /// Adds models that are not in the library yet (a map's own), leaving the ones it has.
    pub(crate) fn add_model_capabilities(&mut self, extra: Vec<(String, Arc<xmodel_runtime::KeptModelAbility>)>) {
        let mut models: BTreeMap<String, Option<Arc<xmodel_runtime::KeptModelAbility>>> = (*self.space.models).clone();
        for (name, capability) in extra {
            models.entry(name).or_insert(Some(capability));
        }
        self.space.models = Arc::new(models);
    }

    /// `None` when the library has no entry by that name, `Some(None)` when it lists the name without a model.
    pub(crate) fn model_capability(&self, name: &str) -> Option<Option<Arc<xmodel_runtime::KeptModelAbility>>> {
        self.space.models.get(name).cloned()
    }
}
