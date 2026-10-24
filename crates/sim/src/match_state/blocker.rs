//! Solid boxes at any turn that player movement clips against.

/// A solid box: its place, its axes (forward, left, up) and its bounds along them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OrientedBlocker {
    pub entnum: u16,
    pub origin: [f32; 3],
    pub axis: [[f32; 3]; 3],
    pub mins: [f32; 3],
    pub maxs: [f32; 3],
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

impl OrientedBlocker {
    /// The six planes (outward normal, distance) of the box in the world: for each axis, the far face then the near.
    pub fn planes(&self) -> [[f32; 4]; 6] {
        let mut planes = [[0.0; 4]; 6];
        for (k, normal) in self.axis.iter().enumerate() {
            let centre = dot(*normal, self.origin);
            let [x, y, z] = *normal;
            planes[2 * k] = [x, y, z, centre + self.maxs[k]];
            planes[2 * k + 1] = [-x, -y, -z, -(centre + self.mins[k])];
        }
        planes
    }
}

#[cfg(test)]
mod tests {
    use super::OrientedBlocker;
    use crate::world::{SimBrush, t6_brushes_trace};

    const PLAYER_MINS: [f32; 3] = [-15.0, -15.0, 0.0];
    const PLAYER_MAXS: [f32; 3] = [15.0, 15.0, 70.0];

    fn turned_wall(degrees: f32) -> (OrientedBlocker, f32, f32) {
        let (s, c) = degrees.to_radians().sin_cos();
        let wall = OrientedBlocker {
            entnum: 1,
            origin: [0.0; 3],
            axis: [[c, s, 0.0], [-s, c, 0.0], [0.0, 0.0, 1.0]],
            mins: [-64.0, -5.0, -64.0],
            maxs: [64.0, 5.0, 64.0],
        };
        (wall, s, c)
    }

    fn brush_of(wall: &OrientedBlocker) -> SimBrush {
        SimBrush {
            planes: wall.planes().to_vec(),
            contents: 1,
            plane_surface_flags: vec![0; 6],
            glass_encoded: 0,
            verts: Vec::new(),
            hit_surface_flags: 0,
        }
    }

    #[test]
    fn an_axis_aligned_box_has_the_obvious_planes() {
        let block = OrientedBlocker {
            entnum: 0,
            origin: [10.0, 20.0, 30.0],
            axis: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            mins: [-1.0, -2.0, -3.0],
            maxs: [4.0, 5.0, 6.0],
        };
        let planes = block.planes();
        assert_eq!(planes[0], [1.0, 0.0, 0.0, 14.0]);
        assert_eq!(planes[1], [-1.0, 0.0, 0.0, -9.0]);
        assert_eq!(planes[2], [0.0, 1.0, 0.0, 25.0]);
        assert_eq!(planes[3], [0.0, -1.0, 0.0, -18.0]);
        assert_eq!(planes[4], [0.0, 0.0, 1.0, 36.0]);
        assert_eq!(planes[5], [0.0, 0.0, -1.0, -27.0]);
    }

    /// A 128 x 10 x 128 wall turned 30 degrees stops a walking player where they first touch it and lets them by
    /// beside its end.
    #[test]
    fn a_turned_wall_stops_the_player_box_and_leaves_its_end_open() {
        let (wall, s, c) = turned_wall(30.0);
        let brush = brush_of(&wall);
        let (start, end) = ([0.0, -100.0, 0.0], [0.0, 100.0, 0.0]);
        let hit = t6_brushes_trace(&[&brush], start, end, PLAYER_MINS, PLAYER_MAXS, 1);
        assert!(hit.fraction < 1.0, "walked through the wall");

        // The player clips as an upright capsule of radius 15, so they stop 15 beyond the wall's half thickness.
        let stop = hit.end_pos(start, end);
        let along_normal = stop[0] * -s + stop[1] * c;
        assert!((along_normal + 20.0).abs() < 0.5, "stopped {along_normal} from the wall's middle, wanted -20");

        let beside = t6_brushes_trace(
            &[&brush],
            [100.0 * c, -100.0, 0.0],
            [100.0 * c, 100.0, 0.0],
            PLAYER_MINS,
            PLAYER_MAXS,
            1,
        );
        assert!(beside.fraction >= 1.0, "hit beside the wall's end at {}", beside.fraction);
    }
}
