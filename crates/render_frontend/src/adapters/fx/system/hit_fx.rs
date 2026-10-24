//! Where shots land: the impact puff, the tracer, the glass crack and the
//! mark each segment of a bullet's path leaves, and the blood of a knife hit.
//!
//! A segment reaches the client two ways. The server's confirmed hits come as
//! `EntityBulletHit` events; they are queued as [`BulletHitFx`] and played by
//! [`play_queued_hits`], which runs once this frame's bolts are posed. The
//! player's own pellets are drawn at once from `PendingPelletFx` without
//! waiting for the server, and [`play_predicted_pellets`] plays those.

use asset_game::FxName;
use assets::PreparedWeapons;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use entity_t6::event as ev;
use fx::FxBoltTarget;
use net::{CEntity, CEntitySlots, LocalPresentClient, PendingPelletFx};
use render_fx::combat::play_pellet_segment;
use render_fx::{FxJournalCursor, PreparedImpactFx, PreparedTracers, TracerDrawGate, TracerWorld};

use super::stage::{FxLedger, FxStage};
use super::viewpoint::Viewpoint;
use crate::adapters::anim::fpv_present::FpvBoltTargets;
use crate::adapters::anim::remote_body::RemoteFxBolts;
use crate::adapters::fx::present::FxScene;

/// A confirmed bullet hit waiting for its frame's poses.
#[derive(Message)]
pub(super) struct BulletHitFx {
    payload: sim::EntityEventPayload,
    /// The hit landed on the local player himself. Black Ops II only sounds
    /// that one; his game never draws a puff on his own body.
    on_self: bool,
}

/// One straight piece of a bullet's path, and what it ended on.
struct Segment {
    attacker: i32,
    weapon: u32,
    correlation: u32,
    pellet: u16,
    hand: u8,
    from: [f32; 3],
    to: [f32; 3],
    normal: [f32; 3],
    surf_type: u8,
    surface_flags: u32,
    flesh_flags: u32,
}

impl From<&sim::EntityEventPayload> for Segment {
    /// An event carries the start of the segment in `origin2`, its end in
    /// `origin`, the surface normal in `direction` and the flesh flags in the
    /// event parameter.
    fn from(event: &sim::EntityEventPayload) -> Self {
        Self {
            attacker: event.attacker_entity_num,
            weapon: event.weapon,
            correlation: event.correlation,
            pellet: event.pellet,
            hand: event.hand,
            from: event.origin2,
            to: event.origin,
            normal: event.direction,
            surf_type: event.surf_type,
            surface_flags: event.surface_flags,
            flesh_flags: event.event_parm as u32,
        }
    }
}

impl From<&sim::PelletFxRecord> for Segment {
    fn from(record: &sim::PelletFxRecord) -> Self {
        Self {
            attacker: record.attacker,
            weapon: record.weapon,
            correlation: record.correlation,
            pellet: record.pellet,
            hand: record.hand,
            from: record.start,
            to: record.end,
            normal: record.normal,
            surf_type: record.surf_type,
            surface_flags: record.surface_flags,
            flesh_flags: u32::from(record.flesh_flags),
        }
    }
}

/// The hit codes Black Ops II sends for a bullet that struck the local
/// player's own body (small, large, or his riot shield).
fn is_hit_on_self(code: i32) -> bool {
    matches!(
        u8::try_from(code),
        Ok(ev::BULLET_HIT_CLIENT_SMALL | ev::BULLET_HIT_CLIENT_LARGE | ev::BULLET_HIT_CLIENT_SHIELD)
    )
}

/// The entity a hit's mark should stick to. A hit on the world, or on nothing
/// (a negative number), leaves its mark on the map instead.
fn mark_entity(other_entity_num: i32) -> Option<u16> {
    u16::try_from(other_entity_num)
        .ok()
        .filter(|&number| u32::from(number) < fx_t6::runtime::FX_ENTITYNUM_WORLD)
}

/// Everything `play_pellet_segment` reads or writes, in one parameter.
#[derive(SystemParam)]
pub(super) struct Pellets<'w, 's> {
    others: Query<'w, 's, &'static RemoteFxBolts>,
    own: Res<'w, FpvBoltTargets>,
    slots: Res<'w, CEntitySlots>,
    impact_fx: Option<Res<'w, PreparedImpactFx>>,
    weapons: Option<Res<'w, PreparedWeapons>>,
    tracers: Option<Res<'w, PreparedTracers>>,
    tracer_world: ResMut<'w, TracerWorld>,
    gate: ResMut<'w, TracerDrawGate>,
    local: Res<'w, LocalPresentClient>,
    stage: FxStage<'w>,
    ledger: FxLedger<'w>,
}

impl Pellets<'_, '_> {
    fn play(&mut self, segment: &Segment, impact: bool) {
        let parts = self.stage.parts();
        play_pellet_segment(
            segment.attacker,
            segment.weapon,
            segment.correlation,
            segment.pellet,
            segment.hand,
            segment.from,
            segment.to,
            segment.normal,
            segment.surf_type,
            segment.surface_flags,
            segment.flesh_flags,
            impact,
            &self.others,
            &self.own,
            &self.slots,
            parts.catalog,
            parts.infos,
            self.impact_fx.as_deref(),
            self.weapons.as_deref(),
            self.tracers.as_deref(),
            &mut self.tracer_world,
            &mut self.gate,
            i32::try_from(self.local.0.0).unwrap_or(0),
            parts.host,
            &mut self.ledger.cursor,
            &mut self.ledger.combat,
            parts.scene.as_ref().map(|scene| scene as &dyn FxScene),
        );
    }
}

pub(super) fn queue_bullet_hit(hit: On<net::EntityBulletHit>, mut queue: MessageWriter<BulletHitFx>) {
    queue.write(BulletHitFx {
        payload: hit.event.payload,
        on_self: is_hit_on_self(hit.event.event.0),
    });
}

pub(super) fn play_queued_hits(mut hits: MessageReader<BulletHitFx>, mut pellets: Pellets) {
    for hit in hits.read() {
        // The mark sticks to the entity that was hit, only for this segment.
        let sticks_to = mark_entity(hit.payload.other_entity_num);
        let before = core::mem::replace(&mut pellets.stage.host().spawn_mark_entity, sticks_to);
        pellets.play(&Segment::from(&hit.payload), !hit.on_self);
        pellets.stage.host().spawn_mark_entity = before;
    }
}

pub(super) fn play_predicted_pellets(
    mut pending: ResMut<PendingPelletFx>,
    mut pellets: Pellets,
) {
    if pending.0.is_empty() {
        return;
    }
    for record in core::mem::take(&mut pending.0) {
        let cursor = &mut pellets.ledger.cursor;
        cursor.pellet_played = cursor.pellet_played.saturating_add(1);
        pellets.play(&Segment::from(&record), true);
    }
}

const KNIFE_HIT_EFFECT: &str = "impacts/flesh_hit_knife";

/// The bolt a knife's blood rides: on the viewmodel's blade for the player
/// who swung it, on the body's blade for everyone else.
fn knife_bolt(
    viewed: bool,
    own: &FpvBoltTargets,
    other: impl FnOnce() -> Option<FxBoltTarget>,
) -> Option<FxBoltTarget> {
    if viewed {
        own.knife[0].or(own.knife[1])
    } else {
        other()
    }
}

pub(super) fn knife_blood(
    hit: On<net::EntityMeleeBlood>,
    identities: Query<&CEntity>,
    others: Query<&RemoteFxBolts>,
    own: Res<FpvBoltTargets>,
    slots: Res<CEntitySlots>,
    view: Viewpoint,
    mut stage: FxStage,
    mut cursor: ResMut<FxJournalCursor>,
) {
    let Some(mut play) = stage.playback() else {
        return;
    };
    let viewed = identities
        .get(hit.entity)
        .ok()
        .is_some_and(|identity| view.is_viewed_body(identity));
    let target = knife_bolt(viewed, &own, || {
        u16::try_from(hit.event.payload.number)
            .ok()
            .and_then(|number| slots.entity_for_number(number))
            .and_then(|entity| others.get(entity).ok())
            .and_then(|bolts| bolts.knife)
    });
    let mut played = 0;
    play.bolted(Some(FxName::engine(KNIFE_HIT_EFFECT)), target, &mut played);
    cursor.impact_played = cursor.impact_played.saturating_add(played);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bolt(dobj: u32) -> FxBoltTarget {
        FxBoltTarget {
            dobj,
            bone: 0,
            centity_teleport: false,
            orientation: fx::FxBoltOrientation {
                origin: [0.0; 3],
                axis: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            },
        }
    }

    fn payload() -> sim::EntityEventPayload {
        sim::EntityEventPayload {
            number: 1,
            other_entity_num: 2,
            attacker_entity_num: 3,
            event_parm: 0x1_0004,
            weapon: 5,
            correlation: 6,
            pellet: 7,
            hand: 1,
            origin: [10.0, 11.0, 12.0],
            origin2: [1.0, 2.0, 3.0],
            direction: [0.0, 0.0, 1.0],
            surf_type: 9,
            surface_flags: 0xABCD,
            simulation_flags: 0,
        }
    }

    #[test]
    fn a_hit_event_puts_each_field_where_the_segment_wants_it() {
        let segment = Segment::from(&payload());
        assert_eq!(segment.attacker, 3);
        assert_eq!((segment.weapon, segment.correlation), (5, 6));
        assert_eq!((segment.pellet, segment.hand), (7, 1));
        // The start travels in origin2, the end in origin.
        assert_eq!(segment.from, [1.0, 2.0, 3.0]);
        assert_eq!(segment.to, [10.0, 11.0, 12.0]);
        assert_eq!(segment.normal, [0.0, 0.0, 1.0]);
        assert_eq!((segment.surf_type, segment.surface_flags), (9, 0xABCD));
        // And the flesh flags in the event parameter, whole.
        assert_eq!(segment.flesh_flags, 0x1_0004);
    }

    #[test]
    fn a_predicted_pellet_carries_the_same_fields() {
        let record = sim::PelletFxRecord {
            attacker: 3,
            weapon: 5,
            correlation: 6,
            pellet: 7,
            hand: 1,
            start: [1.0, 2.0, 3.0],
            end: [10.0, 11.0, 12.0],
            normal: [0.0, 0.0, 1.0],
            surf_type: 9,
            surface_flags: 0xABCD,
            flesh_flags: 4,
        };
        let predicted = Segment::from(&record);
        let confirmed = Segment::from(&sim::EntityEventPayload {
            event_parm: 4,
            ..payload()
        });
        assert_eq!(predicted.attacker, confirmed.attacker);
        assert_eq!(predicted.from, confirmed.from);
        assert_eq!(predicted.to, confirmed.to);
        assert_eq!(predicted.normal, confirmed.normal);
        assert_eq!(predicted.flesh_flags, confirmed.flesh_flags);
        assert_eq!(predicted.surface_flags, confirmed.surface_flags);
    }

    #[test]
    fn only_the_three_own_body_codes_are_hits_on_self() {
        for code in [
            ev::BULLET_HIT_CLIENT_SMALL,
            ev::BULLET_HIT_CLIENT_LARGE,
            ev::BULLET_HIT_CLIENT_SHIELD,
        ] {
            assert!(is_hit_on_self(i32::from(code)));
            // A code that only matches once squeezed into a byte is not one.
            assert!(!is_hit_on_self(i32::from(code) + 256));
        }
        assert!(!is_hit_on_self(0));
        assert!(!is_hit_on_self(-1));
    }

    #[test]
    fn a_mark_sticks_to_an_entity_but_not_to_the_world() {
        let world = fx_t6::runtime::FX_ENTITYNUM_WORLD as i32;
        assert_eq!(mark_entity(0), Some(0));
        assert_eq!(mark_entity(world - 1), Some((world - 1) as u16));
        assert_eq!(mark_entity(world), None);
        assert_eq!(mark_entity(world + 1), None);
        assert_eq!(mark_entity(-1), None);
        assert_eq!(mark_entity(i32::MAX), None);
    }

    #[test]
    fn the_viewer_swings_the_blade_of_his_viewmodel() {
        let mut own = FpvBoltTargets::default();
        own.knife[1] = Some(bolt(8));
        let got = knife_bolt(true, &own, || panic!("his own blade needs no lookup"));
        assert_eq!(got.map(|t| t.dobj), Some(8));
        // The first knife bolt wins when both are present.
        own.knife[0] = Some(bolt(4));
        let got = knife_bolt(true, &own, || None);
        assert_eq!(got.map(|t| t.dobj), Some(4));
    }

    #[test]
    fn anyone_else_swings_the_blade_of_their_body() {
        let mut own = FpvBoltTargets::default();
        own.knife[0] = Some(bolt(4));
        let got = knife_bolt(false, &own, || Some(bolt(15)));
        assert_eq!(got.map(|t| t.dobj), Some(15));
        assert!(knife_bolt(false, &own, || None).is_none());
    }
}
