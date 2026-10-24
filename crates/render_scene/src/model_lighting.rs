//! Asking for a model's lighting, and keeping what the lighting pass answered.

use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;

/// Who a lighting request is for.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ModelLightingOwner {
    Eye,
    RemoteClient(u16),
    Corpse(Entity),
    ScriptModel(Entity),
    Missile(u32),
    PredictedMissile { owner: u32, weapon: u32 },
    Item(u32),
    FxModel(u16),
    Glass(u16),
    DynEnt(Entity),
}

/// One owner wants the lighting at `origin`; `lookup_fallback` is the light
/// slot to use if the lookup finds none.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelLightingRequest {
    pub owner: ModelLightingOwner,
    pub origin: [f32; 3],
    pub lookup_fallback: u8,
}

/// The requests made since the lighting pass last ran.
#[derive(Resource, Debug, Default)]
pub struct ModelLightingRequests {
    pending: Vec<ModelLightingRequest>,
}

impl ModelLightingRequests {
    /// Queues a request and hands it back, so the caller can keep it with its draw.
    pub fn request(&mut self, request: ModelLightingRequest) -> ModelLightingRequest {
        self.pending.push(request);
        request
    }

    pub fn clear(&mut self) {
        self.pending.clear();
    }

    pub fn take_pending(&mut self) -> Vec<ModelLightingRequest> {
        std::mem::take(&mut self.pending)
    }
}

/// What the lighting pass gave an owner: a seat in the atlas, or nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolvedModelLighting {
    Seated {
        handle: u32,
        scene_light_index: u8,
        reflection_probe_index: u8,
        packed_lighting: Option<[u8; 4]>,
    },
    Failed,
}

/// The answers, by owner. The first answer for an owner stands until it is
/// removed or the table is cleared.
#[derive(Resource, Debug, Default)]
pub struct ResolvedModelLightingTable {
    by_owner: HashMap<ModelLightingOwner, ResolvedModelLighting>,
}

impl ResolvedModelLightingTable {
    pub fn get(&self, owner: ModelLightingOwner) -> Option<ResolvedModelLighting> {
        self.by_owner.get(&owner).copied()
    }

    pub fn clear(&mut self) {
        self.by_owner.clear();
    }

    pub fn insert_if_absent(&mut self, owner: ModelLightingOwner, value: ResolvedModelLighting) {
        self.by_owner.entry(owner).or_insert(value);
    }

    /// Like [`Self::insert_if_absent`], but only works out the answer when it is needed.
    pub fn get_or_insert_with(
        &mut self,
        owner: ModelLightingOwner,
        compute: impl FnOnce() -> ResolvedModelLighting,
    ) {
        self.by_owner.entry(owner).or_insert_with(compute);
    }

    pub fn remove(&mut self, owner: ModelLightingOwner) {
        self.by_owner.remove(&owner);
    }

    /// Forgets every glass owner not in `live`; other owners are left alone.
    pub fn retain_glass(&mut self, live: &HashSet<ModelLightingOwner>) {
        self.by_owner
            .retain(|owner, _| !matches!(owner, ModelLightingOwner::Glass(_)) || live.contains(owner));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEATED: ResolvedModelLighting = ResolvedModelLighting::Seated {
        handle: 7,
        scene_light_index: 1,
        reflection_probe_index: 2,
        packed_lighting: None,
    };

    fn request(owner: ModelLightingOwner) -> ModelLightingRequest {
        ModelLightingRequest { owner, origin: [1.0, 2.0, 3.0], lookup_fallback: 4 }
    }

    #[test]
    fn requests_come_back_in_order_and_only_once() {
        let mut requests = ModelLightingRequests::default();
        let first = requests.request(request(ModelLightingOwner::Item(1)));
        requests.request(request(ModelLightingOwner::Item(2)));
        assert_eq!(first.owner, ModelLightingOwner::Item(1));
        let taken = requests.take_pending();
        assert_eq!(taken.len(), 2);
        assert_eq!(taken[1].owner, ModelLightingOwner::Item(2));
        assert!(requests.take_pending().is_empty());
    }

    #[test]
    fn clearing_drops_the_queued_requests() {
        let mut requests = ModelLightingRequests::default();
        requests.request(request(ModelLightingOwner::Eye));
        requests.clear();
        assert!(requests.take_pending().is_empty());
    }

    #[test]
    fn the_first_answer_for_an_owner_stands() {
        let mut table = ResolvedModelLightingTable::default();
        let owner = ModelLightingOwner::Missile(3);
        table.insert_if_absent(owner, SEATED);
        table.insert_if_absent(owner, ResolvedModelLighting::Failed);
        table.get_or_insert_with(owner, || ResolvedModelLighting::Failed);
        assert_eq!(table.get(owner), Some(SEATED));
        table.remove(owner);
        assert_eq!(table.get(owner), None);
    }

    #[test]
    fn get_or_insert_with_computes_only_for_a_new_owner() {
        let mut table = ResolvedModelLightingTable::default();
        let owner = ModelLightingOwner::FxModel(9);
        table.get_or_insert_with(owner, || SEATED);
        table.get_or_insert_with(owner, || unreachable!("already answered"));
        assert_eq!(table.get(owner), Some(SEATED));
    }

    #[test]
    fn glass_not_in_the_live_set_is_forgotten_but_other_owners_stay() {
        let mut table = ResolvedModelLightingTable::default();
        for owner in [
            ModelLightingOwner::Glass(1),
            ModelLightingOwner::Glass(2),
            ModelLightingOwner::Item(1),
        ] {
            table.insert_if_absent(owner, SEATED);
        }
        let live: HashSet<_> = [ModelLightingOwner::Glass(2)].into_iter().collect();
        table.retain_glass(&live);
        assert_eq!(table.get(ModelLightingOwner::Glass(1)), None);
        assert_eq!(table.get(ModelLightingOwner::Glass(2)), Some(SEATED));
        assert_eq!(table.get(ModelLightingOwner::Item(1)), Some(SEATED));
    }

    #[test]
    fn clearing_the_table_forgets_everything() {
        let mut table = ResolvedModelLightingTable::default();
        table.insert_if_absent(ModelLightingOwner::Eye, SEATED);
        table.clear();
        assert_eq!(table.get(ModelLightingOwner::Eye), None);
    }
}
