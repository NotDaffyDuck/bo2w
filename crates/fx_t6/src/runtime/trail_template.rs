//! A trail definition and the vertices of its cross-section.

/// How a trail is laid out and when it splits into segments.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StreakBlueprint {
    /// Time the texture takes to scroll once along the trail, in
    /// milliseconds.
    pub scroll_period_millis: i32,
    /// Distance over which the texture repeats.
    pub recurrence_distance: i32,
    /// One over the distance at which a new segment starts.
    pub reciprocal_split_distance: f32,
    /// One over the arc length at which a new segment starts.
    pub reciprocal_split_arc_length: f32,
    /// One over the time after which a new segment starts.
    pub reciprocal_split_duration: f32,
    /// How many cross-section vertices there are.
    pub vertex_tally: i32,
    /// Address of the vertices.
    pub vertex_points: u32,
    /// How many indices there are.
    pub triangle_slot_total: i32,
    /// Address of the indices.
    pub index_run_values: u32,
}

/// One vertex of a trail's cross-section.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StreakCorner {
    /// Position across the trail.
    pub spot: [f32; 2],
    /// Surface normal across the trail.
    pub normal: [f32; 2],
    /// Texture coordinate along the cross-section.
    pub sheet_v: f32,
}
