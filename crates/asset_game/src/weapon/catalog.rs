use super::*;

impl ArmoryRoster {
    /// The namespace the weapons captured from now on belong to.
    pub fn set_walk_scope(&mut self, scope: crate::AssetCategoryScope) {
        self.into_ns = scope;
    }

    pub fn put(&mut self, entry: RosterWeapon) {
        self.captured.push(entry);
    }

    pub fn len(&self) -> usize {
        self.captured.len()
    }

    pub fn is_empty(&self) -> bool {
        self.captured.is_empty()
    }

    pub fn into_build(self) -> WeaponBuild {
        WeaponBuild::build_from_index(self.captured)
    }
}
