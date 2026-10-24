//! Shared asset identity: text keys, typed catalog positions, names as zones
//! store them, links between assets, and which zone owns what.

mod asset_ref;
mod edge;
mod key;
mod space;
mod t6_draw;
mod t6_wind;
mod zone;

pub use asset_ref::{AssetRef, AssetRefCensus};
pub use edge::{bound_zone_names, AssetEdge, AssetEdgeReason, BoundTarget};
pub use key::{AssetKey, AssetKind, AssetNamespace, KeyError, MaterialKey};
pub use space::{
    CatalogIndex, FpvMeshIndex, FpvMeshSpace, FxIndex, FxModelIndex, FxModelSpace, FxSpace,
    IndexSpace, LoadedSoundIndex, LoadedSoundSpace, MapXModelIndex, MapXModelSpace, MaterialIndex,
    MaterialSpace, ProjectileModelIndex, ProjectileModelSpace, SoundAliasIndex, SoundAliasSpace,
    TracerIndex, TracerSpace, WalkLocalMaterialIndex,
    WorldWeaponIndex, WorldWeaponSpace, XAnimIndex, XAnimSpace,
};
pub use t6_draw::{
    t6_fast, t6_set_fast, t6_water_kind, T6Blend, T6Cull, T6Draw, T6Layers, T6StateBits,
};
pub use t6_wind::{t6_map_wind, t6_wind_table};
pub use zone::{ZoneGame, ZoneOwner};
