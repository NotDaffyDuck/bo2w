//! Where shots land: the impact puff, the tracer, the glass crack and the
//! mark each segment of a bullet's path leaves, and the blood of a knife hit.
//!
//! A segment reaches the client two ways. The server's confirmed hits come as
//! `EntityBulletHit` events; they are queued as [`BulletHitFx`] and played by
//! [`play_queued_hits`], which runs once this frame's bolts are posed. The
//! player's own pellets are drawn at once from `QueuedPelletEffect` without
//! waiting for the server, and [`play_predicted_pellets`] plays those.

use asset_game::FxName;
use assets::StagedWeapons;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use entity_t6::event;
use fx::EffectBoltTarget;
use net::{ClientEntityRecord, EntityTableCells, HostViewClient, QueuedPelletEffect};
use render_fx::battle_fx::spawn_pellet_leg_effect;
use render_fx::{
    EffectJournalCursor, JournalCount, ReadyHitFx, StagedStreaks, TracerRenderGuard,
    StreakLevel,
};

use super::stage::{FxLedger, FxStage};
use super::viewpoint::Viewpoint;
use crate::adapters::motion::gun_present::AttachTagGoals;
use crate::adapters::motion::peer_body::OtherBodyEffectTags;
use crate::adapters::fx::effect_play::EffectStage;

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
    gun_no: u32,
    link_id: u32,
    pellet_no: u16,
    hand: u8,
    from: [f32; 3],
    to: [f32; 3],
    nrm: [f32; 3],
    face_type: u8,
    surf_flags: u32,
    body_flags: u32,
}

impl From<&sim::EntityEventPayload> for Segment {
    /// An event carries the start of the segment in `origin2`, its end in
    /// `origin`, the surface normal in `direction` and the flesh flags in the
    /// event parameter.
    fn from(event: &sim::EntityEventPayload) -> Self {
        Self {
            attacker: event.attacker_entity_num,
            gun_no: event.weapon,
            link_id: event.correlation,
            pellet_no: event.pellet,
            hand: event.hand,
            from: event.origin2,
            to: event.origin,
            nrm: event.direction,
            face_type: event.surf_type,
            surf_flags: event.surface_flags,
            body_flags: event.event_parm as u32,
        }
    }
}

impl From<&sim::PelletFxRecord> for Segment {
    fn from(record: &sim::PelletFxRecord) -> Self {
        Self {
            attacker: record.attacker,
            gun_no: record.weapon,
            link_id: record.correlation,
            pellet_no: record.pellet,
            hand: record.hand,
            from: record.start,
            to: record.end,
            nrm: record.normal,
            face_type: record.surf_type,
            surf_flags: record.surface_flags,
            body_flags: u32::from(record.body_hit_bits),
        }
    }
}

/// The hit codes Black Ops II sends for a bullet that struck the local
/// player's own body (small, large, or his riot shield).
fn is_hit_on_self(code: i32) -> bool {
    matches!(
        u8::try_from(code),
        Ok(event::ROUND_HIT_BODY_SMALL | event::ROUND_HIT_BODY_LARGE | event::BULLET_HIT_CLIENT_SHIELD)
    )
}

/// The entity a hit's mark should stick to. A hit on the world, or on nothing
/// (a negative number), leaves its mark on the map instead.
fn mark_entity(other_entity_num: i32) -> Option<u16> {
    u16::try_from(other_entity_num)
        .ok()
        .filter(|&number| u32::from(number) < fx_t6::runtime::FX_ENTITYNUM_WORLD)
}

/// Where bolts can be found: on the viewmodel, or on another entity's body.
#[derive(SystemParam)]
pub(super) struct BoltLookup<'w, 's> {
    others: Query<'w, 's, &'static OtherBodyEffectTags>,
    own: Res<'w, AttachTagGoals>,
    slots: Res<'w, EntityTableCells>,
}

impl BoltLookup<'_, '_> {
    /// The blade of the knife that the player with entity `number` swung.
    fn knife_of_body(&self, number: i32) -> Option<EffectBoltTarget> {
        let number = u16::try_from(number).ok()?;
        let entity = self.slots.object_at_index(number)?;
        self.others.get(entity).ok()?.knife
    }
}

/// Everything `spawn_pellet_leg_effect` reads or writes, in one parameter.
#[derive(SystemParam)]
pub(super) struct Pellets<'w, 's> {
    bolts: BoltLookup<'w, 's>,
    impact_effect: Option<Res<'w, ReadyHitFx>>,
    gun_table: Option<Res<'w, StagedWeapons>>,
    tracers: Option<Res<'w, StagedStreaks>>,
    streak_level: ResMut<'w, StreakLevel>,
    gate: ResMut<'w, TracerRenderGuard>,
    viewer: Res<'w, HostViewClient>,
    stage: FxStage<'w>,
    ledger: FxLedger<'w>,
}

impl Pellets<'_, '_> {
    fn play(&mut self, segment: &Segment, impact: bool) {
        let parts = self.stage.parts();
        spawn_pellet_leg_effect(
            segment.attacker,
            segment.gun_no,
            segment.link_id,
            segment.pellet_no,
            segment.hand,
            segment.from,
            segment.to,
            segment.nrm,
            segment.face_type,
            segment.surf_flags,
            segment.body_flags,
            impact,
            &self.bolts.others,
            &self.bolts.own,
            &self.bolts.slots,
            parts.catalog,
            parts.infos,
            self.impact_effect.as_deref(),
            self.gun_table.as_deref(),
            self.tracers.as_deref(),
            &mut self.streak_level,
            &mut self.gate,
            i32::try_from(self.viewer.0.0).unwrap_or(0),
            parts.host,
            &mut self.ledger.cursor,
            &mut self.ledger.combat,
            parts.scene.as_ref().map(|scene| scene as &dyn EffectStage),
        );
    }
}

pub(super) fn queue_bullet_hit(hit: On<net::BulletStrike>, mut queue: MessageWriter<BulletHitFx>) {
    queue.write(BulletHitFx {
        payload: hit.event.payload,
        on_self: is_hit_on_self(hit.event.event.0),
    });
}

pub(super) fn play_queued_hits(mut hits: MessageReader<BulletHitFx>, mut pellets: Pellets) {
    for hit in hits.read() {
        // The mark sticks to the entity that was hit, only for this segment.
        let sticks_to = mark_entity(hit.payload.other_entity_num);
        let before = core::mem::replace(&mut pellets.stage.host().stain_target_entity, sticks_to);
        pellets.play(&Segment::from(&hit.payload), !hit.on_self);
        pellets.stage.host().stain_target_entity = before;
    }
}

pub(super) fn play_predicted_pellets(
    mut queued_debris: ResMut<QueuedPelletEffect>,
    mut pellets: Pellets,
) {
    if queued_debris.0.is_empty() {
        return;
    }
    for record in core::mem::take(&mut queued_debris.0) {
        pellets.ledger.cursor.bump(JournalCount::Pellets);
        pellets.play(&Segment::from(&record), true);
    }
}

const KNIFE_HIT_EFFECT: &str = "impacts/flesh_hit_knife";
const T6_KNIFE_HIT_EFFECT: &str = "impacts/fx_flesh_hit_knife_mp";

/// The bolt a knife's blood rides: on the viewmodel's blade for the player
/// who swung it, on the body's blade for everyone else.
fn knife_bolt(
    viewed: bool,
    own: &AttachTagGoals,
    other: impl FnOnce() -> Option<EffectBoltTarget>,
) -> Option<EffectBoltTarget> {
    if viewed {
        own.knife[0].or(own.knife[1])
    } else {
        other()
    }
}

pub(super) fn knife_blood(
    strike: On<net::MeleeBloodAt>,
    identities: Query<&ClientEntityRecord>,
    bolts: BoltLookup,
    view: Viewpoint,
    mut stage: FxStage,
    mut cursor: ResMut<EffectJournalCursor>,
) {
    let Some(mut play) = stage.playback() else {
        return;
    };
    let viewed = identities
        .get(strike.entity)
        .ok()
        .is_some_and(|identity| view.is_viewed_body(identity));
    let target = knife_bolt(viewed, &bolts.own, || {
        bolts.knife_of_body(strike.event.payload.number)
    });
    let mut fired_count = 0;
    // bo2zm: Black Ops II's own knife blood (its name differs from the old engine's).
    let blood = match play.defs().level_scope() {
        asset_core::AssetCategoryScope::T6 => FxName::new(asset_core::AssetCategoryScope::T6, T6_KNIFE_HIT_EFFECT),
        _ => FxName::engine(KNIFE_HIT_EFFECT),
    };
    play.bolted(Some(blood), target, &mut fired_count);
    cursor.add(JournalCount::ImpactPlayed, fired_count);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bolt(dobj: u32) -> EffectBoltTarget {
        EffectBoltTarget {
            dobj,
            bone: 0,
            client_actor_teleport: false,
            orientation: fx::AttachPointFrame {
                origin: Vec3::ZERO.to_array(),
                axis: [Vec3::X, Vec3::Y, Vec3::Z].map(|unit| unit.to_array()),
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
            direction: Vec3::Z.to_array(),
            surf_type: 9,
            surface_flags: 0xABCD,
            wall_pass_bits: 0,
        }
    }

    #[test]
    fn a_hit_event_puts_each_field_where_the_segment_wants_it() {
        let segment = Segment::from(&payload());
        assert_eq!(segment.attacker, 3);
        assert_eq!((segment.gun_no, segment.link_id), (5, 6));
        assert_eq!((segment.pellet_no, segment.hand), (7, 1));
        // The start travels in origin2, the end in origin.
        assert_eq!(segment.from, [1.0, 2.0, 3.0]);
        assert_eq!(segment.to, [10.0, 11.0, 12.0]);
        assert_eq!(segment.nrm, [0.0, 0.0, 1.0]);
        assert_eq!((segment.face_type, segment.surf_flags), (9, 0xABCD));
        // And the flesh flags in the event parameter, whole.
        assert_eq!(segment.body_flags, 0x1_0004);
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
            normal: Vec3::Z.to_array(),
            surf_type: 9,
            surface_flags: 0xABCD,
            body_hit_bits: 4,
        };
        let predicted = Segment::from(&record);
        let confirmed = Segment::from(&sim::EntityEventPayload {
            event_parm: 4,
            ..payload()
        });
        assert_eq!(predicted.attacker, confirmed.attacker);
        assert_eq!(predicted.from, confirmed.from);
        assert_eq!(predicted.to, confirmed.to);
        assert_eq!(predicted.nrm, confirmed.nrm);
        assert_eq!(predicted.body_flags, confirmed.body_flags);
        assert_eq!(predicted.surf_flags, confirmed.surf_flags);
    }

    #[test]
    fn only_the_three_own_body_codes_are_hits_on_self() {
        for code in [
            event::ROUND_HIT_BODY_SMALL,
            event::ROUND_HIT_BODY_LARGE,
            event::BULLET_HIT_CLIENT_SHIELD,
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
        let mut own = AttachTagGoals::default();
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
        let mut own = AttachTagGoals::default();
        own.knife[0] = Some(bolt(4));
        let got = knife_bolt(false, &own, || Some(bolt(15)));
        assert_eq!(got.map(|t| t.dobj), Some(15));
        assert!(knife_bolt(false, &own, || None).is_none());
    }
}
