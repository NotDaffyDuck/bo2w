//! The volume a trigger entity covers.

/// One convex piece of a trigger's volume: a box, narrowed by slabs.
#[derive(Clone, Debug, PartialEq)]
pub struct MapTriggerHull {
    /// Centre of the box.
    pub mid: [f32; 3],
    /// Half the box's size along each axis.
    pub half: [f32; 3],
    /// Pairs of parallel planes that cut the box down, as (unit direction,
    /// centre, half-width). A point is inside a slab while its distance along
    /// the direction is within half-width of the centre.
    pub slabs: Vec<([f32; 3], f32, f32)>,
}
