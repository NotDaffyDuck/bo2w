//! The bridge between the effect simulation (`fx`) and the rest of the game: starting
//! effects from the loaded catalog, advancing them each frame against the world, and
//! turning the live ones into things to draw.
//!
//! Each concern has its own module:
//! - `catalog`: finding effects and the per-effect element tables
//! - `launch`: starting effects by name, on a bone, or from the level's createfx list
//! - `queued`: runner and decal work that starting or ticking an effect leaves behind
//! - `tick`: the per-frame advance, including world collision
//! - `draw`: the per-frame sprite, trail, cloud, light and model output
//! - `t6`: the few rules that only this game's effect data needs

mod catalog;
mod draw;
mod launch;
mod queued;
mod t6;
mod tick;

#[cfg(test)]
mod tests;

use fx::FxSystemHost;

pub use catalog::{FxElemInfoCache, catalog_lookup};
pub use draw::build_fx_verts;
pub use launch::{
    boot_createfx_effects, play_named_bolted_in_world, play_named_oriented_at_msec,
    play_named_oriented_in_world, restamp_missing_packed_lighting, spawn_named_oriented_in_world,
};
pub use tick::{tick_fx_non_dependent, tick_fx_remaining};

/// What the frontend can tell effects about the scene they sit in.
pub trait FxScene {
    /// The light at a world position, when the scene has any to give.
    fn sample_atpoint_rgb(&self, origin: [f32; 3]) -> Option<[u8; 3]>;

    /// Finish the surface marks an impact decal left on the world and/or on models.
    fn finish_impact_marks(
        &self,
        host: &mut FxSystemHost,
        def_index: u8,
        against_world: bool,
        against_models: bool,
    );
}

/// View culling for one draw pass.
#[derive(Clone, Copy)]
pub struct FxDrawCull<'a> {
    /// Whether elements are culled against `planes` at all.
    pub elem_draw: bool,
    pub planes: &'a [[f32; 4]],
}

/// How the level's createfx emitters fared at map start.
#[derive(Debug, Default, Clone)]
pub struct FxCreateFxBoot {
    /// Emitters whose effect was found in the catalog.
    pub named: u32,
    /// Emitters whose effect was found but is held back until released.
    pub held: u32,
    /// Emitters naming an effect the catalog does not have.
    pub miss_def: u32,
    /// Emitters that could not be started, with the reason.
    pub failed: Vec<String>,
}

/// Why elements were dropped, or drawn with a stand-in, in one `build_fx_verts` pass.
#[derive(Debug, Default, Clone)]
pub struct FxVertsGaps {
    /// Elements that ask to be tinted by the light around them.
    pub lighting_frac: u32,
    /// The definition index of each such element whose light was never sampled.
    pub lighting_frac_def_indices: Vec<u8>,
    /// Tinted elements that found a light to tint with.
    pub lighting_applied: u32,
    /// Of those, the ones that used plain white.
    pub lighting_white: u32,
    /// Of those, the ones that used a light sampled at their position.
    pub lighting_atpoint: u32,
    /// Tinted elements whose light was never sampled, so were left untinted.
    pub lighting_miss: u32,
    pub no_def: u32,
    pub no_elem: u32,
    pub no_material_visual: u32,
    pub no_size0_sample: u32,
    pub size0_not_positive: u32,
    pub no_size1_sample: u32,
    pub size1_not_positive: u32,
    /// Elements whose colour curve gave nothing, so were drawn white.
    pub color_fallback: u32,
    /// Catalog lookups made by catalog index.
    pub catalog_index_n: u32,
    /// Lookups that could only name the effect, which the draw path cannot resolve.
    pub catalog_name_n: u32,
}
