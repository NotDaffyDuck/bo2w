//! Typed positions in the load-time catalogs. Each catalog gets its own
//! marker type so a material position can never be handed to code that
//! expects an fx position.

use std::marker::PhantomData;

/// Marker for one catalog's index space.
pub trait IndexSpace: Copy + Eq + std::hash::Hash + std::fmt::Debug + 'static {}

/// Position of an entry in the catalog named by `S`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CatalogIndex<S: IndexSpace> {
    slot: usize,
    space: PhantomData<S>,
}

impl<S: IndexSpace> CatalogIndex<S> {
    pub fn from_order(slot: usize) -> Self {
        Self { slot, space: PhantomData }
    }

    pub fn order(self) -> usize {
        self.slot
    }
}

macro_rules! catalog_spaces {
    ($($space:ident => $index:ident),* $(,)?) => {
        $(
            #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
            pub enum $space {}
            impl IndexSpace for $space {}
            pub type $index = CatalogIndex<$space>;
        )*
    };
}

catalog_spaces! {
    MaterialSpace => MaterialIndex,
    TracerSpace => TracerIndex,
    FxSpace => FxIndex,
    FxModelSpace => FxModelIndex,
    LoadedSoundSpace => LoadedSoundIndex,
    SoundAliasSpace => SoundAliasIndex,
    XAnimSpace => XAnimIndex,
    MapXModelSpace => MapXModelIndex,
    ProjectileModelSpace => ProjectileModelIndex,
    FpvMeshSpace => FpvMeshIndex,
    WorldWeaponSpace => WorldWeaponIndex,
}

/// A material's position inside one zone walk, before the walk's materials
/// are merged into the shared catalog.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WalkLocalMaterialIndex(usize);

impl WalkLocalMaterialIndex {
    pub fn from_walk(slot: usize) -> Self {
        Self(slot)
    }

    pub fn get(self) -> usize {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_keeps_its_slot() {
        assert_eq!(MaterialIndex::from_order(7).order(), 7);
        assert!(FxIndex::from_order(1) < FxIndex::from_order(2));
        assert_eq!(WalkLocalMaterialIndex::from_walk(3).get(), 3);
    }
}
