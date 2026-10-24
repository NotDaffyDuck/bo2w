//! What a tick leaves for the outside to take (events, effects, debris), and the boxes and rules scripts change
//! while it runs.

use super::state::SimState;
use super::{MantleBlocker, Tick};
use crate::match_state::{EntityEventPayload, EntityEventRecord, EventAudience, EventRecord, SimEvent};

/// Stores `new` in `slot` unless it already holds the same thing, so an unchanged table is not rewritten.
fn replace_if_changed<T: PartialEq>(slot: &mut T, new: T) {
    if *slot != new {
        *slot = new;
    }
}

impl SimState {
    // ---- events -----------------------------------------------------------------------------------------------

    /// Adds an event to this tick's journal. On the world that publishes, a death is also counted for the
    /// performance log.
    pub(crate) fn push_event(&mut self, tick: Tick, audience: EventAudience, event: SimEvent) {
        let sequence = self.serials.event;
        self.serials.event = sequence.next();
        if self.publish_snapshot
            && let SimEvent::Died { victim, attacker, .. } = &event
        {
            let suicide = u8::from(attacker.is_none_or(|killer| killer == *victim));
            perf::death(victim.0, attacker.map(|killer| killer.0), suicide, tick.0);
        }
        self.outbox.journal.push(EventRecord { sequence, tick, audience, event });
    }

    pub(crate) fn push_entity_event(
        &mut self,
        tick: Tick,
        audience: EventAudience,
        event: entity_t6::EntityEventKind,
        payload: EntityEventPayload,
    ) {
        let sequence = self.serials.entity_event;
        self.serials.entity_event = sequence.next();
        self.outbox.entity_events.push(EntityEventRecord { sequence, tick, audience, event, payload });
    }

    pub fn pellet_fx(&self) -> &[crate::PelletFxRecord] {
        &self.outbox.pellet_fx
    }

    pub(crate) fn push_pellet_fx(&mut self, record: crate::PelletFxRecord) {
        if std::env::var_os("BO2MP_SHOT_LOG").is_some() {
            diag::info!(Sim, "shot sim: pellet fx weapon {} pellet {}", record.weapon, record.pellet);
        }
        self.outbox.pellet_fx.push(record);
    }

    pub(crate) fn push_debris(&mut self, record: crate::DebrisRecord) {
        self.outbox.debris.push(record);
    }

    /// Empties what the last tick published. Entity events stay until their lifetime is over.
    pub(crate) fn clear_tick_events(&mut self, tick: Tick) {
        let lifetime_ms = crate::gentity::GENTITY_TEMP_EVENT_LIFETIME_MS as u32;
        let outbox = &mut self.outbox;
        outbox.journal.clear();
        outbox.entity_events.retain(|record| {
            tick.0.saturating_sub(record.tick.0).saturating_mul(crate::MATCH_TICK_MS) <= lifetime_ms
        });
        outbox.pickups.clear();
        outbox.pellet_fx.clear();
        outbox.debris.clear();
    }

    // ---- boxes and rules scripts change -----------------------------------------------------------------------

    pub(crate) fn oriented_blockers(&self) -> &[crate::OrientedBlocker] {
        &self.blockers.oriented
    }

    pub(crate) fn set_oriented_blockers(&mut self, blockers: Vec<crate::OrientedBlocker>) {
        replace_if_changed(&mut self.blockers.oriented, blockers);
    }

    pub(crate) fn enemy_models(&self) -> &[crate::EnemyModel] {
        &self.blockers.enemy_models
    }

    pub(crate) fn set_enemy_models(&mut self, rows: Vec<crate::EnemyModel>) {
        replace_if_changed(&mut self.blockers.enemy_models, rows);
    }

    pub(crate) fn tac_insert_blockers(&self) -> &[([f32; 3], [f32; 3])] {
        &self.blockers.tac_insert
    }

    pub(crate) fn set_tac_insert_blockers(&mut self, boxes: Vec<([f32; 3], [f32; 3])>) {
        replace_if_changed(&mut self.blockers.tac_insert, boxes);
    }

    pub(crate) fn mantle_blockers(&self) -> &[MantleBlocker] {
        &self.blockers.mantle
    }

    pub(crate) fn add_mantle_blocker(&mut self, blocker: MantleBlocker) {
        self.blockers.mantle.push(blocker);
    }

    /// Lets go of the blockers whose time is up; the game frees the entity at its next think.
    pub(crate) fn prune_mantle_blockers(&mut self, level_time: i32) {
        self.blockers.mantle.retain(|blocker| blocker.until > level_time);
    }

    /// When the newest climb of `owner` that still holds its landing ends: the time the `mantle_start` notify
    /// carries.
    pub(crate) fn mantle_end_time(&self, owner: u16) -> Option<i32> {
        self.blockers.mantle.iter().rev().find(|blocker| blocker.owner == owner).map(|blocker| blocker.ends)
    }

    pub(crate) fn feel(&self) -> Option<movement_t6::bridge::Bo2Feel> {
        self.blockers.feel
    }

    pub(crate) fn set_feel(&mut self, feel: movement_t6::bridge::Bo2Feel) {
        self.blockers.feel = Some(feel);
    }
}
