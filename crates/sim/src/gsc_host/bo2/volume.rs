//! The space a map trigger covers, and whether a body touches it.

use crate::step_view::StepWorldView;

/// A trigger's reach: a cylinder, the box around a brush model, or the brushes
/// themselves.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Volume<'f> {
    Cylinder {
        origin: [f32; 3],
        reach: f32,
        height: f32,
    },
    Box {
        mins: [f32; 3],
        maxs: [f32; 3],
    },
    Brushes {
        origin: [f32; 3],
        ids: &'f [u16],
        brushes: &'f [crate::SolidBrush],
    },
}

/// The volume of a BO2 trigger: its cylinder when it has one, otherwise the
/// brush model it points at (its brushes, or failing those its bounding box).
pub(crate) fn t6_volume<'f>(
    frame: &'f StepWorldView,
    spot: [f32; 3],
    cylinder: Option<(f32, f32)>,
    brush: Option<u32>,
) -> Option<Volume<'f>> {
    if let Some((radius, height)) = cylinder {
        return Some(Volume::Cylinder { origin: spot, reach: radius, height });
    }
    let model = frame.collision_submodel_set().models.get(brush? as usize)?;
    let first = model.initial_convex_solid as usize;
    let nums = frame
        .collision_tree()
        .brush_leaf_indices
        .get(first..first + usize::from(model.solid_volume_total))
        .unwrap_or(&[]);
    if nums.is_empty() {
        Some(Volume::Box {
            mins: std::array::from_fn(|i| spot[i] + model.lo[i]),
            maxs: std::array::from_fn(|i| spot[i] + model.hi[i]),
        })
    } else {
        Some(Volume::Brushes { origin: spot, ids: nums, brushes: frame.collision_solids() })
    }
}

/// A box held as its middle and how far it reaches from there along each axis.
struct BoxMiddle {
    centre: [f32; 3],
    half: [f32; 3],
}

impl BoxMiddle {
    fn of(mins: [f32; 3], hi: [f32; 3]) -> Self {
        Self {
            centre: std::array::from_fn(|i| (mins[i] + hi[i]) * 0.5),
            half: std::array::from_fn(|i| (hi[i] - mins[i]) * 0.5),
        }
    }
}

impl Volume<'_> {
    /// As `touches`, for brushes turned by `turn` (a rotation, row i = world axis i) about their origin since they
    /// were built: a brush trigger linked to a turning mover (Tranzit's bus window triggers) turns with it. The box's
    /// centre goes into the brushes' frame; its reach along each plane is taken along that plane's turned normal.
    pub(crate) fn touches_turned(&self, mins: [f32; 3], hi: [f32; 3], turn: [[f32; 3]; 3]) -> bool {
        let Volume::Brushes { origin, ids, brushes } = *self else {
            return self.touches(mins, hi);
        };
        let BoxMiddle { centre, half } = BoxMiddle::of(mins, hi);
        let gap_v: [f32; 3] = std::array::from_fn(|at_k| centre[at_k] - origin[at_k]);
        let mid: [f32; 3] = std::array::from_fn(|j| (0..3).map(|at_k| turn[at_k][j] * gap_v[at_k]).sum());
        ids.iter()
            .filter_map(|&id| brushes.get(usize::from(id)))
            .any(|brush| {
                brush.sides.iter().all(|p| {
                    let reach: f32 = (0..3)
                        .map(|at_k| (turn[at_k][0] * p[0] + turn[at_k][1] * p[1] + turn[at_k][2] * p[2]).abs() * half[at_k])
                        .sum();
                    p[0] * mid[0] + p[1] * mid[1] + p[2] * mid[2] - p[3] <= reach
                })
            })
    }

    /// Whether the box `mins..maxs` overlaps the volume.
    pub(crate) fn touches(&self, mins: [f32; 3], maxs: [f32; 3]) -> bool {
        let BoxMiddle { centre, half } = BoxMiddle::of(mins, maxs);
        match *self {
            Volume::Brushes { origin, ids, brushes } => {
                let local: [f32; 3] = std::array::from_fn(|i| centre[i] - origin[i]);
                ids.iter()
                    .filter_map(|&id| brushes.get(usize::from(id)))
                    .any(|brush| {
                        brush.sides.iter().all(|p| {
                            let distance = p[0] * local[0] + p[1] * local[1] + p[2] * local[2] - p[3];
                            let reach = p[0].abs() * half[0] + p[1].abs() * half[1] + p[2].abs() * half[2];
                            distance <= reach
                        })
                    })
            }
            Volume::Cylinder { origin, reach: radius, height } => {
                let (step_along_x, dy) = (origin[0] - centre[0], origin[1] - centre[1]);
                let reach = radius + half[0];
                let mid_z = origin[2] + height * 0.5;
                (mid_z - centre[2]).abs() < height * 0.5 + half[2] && step_along_x * step_along_x + dy * dy < reach * reach
            }
            Volume::Box { mins: lo, maxs: hi } => {
                (0..3).all(|i| mins[i] <= hi[i] && maxs[i] >= lo[i])
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cylinder_reaches_sideways_by_radius_plus_half_width() {
        let tube = Volume::Cylinder { origin: [0.0; 3], reach: 10.0, height: 20.0 };
        assert!(tube.touches([-1.0, 9.5, 0.0], [1.0, 11.5, 4.0]));
        assert!(!tube.touches([-1.0, 20.0, 0.0], [1.0, 22.0, 4.0]));
        assert!(!tube.touches([-1.0, 0.0, 40.0], [1.0, 2.0, 44.0]));
    }

    #[test]
    fn box_needs_overlap_on_every_axis() {
        let cell = Volume::Box { mins: [0.0; 3], maxs: [10.0; 3] };
        assert!(cell.touches([9.0; 3], [12.0; 3]));
        assert!(!cell.touches([9.0, 9.0, 11.0], [12.0, 12.0, 12.0]));
    }
}
