//! Reading the finished weapon table.

use std::collections::BTreeSet;

use super::*;

/// The alias (and the bank it lives in) a sound name resolves to.
fn sound_alias_in_bank<'a>(
    hint: Option<&str>,
    ns: crate::AssetNamespace,
    catalog: &'a crate::SoundCatalog,
) -> Option<(crate::AssetNamespace, &'a str)> {
    let name = hint.filter(|name| !name.is_empty())?;
    let order = catalog.index_in(ns, name)?;
    Some((catalog.namespace_of_alias(order), catalog.name_at(order)?))
}

/// Whether a scope overlay is drawn as a HUD picture rather than a model.
pub fn overlay_name_is_hud_iris(name: &str) -> bool {
    !name.starts_with("mc/")
}

impl WeaponRegistry {
    /// Settle the lookup maps and each weapon's alternate after rows change.
    pub(super) fn rebuild_name_maps(&mut self) {
        self.by_name.clear();
        self.by_namespaced.clear();
        for (id, row) in self.rows.iter().enumerate().skip(1) {
            self.by_namespaced.insert((row.namespace, row.name.clone()), id as u32);
            self.by_name.entry(row.name.clone()).or_insert(id as u32);
        }
        for row in &mut self.rows {
            if let Some(alternate) = &row.alternate_weapon {
                let key = (row.namespace, normalize_weapon_name(alternate));
                row.alternate_index = self.by_namespaced.get(&key).copied().unwrap_or(0);
            }
        }
        // A sight that switches, or a gun that shares its magazine with its
        // alternate, draws from one ammo pool and one clip.
        for id in 1..self.rows.len() {
            let alt = self.rows[id].alternate_index as usize;
            let facts = &self.rows[id].facts;
            if !(facts.optic_switch || facts.shares_alt_magazine) || alt == 0 || alt >= self.rows.len() {
                continue;
            }
            let pool = id.min(alt) as i32;
            for row in [id, alt] {
                self.rows[row].facts.ammo_index = pool;
                self.rows[row].facts.clip_index = pool;
            }
        }
    }

    pub fn len(&self) -> usize {
        self.rows.len().saturating_sub(1)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn world_catalog_identity(&self) -> u64 {
        self.world_catalog_identity
    }

    // Names and ids.

    /// The id of a weapon by name: bare (`ak47`, `ak47_mp`) or as an asset key.
    pub fn resolve_index(&self, name: &str) -> Result<Option<u32>, UnknownWeaponName> {
        if name.is_empty() {
            return Ok(None);
        }
        if name.contains(':') {
            return match crate::AssetKey::parse(name) {
                Ok(key) => self.resolve_key(&key),
                Err(_) => Err(UnknownWeaponName { name: name.to_owned() }),
            };
        }
        let normal = normalize_weapon_name(name);
        match self.by_name.get(&normal) {
            Some(&id) => Ok(Some(id)),
            None => Err(UnknownWeaponName { name: normal }),
        }
    }

    fn resolve_key(&self, key: &crate::AssetKey) -> Result<Option<u32>, UnknownWeaponName> {
        let unknown = || UnknownWeaponName { name: key.display() };
        if key.kind != crate::AssetKind::Weapon {
            return Err(unknown());
        }
        let wanted = (key.namespace, normalize_weapon_name(&key.name));
        self.by_namespaced.get(&wanted).map(|&id| Some(id)).ok_or_else(unknown)
    }

    pub fn namespace_of(&self, index: u32) -> Option<crate::AssetNamespace> {
        if index == 0 {
            return None;
        }
        self.rows.get(index as usize).map(|row| row.namespace)
    }

    pub fn name_of(&self, index: u32) -> &str {
        self.rows.get(index as usize).map_or("", |row| row.name.as_str())
    }

    pub fn script_name_of(&self, index: u32) -> String {
        gsc_weapon_script_name(self.name_of(index))
    }

    pub fn key_of(&self, index: u32) -> Option<crate::AssetKey> {
        let ns = self.namespace_of(index)?;
        let name = self.name_of(index);
        if name.is_empty() {
            return None;
        }
        crate::AssetKey::new(ns, crate::AssetKind::Weapon, gsc_weapon_script_name(name)).ok()
    }

    pub fn alternate_of(&self, index: u32) -> u32 {
        self.rows.get(index as usize).map_or(0, |row| row.alternate_index)
    }

    pub fn dual_wield_weapon_of(&self, index: u32) -> u32 {
        self.linked_weapon(index, |row| row.dual_wield_weapon.as_deref())
    }

    pub fn spawned_grenade_weapon_of(&self, index: u32) -> u32 {
        self.linked_weapon(index, |row| row.spawned_grenade_weapon.as_deref())
    }

    fn linked_weapon(&self, index: u32, name_of: impl Fn(&WeaponRow) -> Option<&str>) -> u32 {
        self.rows
            .get(index as usize)
            .and_then(name_of)
            .and_then(|name| self.resolve_index(name).ok().flatten())
            .unwrap_or(0)
    }

    /// The weapon this one melees with: itself when it is a melee weapon, else
    /// the knife of its game.
    pub fn melee_weapon_of(&self, index: u32) -> u32 {
        let Some(row) = self.rows.get(index as usize) else {
            return index;
        };
        if row.facts.use_as_melee {
            return index;
        }
        ["knife_zm", "knife"]
            .iter()
            .find_map(|knife| {
                self.rows.iter().position(|other| {
                    other.namespace == row.namespace
                        && other.name.strip_suffix("_mp").unwrap_or(&other.name) == *knife
                })
            })
            .map_or(index, |found| found as u32)
    }

    // Tables indexed by weapon id, row 0 included.

    pub fn world_models_table(&self) -> Vec<(String, Vec<String>)> {
        (0..self.rows.len() as u32)
            .map(|id| (self.world_model_of(id).unwrap_or_default().to_owned(), self.hide_tags_of(id).to_vec()))
            .collect()
    }

    pub fn projectile_models_table(&self) -> Vec<String> {
        self.rows
            .iter()
            .map(|row| row.projectile_model.clone().unwrap_or_default())
            .collect()
    }

    pub fn script_names_table(&self) -> Vec<String> {
        (0..self.rows.len() as u32).map(|id| self.script_name_of(id)).collect()
    }

    pub fn runnable_table(&self) -> Vec<bool> {
        (0..self.rows.len() as u32)
            .map(|id| id != 0 && self.configuration_admission(id).is_ok())
            .collect()
    }

    pub fn scales_table(&self) -> Vec<(f32, f32, f32)> {
        self.rows
            .iter()
            .map(|row| {
                let f = &row.facts;
                (f.move_speed_scale, f.ads_move_speed_scale, f.sprint_duration_scale)
            })
            .collect()
    }

    // Facts and timers.

    pub fn facts_of(&self, index: u32) -> Option<WeaponBodyFacts> {
        self.rows.get(index as usize).map(|row| row.facts)
    }

    // Models.

    pub fn gun_xmodel_of(&self, index: u32) -> Option<&str> {
        self.rows.get(index as usize)?.gun_xmodel.as_deref()
    }

    pub fn world_model_of(&self, index: u32) -> Option<&str> {
        self.rows.get(index as usize)?.world_model.as_deref()
    }

    pub fn projectile_model_of(&self, index: u32) -> Option<&str> {
        self.rows.get(index as usize)?.projectile_model.as_deref()
    }

    pub fn hide_tags_of(&self, index: u32) -> &[String] {
        self.rows.get(index as usize).map_or(&[], |row| row.hide_tags.as_slice())
    }

    pub fn gun_xmodel_edge_of(&self, index: u32) -> Option<AssetEdge<FpvMeshSpace>> {
        self.rows.get(index as usize).map(|row| row.gun_xmodel_edge)
    }

    pub fn world_model_edge_of(&self, index: u32) -> Option<AssetEdge<WorldWeaponSpace>> {
        self.rows.get(index as usize).map(|row| row.world_model_edge)
    }

    pub fn attachment_world_model_edges_of(&self, index: u32) -> &[AssetEdge<WorldWeaponSpace>] {
        self.rows
            .get(index as usize)
            .map_or(&[], |row| row.attachment_world_model_edges.as_slice())
    }

    pub fn attachment_world_mounts_of(&self, index: u32) -> &[Option<String>] {
        self.rows
            .get(index as usize)
            .map_or(&[], |row| row.attachment_world_mounts.as_slice())
    }

    /// The loaded world model of a weapon, if `catalog` is the one the edges
    /// were made against.
    pub fn world_model_entry<'a>(
        &self,
        index: u32,
        catalog: &'a crate::WorldWeaponCatalog,
    ) -> Option<&'a crate::WorldWeaponEntry> {
        if self.world_catalog_identity != catalog.identity() {
            return None;
        }
        catalog.get_at(self.world_model_edge_of(index)?.bound_index()?)
    }

    // First-person assemblies.

    pub fn fpv_hands_of(&self, index: u32, axis: bool) -> Option<(&asset_model::FpvHands, crate::FpvMeshIndex)> {
        let (hands, mesh) = self.rows.get(index as usize)?.fpv_hands[usize::from(axis)].as_ref()?;
        Some((hands, *mesh))
    }

    /// Alternates that are held items, with the weapon each was built under.
    pub fn alternate_fpv_pairs(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        self.alternate_fpv.keys().copied()
    }

    pub fn fpv_assemblies_for(&self, index: u32, parent: u32, axis: bool) -> Option<&crate::FpvSideAssemblies> {
        let side = usize::from(axis);
        let held = self.facts_of(index).is_some_and(|f| f.inventory_type == 3);
        let built = if parent != 0 && held {
            self.alternate_fpv.get(&(index, parent))?[side].as_ref()?
        } else {
            self.rows.get(index as usize)?.fpv_assemblies[side].as_ref()?
        };
        built.as_ref().ok()
    }

    /// Why a weapon has no first-person assembly, in words.
    pub fn fpv_assembly_gap_of(&self, index: u32, axis: bool) -> String {
        let Some(row) = self.rows.get(index as usize) else {
            return "no such weapon".to_owned();
        };
        match (&row.fpv_mount_plan, &row.fpv_assemblies[usize::from(axis)]) {
            (None, _) => "unlinked selected models".to_owned(),
            (Some(Err(error)), _) => error.to_string(),
            (_, None) => "effective kit / weapon hands".to_owned(),
            (_, Some(Err(error))) => error.clone(),
            (_, Some(Ok(_))) => "linked".to_owned(),
        }
    }

    pub fn fpv_clip_tracks(&self) -> &crate::FpvClipTracks {
        &self.fpv_clip_tracks
    }

    // Animations.

    pub fn sz_xanims_of(&self, index: u32) -> Option<&[Option<String>; WEAPON_ANIM_SLOTS]> {
        self.rows.get(index as usize).map(|row| &row.sz_xanims)
    }

    pub fn sz_xanim_edges_of(&self, index: u32) -> Option<&[AssetEdge<XAnimSpace>; WEAPON_ANIM_SLOTS]> {
        self.rows.get(index as usize).map(|row| &row.sz_xanim_edges)
    }

    pub fn sz_xanim_right_edges_of(&self, index: u32) -> Option<&[AssetEdge<XAnimSpace>; WEAPON_ANIM_SLOTS]> {
        self.rows.get(index as usize).map(|row| &row.sz_xanim_right_edges)
    }

    pub fn sz_xanim_left_edges_of(&self, index: u32) -> Option<&[AssetEdge<XAnimSpace>; WEAPON_ANIM_SLOTS]> {
        self.rows.get(index as usize).map(|row| &row.sz_xanim_left_edges)
    }

    /// Every animation any weapon plays, once, in order.
    pub fn bound_weapon_xanim_indices(&self) -> Vec<usize> {
        let all: BTreeSet<usize> = self.rows.iter().skip(1).flat_map(|row| row.bound_clips()).collect();
        all.into_iter().collect()
    }

    pub fn notetrack_actions_of(&self, index: u32) -> impl Iterator<Item = (&str, &LinkedNotetrackAction)> {
        self.rows
            .get(index as usize)
            .into_iter()
            .flat_map(|row| row.notetrack_actions.iter())
            .map(|(name, action)| (name.as_str(), action))
    }

    pub fn notetrack_sound_aliases_of(&self, index: u32) -> impl Iterator<Item = &str> {
        self.notetrack_actions_of(index)
            .filter_map(|(_, action)| action.sound_alias.as_deref())
    }

    // Sounds and effects.

    pub fn sounds_of(&self, index: u32) -> Option<&WeaponSoundAliases> {
        self.rows.get(index as usize).map(|row| &row.sounds)
    }

    pub fn combat_fx_of(&self, index: u32) -> Option<&WeaponCombatFx> {
        self.rows.get(index as usize).map(|row| &row.combat_fx)
    }

    pub fn authored_weapon_sound(&self, index: u32, slot: WeaponSoundSlot) -> Option<&str> {
        self.sounds_of(index)?.hint(slot).filter(|name| !name.is_empty())
    }

    pub fn weapon_sound_key<'a>(
        &self,
        index: u32,
        slot: WeaponSoundSlot,
        catalog: &'a crate::SoundCatalog,
    ) -> Option<(crate::AssetNamespace, &'a str)> {
        let ns = self.namespace_of(index).unwrap_or_default();
        sound_alias_in_bank(self.authored_weapon_sound(index, slot), ns, catalog)
    }

    pub fn weapon_sound_alias<'a>(
        &self,
        index: u32,
        slot: WeaponSoundSlot,
        catalog: &'a crate::SoundCatalog,
    ) -> Option<&'a str> {
        self.weapon_sound_key(index, slot, catalog).map(|(_, alias)| alias)
    }

    pub fn bounce_sound_of(&self, index: u32, surf: usize) -> Option<&str> {
        self.sounds_of(index)?.bounce.get(surf)?.as_deref()
    }

    pub fn bounce_sound_alias<'a>(
        &self,
        index: u32,
        surf: usize,
        catalog: &'a crate::SoundCatalog,
    ) -> Option<&'a str> {
        let ns = self.namespace_of(index).unwrap_or_default();
        sound_alias_in_bank(self.bounce_sound_of(index, surf), ns, catalog).map(|(_, alias)| alias)
    }

    pub fn proj_trail_of(&self, index: u32) -> Option<crate::FxName<'_>> {
        let row = self.rows.get(index as usize)?;
        if !row.trail_edge.is_bound() {
            return None;
        }
        Some(crate::FxName::new(row.namespace, row.proj_trail.as_deref()?))
    }

    pub fn proj_ignition_of(&self, index: u32) -> Option<crate::FxName<'_>> {
        let row = self.rows.get(index as usize)?;
        if !row.ignition_edge.is_bound() {
            return None;
        }
        Some(crate::FxName::new(row.namespace, row.proj_ignition.as_deref()?))
    }

    /// Black Ops II projectiles have no beacon effect.
    pub fn proj_beacon_of(&self, _index: u32) -> Option<crate::FxName<'_>> {
        None
    }

    // HUD pictures.

    pub fn reticle_of(&self, index: u32) -> Option<&WeaponReticleAssets> {
        self.rows.get(index as usize).map(|row| &row.reticle)
    }

    pub fn overlay_material_of(&self, index: u32) -> Option<&str> {
        self.rows.get(index as usize)?.overlay_material.as_deref()
    }

    pub fn overlay_image_of(&self, index: u32) -> Option<&str> {
        self.rows.get(index as usize)?.overlay_image.as_deref()
    }

    pub fn overlay_is_hud_iris(&self, index: u32) -> bool {
        self.overlay_material_of(index).is_some_and(overlay_name_is_hud_iris)
    }

    pub fn display_name_key_of(&self, index: u32) -> Option<&str> {
        self.rows.get(index as usize)?.display_name_key.as_deref()
    }

    // Vehicles are not in the table; these answer so callers need no special case.

    pub fn vehicle_turrets(&self) -> Vec<(String, String)> {
        Vec::new()
    }

    pub fn vehicle_compass(&self) -> impl Iterator<Item = (&str, &[String; 2], [i32; 2])> {
        std::iter::empty()
    }

    // Loadouts: families, attachments and what a selection resolves to.

    pub fn weapon_families(&self) -> &crate::WeaponFamilies {
        &self.families
    }

    pub fn resolve_configuration(
        &self,
        selection: &crate::WeaponSelection,
        rules: crate::LoadoutRules,
    ) -> Result<crate::ResolvedConfiguration, crate::ConfigurationRefusal> {
        self.families.resolve(selection, rules, self)
    }

    pub fn describe_configuration(&self, id: u32) -> Option<&crate::WeaponSelection> {
        self.families.describe(id)
    }

    pub fn list_attachment_choices(
        &self,
        selection: &crate::WeaponSelection,
        rules: crate::LoadoutRules,
    ) -> Result<Vec<crate::AttachmentOption>, crate::ConfigurationRefusal> {
        self.families.attachment_options(selection, rules, self)
    }

    /// Attachments to build ahead of time for a selection; none are.
    pub fn prepared_attachments_of(&self, _id: u32) -> &[String] {
        &[]
    }

    pub fn configuration_label(&self, id: u32) -> String {
        match self
            .describe_configuration(id)
            .and_then(|selection| Some((selection.family.as_ref()?, &selection.attachments)))
        {
            Some((family, attachments)) => {
                let mut label = family.short();
                for name in attachments {
                    label.push_str(" +");
                    label.push_str(name);
                }
                label
            }
            None => self.key_of(id).map(|key| key.to_string()).unwrap_or_default(),
        }
    }

    /// Every Black Ops II configuration can be played.
    pub fn configuration_supported(&self, _id: u32) -> bool {
        true
    }

    pub fn configuration_admission(&self, id: u32) -> Result<(), crate::ConfigurationRefusal> {
        match self.dependency_gaps_of(id).first() {
            Some(gap) => Err(crate::ConfigurationRefusal::MissingDependency {
                weapon: self.name_of(id).to_owned(),
                kind: gap.kind,
                name: gap.name.clone(),
            }),
            None => Ok(()),
        }
    }

    /// For each weapon id, the number of the family it can switch within; 0:
    /// none.
    pub fn configuration_transition_groups(&self) -> Vec<u32> {
        let mut keys: Vec<_> = self.families.families().iter().map(|family| family.key.clone()).collect();
        keys.sort_by_key(|key| key.asset_key());
        keys.dedup();
        let groups: HashMap<_, _> = keys
            .into_iter()
            .enumerate()
            .map(|(index, key)| (key, index as u32 + 1))
            .collect();
        let mut result = vec![0; self.rows.len()];
        for id in 1..self.rows.len() as u32 {
            let Some(selection) = self.describe_configuration(id) else {
                continue;
            };
            let Some(key) = selection.family.as_ref() else {
                continue;
            };
            let canonical = self
                .resolve_configuration(selection, crate::LoadoutRules::default())
                .is_ok_and(|resolved| resolved.id == id);
            if canonical {
                result[id as usize] = groups.get(key).copied().unwrap_or(0);
            }
        }
        result
    }

    // What the loaded zones are missing.

    pub fn dependency_gaps(&self) -> Vec<WeaponDependencyGap> {
        (1..self.rows.len() as u32).flat_map(|id| self.dependency_gaps_of(id)).collect()
    }

    fn dependency_gaps_of(&self, id: u32) -> Vec<WeaponDependencyGap> {
        let Some(row) = self.rows.get(id as usize).filter(|_| id != 0) else {
            return Vec::new();
        };
        let wanted_but_unbound = |name: &Option<String>, edge: &AssetEdge<FpvMeshSpace>| {
            name.is_some() && !edge.is_bound()
        };
        let mut wanted: Vec<(bool, &'static str, &Option<String>)> = vec![
            (wanted_but_unbound(&row.gun_xmodel, &row.gun_xmodel_edge), "view model", &row.gun_xmodel),
            (wanted_but_unbound(&row.rocket_model, &row.rocket_model_edge), "FPV rocket model", &row.rocket_model),
            (row.world_model_edge.is_unresolved(), "world model", &row.world_model),
        ];
        for (edge, name) in row.sz_xanim_edges.iter().zip(&row.sz_xanims) {
            wanted.push((edge.is_unresolved(), "anim", name));
        }
        // The off-hand animations only matter to a weapon that is used in pairs.
        let dual = !row.facts.no_dual_wield
            && row.sz_xanims_right[weap_anim::IDLE].as_deref().is_some_and(|name| !name.is_empty());
        for (edge, name) in row.sz_xanim_right_edges.iter().zip(&row.sz_xanims_right) {
            wanted.push((dual && edge.is_unresolved(), "right anim", name));
        }
        for (edge, name) in row.sz_xanim_left_edges.iter().zip(&row.sz_xanims_left) {
            wanted.push((dual && edge.is_unresolved(), "left anim", name));
        }
        let gap = |kind: &'static str, name: String| WeaponDependencyGap { id, kind, name };
        let mut gaps: Vec<_> = wanted
            .into_iter()
            .filter(|(missing, ..)| *missing)
            .map(|(_, kind, name)| gap(kind, name.clone().unwrap_or_default()))
            .collect();

        if let Some(Err(error)) = &row.fpv_mount_plan {
            gaps.push(gap("FPV mount", error.to_string()));
        }
        for (side, built) in row.fpv_assemblies.iter().enumerate() {
            if let Some(Err(error)) = built {
                let who = if side == 0 { "allies" } else { "axis" };
                gaps.push(gap("FPV skeleton", format!("{who}: {error}")));
            }
        }
        if row.gun_xmodel_edge.is_bound() {
            for (side, hands) in row.fpv_hands.iter().enumerate() {
                if hands.is_none() {
                    gaps.push(gap("FPV hands layout", if side == 0 { "allies" } else { "axis" }.to_owned()));
                }
            }
        }
        for (name, edge) in row.attachment_view_models.iter().zip(&row.attachment_view_model_edges) {
            if !edge.is_bound() {
                gaps.push(gap("FPV attachment model", name.clone()));
            }
        }
        for (i, (name, edge)) in row
            .attachment_world_models
            .iter()
            .zip(&row.attachment_world_model_edges)
            .enumerate()
        {
            if !edge.is_bound() {
                gaps.push(gap("world attachment model", name.clone()));
            } else if row.attachment_world_mounts.get(i).is_none_or(Option::is_none) {
                gaps.push(gap("world attachment mount", name.clone()));
            }
        }
        let counts = |kind: &'static str, names: usize, links: usize, gaps: &mut Vec<WeaponDependencyGap>| {
            if names != links {
                gaps.push(gap(kind, format!("{names} names, {links} links")));
            }
        };
        counts(
            "world attachment bindings",
            row.attachment_world_models.len(),
            row.attachment_world_model_edges.len(),
            &mut gaps,
        );
        counts(
            "FPV attachment bindings",
            row.attachment_view_models.len(),
            row.attachment_view_model_edges.len(),
            &mut gaps,
        );
        gaps
    }
}

impl crate::families::FamilyContent for WeaponRegistry {
    fn lookup(&self, namespace: crate::AssetNamespace, name: &str) -> Option<u32> {
        self.by_namespaced.get(&(namespace, normalize_weapon_name(name))).copied()
    }

    fn offhand_class(&self, id: u32) -> i32 {
        self.facts_of(id).map_or(0, |facts| facts.offhand_class)
    }

    fn admission(&self, id: u32) -> Result<(), crate::ConfigurationRefusal> {
        self.configuration_admission(id)
    }

    fn names_in(&self, namespace: crate::AssetNamespace) -> Vec<(u32, String)> {
        (1..=self.len() as u32)
            .filter(|&id| self.namespace_of(id) == Some(namespace))
            .map(|id| (id, normalize_weapon_name(self.name_of(id))))
            .collect()
    }
}
