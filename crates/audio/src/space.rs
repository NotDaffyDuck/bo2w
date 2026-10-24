//! Positions in world inches.

use bevy::math::Vec3;

pub fn distance_inches(a: [f32; 3], b: [f32; 3]) -> f32 {
    Vec3::from_array(a).distance(Vec3::from_array(b))
}

pub fn transform_inches(position: Vec3) -> [f32; 3] {
    position.to_array()
}
