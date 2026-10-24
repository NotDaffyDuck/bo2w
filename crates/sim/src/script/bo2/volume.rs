//! The space a map trigger covers, and whether a body touches it.

use crate::frame::FrameWorld;

/// A trigger's reach: a cylinder, the box around a brush model, or the brushes
/// themselves.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Volume<'f> {
    Cylinder {
        origin: [f32; 3],
        radius: f32,
        height: f32,
    },
    Box {
        mins: [f32; 3],
        maxs: [f32; 3],
    },
    Brushes {
        origin: [f32; 3],
        ids: &'f [u16],
        brushes: &'f [crate::SimBrush],
    },
    /// Never built here; `t6/damage_triggers.rs` still has an arm for it.
    #[allow(dead_code)]
    Hulls {
        origin: [f32; 3],
        hulls: &'f [crate::SimTriggerHull],
    },
}

/// The volume of a BO2 trigger: its cylinder when it has one, otherwise the
/// brush model it points at (its brushes, or failing those its bounding box).
pub(crate) fn t6_volume<'f>(
    frame: &'f FrameWorld,
    origin: [f32; 3],
    cylinder: Option<(f32, f32)>,
    brush: Option<u32>,
) -> Option<Volume<'f>> {
    if let Some((radius, height)) = cylinder {
        return Some(Volume::Cylinder { origin, radius, height });
    }
    let model = frame.clip_cmodels().models.get(brush? as usize)?;
    let first = model.first_brush as usize;
    let ids = frame
        .clip_bsp()
        .leafbrushes
        .get(first..first + usize::from(model.num_brushes))
        .unwrap_or(&[]);
    if ids.is_empty() {
        Some(Volume::Box {
            mins: std::array::from_fn(|i| origin[i] + model.mins[i]),
            maxs: std::array::from_fn(|i| origin[i] + model.maxs[i]),
        })
    } else {
        Some(Volume::Brushes { origin, ids, brushes: frame.clip_brushes() })
    }
}

fn centre_and_half(mins: [f32; 3], maxs: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    (
        std::array::from_fn(|i| (mins[i] + maxs[i]) * 0.5),
        std::array::from_fn(|i| (maxs[i] - mins[i]) * 0.5),
    )
}

impl Volume<'_> {
    /// Whether the box `mins..maxs` overlaps the volume.
    pub(crate) fn touches(&self, mins: [f32; 3], maxs: [f32; 3]) -> bool {
        let (centre, half) = centre_and_half(mins, maxs);
        match *self {
            Volume::Brushes { origin, ids, brushes } => {
                let local: [f32; 3] = std::array::from_fn(|i| centre[i] - origin[i]);
                ids.iter()
                    .filter_map(|&id| brushes.get(usize::from(id)))
                    .any(|brush| {
                        brush.planes.iter().all(|p| {
                            let distance = p[0] * local[0] + p[1] * local[1] + p[2] * local[2] - p[3];
                            let reach = p[0].abs() * half[0] + p[1].abs() * half[1] + p[2].abs() * half[2];
                            distance <= reach
                        })
                    })
            }
            Volume::Cylinder { origin, radius, height } => {
                let (dx, dy) = (origin[0] - centre[0], origin[1] - centre[1]);
                let reach = radius + half[0];
                let mid_z = origin[2] + height * 0.5;
                (mid_z - centre[2]).abs() < height * 0.5 + half[2] && dx * dx + dy * dy < reach * reach
            }
            Volume::Box { mins: lo, maxs: hi } => {
                (0..3).all(|i| mins[i] <= hi[i] && maxs[i] >= lo[i])
            }
            Volume::Hulls { .. } => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cylinder_reaches_sideways_by_radius_plus_half_width() {
        let tube = Volume::Cylinder { origin: [0.0; 3], radius: 10.0, height: 20.0 };
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

    #[test]
    fn hulls_are_never_touched() {
        let hulls = Volume::Hulls { origin: [0.0; 3], hulls: &[] };
        assert!(!hulls.touches([-1.0; 3], [1.0; 3]));
    }
}
