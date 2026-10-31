//! The records the sim queues for clients: sim events, entity events, pellet effects and loose debris.

use crate::sim_parts::StepNo;

/// A sim event with its place in the stream.
#[derive(Clone, Debug, PartialEq)]
pub struct OccurrenceEntry {
    pub sequence: crate::match_ids::EventStamp,
    pub tick: StepNo,
    pub audience: super::audience::EventRecipients,
    pub event: super::sim_event::MatchOccurrence,
}
/// Older spelling EventRecord still calls; the rename sweep moves it over.
pub use self::OccurrenceEntry as EventRecord;

/// The numbers an entity event carries beside its kind.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HappeningBody {
    pub number: i32,
    pub other_entity_num: i32,
    pub attacker_entity_num: i32,
    pub event_parm: i32,
    pub weapon: u32,
    /// Ties this event to the shot or blast that caused it.
    pub correlation: u32,
    pub pellet: u16,
    pub hand: u8,
    pub origin: [f32; 3],
    pub origin2: [f32; 3],
    pub direction: [f32; 3],
    pub surf_type: u8,
    pub surface_flags: u32,
    pub wall_pass_bits: u8,
}
/// Older spelling EntityEventPayload still calls; the rename sweep moves it over.
pub use self::HappeningBody as EntityEventPayload;

/// An entity event with its place in the stream.
#[derive(Clone, Debug, PartialEq)]
pub struct ThingHappening {
    pub sequence: crate::match_ids::EventStamp,
    pub tick: StepNo,
    pub audience: super::audience::EventRecipients,
    pub event: entity_t6::EventCode,
    pub payload: self::HappeningBody,
}
/// Older spelling EntityEventRecord still calls; the rename sweep moves it over.
pub use self::ThingHappening as EntityEventRecord;

/// One pellet's trace, so a client can draw its impact.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScatterPuff {
    pub attacker: i32,
    pub weapon: u32,
    pub correlation: u32,
    pub pellet: u16,
    pub hand: u8,
    pub start: [f32; 3],
    pub end: [f32; 3],
    pub normal: [f32; 3],
    pub surf_type: u8,
    pub surface_flags: u32,
    pub body_hit_bits: u8,
}
/// Older spelling PelletFxRecord still calls; the rename sweep moves it over.
pub use self::ScatterPuff as PelletFxRecord;

/// A piece breaking off a destructible (a mannequin's head). Clients spawn its model where the shown bone was,
/// turned the same way, and fly it themselves, the way Black Ops II's clients treat the pieces of a break.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DebrisRecord {
    pub mesh: String,
    pub origin: [f32; 3],
    /// The bone's turn as a quaternion (x, y, z, w).
    pub rotation: [f32; 4],
    /// The launch speed the hit gave it.
    pub velocity: [f32; 3],
    pub mass: f32,
    pub bounce: f32,
    pub friction: f32,
    /// How hard later bullets and blasts push it (the physics preset's scales).
    pub bullet_force_scale: f32,
    pub explosive_force_scale: f32,
}
