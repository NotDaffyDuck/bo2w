//! Positions in world inches.

use bevy::math::Vec3;

pub fn gap_in_inches(a: [f32; 3], b: [f32; 3]) -> f32 {
    Vec3::from_array(a).distance(Vec3::from_array(b))
}

pub fn inches_of(position: Vec3) -> [f32; 3] {
    position.to_array()
}
