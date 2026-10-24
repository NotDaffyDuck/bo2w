//! Materials prepared at load time: per-pass shader ports, state bits and the
//! split of each pass's arguments into material-local and engine-supplied.

mod args;
mod banks;
mod table;

pub use args::{ExecutableArgumentBinding, PackedCodeArg, PreparedArgBelts, PreparedArgSlice};
pub use banks::{
    PackedLocalBanks, PackedLocalSamplerLane, PackedLocalSamplers, pack_local_banks,
    pack_local_samplers,
};
pub use table::{
    AdmittedPortFacts, GfxPassStateBits, PreparedMaterial, PreparedMaterialTable, PreparedPass,
    PreparedTableCensus, PreparedTechnique,
};
