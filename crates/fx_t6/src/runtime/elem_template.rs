//! An element definition as the effect file stores it.
//!
//! The ranges and graphs are kept as raw bytes (`[u8; N]`) because the file
//! packs them unaligned; the readers that need them decode on use.

/// One element of an effect, as authored.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ElemBlueprint {
    /// The `FX_ELEM_*` flag bits.
    pub flags: i32,
    /// Spawn timing: a looping interval or a one-shot count, by the def's
    /// group.
    pub spawn: [u8; 8],
    /// Spawn range, base and amplitude.
    pub birth_span: [u8; 8],
    /// Fade-in range.
    pub appear_blend_reach: [u8; 8],
    /// Fade-out range.
    pub vanish_blend_reach: [u8; 8],
    /// Radius around the spawn point used to cull the spawn against the view.
    pub birth_view_reject_reach: f32,
    /// Delay before the element appears, base and amplitude (milliseconds).
    pub birth_wait_ms: [u8; 8],
    /// Life span, base and amplitude (milliseconds).
    pub lifespan_window_ms: [u8; 8],
    /// Spawn position ranges for x, y and z.
    pub birth_point: [u8; 24],
    /// Spawn offset radius range.
    pub birth_ring_reach: [u8; 8],
    /// Spawn offset height range.
    pub birth_lift: [u8; 8],
    /// Spawn angle ranges for pitch, yaw and roll.
    pub birth_orientation: [u8; 24],
    /// Angular velocity ranges for pitch, yaw and roll.
    pub twirl_rate: [u8; 24],
    /// Initial rotation range.
    pub birth_spin_angle: [u8; 8],
    /// Gravity range.
    pub gravity: [u8; 8],
    /// Reflection factor range.
    pub mirror_strength: [u8; 8],
    /// The element type.
    pub shape_kind: u8,
    /// How many visuals the element can show.
    pub look_variant_total: u8,
    /// How many velocity intervals the graphs have.
    pub speed_keyframe_total: u8,
    /// How many visual-state intervals the graphs have.
    pub look_keyframe_total: u8,
    /// Address of the velocity samples.
    pub speed_readings: u32,
    /// Address of the visual-state samples.
    pub visibility_readings: u32,
    /// Address of the visuals.
    pub visuals: u32,
    /// Collision bounds, minimum then maximum corner.
    pub bounce_box: [u8; 24],
    /// Effect played where the element hits something.
    pub effect_upon_strike: u32,
    /// Effect played where the element dies.
    pub effect_upon_demise: u32,
    /// Effect emitted from the element while it lives.
    pub effect_spawned: u32,
    /// Distance between emissions, base and amplitude.
    pub emission_spacing: [u8; 8],
    /// Random variance on the emission distance.
    pub emission_spacing_spread: [u8; 8],
    /// Address of the extended data.
    pub extended: u32,
    /// Draw sort order.
    pub paint_rank: u8,
    /// How much the world lighting tints the element (0 for none).
    pub shading_blend: u8,
    /// Whether the element collides against item clip as well.
    pub collide_with_pickups: u8,
}
