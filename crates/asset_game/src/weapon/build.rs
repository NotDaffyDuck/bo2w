//! Making the table from a capture, and the passes that link each row's names
//! to the catalogs the zone walk fills in afterwards.

use std::collections::{BTreeMap, BTreeSet, btree_map::Entry};

use super::*;

impl WeaponRow {
    /// Every animation clip any of this weapon's three animation sets uses.
    pub(super) fn bound_clips(&self) -> BTreeSet<usize> {
        self.sz_xanim_edges
            .iter()
            .chain(&self.sz_xanim_right_edges)
            .chain(&self.sz_xanim_left_edges)
            .filter_map(|edge| edge.bound_index())
            .collect()
    }

    fn from_capture(name: String, entry: CatalogWeapon) -> Self {
        let (ads_view_models, ads_view_offsets): (Vec<_>, Vec<_>) = entry
            .ads_view_attachments
            .into_iter()
            .map(|(model, at)| (model, Some(at)))
            .unzip();
        let (attachment_view_models, attachment_view_offsets): (Vec<_>, Vec<_>) = entry
            .view_attachments
            .into_iter()
            .map(|(model, at)| (model, Some(at)))
            .unzip();
        let (attachment_world_models, attachment_world_tags): (Vec<_>, Vec<_>) =
            entry.world_attachments.into_iter().unzip();
        Self {
            name,
            alternate_weapon: entry.alternate_weapon,
            facts: entry.facts,
            gun_xmodel: entry.gun_xmodel,
            ads_view_models,
            ads_view_offsets,
            attachment_view_models,
            attachment_view_offsets,
            attachment_world_models,
            attachment_world_tags,
            world_model: entry.world_model,
            projectile_model: entry.projectile_model,
            rocket_model: entry.rocket_model,
            sz_xanims: entry.sz_xanims,
            sz_xanims_right: entry.sz_xanims_right,
            sz_xanims_left: entry.sz_xanims_left,
            hide_tags: entry.hide_tags,
            dual_wield_weapon: entry.dual_wield_weapon,
            spawned_grenade_weapon: entry.spawned_grenade_weapon,
            sounds: entry.sounds,
            combat_fx: entry.combat_fx,
            reticle: entry.reticle,
            display_name_key: entry.display_name_key,
            overlay_material: entry.overlay_material,
            hud_icon: entry.hud_icon,
            kill_icon: entry.kill_icon,
            proj_trail: entry.proj_trail,
            proj_ignition: entry.proj_ignition,
            ..Self::default()
        }
    }
}

fn fill<T>(slot: &mut Option<T>, from: Option<T>) {
    if slot.is_none() {
        *slot = from;
    }
}

fn fill_anims(
    slots: &mut [Option<String>; WEAPON_ANIM_SLOTS],
    from: [Option<String>; WEAPON_ANIM_SLOTS],
) {
    for (slot, name) in slots.iter_mut().zip(from) {
        fill(slot, name);
    }
}

/// A weapon two zones both define: the first capture stands and the second
/// only fills what the first left empty.
fn merge_duplicate(first: &mut CatalogWeapon, later: CatalogWeapon) {
    fill(&mut first.alternate_weapon, later.alternate_weapon);
    fill(&mut first.display_name_key, later.display_name_key);
    fill(&mut first.overlay_material, later.overlay_material);
    fill(&mut first.hud_icon, later.hud_icon);
    fill(&mut first.kill_icon, later.kill_icon);
    fill(&mut first.proj_trail, later.proj_trail);
    fill(&mut first.proj_ignition, later.proj_ignition);
    fill(&mut first.gun_xmodel, later.gun_xmodel);
    fill(&mut first.world_model, later.world_model);
    fill(&mut first.projectile_model, later.projectile_model);
    fill(&mut first.rocket_model, later.rocket_model);
    fill(&mut first.dual_wield_weapon, later.dual_wield_weapon);
    fill(&mut first.spawned_grenade_weapon, later.spawned_grenade_weapon);

    let icon = first
        .reticle
        .indicator_icon
        .take()
        .or_else(|| later.reticle.indicator_icon.clone());
    let authored = |r: &WeaponReticleAssets| r.center_authored || r.side_authored;
    if !authored(&first.reticle) && authored(&later.reticle) {
        first.reticle = later.reticle;
    }
    first.reticle.indicator_icon = icon;

    fill_anims(&mut first.sz_xanims, later.sz_xanims);
    fill_anims(&mut first.sz_xanims_right, later.sz_xanims_right);
    fill_anims(&mut first.sz_xanims_left, later.sz_xanims_left);
    if first.hide_tags.is_empty() {
        first.hide_tags = later.hide_tags;
    }
    first.sounds.fill_gaps_from(&later.sounds);
    first.combat_fx.fill_gaps_from(&later.combat_fx);
    if !first.facts.body_resolved {
        first.facts = later.facts;
    }
}

fn material_hint_edge(
    hint: Option<&str>,
    ns: crate::AssetNamespace,
    authored: bool,
    materials: &crate::MaterialDefinitions,
) -> AssetEdge<MaterialSpace> {
    let hint = hint.filter(|name| !name.is_empty());
    if !authored && hint.is_none() {
        return AssetEdge::Absent;
    }
    match hint.and_then(|name| materials.material_index_by_ns(ns, name)) {
        Some(index) => AssetEdge::bind(index, materials.zone_of(index.order())),
        None => AssetEdge::Unresolved(AssetEdgeReason::CatalogMiss),
    }
}

fn fpv_model_edge(
    hint: Option<&str>,
    ns: crate::AssetNamespace,
    fpv: &crate::FpvMeshCatalog,
) -> AssetEdge<FpvMeshSpace> {
    let Some(name) = hint.filter(|name| !name.is_empty()) else {
        return AssetEdge::Absent;
    };
    match fpv.index_by_name(ns, name) {
        Some(index) => AssetEdge::bind_order(index, fpv.zone_of(index)),
        None => AssetEdge::Unresolved(AssetEdgeReason::CatalogMiss),
    }
}

fn world_model_edge(
    hint: Option<&str>,
    ns: crate::AssetNamespace,
    catalog: &crate::WorldWeaponCatalog,
) -> AssetEdge<WorldWeaponSpace> {
    let Some(name) = hint.filter(|name| !name.is_empty()) else {
        return AssetEdge::Absent;
    };
    match catalog.index_by_name(ns, name) {
        Some(index) => AssetEdge::bind_order(index, catalog.zone_of(index)),
        None => AssetEdge::Unresolved(AssetEdgeReason::CatalogMiss),
    }
}

fn fpv_edges_of(
    names: &[String],
    ns: crate::AssetNamespace,
    fpv: &crate::FpvMeshCatalog,
) -> Vec<AssetEdge<FpvMeshSpace>> {
    names
        .iter()
        .map(|name| fpv_model_edge(Some(name), ns, fpv))
        .collect()
}

fn xanim_edges_of(
    names: &[Option<String>; WEAPON_ANIM_SLOTS],
    ns: crate::AssetNamespace,
    xanims: &crate::XAnimCatalog,
) -> [AssetEdge<XAnimSpace>; WEAPON_ANIM_SLOTS] {
    std::array::from_fn(|slot| xanims.hint_edge(names[slot].as_deref(), ns))
}

fn stamp_fx_edge(
    fx: &crate::FxCatalog,
    ns: crate::AssetNamespace,
    edge: &mut AssetEdge<FxSpace>,
    hint: &mut Option<String>,
) {
    hint.take_if(|name| name.is_empty());
    *edge = fx.hint_edge(false, hint.as_deref(), ns);
}

/// The alias an inline notetrack carries after its prefix (`sndnt#alias`).
fn inline_note_alias<'a>(note: &'a str, prefix: &str) -> Option<&'a str> {
    let (head, tail) = note.split_at_checked(prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then_some(tail)
        .filter(|alias| !alias.is_empty())
}

/// Where the gun's attachments go on it, when the gun and every attachment the
/// zone did not place itself are in the catalog. An attachment with a placed
/// offset that the catalog lacks is left off; the gun still draws.
fn mount_plan(
    fpv: &crate::FpvMeshCatalog,
    gun: AssetEdge<FpvMeshSpace>,
    rocket: AssetEdge<FpvMeshSpace>,
    edges: &[AssetEdge<FpvMeshSpace>],
    offsets_of: &[Option<asset_model::FpvMountOffset>],
) -> Option<Result<asset_model::FpvMountPlan, asset_model::FpvMountError>> {
    let gun = gun.bound_index()?;
    let mut attachments = Some(Vec::new());
    let mut offsets = Vec::new();
    for (i, edge) in edges.iter().enumerate() {
        let offset = offsets_of.get(i).cloned().flatten();
        match (edge.bound_index(), &mut attachments) {
            (Some(index), Some(list)) => {
                list.push(crate::FpvMeshIndex::from_order(index));
                offsets.push(offset);
            }
            (None, _) if offset.is_some() => {}
            _ => attachments = None,
        }
    }
    let rocket = if rocket.is_absent() {
        None
    } else {
        Some(crate::FpvMeshIndex::from_order(rocket.bound_index()?))
    };
    Some(asset_model::plan_fpv_mounts(
        fpv,
        crate::FpvMeshIndex::from_order(gun),
        &attachments?,
        &offsets,
        rocket,
    ))
}

impl WeaponBuild {
    /// Turn a capture into rows. Two captures of one weapon become one row.
    pub(super) fn from_catalog(entries: Vec<CatalogWeapon>) -> Self {
        let mut merged: BTreeMap<String, CatalogWeapon> = BTreeMap::new();
        for entry in entries {
            match merged.entry(normalize_weapon_name(&entry.name)) {
                Entry::Vacant(slot) => {
                    slot.insert(entry);
                }
                Entry::Occupied(mut slot) => merge_duplicate(slot.get_mut(), entry),
            }
        }
        let mut rows = Vec::with_capacity(merged.len() + 1);
        rows.push(WeaponRow::default());
        rows.extend(
            merged
                .into_iter()
                .map(|(name, entry)| WeaponRow::from_capture(name, entry)),
        );
        let mut registry = WeaponRegistry {
            rows,
            world_catalog_identity: 0,
            by_name: HashMap::new(),
            by_namespaced: HashMap::new(),
            families: crate::WeaponFamilies::default(),
            alternate_fpv: HashMap::new(),
            fpv_clip_tracks: Arc::default(),
            revision: mint_weapon_revision(),
        };
        registry.rebuild_name_maps();
        Self { registry, family_tables: Vec::new() }
    }

    pub fn publish(self) -> WeaponRegistry {
        let mut registry = self.registry;
        registry.families = crate::WeaponFamilies::build(&self.family_tables, &registry);
        registry
    }

    pub fn set_family_tables(
        &mut self,
        tables: Vec<(crate::AssetNamespace, crate::CapturedStringTable)>,
    ) {
        self.family_tables = tables;
    }

    /// Add another zone walk's weapons after this one's.
    pub fn absorb(&mut self, mut other: Self) {
        if other.registry.is_empty() {
            self.family_tables.append(&mut other.family_tables);
            return;
        }
        if self.registry.rows.is_empty() {
            *self = other;
            return;
        }
        self.registry.rows.extend(other.registry.rows.into_iter().skip(1));
        self.family_tables.append(&mut other.family_tables);
        self.registry.rebuild_name_maps();
        self.registry.revision = mint_weapon_revision();
    }

    pub fn stamp_namespace(&mut self, ns: crate::AssetNamespace) {
        for row in self.registry.rows.iter_mut().skip(1) {
            row.namespace = ns;
        }
        self.registry.rebuild_name_maps();
        self.registry.revision = mint_weapon_revision();
    }

    pub fn resolve_combat_fx(&mut self, fx: &crate::FxCatalog, tracers: &crate::TracerCatalog) {
        for row in self.registry.rows.iter_mut().skip(1) {
            let ns = row.namespace;
            let combat = &mut row.combat_fx;
            combat.namespace = ns;
            for (edge, hint) in [
                (&mut combat.view_flash, &mut combat.view_flash_hint),
                (&mut combat.world_flash, &mut combat.world_flash_hint),
                (&mut combat.view_shell_eject, &mut combat.view_shell_eject_hint),
                (&mut combat.world_shell_eject, &mut combat.world_shell_eject_hint),
                (&mut combat.explosion, &mut combat.explosion_hint),
            ] {
                stamp_fx_edge(fx, ns, edge, hint);
            }
            // The hint becomes the tracer's own spelling once the table has it.
            let named = combat.tracer_hint.clone().filter(|name| !name.is_empty());
            let found = named
                .as_deref()
                .and_then(|name| tracers.index_of(ns, name))
                .and_then(|index| Some((index, tracers.def_at(index)?)));
            combat.tracer_hint = found.map(|(_, def)| def.name.clone()).or_else(|| named.clone());
            combat.tracer = match (named.is_some(), found) {
                (false, _) => AssetEdge::Absent,
                (true, Some((index, _))) => AssetEdge::bind_order(index, tracers.zone_of(index)),
                (true, None) => AssetEdge::Unresolved(AssetEdgeReason::CatalogMiss),
            };
        }
    }

    pub fn resolve_hud_material_edges(&mut self, materials: &crate::MaterialDefinitions) {
        for row in &mut self.registry.rows {
            let ns = row.namespace;
            if row.overlay_image.is_none()
                && let Some(name) = row.overlay_material.as_deref()
                && let Some(index) = materials.material_index_by_ns(ns, name)
                && let Some(material) = materials.materials.get(index.order())
            {
                row.overlay_image = materials.hud_image_name(material).map(str::to_owned);
            }
            let reticle = &mut row.reticle;
            reticle.center_edge = material_hint_edge(
                reticle.center_material.as_deref(),
                ns,
                reticle.center_authored,
                materials,
            );
            reticle.side_edge = material_hint_edge(
                reticle.side_material.as_deref(),
                ns,
                reticle.side_authored,
                materials,
            );
            row.overlay_edge = material_hint_edge(row.overlay_material.as_deref(), ns, false, materials);
            row.hud_icon_edge = material_hint_edge(row.hud_icon.as_deref(), ns, false, materials);
            row.kill_icon_edge = material_hint_edge(row.kill_icon.as_deref(), ns, false, materials);
        }
    }

    pub fn resolve_projectile_fx_edges(&mut self, fx: &crate::FxCatalog) {
        for row in &mut self.registry.rows {
            row.trail_edge = fx.hint_edge(false, row.proj_trail.as_deref(), row.namespace);
            row.ignition_edge = fx.hint_edge(false, row.proj_ignition.as_deref(), row.namespace);
        }
    }

    pub fn resolve_sz_xanim_edges(&mut self, xanims: &crate::XAnimCatalog) {
        for row in &mut self.registry.rows {
            let ns = row.namespace;
            row.sz_xanim_edges = xanim_edges_of(&row.sz_xanims, ns, xanims);
            row.sz_xanim_right_edges = xanim_edges_of(&row.sz_xanims_right, ns, xanims);
            row.sz_xanim_left_edges = xanim_edges_of(&row.sz_xanims_left, ns, xanims);
        }
    }

    /// Read each animation's inline `sndnt#` and `rmbnt#` notetracks into what
    /// the weapon plays when they fire. Returns how many actions were linked.
    pub fn resolve_notetrack_actions(&mut self, xanims: &crate::XAnimCatalog) -> usize {
        let mut linked = 0;
        for row in &mut self.registry.rows {
            let mut actions: HashMap<String, LinkedNotetrackAction> = HashMap::new();
            for index in row.bound_clips() {
                let Some(clip) = xanims.clip_at(index) else {
                    continue;
                };
                for notify in &clip.notifies {
                    let sound = inline_note_alias(&notify.name, "sndnt#");
                    let rumble = inline_note_alias(&notify.name, "rmbnt#");
                    if sound.is_none() && rumble.is_none() {
                        continue;
                    }
                    let action = actions.entry(notify.name.to_ascii_lowercase()).or_default();
                    action.sound_alias = sound.map(str::to_owned);
                    action.rumble_alias = rumble.map(str::to_owned);
                }
            }
            linked += actions.len();
            row.notetrack_actions = actions;
        }
        linked
    }

    pub fn resolve_fpv_mesh_edges(&mut self, fpv: &crate::FpvMeshCatalog) {
        for row in &mut self.registry.rows {
            let ns = row.namespace;
            row.gun_xmodel_edge = fpv_model_edge(row.gun_xmodel.as_deref(), ns, fpv);
            row.rocket_model_edge = fpv_model_edge(row.rocket_model.as_deref(), ns, fpv);
            row.attachment_view_model_edges = fpv_edges_of(&row.attachment_view_models, ns, fpv);
            row.fpv_mount_plan = mount_plan(
                fpv,
                row.gun_xmodel_edge,
                row.rocket_model_edge,
                &row.attachment_view_model_edges,
                &row.attachment_view_offsets,
            );
            // The aimed plan exists only when a sight swaps its model.
            let ads_edges = fpv_edges_of(&row.ads_view_models, ns, fpv);
            row.fpv_mount_plan_ads = if ads_edges.is_empty() {
                None
            } else {
                mount_plan(
                    fpv,
                    row.gun_xmodel_edge,
                    row.rocket_model_edge,
                    &ads_edges,
                    &row.ads_view_offsets,
                )
            };
        }
    }

    pub fn resolve_fpv_hands(
        &mut self,
        fpv: &crate::FpvMeshCatalog,
        bodies: &asset_model::BodyMeshCatalog,
    ) {
        for row in &mut self.registry.rows {
            let map_ns = fpv.map_namespace.unwrap_or(row.namespace);
            row.fpv_hands = std::array::from_fn(|side| {
                let kit = bodies.kits().kit(side == 1);
                let choice = asset_model::FpvHands::resolve(fpv, map_ns, kit, None, row.namespace);
                let (ns, name) = choice.key()?;
                let index = fpv.index_by_name(ns, name)?;
                let skel = &fpv.get_at(index)?.skel;
                if skel.pose.is_none() || !skel.bone_names.iter().any(|bone| bone == "tag_weapon") {
                    return None;
                }
                Some((choice, crate::FpvMeshIndex::from_order(index)))
            });
        }
    }

    /// Put each weapon's gun, attachments and hands together for both sides,
    /// and bind every animation it plays to the parts.
    pub fn resolve_fpv_assemblies(
        &mut self,
        fpv: &crate::FpvMeshCatalog,
        xanims: &crate::XAnimCatalog,
    ) -> FpvAssemblyCensus {
        let mut shared: HashMap<crate::FpvAssemblyKey, Result<Arc<crate::FpvAssembly>, String>> =
            HashMap::new();
        let mut tracks = crate::FpvClipTracks::default();
        let mut census = FpvAssemblyCensus::default();
        self.registry.alternate_fpv.clear();

        // A weapon that is another weapon's alternate and sits in the
        // inventory as a held item is also built as it looks in that parent's
        // hands, which hides different tags.
        let registry = &self.registry;
        let variants = (1..registry.rows.len() as u32).filter_map(|parent| {
            let alternate = registry.alternate_of(parent);
            let held = registry.facts_of(alternate).is_some_and(|f| f.inventory_type == 3);
            (alternate != 0 && held).then_some((alternate, parent))
        });
        let preparations: Vec<_> = (0..registry.rows.len() as u32)
            .map(|id| (id, 0))
            .chain(variants)
            .map(|(id, parent)| {
                let hide_tags =
                    crate::effective_hide_tags(registry, if parent == 0 { id } else { parent });
                (id, parent, hide_tags)
            })
            .collect();

        for (id, parent, hide_tags) in preparations {
            let row = &mut self.registry.rows[id as usize];
            if parent == 0 {
                row.fpv_assemblies = [None, None];
            }
            let Some(Ok(mounts)) = &row.fpv_mount_plan else {
                continue;
            };
            let ads_mounts = match &row.fpv_mount_plan_ads {
                Some(Ok(ads)) => Some(ads),
                _ => None,
            };
            let clips = row.bound_clips();
            let mut assemble = |hands: crate::FpvMeshIndex,
                                mounts: &asset_model::FpvMountPlan,
                                rocket: bool| {
                let key = crate::FpvAssemblyKey {
                    hands,
                    gun: mounts.gun,
                    attachments: mounts.attachments.iter().map(|mount| mount.model).collect(),
                    attachment_binds: mounts
                        .attachments
                        .iter()
                        .map(|mount| {
                            mount
                                .root_bind
                                .map(|(q, t)| (q.map(f32::to_bits), t.map(f32::to_bits)))
                        })
                        .collect(),
                    rocket: rocket
                        .then(|| mounts.rocket.as_ref().map(|mount| mount.model))
                        .flatten(),
                    knife: None,
                    hide_tags: hide_tags.clone(),
                };
                shared
                    .entry(key)
                    .or_insert_with(|| {
                        census.built += 1;
                        crate::FpvAssembly::build(fpv, hands, mounts, rocket, None, &hide_tags)
                            .map(Arc::new)
                            .map_err(|error| error.to_string())
                    })
                    .clone()
            };
            let sides: [Option<Result<crate::FpvSideAssemblies, String>>; 2] =
                std::array::from_fn(|side| {
                    let (_, hands) = row.fpv_hands[side].as_ref()?;
                    let built = (|| -> Result<crate::FpvSideAssemblies, String> {
                        let bare = assemble(*hands, mounts, false)?;
                        let rocket = mounts
                            .rocket
                            .is_some()
                            .then(|| assemble(*hands, mounts, true))
                            .transpose()?;
                        let ads = ads_mounts
                            .map(|ads| assemble(*hands, ads, false))
                            .transpose()?;
                        Ok(crate::FpvSideAssemblies { bare, rocket, melee: None, ads })
                    })();
                    Some(built)
                });
            for side in sides.iter().flatten().flatten() {
                let assemblies = std::iter::once(&side.bare).chain(&side.rocket).chain(&side.ads);
                for assembly in assemblies {
                    for &clip_index in &clips {
                        let Some(clip) = xanims.clip_at(clip_index) else {
                            continue;
                        };
                        for part in &assembly.parts {
                            tracks.bind(fpv, clip_index, &clip, part.model);
                        }
                    }
                }
            }
            for side in sides.iter().flatten() {
                match side {
                    Ok(_) => census.linked += 1,
                    Err(_) => census.refused += 1,
                }
            }
            if parent == 0 {
                row.fpv_assemblies = sides;
            } else {
                self.registry.alternate_fpv.insert((id, parent), sides);
            }
        }
        census.clip_tables = tracks.len();
        self.registry.fpv_clip_tracks = Arc::new(tracks);
        census
    }

    pub fn resolve_world_model_edges(&mut self, catalog: &crate::WorldWeaponCatalog) {
        self.registry.world_catalog_identity = catalog.identity();
        for row in &mut self.registry.rows {
            let ns = row.namespace;
            row.world_model_edge = world_model_edge(row.world_model.as_deref(), ns, catalog);
            row.attachment_world_model_edges = row
                .attachment_world_models
                .iter()
                .map(|name| world_model_edge(Some(name), ns, catalog))
                .collect();
            let bones = row
                .world_model_edge
                .bound_index()
                .and_then(|index| catalog.get_at(index))
                .map(|gun| &gun.skel.bone_names);
            // The gun bone an attachment sits on: the tag it names, else the
            // gun's bone that matches the attachment model's own root.
            let bone_named = |wanted: &str| {
                bones?
                    .iter()
                    .find(|bone| bone.eq_ignore_ascii_case(wanted))
                    .cloned()
            };
            row.attachment_world_mounts = row
                .attachment_world_model_edges
                .iter()
                .enumerate()
                .map(|(i, edge)| {
                    match row.attachment_world_tags.get(i).filter(|tag| !tag.is_empty()) {
                        Some(tag) => bone_named(tag),
                        None => {
                            let model = catalog.get_at(edge.bound_index()?)?;
                            bone_named(model.skel.bone_names.first()?)
                        }
                    }
                })
                .collect();
        }
    }
}
