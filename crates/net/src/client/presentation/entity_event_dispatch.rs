use bevy::prelude::*;
use entity_t6::{
    EntityEventAction, EntityEventKind, UnsupportedEntityEvent, consume_entity_events,
    entity_event_action, packet_entity_uses_event_ring,
};
use sim::{EntityEventPayload, EventSequence, Tick};

use crate as net;
use crate::CEntity;
use crate::CEntitySlots;
use crate::client::input::ClientActionInput;
use crate::client::presentation::centity_runtime::CEntityRuntime;
use crate::client::presentation::entity_event_registry::{EntityEventDispatch, ev_dispatch_row};
use crate::client::presentation::presented::LocalPresentClient;
use crate::client::drive::{LastAdoptedSnapshot, PendingPresentedEntityEvents};
use crate::gaps::{NetGapCause, NetIdentityGaps};
use crate::schedule::ClientSet;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DispatchedEntityEvent {
    pub sequence: EventSequence,
    pub tick: Tick,
    pub event: EntityEventKind,
    pub payload: EntityEventPayload,
}

#[derive(EntityEvent, Clone, Copy, Debug, PartialEq)]
pub struct EntityEventSound {
    pub entity: Entity,
    pub event: DispatchedEntityEvent,
}

#[derive(EntityEvent, Clone, Copy, Debug, PartialEq)]
pub struct EntityWeaponFire {
    pub entity: Entity,
    pub event: DispatchedEntityEvent,
}

#[derive(EntityEvent, Clone, Copy, Debug, PartialEq)]
pub struct EntityEjectBrass {
    pub entity: Entity,
    pub event: DispatchedEntityEvent,
}

#[derive(EntityEvent, Clone, Copy, Debug, PartialEq)]
pub struct EntityBulletHit {
    pub entity: Entity,
    pub event: DispatchedEntityEvent,
}

#[derive(EntityEvent, Clone, Copy, Debug, PartialEq)]
pub struct EntityGrenadeContact {
    pub entity: Entity,
    pub event: DispatchedEntityEvent,
}

/// bo2mp: a crossbow bolt or a thrown blade striking (BO2's
/// EV_BOLT_IMPACT): where it went in, the hit normal, the surface (7 a
/// player's body, 31 a riot shield), the player struck.
#[derive(EntityEvent, Clone, Copy, Debug, PartialEq)]
pub struct EntityBoltImpact {
    pub entity: Entity,
    pub event: DispatchedEntityEvent,
}

/// bo2mp: the Hellstorm's rider boosted it, or it began coasting still
/// armed (BO2's EV_MISSILE_REMOTE_BOOST, on the missile): a rider's view
/// shakes.
#[derive(EntityEvent, Clone, Copy, Debug, PartialEq)]
pub struct EntityRemoteMissileBoost {
    pub entity: Entity,
    pub event: DispatchedEntityEvent,
}

/// bo2mp: a script's earthquake (BO2's EV_EARTHQUAKE): the view shakes by
/// the scale (origin2's first) for the length (event_parm, in
/// milliseconds), weaker out to the radius (origin2's second).
#[derive(EntityEvent, Clone, Copy, Debug, PartialEq)]
pub struct EntityEarthquake {
    pub entity: Entity,
    pub event: DispatchedEntityEvent,
}

/// bo2mp: a script's rumble (BO2's EV_PLAY_RUMBLE_ON_ENT / _ON_POS,
/// EV_PLAY_RUMBLELOOP_ON_ENT / _ON_POS, EV_STOP_RUMBLE, EV_STOP_ALL_RUMBLES;
/// number: the entity, or the world for a spot; event_parm: the rumble's
/// name, as a sound alias index `rumble:<name>`; origin: the spot).
#[derive(EntityEvent, Clone, Copy, Debug, PartialEq)]
pub struct EntityRumble {
    pub entity: Entity,
    pub event: DispatchedEntityEvent,
    pub in_killcam: bool,
}

/// bo2mp: a missile struck a riot shield, or a blast the shield partly
/// stopped (EV_EXPLOSIVE_IMPACT_ON_SHIELD / _SPLASH_ON_SHIELD; number: its
/// bearer; a blast's event_parm: what it would have done unshielded).
#[derive(EntityEvent, Clone, Copy, Debug, PartialEq)]
pub struct EntityShieldBlast {
    pub entity: Entity,
    pub event: DispatchedEntityEvent,
}

/// bo2mp: a melee swing (BO2's EV_FIRE_MELEE; number: the swinger, origin:
/// his eye, direction: his view angles, weapon: the melee weapon).
#[derive(EntityEvent, Clone, Copy, Debug, PartialEq)]
pub struct EntityFireMelee {
    pub entity: Entity,
    pub event: DispatchedEntityEvent,
}

// Fires before the first events of the other world, so FX for the switch
// are cleared before the replay plays them again.
#[derive(Event, Clone, Copy, Debug, PartialEq, Eq)]
pub struct KillcamFxTransition {
    pub entering: bool,
}

#[derive(EntityEvent, Clone, Copy, Debug, PartialEq)]
pub struct EntityExplosion {
    pub entity: Entity,
    pub event: DispatchedEntityEvent,
}

#[derive(EntityEvent, Clone, Copy, Debug, PartialEq)]
pub struct EntityPlayFx {
    pub entity: Entity,
    pub event: DispatchedEntityEvent,
}

#[derive(EntityEvent, Clone, Copy, Debug, PartialEq)]
pub struct EntityObituary {
    pub entity: Entity,
    pub event: DispatchedEntityEvent,
    pub in_killcam: bool,
}

#[derive(EntityEvent, Clone, Copy, Debug, PartialEq)]
pub struct EntityMovementSound {
    pub entity: Entity,
    pub event: DispatchedEntityEvent,
}

#[derive(EntityEvent, Clone, Copy, Debug, PartialEq)]
pub struct EntityResetAds {
    pub entity: Entity,
    pub event: DispatchedEntityEvent,
}

#[derive(EntityEvent, Clone, Copy, Debug, PartialEq)]
pub struct EntityMeleeBlood {
    pub entity: Entity,
    pub event: DispatchedEntityEvent,
}

#[derive(Resource, Default, Debug)]
pub struct EntityEventCursor {
    seen_through: u32,

    archived_through: Option<(EventSequence, Tick)>,
    in_killcam: bool,
}

#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppliedEntityEventWalk {
    pub walked: u32,
    pub dispatched: u32,
    pub local_fire: u32,
    pub seen_through: u32,

    pub occupancy_fired: u32,

    pub last_event: i32,

    pub last_number: i32,
}

#[derive(Resource, Default, Debug)]
pub struct UnsupportedEntityEvents {
    pub total: u32,

    pub first: Option<EntityEventKind>,
    warned: bool,
}

impl UnsupportedEntityEvents {
    fn note(&mut self, event: EntityEventKind) {
        self.total = self.total.saturating_add(1);
        if self.first.is_none() {
            self.first = Some(event);
        }
        if !self.warned {
            self.warned = true;

            let (name, why) = match ev_dispatch_row(event) {
                Some(row) => (
                    row.name,
                    match row.dispatch {
                        EntityEventDispatch::Unsupported(reason) => reason,
                        EntityEventDispatch::Observer(_) => {
                            "declared as classified in EV_DISPATCH_REGISTRY, yet the classifier \
                             rejected it — G-BUS-1 should have caught this"
                        }
                    },
                ),
                None => (
                    "unmodelled",
                    "not in entity_t6::EntityEventKind::TAXONOMY: a table slot this build \
                     does not model at all",
                ),
            };
            diag::warn!(
                Net,
                "entity events: {} (#{:#x}) has no client handler — {} \
                 (further unsupported events counted, not printed)",
                name,
                event.0,
                why
            );
        }
    }
}

impl EntityEventCursor {
    pub fn accepts(&mut self, record: &sim::EntityEventRecord, local: sim::ClientId) -> bool {
        if !record.audience.projects_to(local) {
            return false;
        }
        if !record
            .sequence
            .is_newer_than(EventSequence(self.seen_through))
        {
            return false;
        }
        self.seen_through = record.sequence.0;
        true
    }

    pub fn accepts_archived(
        &mut self,
        record: &sim::EntityEventRecord,
        local: sim::ClientId,
    ) -> bool {
        if !record.audience.projects_to(local) {
            return false;
        }
        // Each snapshot re-carries the records of the last temp-event lifetime,
        // so only a tick further back than that window marks a rewind.
        if let Some((sequence, tick)) = self.archived_through
            && tick.0.saturating_sub(record.tick.0) * sim::MATCH_TICK_MS
                <= sim::GENTITY_TEMP_EVENT_LIFETIME_MS as u32
            && !record.sequence.is_newer_than(sequence)
        {
            return false;
        }
        self.archived_through = Some((record.sequence, record.tick));
        true
    }

    pub const fn seen_through(&self) -> EventSequence {
        EventSequence(self.seen_through)
    }
}

fn dispatch_entity_events(
    mut commands: Commands,
    adopted: Res<LastAdoptedSnapshot>,
    mut pending: ResMut<PendingPresentedEntityEvents>,
    local: Res<LocalPresentClient>,
    slots: Res<CEntitySlots>,
    mut cursor: ResMut<EntityEventCursor>,
    mut walk: ResMut<AppliedEntityEventWalk>,
    mut unsupported: ResMut<UnsupportedEntityEvents>,
    mut target_gaps: ResMut<NetIdentityGaps>,
    mut runtimes: Query<(Entity, &CEntity, &mut CEntityRuntime)>,
) {
    walk.walked = 0;
    walk.dispatched = 0;
    walk.local_fire = 0;
    walk.occupancy_fired = 0;
    walk.last_event = 0;
    walk.last_number = 0;
    walk.seen_through = cursor.seen_through;
    if !adopted.applied_this_frame {
        return;
    }
    let tick = adopted
        .next()
        .map(|snapshot| snapshot.tick)
        .unwrap_or(Tick(0));
    let local_number = i32::try_from(local.0.0).unwrap_or(-1);
    let killcam_transition = pending.in_killcam != cursor.in_killcam;
    if killcam_transition {
        cursor.in_killcam = pending.in_killcam;
        cursor.archived_through = None;
        commands.trigger(KillcamFxTransition {
            entering: pending.in_killcam,
        });
    }
    for (entity, identity, mut runtime) in runtimes.iter_mut() {
        if !packet_entity_uses_event_ring(runtime.next_state.e_type) {
            continue;
        }

        let next_state = runtime.next_state;
        // The archive and the live world share entity numbers but not event
        // rings: adopt the new ring position instead of replaying across it.
        if killcam_transition {
            runtime.previous_event_sequence = i32::from(next_state.event_sequence);
            continue;
        }
        let origin = runtime.origin;
        let mut cursor = runtime.previous_event_sequence;
        consume_entity_events(&next_state, &mut cursor, |ev| {
            walk.occupancy_fired = walk.occupancy_fired.saturating_add(1);
            let number = i32::from(identity.number());
            dispatch_classified(
                &mut commands,
                local_number,
                number,
                entity,
                DispatchedEntityEvent {
                    sequence: EventSequence(u32::try_from(ev.sequence).unwrap_or(0)),
                    tick,
                    event: ev.event,
                    payload: EntityEventPayload {
                        number,
                        other_entity_num: i32::from(next_state.other_entity_num),
                        attacker_entity_num: i32::from(next_state.attacker_entity_num),
                        event_parm: ev.event_parm,
                        origin,
                        surf_type: (ev.event_parm & 0x1f) as u8,
                        weapon: next_state.weapon.0,
                        ..Default::default()
                    },
                },
                pending.in_killcam,
                &mut walk,
                &mut unsupported,
            );
        });
        runtime.previous_event_sequence = cursor;
    }
    let pending = &mut *pending;
    let records = pending
        .live
        .iter()
        .map(|record| (false, record))
        .chain(pending.archived.iter().map(|record| (true, record)));
    for (archived, record) in records {
        walk.walked = walk.walked.saturating_add(1);
        let accepted = if archived {
            cursor.accepts_archived(record, local.0)
        } else {
            cursor.accepts(record, local.0)
        };
        if !accepted {
            continue;
        }
        walk.last_event = record.event.0;
        walk.last_number = record.payload.number;

        let dispatched = DispatchedEntityEvent {
            sequence: record.sequence,
            tick: record.tick,
            event: record.event,
            payload: record.payload,
        };
        let number = record.payload.number;
        let resolved = match u16::try_from(number) {
            Ok(number) => slots.entity_for_number(number),
            Err(_) => {
                target_gaps.raise(NetGapCause::EventNumberOutOfRange { number });
                continue;
            }
        };

        let entity = match resolved {
            Some(entity) => entity,
            None if event_without_centity(record.event) => Entity::PLACEHOLDER,
            None => {
                target_gaps.raise(NetGapCause::EventNumberHasNoEntity { number });
                continue;
            }
        };
        dispatch_classified(
            &mut commands,
            local_number,
            number,
            entity,
            dispatched,
            archived,
            &mut walk,
            &mut unsupported,
        );
    }
    pending.live.clear();
    pending.archived.clear();
    walk.seen_through = cursor.seen_through;
}

fn dispatch_classified(
    commands: &mut Commands,
    local_number: i32,
    number: i32,
    entity: Entity,
    dispatched: DispatchedEntityEvent,
    in_killcam: bool,
    walk: &mut AppliedEntityEventWalk,
    unsupported: &mut UnsupportedEntityEvents,
) {
    let mut did = false;
    match entity_event_action(dispatched.event) {
        Ok(EntityEventAction::None) => {}
        Ok(EntityEventAction::Sound) => {
            did = true;
            commands.trigger(EntityEventSound {
                entity,
                event: dispatched,
            });
        }
        Ok(EntityEventAction::WeaponFire) => {
            did = true;
            if number == local_number {
                walk.local_fire = walk.local_fire.saturating_add(1);
            }
            commands.trigger(EntityWeaponFire {
                entity,
                event: dispatched,
            });
        }
        Ok(EntityEventAction::EjectBrass) => {
            did = true;
            commands.trigger(EntityEjectBrass {
                entity,
                event: dispatched,
            });
        }
        Ok(EntityEventAction::BulletHit) => {
            did = true;
            commands.trigger(EntityBulletHit {
                entity,
                event: dispatched,
            });
        }
        Ok(EntityEventAction::GrenadeContact) => {
            did = true;
            commands.trigger(EntityGrenadeContact {
                entity,
                event: dispatched,
            });
        }
        Ok(EntityEventAction::BoltImpact) => {
            did = true;
            commands.trigger(EntityBoltImpact {
                entity,
                event: dispatched,
            });
        }
        Ok(EntityEventAction::RemoteMissileBoost) => {
            did = true;
            commands.trigger(EntityRemoteMissileBoost {
                entity,
                event: dispatched,
            });
        }
        Ok(EntityEventAction::Earthquake) => {
            did = true;
            commands.trigger(EntityEarthquake {
                entity,
                event: dispatched,
            });
        }
        Ok(EntityEventAction::Rumble) => {
            did = true;
            commands.trigger(EntityRumble {
                entity,
                event: dispatched,
                in_killcam,
            });
        }
        Ok(EntityEventAction::ShieldBlast) => {
            did = true;
            commands.trigger(EntityShieldBlast {
                entity,
                event: dispatched,
            });
        }
        Ok(EntityEventAction::FireMelee) => {
            did = true;
            commands.trigger(EntityFireMelee {
                entity,
                event: dispatched,
            });
        }
        Ok(EntityEventAction::Explosion) => {
            did = true;
            commands.trigger(EntityExplosion {
                entity,
                event: dispatched,
            });
        }
        Ok(EntityEventAction::PlayFx) => {
            did = true;
            commands.trigger(EntityPlayFx {
                entity,
                event: dispatched,
            });
        }
        Ok(EntityEventAction::Obituary) => {
            did = true;
            commands.trigger(EntityObituary {
                entity,
                event: dispatched,
                in_killcam,
            });
        }
        Ok(EntityEventAction::MovementSound) => {
            did = true;
            commands.trigger(EntityMovementSound {
                entity,
                event: dispatched,
            });
        }
        Ok(EntityEventAction::ResetAds) => {
            did = true;
            commands.trigger(EntityResetAds {
                entity,
                event: dispatched,
            });
        }
        Ok(EntityEventAction::MeleeBlood) => {
            did = true;
            commands.trigger(EntityMeleeBlood {
                entity,
                event: dispatched,
            });
        }
        Err(UnsupportedEntityEvent(event)) => {
            did = true;
            unsupported.note(event);
        }
    }
    if did {
        walk.dispatched = walk.dispatched.saturating_add(1);
    }
}

fn event_without_centity(event: EntityEventKind) -> bool {
    matches!(
        entity_event_action(event),
        Ok(EntityEventAction::PlayFx | EntityEventAction::Obituary | EntityEventAction::Rumble)
    ) || event == EntityEventKind::SOUND_ALIAS
}

fn set_ads_from_reset(
    reset: On<net::EntityResetAds>,
    mut input: ResMut<ClientActionInput>,
    local: Res<LocalPresentClient>,
) {
    if (*reset).event.payload.number != local.0.0 as i32 {
        return;
    }
    input_t6::set_ads(&mut input.client, false);
}

/// BO2's out-of-ammo switch: the local player's dry-fire event picks the next gun the way BO2's
/// client does on EV_NOAMMO (`sim::out_of_ammo_pick`, window 2). Not in the killcam.
fn switch_when_out_of_ammo(
    sound: On<net::EntityEventSound>,
    local: Res<LocalPresentClient>,
    prediction: Option<Res<crate::client::drive::ClientPredictionState>>,
    view: Option<Res<frame::ViewSubject>>,
    clock: Option<Res<crate::client::frame_clock::FrameClock>>,
    select: Option<ResMut<crate::client::drive::WeaponSelect>>,
    mut input: ResMut<ClientActionInput>,
) {
    let event = &(*sound).event;
    if event.event != EntityEventKind::NOAMMO
        || event.payload.number != local.0.0 as i32
        || view.is_some_and(|v| v.in_killcam())
    {
        return;
    }
    let (Some(prediction), Some(clock), Some(mut select)) = (prediction, clock, select) else {
        return;
    };
    let world = prediction.0.world();
    let left = event.payload.event_parm & sim::NOAMMO_PARM_LEFT != 0;
    let Some(pick) = sim::out_of_ammo_pick(
        world,
        local.0,
        select.index,
        select.last_primary,
        &select.last_mode,
        left,
    ) else {
        return;
    };
    if pick == select.index {
        return;
    }
    if select.index != 0
        && world
            .weapon_combat_row(select.index)
            .is_some_and(|f| f.inventory_type == 0)
    {
        select.last_primary = select.index;
    }
    select.index = pick;
    select.mapped_index = world
        .weapon_combat_row(pick)
        .filter(|f| f.inventory_type == 3)
        .map_or(pick, |f| f.alternate_weapon);
    select.time = clock.time();
    if let Some(ps) = world.player(local.0) {
        let weapons = ps.weapons;
        sim::remember_mode(world, &weapons, &mut select.last_mode, pick);
    }
    input_t6::set_ads(&mut input.client, false);
}

pub fn register_entity_event_dispatch(app: &mut App) {
    app.init_resource::<EntityEventCursor>()
        .init_resource::<AppliedEntityEventWalk>()
        .init_resource::<UnsupportedEntityEvents>()
        .init_resource::<NetIdentityGaps>()
        .add_observer(set_ads_from_reset)
        .add_observer(switch_when_out_of_ammo)
        .add_systems(
            Update,
            dispatch_entity_events
                .in_set(ClientSet::Reconcile)
                .after(crate::sync_client_entities),
        );
}
