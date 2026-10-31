//! Solid boxes at any turn that player movement clips against.

/// A solid box: its place, its axes (forward, left, up) and its bounds along them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OrientedBlocker {
    pub ent_no: u16,
    pub origin: [f32; 3],
    pub frame: [[f32; 3]; 3],
    pub lo: [f32; 3],
    pub hi: [f32; 3],
    /// Its faces' surface flags (0 for a plain wall; the ladder bit makes it climbable, as Tranzit's bus ladder).
    pub face_bits: u32,
}

fn dot(lhs: [f32; 3], b: [f32; 3]) -> f32 {
    lhs[0] * b[0] + lhs[1] * b[1] + lhs[2] * b[2]
}

impl OrientedBlocker {
    /// The six planes (outward normal, distance) of the box in the world: for each axis, the far face then the near.
    pub fn planes(&self) -> [[f32; 4]; 6] {
        let mut planes = [[0.0; 4]; 6];
        for (k, normal) in self.frame.iter().enumerate() {
            let centre = dot(*normal, self.origin);
            let [x, y, z] = *normal;
            planes[2 * k] = [x, y, z, centre + self.hi[k]];
            planes[2 * k + 1] = [-x, -y, -z, -(centre + self.lo[k])];
        }
        planes
    }
}

#[cfg(test)]
mod tests {
    use super::OrientedBlocker;
    use crate::sim_parts::{SolidBrush, t6_brushes_trace};

    use crate::frame_step::{STANDING_FOOTPRINT_HIGH, STANDING_FOOTPRINT_LOW};

    fn turned_wall(degrees: f32) -> (OrientedBlocker, f32, f32) {
        let (s, c) = degrees.to_radians().sin_cos();
        let wall = OrientedBlocker {
            ent_no: 1,
            origin: [0.0; 3],
            frame: [[c, s, 0.0], [-s, c, 0.0], [0.0, 0.0, 1.0]],
            lo: [-64.0, -5.0, -64.0],
            hi: [64.0, 5.0, 64.0],
            face_bits: 0,
        };
        (wall, s, c)
    }

    fn brush_of(wall: &OrientedBlocker) -> SolidBrush {
        SolidBrush {
            sides: wall.planes().to_vec(),
            solid_bits: 1,
            plane_material_flags: vec![0; 6],
            glass_hit_code: 0,
            vertex_points: Vec::new(),
            hit_surface_flags: 0,
        }
    }

    #[test]
    fn an_axis_aligned_box_has_the_obvious_planes() {
        let block = OrientedBlocker {
            ent_no: 0,
            origin: [10.0, 20.0, 30.0],
            frame: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            lo: [-1.0, -2.0, -3.0],
            hi: [4.0, 5.0, 6.0],
            face_bits: 0,
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
        let hit = t6_brushes_trace(&[&brush], start, end, STANDING_FOOTPRINT_LOW, STANDING_FOOTPRINT_HIGH, 1);
        assert!(hit.fraction < 1.0, "walked through the wall");

        // The player clips as an upright capsule of radius 15, so they stop 15 beyond the wall's half thickness.
        let stop = hit.finish_location(start, end);
        let along_normal = stop[0] * -s + stop[1] * c;
        assert!((along_normal + 20.0).abs() < 0.5, "stopped {along_normal} from the wall's middle, wanted -20");

        let beside = t6_brushes_trace(
            &[&brush],
            [100.0 * c, -100.0, 0.0],
            [100.0 * c, 100.0, 0.0],
            STANDING_FOOTPRINT_LOW,
            STANDING_FOOTPRINT_HIGH,
            1,
        );
        assert!(beside.fraction >= 1.0, "hit beside the wall's end at {}", beside.fraction);
    }
}
