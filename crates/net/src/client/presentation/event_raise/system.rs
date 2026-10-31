//! The system that runs the dispatch once per adopted snapshot.

use bevy::prelude::*;
use entity_t6::{
    EntitySnapshot, OrderedEntityEvent, drain_entity_events, packet_entity_uses_event_ring,
};
use sim::{ClientId, EntityEventPayload, EntityEventRecord, EventSequence, Tick};

use super::classify::{Dispatcher, event_without_centity};
use super::cursor::EntityEventReadPos;
use super::raised_events::{RoutedEntityEvent, KillcamFxTransition};
use super::unsupported::UnhandledEntityEvents;
use super::walk::InstalledEntityEventTraversal;
use crate::client::drive::{NewestTakenSnapshot, StagedShownEntityEvents};
use crate::client::presentation::entity_records::ClientEntityPoseState;
use crate::client::presentation::shown_frame::HostViewClient;
use crate::identity_slips::{LinkDeficiencyOrigin, LinkIdentityDeficits};
use crate::{ClientEntityRecord, EntityTableCells};

pub(super) fn dispatch_entity_events(
    mut ring_rows: Query<(Entity, &ClientEntityRecord, &mut ClientEntityPoseState)>,
    mut commands: Commands,
    mut cursor: ResMut<EntityEventReadPos>,
    mut tally: ResMut<InstalledEntityEventTraversal>,
    mut unhandled: ResMut<UnhandledEntityEvents>,
    mut id_gaps: ResMut<LinkIdentityDeficits>,
    mut queue: ResMut<StagedShownEntityEvents>,
    entity_slots: Res<EntityTableCells>,
    me: Res<HostViewClient>,
    latest: Res<NewestTakenSnapshot>,
) {
    tally.begin_frame(cursor.raw_seen_through());
    if !latest.taken_in_current_step {
        return;
    }

    // Moving between the live world and the archive restarts every ring and the archive cursor.
    let crossed = cursor.switch_killcam(queue.replaying_final_death);
    if crossed {
        let entering = queue.replaying_final_death;
        commands.trigger(KillcamFxTransition { entering });
    }

    let mut dispatcher = Dispatcher {
        local_number: i32::try_from(me.0.0).unwrap_or(-1),
        commands: &mut commands,
        walk: &mut tally,
        unsupported: &mut unhandled,
    };

    let replaying = queue.replaying_final_death;
    let tick = latest.next().map_or(Tick(0), |snap| snap.tick);
    dispatch_rings(&mut dispatcher, &mut ring_rows, tick, crossed, replaying);

    let queue = &mut *queue;
    for (records, archived) in [(&queue.live, false), (&queue.archived, true)] {
        dispatch_records(
            &mut dispatcher,
            records,
            archived,
            &mut cursor,
            &mut id_gaps,
            &entity_slots,
            me.0,
        );
    }
    queue.live.clear();
    queue.archived.clear();
    tally.observed_up_to = cursor.raw_seen_through();
}

/// Events riding on the entities' own ring_rows.
fn dispatch_rings(
    dispatcher: &mut Dispatcher,
    ring_rows: &mut Query<(Entity, &ClientEntityRecord, &mut ClientEntityPoseState)>,
    tick: Tick,
    crossed_worlds: bool,
    replaying: bool,
) {
    for (entity, identity, mut runtime) in ring_rows {
        if !packet_entity_uses_event_ring(runtime.upcoming_condition.e_type) {
            continue;
        }
        let state = runtime.upcoming_condition;
        let mut position = i32::from(state.event_sequence);
        // The archive and the live world share entity numbers but not ring_rows: across a switch the
        // new ring position is adopted and nothing is replayed.
        if !crossed_worlds {
            position = runtime.prior_occurrence_number;
            let entity_number = i32::from(identity.number());
            let at = runtime.origin;
            drain_entity_events(&state, &mut position, |ring_event| {
                let fired = &mut dispatcher.walk.seat_event_triggered;
                *fired = fired.saturating_add(1);
                let event = from_ring(&state, entity_number, at, tick, ring_event);
                dispatcher.dispatch(entity, event, replaying);
            });
        }
        runtime.prior_occurrence_number = position;
    }
}

fn from_ring(
    ring_state: &EntitySnapshot,
    ent_no: i32,
    pos: [f32; 3],
    tick: Tick,
    ring_event: OrderedEntityEvent,
) -> RoutedEntityEvent {
    let body = EntityEventPayload {
        weapon: ring_state.weapon.0,
        surf_type: (ring_event.parm_value & 0x1f) as u8,
        origin: pos,
        event_parm: ring_event.parm_value,
        attacker_entity_num: i32::from(ring_state.hitter_num),
        other_entity_num: i32::from(ring_state.partner_num),
        number: ent_no,
        ..Default::default()
    };
    let sequence = u32::try_from(ring_event.ring_seq).unwrap_or(0);
    let mut routed = RoutedEntityEvent {
        order_no: EventSequence(sequence),
        tick,
        event: ring_event.code,
        payload: EntityEventPayload::default(),
    };
    routed.payload = body;
    routed
}

/// Records the server sent with the snapshot, either the live ones or the archived ones.
fn dispatch_records(
    dispatcher: &mut Dispatcher,
    records: &[EntityEventRecord],
    archived: bool,
    cursor: &mut EntityEventReadPos,
    id_gaps: &mut LinkIdentityDeficits,
    entity_slots: &EntityTableCells,
    me: ClientId,
) {
    for record in records.iter() {
        let counts = &mut *dispatcher.walk;
        counts.examined = counts.examined.saturating_add(1);

        let admitted = match archived {
            true => cursor.accepts_archived(record, me),
            false => cursor.takes_record(record, me),
        };
        if !admitted {
            continue;
        }
        counts.newest_happening = record.event.0;
        counts.latest_index = record.payload.number;

        let Some(entity) = target_entity(record, entity_slots, id_gaps) else {
            continue;
        };
        let EntityEventRecord {
            sequence,
            tick,
            event,
            payload,
            ..
        } = record;
        let forwarded = RoutedEntityEvent {
            order_no: *sequence,
            tick: *tick,
            event: *event,
            payload: *payload,
        };
        dispatcher.dispatch(entity, forwarded, archived);
    }
}

/// The entity a record is about. A number outside the table, or one with no live entity, is
/// reported as a gap, unless the event needs no entity at all.
fn target_entity(
    record: &EntityEventRecord,
    table: &EntityTableCells,
    holes: &mut LinkIdentityDeficits,
) -> Option<Entity> {
    let named = record.payload.number;
    let Ok(slot) = u16::try_from(named) else {
        holes.raise(LinkDeficiencyOrigin::EventNumberOutOfRange { number: named });
        return None;
    };
    table
        .object_at_index(slot)
        .or_else(|| event_without_centity(record.event).then_some(Entity::PLACEHOLDER))
        .or_else(|| {
            holes.raise(LinkDeficiencyOrigin::EventNumberHasNoEntity { number: named });
            None
        })
}
