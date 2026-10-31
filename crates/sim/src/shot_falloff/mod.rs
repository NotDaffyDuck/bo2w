//! Bullet maths: how much damage a shot does at a distance, and which way a pair of view angles faces.

mod damage;
mod direction;

pub use damage::shot_damage_at_range;
pub use direction::angles_to_forward;
