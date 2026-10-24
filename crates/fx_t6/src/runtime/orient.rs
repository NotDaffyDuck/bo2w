//! Moving a point between a frame's own space and the world.

/// The world position of `local`, a point in the frame (`origin`, `axis`).
pub fn orientation_pos_to_world(
    origin: [f32; 3],
    axis: [[f32; 3]; 3],
    local: [f32; 3],
) -> [f32; 3] {
    let mut world = origin;
    for (i, slot) in world.iter_mut().enumerate() {
        *slot = origin[i] + axis[0][i] * local[0] + axis[1][i] * local[1] + axis[2][i] * local[2];
    }
    world
}

/// The position of the world point `world` inside the frame (`origin`,
/// `axis`).
pub fn orientation_pos_from_world(
    origin: [f32; 3],
    axis: [[f32; 3]; 3],
    world: [f32; 3],
) -> [f32; 3] {
    let d = [
        world[0] - origin[0],
        world[1] - origin[1],
        world[2] - origin[2],
    ];
    axis.map(|row| d[0] * row[0] + d[1] * row[1] + d[2] * row[2])
}

#[cfg(test)]
mod tests {
    use super::*;

    const TURNED: [[f32; 3]; 3] = [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];

    #[test]
    fn a_point_makes_the_round_trip() {
        let origin = [10.0, -4.0, 2.0];
        let local = [3.0, 5.0, -7.0];
        let world = orientation_pos_to_world(origin, TURNED, local);
        assert_eq!(world, [10.0 - 5.0, -4.0 + 3.0, 2.0 - 7.0]);
        assert_eq!(orientation_pos_from_world(origin, TURNED, world), local);
    }
}
