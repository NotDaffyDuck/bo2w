//! Asking for a model's lighting, and keeping what the lighting pass answered.

use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;

/// Who a lighting request is for.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ModelIllumOwner {
    Eye,
    OtherPlayerSlot(u16),
    Cadaver(Entity),
    ScriptModel(bevy::ecs::entity::Entity),
    Shell(u32),
    PredictedShell { shooter: u32, gun: u32 },
    Item(u32),
    EffectModel(u16),
    Pane(u16),
    DynamicProp(Entity),
}

/// One owner wants the lighting at `origin`; `lookup_backup` is the light
/// slot to use if the lookup finds none.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelIllumRequest {
    pub owner: ModelIllumOwner,
    pub origin: [f32; 3],
    pub lookup_backup: u8,
}

/// The requests made since the lighting pass last ran.
#[derive(Resource, Debug, Default)]
pub struct ModelIllumRequests {
    waiting: Vec<ModelIllumRequest>,
}

impl ModelIllumRequests {
    /// Queues an ask and hands it back, so the caller can keep it with its draw.
    pub fn queue(&mut self, ask: ModelIllumRequest) -> ModelIllumRequest {
        self.waiting.push(ask);
        ask
    }

    pub fn clear(&mut self) {
        self.waiting.truncate(0);
    }

    pub fn take_queued(&mut self) -> Vec<ModelIllumRequest> {
        std::mem::take(&mut self.waiting)
    }
}

/// What the lighting pass gave an owner: a seat in the atlas, or nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolvedModelIllum {
    Seated {
        handle: u32,
        stage_light_index: u8,
        env_probe_index: u8,
        compact_illum: Option<[u8; 4]>,
    },
    Failed,
}

/// The answers, by owner. The first answer for an owner stands until it is
/// forgotten or the table is cleared.
#[derive(Resource, Debug, Default)]
pub struct ResolvedModelIllumTable {
    answers: HashMap<ModelIllumOwner, ResolvedModelIllum>,
}

impl ResolvedModelIllumTable {
    pub fn answer_for(&self, whose: ModelIllumOwner) -> Option<ResolvedModelIllum> {
        self.answers.get(&whose).copied()
    }

    pub fn clear(&mut self) {
        self.answers = HashMap::default();
    }

    pub fn keep_first(&mut self, whose: ModelIllumOwner, answer: ResolvedModelIllum) {
        self.keep_first_with(whose, || answer);
    }

    /// Like [`Self::keep_first`], but only works out the answer when it is needed.
    pub fn keep_first_with(&mut self, whose: ModelIllumOwner, work_out: impl FnOnce() -> ResolvedModelIllum) {
        if !self.answers.contains_key(&whose) {
            self.answers.insert(whose, work_out());
        }
    }

    pub fn forget(&mut self, whose: ModelIllumOwner) {
        self.answers.remove(&whose);
    }

    /// Forgets every glass owner not in `live`; other owners are left alone.
    pub fn retain_glass(&mut self, live: &HashSet<ModelIllumOwner>) {
        let is_glass = |whose: &ModelIllumOwner| matches!(whose, ModelIllumOwner::Pane(_));
        self.answers.retain(|whose, _| live.contains(whose) || !is_glass(whose));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEATED: ResolvedModelIllum = ResolvedModelIllum::Seated {
        handle: 7,
        stage_light_index: 1,
        env_probe_index: 2,
        compact_illum: None,
    };

    fn ask_for(whose: ModelIllumOwner) -> ModelIllumRequest {
        let origin = Vec3::new(1.0, 2.0, 3.0).to_array();
        ModelIllumRequest { owner: whose, origin, lookup_backup: 4 }
    }

    #[test]
    fn requests_come_back_in_order_and_only_once() {
        let mut queue = ModelIllumRequests::default();
        let first = queue.queue(ask_for(ModelIllumOwner::Item(1)));
        queue.queue(ask_for(ModelIllumOwner::Item(2)));
        assert_eq!(first.owner, ModelIllumOwner::Item(1));
        let taken = queue.take_queued();
        assert_eq!(taken.len(), 2);
        assert_eq!(taken[1].owner, ModelIllumOwner::Item(2));
        assert!(queue.take_queued().is_empty());
    }

    #[test]
    fn clearing_drops_the_queued_requests() {
        let mut queue = ModelIllumRequests::default();
        queue.queue(ask_for(ModelIllumOwner::Eye));
        queue.clear();
        assert!(queue.take_queued().is_empty());
    }

    #[test]
    fn the_first_answer_for_an_owner_stands() {
        let mut table = ResolvedModelIllumTable::default();
        let whose = ModelIllumOwner::Shell(3);
        table.keep_first(whose, SEATED);
        table.keep_first(whose, ResolvedModelIllum::Failed);
        table.keep_first_with(whose, || ResolvedModelIllum::Failed);
        assert_eq!(table.answer_for(whose), Some(SEATED));
        table.forget(whose);
        assert_eq!(table.answer_for(whose), None);
    }

    #[test]
    fn the_answer_is_worked_out_only_for_a_new_owner() {
        let mut table = ResolvedModelIllumTable::default();
        let whose = ModelIllumOwner::EffectModel(9);
        table.keep_first_with(whose, || SEATED);
        table.keep_first_with(whose, || unreachable!("already answered"));
        assert_eq!(table.answer_for(whose), Some(SEATED));
    }

    #[test]
    fn glass_not_in_the_live_set_is_forgotten_but_other_owners_stay() {
        let mut table = ResolvedModelIllumTable::default();
        for whose in [
            ModelIllumOwner::Pane(1),
            ModelIllumOwner::Pane(2),
            ModelIllumOwner::Item(1),
        ] {
            table.keep_first(whose, SEATED);
        }
        let live: HashSet<_> = [ModelIllumOwner::Pane(2)].into_iter().collect();
        table.retain_glass(&live);
        assert_eq!(table.answer_for(ModelIllumOwner::Pane(1)), None);
        assert_eq!(table.answer_for(ModelIllumOwner::Pane(2)), Some(SEATED));
        assert_eq!(table.answer_for(ModelIllumOwner::Item(1)), Some(SEATED));
    }

    #[test]
    fn clearing_the_table_forgets_everything() {
        let mut table = ResolvedModelIllumTable::default();
        table.keep_first(ModelIllumOwner::Eye, SEATED);
        table.clear();
        assert_eq!(table.answer_for(ModelIllumOwner::Eye), None);
    }
}
