//! Typed positions in the load-time catalogs. Each catalog gets its own
//! marker type so a material position can never be handed to code that
//! expects an fx position.

use std::marker::PhantomData;

/// Marker for one catalog's index space.
pub trait KeyDomain: Copy + Eq + std::hash::Hash + std::fmt::Debug + 'static {}

/// Position of an entry in the catalog named by `S`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CatalogSlot<S: KeyDomain> {
    seq: usize,
    space: PhantomData<S>,
}

impl<S: KeyDomain> CatalogSlot<S> {
    pub fn from_sequence(seq: usize) -> Self {
        Self { seq, space: PhantomData }
    }

    pub fn order(self) -> usize {
        self.seq
    }
}

macro_rules! catalog_spaces {
    ($($space:ident => $index:ident),* $(,)?) => {
        $(
            #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
            pub enum $space {}
            impl KeyDomain for $space {}
            pub type $index = CatalogSlot<$space>;
        )*
    };
}

catalog_spaces! {
    SurfaceShaderSpace => SurfaceMaterialId,
    TracerRange => TracerSlot,
    FxRange => FxSlot,
    FxModelRange => FxModelSlot,
    LoadedSoundRange => LoadedSoundSlot,
    SoundAliasRange => SoundAliasSlot,
    XAnimRange => XAnimSlot,
    MapXModelRange => MapXModelSlot,
    ProjectileModelRange => ProjectileModelSlot,
    FpvMeshRange => FpvMeshSlot,
    WorldWeaponRange => WorldWeaponSlot,
}

/// A material's position inside one zone walk, before the walk's materials
/// are merged into the shared catalog.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TraversalMaterialSlot(usize);

impl TraversalMaterialSlot {
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
        assert_eq!(SurfaceMaterialId::from_sequence(7).order(), 7);
        assert!(FxSlot::from_sequence(1) < FxSlot::from_sequence(2));
        assert_eq!(TraversalMaterialSlot::from_walk(3).get(), 3);
    }
}
