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

use fx::EffectSystemHost;

pub use catalog::{EffectElementInfoMemo, recipe_lookup};
pub use draw::assemble_effect_vertices;
pub use launch::{
    start_map_placed_effects, spawn_named_attached_world_effect, play_named_oriented_at_msec,
    play_named_oriented_in_level, refill_missing_packed_lighting, spawn_named_oriented_in_world,
};
pub use tick::{tick_effect_non_dependent, tick_effect_remaining};

/// What the frontend can tell effects about the scene they sit in.
pub trait EffectStage {
    /// The light at a world position, when the scene has any to give.
    fn light_at(&self, spot: [f32; 3]) -> Option<[u8; 3]>;

    /// Finish the surface marks an impact decal left on the world and/or on models.
    fn finish_impact_stains(
        &self,
        host: &mut EffectSystemHost,
        definition_slot: u8,
        against_level: bool,
        versus_models: bool,
    );
}

/// View culling for one draw pass.
#[derive(Clone, Copy)]
pub struct EffectDrawHide<'a> {
    /// Whether elements are culled against `planes` at all.
    pub element_drawn: bool,
    pub plane_list: &'a [[f32; 4]],
}

/// How the level's createfx emitters fared at map start.
#[derive(Debug, Default, Clone)]
pub struct EffectsBringupParams {
    /// Emitters whose effect was found in the catalog.
    pub by_name: u32,
    /// Emitters whose effect was found but is held back until released.
    pub kept_count: u32,
    /// Emitters naming an effect the catalog does not have.
    pub missing_definition_total: u32,
    /// Emitters that could not be started, with the reason.
    pub refused: Vec<String>,
}

/// Why elements were dropped, or drawn with a stand-in, in one `assemble_effect_vertices` pass.
#[derive(Debug, Default, Clone)]
pub struct EffectVertexHoles {
    /// Elements that ask to be tinted by the light around them.
    pub shading_blend: u32,
    /// The definition index of each such element whose light was never sampled.
    pub light_fraction_default_slots: Vec<u8>,
    /// Tinted elements that found a light to tint with.
    pub lit_draw_total: u32,
    /// Of those, the ones that used plain white.
    pub white_fallback_total: u32,
    /// Of those, the ones that used a light sampled at their position.
    pub point_sampled_lit_total: u32,
    /// Tinted elements whose light was never sampled, so were left untinted.
    pub light_lookup_failures: u32,
    pub without_definition: u32,
    pub without_element: u32,
    pub without_material_look: u32,
    pub no_first_extent_sample: u32,
    pub first_extent_nonpositive_total: u32,
    pub no_second_extent_sample: u32,
    pub second_extent_nonpositive_total: u32,
    /// Elements whose colour curve gave nothing, so were drawn white.
    pub substitute_colour_count: u32,
    /// Catalog lookups made by catalog index.
    pub catalog_index_total: u32,
    /// Lookups that could only name the effect, which the draw path cannot resolve.
    pub catalog_name_total: u32,
}
