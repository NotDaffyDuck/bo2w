//! An element definition as the effect file stores it.
//!
//! The ranges and graphs are kept as raw bytes (`[u8; N]`) because the file
//! packs them unaligned; the readers that need them decode on use.

/// One element of an effect, as authored.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FxElemDef {
    /// The `FX_ELEM_*` flag bits.
    pub flags: i32,
    /// Spawn timing: a looping interval or a one-shot count, by the def's
    /// group.
    pub spawn: [u8; 8],
    /// Spawn range, base and amplitude.
    pub spawn_range: [u8; 8],
    /// Fade-in range.
    pub fade_in_range: [u8; 8],
    /// Fade-out range.
    pub fade_out_range: [u8; 8],
    /// Radius around the spawn point used to cull the spawn against the view.
    pub spawn_frustum_cull_radius: f32,
    /// Delay before the element appears, base and amplitude (milliseconds).
    pub spawn_delay_msec: [u8; 8],
    /// Life span, base and amplitude (milliseconds).
    pub life_span_msec: [u8; 8],
    /// Spawn position ranges for x, y and z.
    pub spawn_origin: [u8; 24],
    /// Spawn offset radius range.
    pub spawn_offset_radius: [u8; 8],
    /// Spawn offset height range.
    pub spawn_offset_height: [u8; 8],
    /// Spawn angle ranges for pitch, yaw and roll.
    pub spawn_angles: [u8; 24],
    /// Angular velocity ranges for pitch, yaw and roll.
    pub angular_velocity: [u8; 24],
    /// Initial rotation range.
    pub initial_rotation: [u8; 8],
    /// Gravity range.
    pub gravity: [u8; 8],
    /// Reflection factor range.
    pub reflection_factor: [u8; 8],
    /// The element type.
    pub elem_type: u8,
    /// How many visuals the element can show.
    pub visual_count: u8,
    /// How many velocity intervals the graphs have.
    pub vel_interval_count: u8,
    /// How many visual-state intervals the graphs have.
    pub vis_state_interval_count: u8,
    /// Address of the velocity samples.
    pub vel_samples: u32,
    /// Address of the visual-state samples.
    pub vis_samples: u32,
    /// Address of the visuals.
    pub visuals: u32,
    /// Collision bounds, minimum then maximum corner.
    pub coll_bounds: [u8; 24],
    /// Effect played where the element hits something.
    pub effect_on_impact: u32,
    /// Effect played where the element dies.
    pub effect_on_death: u32,
    /// Effect emitted from the element while it lives.
    pub effect_emitted: u32,
    /// Distance between emissions, base and amplitude.
    pub emit_dist: [u8; 8],
    /// Random variance on the emission distance.
    pub emit_dist_variance: [u8; 8],
    /// Address of the extended data.
    pub extended: u32,
    /// Draw sort order.
    pub sort_order: u8,
    /// How much the world lighting tints the element (0 for none).
    pub lighting_frac: u8,
    /// Whether the element collides against item clip as well.
    pub use_item_clip: u8,
}
