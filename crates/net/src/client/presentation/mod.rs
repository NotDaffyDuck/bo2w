//! Turning the adopted snapshot into entities, events and the values the draw side reads.

pub mod entity_records; // each entity's pose and handles
pub mod entity_slots; // the client entity slots
pub mod event_raise; // events raised by entities
pub mod event_table; // which event goes to which handler
pub mod shown_frame; // the presented snapshot and local view data
pub mod seen_throws; // projectiles merged for drawing
