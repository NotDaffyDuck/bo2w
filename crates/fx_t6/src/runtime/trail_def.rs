//! A trail definition and the vertices of its cross-section.

/// How a trail is laid out and when it splits into segments.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FxTrailDef {
    /// Time the texture takes to scroll once along the trail, in
    /// milliseconds.
    pub scroll_time_msec: i32,
    /// Distance over which the texture repeats.
    pub repeat_dist: i32,
    /// One over the distance at which a new segment starts.
    pub inv_split_dist: f32,
    /// One over the arc length at which a new segment starts.
    pub inv_split_arc_dist: f32,
    /// One over the time after which a new segment starts.
    pub inv_split_time: f32,
    /// How many cross-section vertices there are.
    pub vert_count: i32,
    /// Address of the vertices.
    pub verts: u32,
    /// How many indices there are.
    pub ind_count: i32,
    /// Address of the indices.
    pub inds: u32,
}

/// One vertex of a trail's cross-section.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FxTrailVertex {
    /// Position across the trail.
    pub pos: [f32; 2],
    /// Surface normal across the trail.
    pub normal: [f32; 2],
    /// Texture coordinate along the cross-section.
    pub tex_coord: f32,
}
