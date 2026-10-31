//! Retained draw lanes: item lists built when their inputs change and reused on every other frame.
//!
//! `item` is what every lane shares: turning a key and a sort rank into an item through the
//! material catalog, filing it under its frame product, and the tie that keeps sorting stable.
//! `model_filing_t6` files static models into the game's surface buckets and drains them into
//! items; the static lane and the sun shadow bake both go through it.
//!
//! The static lane (world surfaces and static models) lives in `static_lane`. It picks models
//! and their detail levels in `static_lod`, makes world items in `static_world`, and hands the
//! chosen models to the filer in `static_models_t6`. The sun shadow's casters are baked per
//! partition and merged in `sun_casters_t6`.
//!
//! The model and effect lanes sit beside the static one: `xmodel_lane_t6` holds the dynamic
//! models and `fx_lane_t6` the effect meshes, each rebuilt when its plan's layout changes.

mod draw_entry;
mod model_sorting_t6;
mod fixed_lane;
mod static_lod;
mod static_models_t6;
mod static_world;
mod sun_casters_t6;
mod fx_lane_t6;
mod xmodel_lane_t6;

pub use render_frame::{LevelViewLane, HeldDrawItem, HeldDrawKind, PropPrebuiltRange};

pub(crate) use draw_entry::{
    catalog_item, draw_order, draw_order_tie, mix_identity, pack_draw, route_to_products,
};
pub(crate) use fixed_lane::refill_static_draw_lane;
pub use fixed_lane::{BucketTally, StaticDrawTrack, StaticLaneCounts};
pub use sun_casters_t6::DaylightShadowCasterPlan;
pub(crate) use sun_casters_t6::{
    SunBakeInputs, SunScratch, SunVisibility, bake_partition, fill_prop_draw_inst_shadow,
    merge_partitions,
};
pub use fx_lane_t6::EffectDrawTrack;
pub(crate) use fx_lane_t6::refill_fx_draw_lane;
pub use xmodel_lane_t6::FigureDrawTrack;
pub(crate) use xmodel_lane_t6::{rebuild_figure_draw_track, refresh_xmodel_lane};
