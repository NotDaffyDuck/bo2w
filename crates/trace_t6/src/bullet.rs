//! BO2's `BulletFireParams`.

/// What one bullet trace carries from shot to shot as it goes through
/// surfaces.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BulletFireParams {
    pub weapon_ent_index: i32,
    pub ignore_ent_index: i32,
    pub damage_multiplier: f32,
    pub method_of_death: i32,
    pub orig_start: [f32; 3],
    pub start: [f32; 3],
    pub end: [f32; 3],
    pub dir: [f32; 3],
}
