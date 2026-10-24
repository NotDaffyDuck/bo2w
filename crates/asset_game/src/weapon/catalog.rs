use super::*;

impl WeaponCatalog {
    /// The namespace the weapons captured from now on belong to.
    pub fn set_capture_ns(&mut self, ns: crate::AssetNamespace) {
        self.capture_ns = ns;
    }

    pub fn push(&mut self, entry: CatalogWeapon) {
        self.entries.push(entry);
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn into_build(self) -> WeaponBuild {
        WeaponBuild::from_catalog(self.entries)
    }
}
