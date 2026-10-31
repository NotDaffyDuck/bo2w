//! Reading the finished weapon table.

use std::collections::BTreeSet;

use super::*;

/// The alias (and the bank it lives in) a sound name resolves to.
fn sound_alias_in_bank<'a>(
    bank_hint: Option<&str>,
    space: crate::AssetCategoryScope,
    bank: &'a crate::AudioRoster,
) -> Option<(crate::AssetCategoryScope, &'a str)> {
    let slot = bank.slot_in(space, bank_hint.filter(|text| !text.is_empty())?)?;
    let alias = bank.label_at(slot)?;
    Some((bank.space_of_slot(slot), alias))
}

/// Whether a scope overlay is drawn as a HUD picture rather than a model.
pub fn overlay_name_is_hud_iris(caption: &str) -> bool {
    !caption.starts_with("mc/")
}

fn side_label(side: usize) -> &'static str {
    ["allies", "axis"][side.min(1)]
}

/// Collects what one weapon is missing, in the order a caller should see it.
struct GapLog {
    owner: u32,
    found: Vec<WeaponDependencyGap>,
}

impl GapLog {
    fn new(owner: u32) -> Self {
        GapLog { owner, found: vec![] }
    }

    fn plus(&mut self, gap_kind: &'static str, what: impl Into<String>) {
        let gap = WeaponDependencyGap { id: self.owner, name: what.into(), kind: gap_kind };
        self.found.push(gap);
    }

    fn add_labelled(&mut self, named_kind: &'static str, caption: &Option<String>) {
        self.plus(named_kind, caption.clone().unwrap_or_default());
    }

    /// One gap per animation slot whose link did not resolve.
    fn add_unresolved_anims(
        &mut self,
        what: &'static str,
        edges: &[AssetDependencyLink<XAnimRange>],
        labels: &[Option<String>],
    ) {
        let missing = edges.iter().zip(labels).filter(|pair| pair.0.is_unlinked_ref());
        missing.for_each(|(_, label)| self.add_labelled(what, label));
    }

    /// Two lists that should be the same length, and are not.
    fn add_if_uneven(&mut self, list_kind: &'static str, name_count: usize, links: usize) {
        if name_count != links {
            self.plus(list_kind, format!("{name_count} names, {links} links"));
        }
    }
}

impl WeaponLedger {
    fn entry_at(&self, gun_catalog_no: u32) -> Option<&WeaponEntry> {
        self.records.get(gun_catalog_no as usize)
    }

    /// One value per table row, row 0 included.
    fn column<T>(&self, mut make: impl FnMut(u32, &WeaponEntry) -> T) -> Vec<T> {
        let mut rows = Vec::with_capacity(self.records.len());
        for (id, rec) in (0u32..).zip(&self.records) {
            rows.push(make(id, rec));
        }
        rows
    }

    /// Settle the lookup maps and each weapon's alternate after rows change.
    pub(super) fn rebuild_name_maps(&mut self) {
        self.index_names();
        self.link_partners();
        self.pool_shared_ammo();
    }

    fn index_names(&mut self) {
        self.slot_for_label.clear();
        self.index_by_ns_name.clear();
        for (rec, id) in self.records.iter().zip(0u32..).skip(1) {
            self.index_by_ns_name
                .insert((rec.source_ns, rec.name.clone()), id);
            self.slot_for_label.entry(rec.name.clone()).or_insert(id);
        }
    }

    fn link_partners(&mut self) {
        let known = &self.index_by_ns_name;
        for rec in self.records.iter_mut() {
            let Some(partner) = rec.alt.0.as_deref() else {
                continue;
            };
            rec.alt.1 = known
                .get(&(rec.source_ns, weapon_key(partner)))
                .copied()
                .unwrap_or(0);
        }
    }

    /// The alternate a record shares its ammo pool with, if it has one in range.
    fn shared_pool_partner(&self, id: usize, count: usize) -> Option<(usize, usize)> {
        let rec = &self.records[id];
        let alt = rec.alt.1 as usize;
        let wants_pool = rec.body.optic_switch || rec.body.shares_alt_magazine;
        if !wants_pool {
            return None;
        }
        (1..count).contains(&alt).then_some((id, alt))
    }

    /// A sight that switches, or a gun that shares its magazine with its
    /// alternate, draws from one ammo pool and one clip.
    fn pool_shared_ammo(&mut self) {
        let count = self.records.len();
        let pairs: Vec<(usize, usize)> = (1..count)
            .filter_map(|id| self.shared_pool_partner(id, count))
            .collect();
        for (id, alt) in pairs {
            let pool = id.min(alt) as i32;
            for slot in [id, alt] {
                let facts = &mut self.records[slot].body;
                facts.ammo_type_slot = pool;
                facts.motion_clip_slot = pool;
            }
        }
    }

    pub fn len(&self) -> usize {
        self.records.len().saturating_sub(1)
    }

    pub fn is_empty(&self) -> bool {
        self.records.len() <= 1
    }

    pub fn epoch(&self) -> u64 {
        let Self { epoch_no, .. } = self;
        *epoch_no
    }

    pub fn world_catalog_identity(&self) -> u64 {
        self.world_catalog_identity
    }

    // Names and ids.

    /// The id of a weapon by name: bare (`ak47`, `ak47_mp`) or as an asset key.
    pub fn follow_slot_link(&self, label: &str) -> Result<Option<u32>, UnrecognizedArmoryLabel> {
        if label.is_empty() {
            return Ok(None);
        }
        if !label.contains(':') {
            let plain = weapon_key(label);
            return self
                .slot_for_label
                .get(&plain)
                .map(|&id| Some(id))
                .ok_or(UnrecognizedArmoryLabel { name: plain });
        }
        let parsed = crate::AssetIdentity::parse(label).map_err(|_| UnrecognizedArmoryLabel {
            name: label.to_owned(),
        })?;
        self.resolve_key(&parsed)
    }

    fn resolve_key(&self, lookup: &crate::AssetIdentity) -> Result<Option<u32>, UnrecognizedArmoryLabel> {
        let key = (lookup.namespace, weapon_key(&lookup.name));
        let found = (lookup.kind == crate::AssetVariety::Weapon)
            .then(|| self.index_by_ns_name.get(&key).copied())
            .flatten();
        found.map(Some).ok_or_else(|| UnrecognizedArmoryLabel { name: lookup.display() })
    }

    pub fn scope_for(&self, gun_catalog_no: u32) -> Option<crate::AssetCategoryScope> {
        self.entry_at(gun_catalog_no)
            .filter(|_| gun_catalog_no != 0)
            .map(|rec| rec.source_ns)
    }

    pub fn label_for(&self, gun_catalog_no: u32) -> &str {
        self.entry_at(gun_catalog_no).map_or("", |rec| rec.name.as_str())
    }

    pub fn gsc_name_for(&self, gun_catalog_no: u32) -> String {
        gsc_weapon_script_name(self.label_for(gun_catalog_no))
    }

    pub fn lookup_key_for(&self, gun_catalog_no: u32) -> Option<crate::AssetIdentity> {
        let label = Some(self.label_for(gun_catalog_no)).filter(|text| !text.is_empty())?;
        let space = self.scope_for(gun_catalog_no)?;
        crate::AssetIdentity::new(
            space,
            crate::AssetVariety::Weapon,
            gsc_weapon_script_name(label),
        )
        .ok()
    }

    pub fn alternate_of(&self, gun_catalog_no: u32) -> u32 {
        self.entry_at(gun_catalog_no).map_or(0, |rec| rec.alt.1)
    }

    pub fn dual_wield_weapon_of(&self, gun_catalog_no: u32) -> u32 {
        self.linked_weapon(gun_catalog_no, |rec| rec.dual_wield.as_deref())
    }

    pub fn spawned_grenade_weapon_of(&self, gun_catalog_no: u32) -> u32 {
        self.linked_weapon(gun_catalog_no, |rec| rec.spawned_grenade.as_deref())
    }

    fn linked_weapon(&self, gun_catalog_no: u32, name_of: impl Fn(&WeaponEntry) -> Option<&str>) -> u32 {
        let target = self.entry_at(gun_catalog_no).and_then(name_of);
        let resolved = target.map(|label| self.follow_slot_link(label));
        resolved.and_then(Result::ok).flatten().unwrap_or(0)
    }

    /// The weapon this one melees with: itself when it is a melee weapon, else
    /// the knife of its game.
    pub fn melee_weapon_of(&self, gun_catalog_no: u32) -> u32 {
        let Some(rec) = self.entry_at(gun_catalog_no) else {
            return gun_catalog_no;
        };
        if rec.body.use_as_melee {
            return gun_catalog_no;
        }
        let stem = |other: &WeaponEntry| {
            let full = other.name.as_str();
            full.strip_suffix("_mp").map_or_else(|| full.to_owned(), str::to_owned)
        };
        for knife in ["knife_zm", "knife"] {
            let hit = self
                .records
                .iter()
                .position(|other| other.source_ns == rec.source_ns && stem(other) == knife);
            if let Some(found) = hit {
                return found as u32;
            }
        }
        gun_catalog_no
    }

    // Tables indexed by weapon id, row 0 included.

    pub fn world_models_table(&self) -> Vec<(String, Vec<String>)> {
        self.column(|_, rec| {
            let model = rec.world.gun.caption.clone().unwrap_or_default();
            (model, rec.hidden.clone())
        })
    }

    /// bo2zm: each weapon's world parts (see `RosterWeapon::world_parts`):
    /// (model, tag, offset, rotation), the tag never empty.
    pub fn world_parts_table(&self) -> Vec<Vec<(String, String, [f32; 3], [f32; 3])>> {
        self.column(|_, rec| {
            rec.world
                .pieces
                .iter()
                .map(|(mount_mesh, at)| {
                    let tag = if at.tag.is_empty() { "tag_weapon" } else { at.tag.as_str() };
                    (mount_mesh.clone(), tag.to_owned(), at.offset, at.rotation)
                })
                .collect()
        })
    }

    /// Each weapon's display name key (`ZMWEAPON_...`), empty if none.
    pub fn display_names_table(&self) -> Vec<String> {
        self.column(|_, rec| rec.display_key.clone().unwrap_or_default())
    }

    pub fn projectile_models_table(&self) -> Vec<String> {
        self.column(|_, rec| rec.world.shot.clone().unwrap_or_default())
    }

    pub fn script_label_sheet(&self) -> Vec<String> {
        self.column(|id, _| self.gsc_name_for(id))
    }

    pub fn runnable_table(&self) -> Vec<bool> {
        self.column(|id, _| id != 0 && self.configuration_admission(id).is_ok())
    }

    pub fn multiplier_sheet(&self) -> Vec<(f32, f32, f32)> {
        self.column(|_, rec| {
            let facts = &rec.body;
            (
                facts.walk_factor,
                facts.aimed_walk_factor,
                facts.run_time_multiplier,
            )
        })
    }

    // Facts and timers.

    pub fn summary_for(&self, gun_catalog_no: u32) -> Option<WeaponChassisSummary> {
        self.entry_at(gun_catalog_no).map(|rec| rec.body)
    }

    // Models.

    pub fn view_mesh_for(&self, gun_catalog_no: u32) -> Option<&str> {
        self.entry_at(gun_catalog_no)?.view.gun.caption.as_deref()
    }

    pub fn world_mesh_for(&self, gun_catalog_no: u32) -> Option<&str> {
        self.entry_at(gun_catalog_no)?.world.gun.caption.as_deref()
    }

    pub fn projectile_mesh_for(&self, gun_catalog_no: u32) -> Option<&str> {
        self.entry_at(gun_catalog_no)?.world.shot.as_deref()
    }

    pub fn hidden_tags_for(&self, gun_catalog_no: u32) -> &[String] {
        self.entry_at(gun_catalog_no).map_or(&[], |rec| &rec.hidden[..])
    }

    pub fn gun_mesh_link_for(&self, gun_catalog_no: u32) -> Option<AssetDependencyLink<FpvMeshRange>> {
        self.entry_at(gun_catalog_no).map(|rec| rec.view.gun.resolved)
    }

    pub fn world_mesh_link_for(&self, gun_catalog_no: u32) -> Option<AssetDependencyLink<WorldWeaponRange>> {
        self.entry_at(gun_catalog_no).map(|rec| rec.world.gun.resolved)
    }

    pub fn attachment_world_model_edges_of(&self, wid: u32) -> &[AssetDependencyLink<WorldWeaponRange>] {
        let edges = self.entry_at(wid).map(|rec| &rec.world.attached.edges);
        edges.map_or(&[][..], Vec::as_slice)
    }

    pub fn attachment_world_mounts_of(&self, gun_catalog_no: u32) -> &[Option<String>] {
        self.entry_at(gun_catalog_no).map_or(&[], |rec| &rec.world.mounts[..])
    }

    /// The loaded world model of a weapon, if `catalog` is the one the edges
    /// were made against.
    pub fn world_model_entry<'a>(&self, wid: u32, bank: &'a crate::LevelWeaponRoster) -> Option<&'a crate::LevelWeaponRecord> {
        let slot = self.world_mesh_link_for(wid)?.extent_position()?;
        if self.world_catalog_identity != bank.identity() {
            return None;
        }
        bank.at_row(slot)
    }

    // First-person assemblies.

    pub fn fpv_hands_of(
        &self,
        wid: u32,
        along: bool,
    ) -> Option<(&asset_model::FirstPersonArms, crate::FpvMeshSlot)> {
        let (hands, mesh) = self.entry_at(wid)?.view.hands.get(usize::from(along))?.as_ref()?;
        Some((hands, *mesh))
    }

    /// Alternates that are held items, with the weapon each was built under.
    pub fn alternate_fpv_pairs(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        self.alternate_fpv.keys().copied()
    }

    pub fn fpv_assemblies_for(
        &self,
        wid: u32,
        owner: u32,
        along: bool,
    ) -> Option<&crate::FpvSideAssemblies> {
        let side = usize::from(along);
        let held_alt = owner != 0 && self.summary_for(wid).map(|f| f.carry_slot_kind) == Some(3);
        let cell = match held_alt {
            true => &self.alternate_fpv.get(&(wid, owner))?[side],
            false => &self.entry_at(wid)?.view.sides[side],
        };
        cell.as_ref().and_then(|built| built.as_ref().ok())
    }

    /// Why a weapon has no first-person assembly, in words.
    pub fn fpv_assembly_gap_of(&self, gun_catalog_no: u32, along: bool) -> String {
        let Some(rec) = self.entry_at(gun_catalog_no) else {
            return String::from("no such weapon");
        };
        let Some(mount) = &rec.view.plan_hip else {
            return String::from("unlinked selected models");
        };
        if let Err(error) = mount {
            return error.to_string();
        }
        match &rec.view.sides[usize::from(along)] {
            None => String::from("effective kit / weapon hands"),
            Some(Err(error)) => error.clone(),
            Some(Ok(_)) => String::from("linked"),
        }
    }

    pub fn fpv_clip_tracks(&self) -> &crate::FpvClipTracks {
        &self.fpv_clip_tracks
    }

    // Animations.

    pub fn clips_for(&self, gun_catalog_no: u32) -> Option<&[Option<String>; WEAPON_ANIM_SLOTS]> {
        self.entry_at(gun_catalog_no).map(|rec| &rec.anims.names[ANIM_SZ])
    }

    fn anim_edge_set(&self, wid: u32, set: usize) -> Option<&[AssetDependencyLink<XAnimRange>; WEAPON_ANIM_SLOTS]> {
        Some(&self.entry_at(wid)?.anims.edges[set])
    }

    pub fn sz_xanim_edges_of(&self, wid: u32) -> Option<&[AssetDependencyLink<XAnimRange>; WEAPON_ANIM_SLOTS]> {
        self.anim_edge_set(wid, ANIM_SZ)
    }

    pub fn sz_xanim_right_edges_of(&self, wid: u32) -> Option<&[AssetDependencyLink<XAnimRange>; WEAPON_ANIM_SLOTS]> {
        self.anim_edge_set(wid, ANIM_RIGHT)
    }

    pub fn sz_xanim_left_edges_of(&self, wid: u32) -> Option<&[AssetDependencyLink<XAnimRange>; WEAPON_ANIM_SLOTS]> {
        self.anim_edge_set(wid, ANIM_LEFT)
    }

    /// Every animation any weapon plays, once, in order.
    pub fn bound_weapon_xanim_indices(&self) -> Vec<usize> {
        let clips: BTreeSet<usize> = self.records.iter().skip(1).flat_map(|rec| rec.bound_clips()).collect();
        clips.into_iter().collect()
    }

    pub fn notetrack_actions_of(&self, wid: u32) -> impl Iterator<Item = (&str, &LinkedNotetrackAction)> {
        let tracks = self.entry_at(wid).into_iter().flat_map(|rec| &rec.notetracks);
        tracks.map(|(label, action)| (label.as_str(), action))
    }

    pub fn notetrack_sound_aliases_of(&self, gun_catalog_no: u32) -> impl Iterator<Item = &str> {
        self.notetrack_actions_of(gun_catalog_no)
            .filter_map(|(_, action)| action.sound_alias.as_deref())
    }

    // Sounds and effects.

    pub fn audio_for(&self, gun_catalog_no: u32) -> Option<&WeaponAudioNicknames> {
        self.entry_at(gun_catalog_no).map(|rec| &rec.aliases)
    }

    pub fn effects_for(&self, gun_catalog_no: u32) -> Option<&WeaponBattleEffects> {
        self.entry_at(gun_catalog_no).map(|rec| &rec.fx)
    }

    pub fn authored_weapon_sound(&self, gun_catalog_no: u32, which: WeaponAudioRole) -> Option<&str> {
        let hint = self.audio_for(gun_catalog_no)?.hint(which)?;
        (!hint.is_empty()).then_some(hint)
    }

    pub fn weapon_sound_key<'a>(&self, wid: u32, which: WeaponAudioRole, bank: &'a crate::AudioRoster) -> Option<(crate::AssetCategoryScope, &'a str)> {
        let space = self.scope_for(wid).unwrap_or_default();
        sound_alias_in_bank(self.authored_weapon_sound(wid, which), space, bank)
    }

    pub fn weapon_sound_alias<'a>(&self, wid: u32, which: WeaponAudioRole, bank: &'a crate::AudioRoster) -> Option<&'a str> {
        self.weapon_sound_key(wid, which, bank).map(|(_, alias)| alias)
    }

    pub fn bounce_cue_for(&self, gun_catalog_no: u32, surface_kind: usize) -> Option<&str> {
        let sounds = self.audio_for(gun_catalog_no)?;
        sounds.ricochet.get(surface_kind)?.as_deref()
    }

    pub fn bounce_sound_alias<'a>(&self, wid: u32, surface_kind: usize, bank: &'a crate::AudioRoster) -> Option<&'a str> {
        let space = self.scope_for(wid).unwrap_or_default();
        let hit = sound_alias_in_bank(self.bounce_cue_for(wid, surface_kind), space, bank);
        hit.map(|(_, alias)| alias)
    }

    /// An effect name, only when its link to the effect table is bound.
    fn bound_fx_name(
        &self,
        wid: u32,
        pick: impl Fn(&WeaponEntry) -> (bool, Option<&str>),
    ) -> Option<crate::FxName<'_>> {
        let rec = self.entry_at(wid)?;
        let (linked, label) = pick(rec);
        label
            .filter(|_| linked)
            .map(|text| crate::FxName::new(rec.source_ns, text))
    }

    pub fn shot_wake_for(&self, gun_catalog_no: u32) -> Option<crate::FxName<'_>> {
        self.bound_fx_name(gun_catalog_no, |rec| {
            (rec.trail.resolved.is_attached(), rec.trail.caption.as_deref())
        })
    }

    pub fn shot_ignite_for(&self, gun_catalog_no: u32) -> Option<crate::FxName<'_>> {
        self.bound_fx_name(gun_catalog_no, |rec| {
            (rec.ignition.resolved.is_attached(), rec.ignition.caption.as_deref())
        })
    }

    /// Black Ops II projectiles have no beacon effect.
    pub fn shot_light_for(&self, _index: u32) -> Option<crate::FxName<'_>> {
        None
    }

    // HUD pictures.

    pub fn crosshair_for(&self, gun_catalog_no: u32) -> Option<&WeaponCrosshairResources> {
        self.entry_at(gun_catalog_no).map(|rec| &rec.sight)
    }

    pub fn scope_surface_for(&self, gun_catalog_no: u32) -> Option<&str> {
        self.entry_at(gun_catalog_no)?.hud.scope.caption.as_deref()
    }

    pub fn scope_picture_for(&self, gun_catalog_no: u32) -> Option<&str> {
        self.entry_at(gun_catalog_no)?.hud.scope_image.as_deref()
    }

    pub fn scope_is_iris(&self, gun_catalog_no: u32) -> bool {
        matches!(self.scope_surface_for(gun_catalog_no), Some(name) if overlay_name_is_hud_iris(name))
    }

    pub fn title_token_for(&self, gun_catalog_no: u32) -> Option<&str> {
        self.entry_at(gun_catalog_no)?.display_key.as_deref()
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
        self.families.settle(selection, rules, self)
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
        let described = self.describe_configuration(id);
        if let Some((family, extras)) =
            described.and_then(|sel| Some((sel.family.as_ref()?, &sel.attachments)))
        {
            let mut text = family.short();
            for extra in extras {
                text += &format!(" +{extra}");
            }
            return text;
        }
        self.lookup_key_for(id)
            .map(|lookup| lookup.to_string())
            .unwrap_or_default()
    }

    /// Every Black Ops II configuration can be played.
    pub fn setup_known(&self, _id: u32) -> bool {
        true
    }

    pub fn configuration_admission(&self, id: u32) -> Result<(), crate::ConfigurationRefusal> {
        let Some(gap) = self.dependency_gaps_of(id).into_iter().next() else {
            return Ok(());
        };
        Err(crate::ConfigurationRefusal::MissingDependency {
            gun_name: self.label_for(id).to_owned(),
            gap_kind: gap.kind,
            name: gap.name,
        })
    }

    /// For each weapon id, the number of the family it can switch within; 0:
    /// none.
    pub fn configuration_transition_groups(&self) -> Vec<u32> {
        let mut keys: Vec<_> = self
            .families
            .families()
            .iter()
            .map(|family| family.key.clone())
            .collect();
        keys.sort_by_key(|lookup| lookup.asset_key());
        keys.dedup();
        let number_of: HashMap<_, _> = keys.into_iter().zip(1u32..).collect();
        self.column(|id, _| {
            let group = || {
                let selection = self.describe_configuration(id).filter(|_| id != 0)?;
                let family = selection.family.as_ref()?;
                let resolved =
                    self.resolve_configuration(selection, crate::LoadoutRules::default());
                resolved.ok().filter(|chosen| chosen.id == id)?;
                number_of.get(family).copied()
            };
            group().unwrap_or(0)
        })
    }

    // What the loaded zones are missing.

    pub fn dependency_gaps(&self) -> Vec<WeaponDependencyGap> {
        (1..self.records.len() as u32)
            .flat_map(|id| self.dependency_gaps_of(id))
            .collect()
    }

    fn dependency_gaps_of(&self, id: u32) -> Vec<WeaponDependencyGap> {
        let Some(rec) = self.entry_at(id).filter(|_| id != 0) else {
            return vec![];
        };
        let mut log = GapLog::new(id);
        let (view, world, anims) = (&rec.view, &rec.world, &rec.anims);

        let dangling = |link: &NamedLink<FpvMeshRange>| link.caption.is_some() && !link.resolved.is_attached();
        if dangling(&view.gun) {
            log.add_labelled("view model", &view.gun.caption);
        }
        if dangling(&view.missile) {
            log.add_labelled("FPV rocket model", &view.missile.caption);
        }
        if world.gun.resolved.is_unlinked_ref() {
            log.add_labelled("world model", &world.gun.caption);
        }
        log.add_unresolved_anims("anim", &anims.edges[ANIM_SZ], &anims.names[ANIM_SZ]);
        // The off-hand animations only matter to a weapon that is used in pairs.
        let right_idle = anims.names[ANIM_RIGHT][weap_anim_slot::IDLE].as_deref();
        if !rec.body.forbid_twin_grip && right_idle.map_or(0, str::len) > 0 {
            for (what, set) in [("right anim", ANIM_RIGHT), ("left anim", ANIM_LEFT)] {
                log.add_unresolved_anims(what, &anims.edges[set], &anims.names[set]);
            }
        }

        if let Some(Err(error)) = &view.plan_hip {
            log.plus("FPV mount", error.to_string());
        }
        let broken_sides = view
            .sides
            .iter()
            .enumerate()
            .filter_map(|(side, built)| Some((side, built.as_ref()?.as_ref().err()?)));
        for (side, error) in broken_sides {
            log.plus("FPV skeleton", format!("{}: {error}", side_label(side)));
        }
        if view.gun.resolved.is_attached() {
            let bare_sides = (0..view.hands.len()).filter(|&side| view.hands[side].is_none());
            bare_sides.for_each(|side| log.plus("FPV hands layout", side_label(side)));
        }
        let hip = &view.hip;
        let unbound_hip = hip.models.iter().zip(&hip.edges).filter(|(_, edge)| !edge.is_attached());
        unbound_hip.for_each(|(label, _)| log.plus("FPV attachment model", label));
        let carried = &world.attached;
        for (slot, (label, edge)) in carried.models.iter().zip(&carried.edges).enumerate() {
            let mounted = world.mounts.get(slot).and_then(Option::as_ref).is_some();
            let kind = match (edge.is_attached(), mounted) {
                (false, _) => "world attachment model",
                (true, false) => "world attachment mount",
                (true, true) => continue,
            };
            log.plus(kind, label.as_str());
        }
        for (kind, names, links) in [
            ("world attachment bindings", carried.models.len(), carried.edges.len()),
            ("FPV attachment bindings", hip.models.len(), hip.edges.len()),
        ] {
            log.add_if_uneven(kind, names, links);
        }
        log.found
    }
}

impl crate::families::FamilyContent for WeaponLedger {
    fn lookup(&self, lookup_scope: crate::AssetCategoryScope, label: &str) -> Option<u32> {
        self.index_by_ns_name
            .get(&(lookup_scope, weapon_key(label)))
            .copied()
    }

    fn secondary_hand_category(&self, id: u32) -> i32 {
        self.summary_for(id).map_or(0, |facts| facts.secondary_hand_category)
    }

    fn admission(&self, id: u32) -> Result<(), crate::ConfigurationRefusal> {
        self.configuration_admission(id)
    }

    fn names_in(&self, namespace: crate::AssetCategoryScope) -> Vec<(u32, String)> {
        let rows = 1..self.records.len() as u32;
        rows.filter_map(|id| {
            let here = self.scope_for(id)? == namespace;
            here.then(|| (id, weapon_key(self.label_for(id))))
        })
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asset_core::ZoneCustodian;

    const PINNED: &str = "0: None \"\" \"\" None (0, 0, 0, 3) (None, None, None, []) (Some(NothingThere), Some(NothingThere)) (0, []) Some((0, 0))\n  Some(None) (Some(NothingThere), Some(NothingThere), Some(NothingThere)) ([], []) (None, None, None, None, None, None, None, None)\n  None None None None false None (false, \"unlinked selected models\", \"unlinked selected models\")\n  []\n1: Some(T6) \"ak47\" \"ak47_mp\" Some(\"t6:weapon/ak47_mp\") (2, 2, 0, 3) (Some(\"vm_ak\"), Some(\"wm_ak\"), Some(\"shot\"), [\"tag_a\"]) (Some(Bound(AttachedDestination { at: CatalogSlot { seq: 3, space: PhantomData<asset_core::space::FpvMeshRange> }, from_zone: ZoneCustodian(\"test\") })), Some(Unresolved(IndexLookupFailure))) (3, [Some(\"tag_x\"), None]) Some((1, 1))\n  Some(Some(\"idle_a\")) (Some(Unresolved(IndexLookupFailure)), Some(Unresolved(IndexLookupFailure)), Some(Bound(AttachedDestination { at: CatalogSlot { seq: 9, space: PhantomData<asset_core::space::XAnimRange> }, from_zone: ZoneCustodian(\"test\") }))) ([(\"ping\", Some(\"snd_ping\"))], [\"snd_ping\"]) (Some(\"snd_fire\"), None, None, None, Some(\"snd_bounce\"), None, None, None)\n  Some((T6, \"fx_trail\")) None Some(\"mc/scope\") Some(\"img\") false Some(\"WEAPON_AK\") (false, \"unlinked selected models\", \"unlinked selected models\")\n  [WeaponDependencyGap { id: 1, kind: \"FPV rocket model\", name: \"vm_rocket\" }, WeaponDependencyGap { id: 1, kind: \"world model\", name: \"wm_ak\" }, WeaponDependencyGap { id: 1, kind: \"anim\", name: \"idle_a\" }, WeaponDependencyGap { id: 1, kind: \"right anim\", name: \"rx\" }, WeaponDependencyGap { id: 1, kind: \"FPV skeleton\", name: \"axis: bad skeleton\" }, WeaponDependencyGap { id: 1, kind: \"FPV hands layout\", name: \"allies\" }, WeaponDependencyGap { id: 1, kind: \"FPV hands layout\", name: \"axis\" }, WeaponDependencyGap { id: 1, kind: \"world attachment mount\", name: \"w2\" }, WeaponDependencyGap { id: 1, kind: \"world attachment model\", name: \"w3\" }, WeaponDependencyGap { id: 1, kind: \"FPV attachment bindings\", name: \"2 names, 1 links\" }]\n2: Some(T6) \"m16\" \"m16_mp\" Some(\"t6:weapon/m16_mp\") (1, 0, 0, 2) (None, None, None, []) (Some(NothingThere), Some(NothingThere)) (0, []) Some((1, 1))\n  Some(None) (Some(NothingThere), Some(NothingThere), Some(NothingThere)) ([], []) (None, None, None, None, None, None, None, None)\n  None None None None false None (false, \"unlinked selected models\", \"unlinked selected models\")\n  []\n3: Some(T6) \"knife\" \"knife_mp\" Some(\"t6:weapon/knife_mp\") (0, 0, 0, 3) (None, None, None, []) (Some(NothingThere), Some(NothingThere)) (0, []) Some((0, 0))\n  Some(None) (Some(NothingThere), Some(NothingThere), Some(NothingThere)) ([], []) (None, None, None, None, None, None, None, None)\n  None None None None false None (false, \"unlinked selected models\", \"unlinked selected models\")\n  []\n4: None \"\" \"\" None (0, 0, 0, 4) (None, None, None, []) (None, None) (0, []) None\n  None (None, None, None) ([], []) (None, None, None, None, None, None, None, None)\n  None None None None false None (false, \"no such weapon\", \"no such weapon\")\n  []\n3 false 7 11 (Ok(None), Ok(Some(1)), Ok(Some(1)), Ok(Some(2)), Err(UnrecognizedArmoryLabel { name: \"t6:fx/m16\" }), Err(UnrecognizedArmoryLabel { name: \"zzz\" }))\n[(\"\", []), (\"wm_ak\", [\"tag_a\"]), (\"\", []), (\"\", [])]\n[\"\", \"shot\", \"\", \"\"]\n[\"\", \"ak47_mp\", \"m16_mp\", \"knife_mp\"]\n([false, false, true, true], 4)\n([9], 10)\n(\"t6:weapon/ak47_mp\", \"\", [0, 0, 0, 0])\n([(1, \"ak47\"), (2, \"m16\"), (3, \"knife\")], Some(1))";

    fn bound<S: asset_core::KeyDomain>(bay: usize) -> AssetDependencyLink<S> {
        AssetDependencyLink::bind_order(bay, ZoneCustodian::TEST)
    }

    fn sample() -> WeaponLedger {
        let mut a = WeaponEntry {
            name: "ak47".into(),
            alt: (Some("m16".into()), 0),
            dual_wield: Some("m16".into()),
            spawned_grenade: Some("nope".into()),
            hidden: vec!["tag_a".into()],
            ..WeaponEntry::default()
        };
        a.body.optic_switch = true;
        a.view.gun = NamedLink { caption: Some("vm_ak".into()), resolved: bound(3) };
        a.view.missile = NamedLink::named(Some("vm_rocket".into()));
        a.view.plan_hip = None;
        a.world.gun = NamedLink { caption: Some("wm_ak".into()), resolved: AssetDependencyLink::Unresolved(DependencyCause::IndexLookupFailure) };
        a.world.shot = Some("shot".into());
        a.world.attached = Attached {
            models: vec!["w1".into(), "w2".into(), "w3".into()],
            offsets: vec![],
            edges: vec![bound(1), bound(2), AssetDependencyLink::NothingThere],
        };
        a.world.mounts = vec![Some("tag_x".into()), None];
        a.view.hip = Attached {
            models: vec!["h1".into(), "h2".into()],
            offsets: vec![],
            edges: vec![bound(0)],
        };
        a.anims.names[ANIM_SZ][1] = Some("idle_a".into());
        a.anims.edges[ANIM_SZ][1] = AssetDependencyLink::Unresolved(DependencyCause::IndexLookupFailure);
        a.anims.names[ANIM_RIGHT][weap_anim_slot::IDLE] = Some("ridle".into());
        a.anims.names[ANIM_RIGHT][2] = Some("rx".into());
        a.anims.edges[ANIM_RIGHT][2] = AssetDependencyLink::Unresolved(DependencyCause::IndexLookupFailure);
        a.anims.edges[ANIM_LEFT][4] = bound(9);
        a.view.sides[1] = Some(Err("bad skeleton".into()));
        a.aliases.fire = Some("snd_fire".into());
        a.aliases.ricochet[0] = Some("snd_bounce".into());
        a.trail = NamedLink { caption: Some("fx_trail".into()), resolved: bound(4) };
        a.ignition = NamedLink { caption: Some("fx_ign".into()), resolved: AssetDependencyLink::NothingThere };
        a.hud.scope = NamedLink::named(Some("mc/scope".into()));
        a.hud.scope_image = Some("img".into());
        a.display_key = Some("WEAPON_AK".into());
        a.notetracks.insert("ping".into(), LinkedNotetrackAction { sound_alias: Some("snd_ping".into()), rumble_alias: None });
        let mut b = WeaponEntry { name: "m16".into(), ..WeaponEntry::default() };
        b.body.use_as_melee = true;
        b.body.shares_alt_magazine = true;
        b.alt.0 = Some("ak47".into());
        let knife = WeaponEntry { name: "knife".into(), ..WeaponEntry::default() };
        let mut reg = WeaponLedger {
            records: vec![WeaponEntry::default(), a, b, knife],
            epoch_no: 7,
            world_catalog_identity: 11,
            ..WeaponLedger::default()
        };
        reg.rebuild_name_maps();
        reg
    }

    #[test]
    fn registry_reads_are_pinned() {
        let reg = sample();
        let bank = crate::AudioRoster::default();
        let mut listing = String::new();
        for id in 0..=4u32 {
            listing += &format!(
                "{id}: {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?}\n",
                reg.scope_for(id),
                reg.label_for(id),
                reg.gsc_name_for(id),
                reg.lookup_key_for(id).map(|k| k.to_string()),
                (reg.alternate_of(id), reg.dual_wield_weapon_of(id), reg.spawned_grenade_weapon_of(id), reg.melee_weapon_of(id)),
                (reg.view_mesh_for(id), reg.world_mesh_for(id), reg.projectile_mesh_for(id), reg.hidden_tags_for(id)),
                (reg.gun_mesh_link_for(id), reg.world_mesh_link_for(id)),
                (reg.attachment_world_model_edges_of(id).len(), reg.attachment_world_mounts_of(id)),
                reg.summary_for(id).map(|f| (f.ammo_type_slot, f.motion_clip_slot)),
            );
            listing += &format!(
                "  {:?} {:?} {:?} {:?}\n",
                reg.clips_for(id).map(|n| n[1].clone()),
                (
                    reg.sz_xanim_edges_of(id).map(|e| e[1]),
                    reg.sz_xanim_right_edges_of(id).map(|e| e[2]),
                    reg.sz_xanim_left_edges_of(id).map(|e| e[4]),
                ),
                (
                    reg.notetrack_actions_of(id).map(|(n, a)| (n.to_owned(), a.sound_alias.clone())).collect::<Vec<_>>(),
                    reg.notetrack_sound_aliases_of(id).collect::<Vec<_>>(),
                ),
                (
                    reg.authored_weapon_sound(id, WeaponAudioRole::Fire),
                    reg.authored_weapon_sound(id, WeaponAudioRole::Reload),
                    reg.weapon_sound_key(id, WeaponAudioRole::Fire, &bank),
                    reg.weapon_sound_alias(id, WeaponAudioRole::Fire, &bank),
                    reg.bounce_cue_for(id, 0),
                    reg.bounce_cue_for(id, 1),
                    reg.bounce_cue_for(id, 99),
                    reg.bounce_sound_alias(id, 0, &bank),
                ),
            );
            listing += &format!(
                "  {:?} {:?} {:?} {:?} {:?} {:?} {:?}\n",
                reg.shot_wake_for(id).map(|f| (f.namespace, f.name)),
                reg.shot_ignite_for(id).map(|f| (f.namespace, f.name)),
                reg.scope_surface_for(id),
                reg.scope_picture_for(id),
                reg.scope_is_iris(id),
                reg.title_token_for(id),
                (reg.fpv_hands_of(id, true).is_some(), reg.fpv_assembly_gap_of(id, true), reg.fpv_assembly_gap_of(id, false)),
            );
            listing += &format!("  {:?}\n", reg.dependency_gaps_of(id));
        }
        listing += &format!(
            "{} {} {} {} {:?}\n{:?}\n{:?}\n{:?}\n{:?}\n{:?}\n{:?}\n{:?}",
            reg.len(),
            reg.is_empty(),
            reg.epoch(),
            reg.world_catalog_identity(),
            (reg.follow_slot_link(""), reg.follow_slot_link("ak47"), reg.follow_slot_link("AK47_MP"), reg.follow_slot_link("t6:weapon/M16_mp"), reg.follow_slot_link("t6:fx/m16"), reg.follow_slot_link("zzz")),
            reg.world_models_table(),
            reg.projectile_models_table(),
            reg.script_label_sheet(),
            (reg.runnable_table(), reg.multiplier_sheet().len()),
            (reg.bound_weapon_xanim_indices(), reg.dependency_gaps().len()),
            (reg.configuration_label(1), reg.configuration_label(0), reg.configuration_transition_groups()),
            (crate::families::FamilyContent::names_in(&reg, crate::AssetCategoryScope::T6), crate::families::FamilyContent::lookup(&reg, crate::AssetCategoryScope::T6, "ak47")),
        );
        assert_eq!(listing, PINNED);
    }
}
