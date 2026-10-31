//! Shared asset identity: text keys, typed catalog positions, names as zones
//! store them, links between assets, and which zone owns what.

mod asset_ref;
mod edge;
mod key;
mod space;
mod t6_draw;
mod t6_wind;
mod zone_tag;

pub use asset_ref::{AssetPointer, AssetPointerTally};
pub use edge::{linked_zone_names, AssetDependencyLink, DependencyCause, AttachedDestination};
pub use key::{AssetIdentity, AssetVariety, AssetCategoryScope, KeyError, MaterialKey};
pub use space::{
    CatalogSlot, FpvMeshSlot, FpvMeshRange, FxSlot, FxModelSlot, FxModelRange, FxRange,
    KeyDomain, LoadedSoundSlot, LoadedSoundRange, MapXModelSlot, MapXModelRange, SurfaceMaterialId,
    SurfaceShaderSpace, ProjectileModelSlot, ProjectileModelRange, SoundAliasSlot, SoundAliasRange,
    TracerSlot, TracerRange, TraversalMaterialSlot,
    WorldWeaponSlot, WorldWeaponRange, XAnimSlot, XAnimRange,
};
pub use t6_draw::{
    t6_aniso, t6_fast, t6_set_aniso, t6_set_fast, t6_water_kind, T6Blend, T6Cull, T6Draw, T6Layers, T6StateBits,
};
pub use t6_wind::{t6_map_wind, t6_wind_table};
pub use zone_tag::{ZoneTitleKind, ZoneCustodian};
