//! Where things are: the area grid players and entities are linked into, the entity kernel and its frame phases,
//! the collision owners, the model library, and missiles that are in their last moments.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::state::SimState;
use super::{CONTENTS_BODY, ClientId, Tick};
use crate::area_world::{AreaBox, AreaWorld};
use crate::bullet_collision::EntityCollisionCapabilities;
use crate::equipment::ProjectileState;
use crate::gentity::{EntityAllocError, EntityKernel, EntityRef, EntityRunKind, KERNEL_PHASE_ORDER, KernelPhase};
use crate::identities::ScriptModelId;

/// The slot a player takes in the area grid: its client number.
pub(super) fn area_entity_num(id: ClientId) -> usize {
    let slot = id.0 as usize;
    assert!(slot < clipmap_t6::LINKED_ENTITIES, "player entity index does not fit the 1024-entry collision world");
    slot
}

type ModelMap = BTreeMap<String, Option<Arc<xmodel_runtime::RetainedModelCapability>>>;

impl SimState {
    // ---- the area grid ----------------------------------------------------------------------------------------

    /// Builds an empty area grid over the first collision model of the loaded map, or none when there is no map.
    pub(super) fn reset_area_world(&mut self) {
        let Some(root) = self.content.clip_cmodels().models.first() else {
            self.space.area = None;
            return;
        };
        let bounds = AreaBox { mins: root.mins, maxs: root.maxs };
        assert!(bounds.is_valid(), "area query: world cmodel Bounds are invalid");
        self.space.area = Some(AreaWorld::new(bounds));
    }

    /// Links player `id` at `origin` with its box, grown one unit each way. False when there is no grid.
    pub(crate) fn link_player_area(&mut self, id: ClientId, origin: [f32; 3], mins: [f32; 3], maxs: [f32; 3]) -> bool {
        let Some(area) = self.space.area.as_mut() else {
            return false;
        };
        let slot = area_entity_num(id);
        let mut bounds = AreaBox { mins: origin, maxs: origin };
        for axis in 0..3 {
            bounds.mins[axis] = origin[axis] + mins[axis] - 1.0;
            bounds.maxs[axis] = origin[axis] + maxs[axis] + 1.0;
        }
        assert!(bounds.is_valid(), "player link Bounds are invalid");
        area.link(slot, CONTENTS_BODY, bounds);
        true
    }

    pub(crate) fn translate_player_area(&mut self, id: ClientId, delta: [f32; 3]) -> bool {
        let Some(area) = self.space.area.as_mut() else {
            return false;
        };
        assert!(delta.iter().all(|d| d.is_finite()), "collision link rejected translated player Bounds");
        area.translate(area_entity_num(id), delta)
    }

    pub(crate) fn unlink_player_area(&mut self, id: ClientId) -> bool {
        match self.space.area.as_mut() {
            Some(area) => area.unlink(area_entity_num(id)),
            None => false,
        }
    }

    /// Forgets everything held for a client that left.
    pub(crate) fn forget_client_membership(&mut self, id: ClientId) {
        self.unlink_player_area(id);
        self.roster.members.retain(|member| member.0 != id);
        self.bodies.dobjs.remove(&id.0);
        self.replay.sample.remove(&id);
        self.replay.commands.retain(|key, _| key.0 != id);
        self.roster.walking.remove(&id);
        self.roster.view_change.remove(&id);
        self.roster.movetype.remove(&id);
    }

    /// The linked entities whose box meets `bounds` and whose contents meet `mask`, at most `capacity` of them.
    pub(crate) fn area_entity_candidates(&self, bounds: AreaBox, mask: u32, capacity: usize) -> Vec<u16> {
        let Some(area) = self.space.area.as_ref() else {
            panic!("area query needs an initialized world cmodel Bounds root");
        };
        area.query(bounds, mask, capacity)
    }

    pub(crate) fn player_area_bounds(&self, id: ClientId) -> Option<AreaBox> {
        let slot = usize::try_from(id.0).ok()?;
        self.space.area.as_ref()?.bounds(slot)
    }

    // ---- the entity kernel ------------------------------------------------------------------------------------

    #[must_use]
    pub fn entity_kernel(&self) -> &EntityKernel {
        &self.space.kernel
    }

    pub(crate) fn entity_kernel_mut(&mut self) -> &mut EntityKernel {
        &mut self.space.kernel
    }

    /// Starts a frame of the kernel at `tick` and puts it in its first phase.
    pub(crate) fn begin_entity_frame(&mut self, tick: Tick) {
        self.space.kernel.begin_frame(crate::corpse::level_time_ms(tick));
        self.space.phases.clear();
        self.enter_kernel_phase(KernelPhase::AdvanceTime);
    }

    /// Moves the kernel to `phase`, which must be the next one in the fixed order.
    pub(crate) fn enter_kernel_phase(&mut self, phase: KernelPhase) {
        let due = KERNEL_PHASE_ORDER.get(self.space.phases.len()).copied();
        assert_eq!(due, Some(phase), "sim kernel phase order diverged");
        self.space.phases.push(phase);
    }

    pub(crate) fn allocate_dynamic_entity(&mut self, kind: EntityRunKind) -> Result<EntityRef, EntityAllocError> {
        let entity = self.space.kernel.allocate(kind)?;
        self.space.kernel.set_linked(entity, true).expect("newly allocated dynamic entity must resolve");
        Ok(entity)
    }

    pub(crate) fn free_dynamic_entity_number(&mut self, number: i32) {
        let entity = self
            .space
            .kernel
            .current_ref(number)
            .expect("dynamic store referenced a free or invalid entity slot");
        self.space
            .kernel
            .free(entity)
            .expect("dynamic entity generation changed while its typed store was alive");
    }

    // ---- missiles that just ended -----------------------------------------------------------------------------

    /// Lets go of the missiles whose last-moment events have run their time out.
    pub(crate) fn expire_dying_missiles(&mut self, tick: Tick) {
        debug_assert_eq!(self.space.kernel.level_time_ms(), crate::corpse::level_time_ms(tick));
        let gone: Vec<i32> = self
            .space
            .kernel
            .expire_transient_events()
            .into_iter()
            .map(EntityRef::number)
            .collect();
        self.space.dying_missiles.retain(|state| !gone.contains(&state.number));
    }

    /// Keeps a missile that hit something visible to clients for a moment more, so they see how it ended.
    pub(crate) fn note_dying_missile(&mut self, tick: Tick, projectile: ProjectileState) {
        if projectile.entnum == playerstate_t6::game::ENTITYNUM_NONE {
            return;
        }
        let entity = self
            .space
            .kernel
            .current_ref(projectile.entnum)
            .expect("terminal projectile lost its dynamic entity slot");
        self.space
            .kernel
            .mark_transient_event(entity, crate::corpse::level_time_ms(tick))
            .expect("terminal projectile generation changed before event retention");
        let state = crate::gentity::init_missile_state(
            projectile.entnum,
            projectile.weapon,
            projectile.pos,
            projectile.apos,
            projectile.launch_time,
        );
        self.space.dying_missiles.push(state);
    }

    // ---- collision owners -------------------------------------------------------------------------------------

    /// Replaces the collision owners (kept sorted by owner) and hashes the content again.
    pub fn install_entity_collision_capabilities(&mut self, mut owners: Vec<EntityCollisionCapabilities>) {
        owners.sort_by_key(|row| row.owner);
        self.space.caps = owners;
        self.refresh_digest();
    }

    #[must_use]
    pub fn entity_collision_capabilities(&self) -> &[EntityCollisionCapabilities] {
        &self.space.caps
    }

    pub(crate) fn entity_collision_capabilities_mut(&mut self) -> &mut [EntityCollisionCapabilities] {
        &mut self.space.caps
    }

    pub(crate) fn collision_owner_mut(&mut self, id: ScriptModelId) -> Option<&mut EntityCollisionCapabilities> {
        let owner = crate::AuthorityModelOwner::ScriptModel(id);
        let at = self.space.caps.binary_search_by_key(&owner, |row| row.owner).ok()?;
        self.space.caps.get_mut(at)
    }

    pub(crate) fn insert_collision_owner(&mut self, row: EntityCollisionCapabilities) {
        match self.space.caps.binary_search_by_key(&row.owner, |have| have.owner) {
            Ok(at) => self.space.caps[at] = row,
            Err(at) => self.space.caps.insert(at, row),
        }
    }

    pub(crate) fn remove_collision_owner(&mut self, id: ScriptModelId) {
        let owner = crate::AuthorityModelOwner::ScriptModel(id);
        self.space.caps.retain(|row| row.owner != owner);
    }

    // ---- the model library ------------------------------------------------------------------------------------

    pub fn install_model_library(&mut self, models: ModelMap) {
        self.space.models = Arc::new(models);
    }

    /// Adds models that are not in the library yet (a map's own), leaving the ones it has.
    pub(crate) fn add_model_capabilities(&mut self, extra: Vec<(String, Arc<xmodel_runtime::RetainedModelCapability>)>) {
        let mut models: ModelMap = (*self.space.models).clone();
        for (name, capability) in extra {
            models.entry(name).or_insert(Some(capability));
        }
        self.space.models = Arc::new(models);
    }

    /// `None` when the library has no entry by that name, `Some(None)` when it lists the name without a model.
    pub(crate) fn model_capability(&self, name: &str) -> Option<Option<Arc<xmodel_runtime::RetainedModelCapability>>> {
        self.space.models.get(name).cloned()
    }
}
