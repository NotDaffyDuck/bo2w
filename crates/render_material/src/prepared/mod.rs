//! Materials prepared at load time: per-pass shader ports, state bits and the
//! split of each pass's arguments into material-local and engine-supplied.

mod args;
mod banks;
mod table;

pub use args::RunnableParamSource;
pub use args::PackedCodeParam;
pub use args::PreparedParamStrips;
pub use args::PreparedParamSpan;
pub use banks::{
    LocalSamplerBanks, CompactSamplerSlot, LocalSamplerBundle, encode_local_register_banks,
};
pub use banks::pack_local_samplers;
pub use table::{
    ResolvedTableTally, ResolvedShadingRecipe, VertexVariant,
};
pub use table::AcceptedPortSummary;
pub use table::VisualPassStateBits;
pub use table::StagedMaterial;
pub use table::StagedMaterialTable;
pub use table::ResolvedDrawPass;
