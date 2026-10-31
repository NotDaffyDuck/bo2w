//! Making the table from a capture, and the passes that link each row's names
//! to the catalogs the zone walk fills in afterwards.

use std::collections::{BTreeMap, BTreeSet};

use super::*;

/// The single-name fields a second capture of a weapon may still fill in.
const FILLABLE_NAMES: [fn(&mut RosterWeapon) -> &mut Option<String>; 13] = [
    |w| &mut w.alternate_weapon,
    |w| &mut w.shown_title_token,
    |w| &mut w.hud_layer_surface,
    |w| &mut w.badge_pic,
    |w| &mut w.elimination_emblem,
    |w| &mut w.shell_wake,
    |w| &mut w.shell_ignite,
    |w| &mut w.weapon_mesh,
    |w| &mut w.third_person_mesh,
    |w| &mut w.shell_mesh,
    |w| &mut w.missile_mesh,
    |w| &mut w.dual_wield_weapon,
    |w| &mut w.spawned_grenade_weapon,
];

const FILLABLE_ANIMS: [fn(&mut RosterWeapon) -> &mut [Option<String>; WEAPON_ANIM_SLOTS]; 3] = [
    |w| &mut w.clip_names,
    |w| &mut w.right_clip_names,
    |w| &mut w.left_clip_names,
];

impl WeaponCrosshairResources {
    fn comes_from_script(&self) -> bool {
        self.center_authored || self.side_authored
    }
}

/// A weapon two zones both define: the first capture stands and the second
/// only fills what the first left empty.
fn merge_duplicate(first: &mut RosterWeapon, mut later: RosterWeapon) {
    for pick in FILLABLE_NAMES {
        let spare = pick(&mut later).take();
        let mine = pick(first);
        *mine = mine.take().or(spare);
    }
    for pick in FILLABLE_ANIMS {
        for (mine, theirs) in pick(first).iter_mut().zip(pick(&mut later).iter_mut()) {
            if mine.is_none() {
                *mine = theirs.take();
            }
        }
    }

    let icon = first
        .reticle
        .indicator_icon
        .take()
        .or_else(|| later.reticle.indicator_icon.take());
    if later.reticle.comes_from_script() && !first.reticle.comes_from_script() {
        first.reticle = later.reticle;
    }
    first.reticle.indicator_icon = icon;

    if first.concealed_bones.is_empty() {
        first.concealed_bones = later.concealed_bones;
    }
    if first.world_parts.is_empty() {
        first.world_parts = later.world_parts;
    }
    first.sounds.fill_gaps_from(&later.sounds);
    first.combat_fx.fill_gaps_from(&later.combat_fx);
    if !first.facts.payload_linked {
        first.facts = later.facts;
    }
}

/// Names and placements out of a list of (model, placement) pairs.
fn split_placed(
    pairs: Vec<(String, asset_model::FpvMountOffset)>,
) -> (Vec<String>, Vec<Option<asset_model::FpvMountOffset>>) {
    pairs
        .into_iter()
        .map(|(mesh, at)| (mesh, Some(at)))
        .unzip()
}

impl WeaponEntry {
    /// Every animation clip any of this weapon's three animation sets uses.
    pub(super) fn bound_clips(&self) -> BTreeSet<usize> {
        self.anims
            .edges
            .iter()
            .flatten()
            .filter_map(AssetDependencyLink::extent_position)
            .collect()
    }

    fn from_capture(name: String, cap: RosterWeapon) -> Self {
        let (aim_models, aim_offsets) = split_placed(cap.ads_view_attachments);
        let (hip_models, hip_offsets) = split_placed(cap.view_attachments);
        let (world_models, tags) = cap.world_attachments.into_iter().unzip();
        let [sz, right, left] = [cap.clip_names, cap.right_clip_names, cap.left_clip_names];

        Self {
            name,
            body: cap.facts,
            alt: (cap.alternate_weapon, 0),
            dual_wield: cap.dual_wield_weapon,
            spawned_grenade: cap.spawned_grenade_weapon,
            view: ViewLinks {
                gun: NamedLink::named(cap.weapon_mesh),
                missile: NamedLink::named(cap.missile_mesh),
                hip: Attached {
                    models: hip_models,
                    offsets: hip_offsets,
                    edges: Vec::new(),
                },
                aimed: Attached {
                    models: aim_models,
                    offsets: aim_offsets,
                    edges: Vec::new(),
                },
                ..ViewLinks::default()
            },
            world: WorldLinks {
                gun: NamedLink::named(cap.third_person_mesh),
                attached: Attached {
                    models: world_models,
                    ..Attached::default()
                },
                tags,
                shot: cap.shell_mesh,
                pieces: cap.world_parts,
                ..WorldLinks::default()
            },
            anims: AnimLinks {
                names: [sz, right, left],
                ..AnimLinks::default()
            },
            hidden: cap.concealed_bones,
            aliases: cap.sounds,
            fx: cap.combat_fx,
            sight: cap.reticle,
            display_key: cap.shown_title_token,
            hud: HudLinks {
                scope: NamedLink::named(cap.hud_layer_surface),
                icon: NamedLink::named(cap.badge_pic),
                elimination_emblem: NamedLink::named(cap.elimination_emblem),
                ..HudLinks::default()
            },
            trail: NamedLink::named(cap.shell_wake),
            ignition: NamedLink::named(cap.shell_ignite),
            ..Self::default()
        }
    }
}

/// A name a row wants linked, and the namespace it is looked up in. Each
/// method binds it against one catalog: absent when nothing is named, a
/// catalog miss when the catalog does not know the name.
#[derive(Clone, Copy)]
struct Slot<'a> {
    name: Option<&'a str>,
    space: crate::AssetCategoryScope,
}

impl<'a> Slot<'a> {
    fn new(name: Option<&'a str>, space: crate::AssetCategoryScope) -> Self {
        Self { name, space }
    }

    fn named(words: &'a str, space: crate::AssetCategoryScope) -> Self {
        Self::new(Some(words), space)
    }

    fn bind<S: asset_core::KeyDomain>(
        self,
        lookup: impl FnOnce(&str) -> Option<AssetDependencyLink<S>>,
    ) -> AssetDependencyLink<S> {
        let Some(text) = self.name.filter(|text| !text.is_empty()) else {
            return AssetDependencyLink::NothingThere;
        };
        lookup(text).unwrap_or(AssetDependencyLink::Unresolved(DependencyCause::IndexLookupFailure))
    }

    /// A script-authored slot with no usable name is a miss, not an absence.
    fn material(self, defs: &crate::MaterialDefinitions, from_script: bool) -> AssetDependencyLink<SurfaceShaderSpace> {
        let space = self.space;
        let edge = self.bind(|text| {
            let index = defs.material_index_by_ns(space, text)?;
            Some(AssetDependencyLink::bind(index, defs.owning_zone(index.order())))
        });
        if from_script && edge.lacks_entry() {
            return AssetDependencyLink::Unresolved(DependencyCause::IndexLookupFailure);
        }
        edge
    }

    fn fpv(self, meshes: &crate::FirstPersonMeshRoster) -> AssetDependencyLink<FpvMeshRange> {
        let space = self.space;
        self.bind(|text| {
            let at = meshes.row_of(space, text)?;
            Some(AssetDependencyLink::bind_order(at, meshes.supplier_at(at)))
        })
    }

    fn world_gun(self, guns: &crate::LevelWeaponRoster) -> AssetDependencyLink<WorldWeaponRange> {
        let space = self.space;
        self.bind(|text| {
            let at = guns.row_of(space, text)?;
            Some(AssetDependencyLink::bind_order(at, guns.supplier_at(at)))
        })
    }
}

/// The alias an inline notetrack carries after its prefix (`sndnt#alias`).
fn inline_note_alias<'a>(track: &'a str, lead: &str) -> Option<&'a str> {
    let (front, rest) = track.split_at_checked(lead.len())?;
    if rest.is_empty() || !front.eq_ignore_ascii_case(lead) {
        return None;
    }
    Some(rest)
}

/// What a notetrack triggers, if it is one of the two kinds a weapon acts on.
fn notetrack_effect(track: &str) -> Option<LinkedNotetrackAction> {
    let sound_alias = inline_note_alias(track, "sndnt#").map(str::to_owned);
    let rumble_alias = inline_note_alias(track, "rmbnt#").map(str::to_owned);
    if sound_alias.is_none() && rumble_alias.is_none() {
        return None;
    }
    Some(LinkedNotetrackAction {
        sound_alias,
        rumble_alias,
    })
}

/// Where the gun's attachments go on it, when the gun and every attachment the
/// zone did not place itself are in the catalog. An attachment with a placed
/// offset that the catalog lacks is left off; the gun still draws.
fn mount_plan(
    meshes: &crate::FirstPersonMeshRoster,
    gun: AssetDependencyLink<FpvMeshRange>,
    rocket: AssetDependencyLink<FpvMeshRange>,
    edges: &[AssetDependencyLink<FpvMeshRange>],
    placed: &[Option<asset_model::FpvMountOffset>],
) -> Option<Result<asset_model::FpvMountPlan, asset_model::FpvMountError>> {
    let gun = crate::FpvMeshSlot::from_sequence(gun.extent_position()?);
    let offered = placed.iter().cloned().chain(std::iter::repeat(None));
    let mut mounted = Vec::with_capacity(edges.len());
    for (edge, offset) in edges.iter().zip(offered) {
        match (edge.extent_position(), offset) {
            (Some(order), offset) => mounted.push((crate::FpvMeshSlot::from_sequence(order), offset)),
            (None, Some(_)) => {}
            (None, None) => return None,
        }
    }
    let (mesh_names, offsets): (Vec<_>, Vec<_>) = mounted.into_iter().unzip();
    let bound_rocket = rocket.extent_position().map(crate::FpvMeshSlot::from_sequence);
    if bound_rocket.is_none() && !rocket.lacks_entry() {
        return None;
    }
    let rocket = bound_rocket;
    Some(asset_model::plan_fpv_mounts(meshes, gun, &mesh_names, &offsets, rocket))
}

/// What one assembly shows: the mount plan, whether the rocket rides on it,
/// and the tags that stay hidden.
#[derive(Clone, Copy)]
struct Look<'a> {
    plan: &'a asset_model::FpvMountPlan,
    with_rocket: bool,
    concealed_bones: &'a [String],
}

/// Builds each distinct first-person assembly once and hands out shares.
struct Assembler<'m> {
    view_roster: &'m crate::FirstPersonMeshRoster,
    cache: HashMap<crate::FpvAssemblyKey, Result<Arc<crate::FpvAssembly>, String>>,
    built: usize,
}

impl Assembler<'_> {
    fn assembly(
        &mut self,
        hands: crate::FpvMeshSlot,
        look: Look<'_>,
    ) -> Result<Arc<crate::FpvAssembly>, String> {
        let Look { plan, with_rocket, concealed_bones } = look;
        let key = crate::FpvAssemblyKey {
            hands,
            gun: plan.gun,
            attachments: plan.attachments.iter().map(|m| m.model).collect(),
            attachment_binds: plan
                .attachments
                .iter()
                .map(|m| {
                    m.root_bind
                        .map(|(q, t)| (q.map(f32::to_bits), t.map(f32::to_bits)))
                })
                .collect(),
            rocket: plan
                .rocket
                .as_ref()
                .filter(|_| with_rocket)
                .map(|m| m.model),
            knife: None,
            concealed_bones: concealed_bones.to_vec(),
        };
        if let Some(done) = self.cache.get(&key) {
            return done.clone();
        }
        self.built += 1;
        let made = match crate::FpvAssembly::build(
            self.view_roster,
            hands,
            plan,
            with_rocket,
            None,
            concealed_bones,
        ) {
            Ok(assembly) => Ok(Arc::new(assembly)),
            Err(failure) => Err(failure.to_string()),
        };
        self.cache.insert(key, made.clone());
        made
    }

    /// Everything one pair of hands can show of this weapon.
    fn side(
        &mut self,
        hands: crate::FpvMeshSlot,
        hip: &asset_model::FpvMountPlan,
        aimed: Option<&asset_model::FpvMountPlan>,
        concealed_bones: &[String],
    ) -> Result<crate::FpvSideAssemblies, String> {
        let look = |plan, with_rocket| Look { plan, with_rocket, concealed_bones };
        let bare = self.assembly(hands, look(hip, false))?;
        let rocket = match hip.rocket {
            Some(_) => Some(self.assembly(hands, look(hip, true))?),
            None => None,
        };
        let ads = match aimed {
            Some(plan) => Some(self.assembly(hands, look(plan, false))?),
            None => None,
        };
        Ok(crate::FpvSideAssemblies {
            bare,
            rocket,
            melee: None,
            ads,
        })
    }
}

/// Point every clip the weapon plays at each part of each assembly it built.
fn bind_clips_to_parts(
    tracks: &mut crate::FpvClipTracks,
    meshes: &crate::FirstPersonMeshRoster,
    anim_cat: &crate::MotionClipRoster,
    clips: &BTreeSet<usize>,
    sides: &[Option<Result<crate::FpvSideAssemblies, String>>; 2],
) {
    let built = sides.iter().flatten().filter_map(|side| side.as_ref().ok());
    for side in built {
        let views = std::iter::once(&side.bare)
            .chain(&side.rocket)
            .chain(&side.ads);
        for view in views {
            for &index in clips {
                let Some(clip) = anim_cat.node_clip(index) else {
                    continue;
                };
                for part in &view.pieces {
                    tracks.bind(meshes, index, &clip, part.mdl);
                }
            }
        }
    }
}

/// The animation edges for one set of authored names.
fn xanim_edges_of(
    names: &[Option<String>; WEAPON_ANIM_SLOTS],
    space: crate::AssetCategoryScope,
    anims: &crate::MotionClipRoster,
) -> [AssetDependencyLink<XAnimRange>; WEAPON_ANIM_SLOTS] {
    names
        .each_ref()
        .map(|name| anims.hint_edge(name.as_deref(), space))
}

/// Each of the five table effects with the spelling it was authored under.
/// Points every effect slot of a weapon's combat set at the catalog, then its tracer.
fn relink_combat_fx(
    combat: &mut WeaponBattleEffects,
    space: crate::AssetCategoryScope,
    effects: &crate::EffectRoster,
    tracers: &crate::TracerRoster,
) {
    combat.namespace = space;
    for (edge, hint_text) in fx_slots(combat) {
        hint_text.take_if(|text| text.is_empty());
        *edge = effects.hint_edge(false, hint_text.as_deref(), space);
    }
    link_tracer(combat, space, tracers);
}

/// Every notify of the weapon's bound clips that triggers an effect, keyed by
/// lower-cased name; a later clip's notify replaces an earlier one's.
fn notetrack_table(
    row: &WeaponEntry,
    anims: &crate::MotionClipRoster,
) -> HashMap<String, LinkedNotetrackAction> {
    row.bound_clips()
        .into_iter()
        .filter_map(|at| anims.node_clip(at))
        .flat_map(|clip| {
            let reacting = clip.notifies.iter();
            reacting
                .filter_map(|n| Some((n.name.to_ascii_lowercase(), notetrack_effect(&n.name)?)))
                .collect::<Vec<_>>()
        })
        .collect()
}

fn fx_slots(combat: &mut WeaponBattleEffects) -> [(&mut AssetDependencyLink<FxRange>, &mut Option<String>); 5] {
    let WeaponBattleEffects {
        view_flash,
        own_muzzle_name: view_flash_hint,
        world_flash,
        far_muzzle_name: world_flash_hint,
        view_shell_eject,
        own_casing_name: view_shell_eject_hint,
        world_shell_eject,
        far_casing_name: world_shell_eject_hint,
        explosion,
        blast_clue,
        ..
    } = combat;
    [
        (view_flash, view_flash_hint),
        (world_flash, world_flash_hint),
        (view_shell_eject, view_shell_eject_hint),
        (world_shell_eject, world_shell_eject_hint),
        (explosion, blast_clue),
    ]
}

/// The tracer link, and the tracer's own spelling once the table has it.
fn link_tracer(
    combat: &mut WeaponBattleEffects,
    space: crate::AssetCategoryScope,
    tracers: &crate::TracerRoster,
) {
    let Some(name) = combat.streak_name.take().filter(|text| !text.is_empty()) else {
        combat.tracer = AssetDependencyLink::NothingThere;
        return;
    };
    let hit = tracers
        .position_of(space, &name)
        .and_then(|at| Some((at, tracers.recipe_at(at)?.name.clone())));
    (combat.tracer, combat.streak_name) = match hit {
        Some((at, spelled)) => (
            AssetDependencyLink::bind_order(at, tracers.source_zone(at)),
            Some(spelled),
        ),
        None => (
            AssetDependencyLink::Unresolved(DependencyCause::IndexLookupFailure),
            Some(name),
        ),
    };
}

impl WeaponBuild {
    /// Turn a capture into rows. Two captures of one weapon become one row.
    pub(super) fn build_from_index(entries: Vec<RosterWeapon>) -> Self {
        let mut by_key: BTreeMap<String, RosterWeapon> = BTreeMap::new();
        for captured in entries {
            let key = weapon_key(&captured.name);
            match by_key.get_mut(&key) {
                Some(first) => merge_duplicate(first, captured),
                None => {
                    by_key.insert(key, captured);
                }
            }
        }
        let records = std::iter::once(WeaponEntry::default())
            .chain(
                by_key
                    .into_iter()
                    .map(|(gun_name, cap)| WeaponEntry::from_capture(gun_name, cap)),
            )
            .collect();
        let mut registry = WeaponLedger {
            records,
            epoch_no: fresh_epoch(),
            ..WeaponLedger::default()
        };
        registry.rebuild_name_maps();
        Self {
            registry,
            family_tables: Vec::new(),
        }
    }

    pub fn publish(self) -> WeaponLedger {
        let Self {
            registry: mut ledger,
            family_tables,
        } = self;
        ledger.families = crate::WeaponFamilies::build(&family_tables, &ledger);
        ledger
    }

    pub fn set_family_tables(
        &mut self,
        tables: Vec<(crate::AssetCategoryScope, crate::SnapshotStringTable)>,
    ) {
        self.family_tables = tables;
    }

    /// Names changed: rebuild the lookups and take a new revision.
    fn reindex(&mut self) {
        self.registry.rebuild_name_maps();
        self.registry.epoch_no = fresh_epoch();
    }

    /// The real weapons, without the empty row 0.
    fn weapons_mut(&mut self) -> impl Iterator<Item = &mut WeaponEntry> {
        self.registry.records.iter_mut().skip(1)
    }

    /// Add another zone walk's weapons after this one's.
    pub fn merge_in(&mut self, mut incoming: Self) {
        match (
            incoming.registry.is_empty(),
            self.registry.records.is_empty(),
        ) {
            (true, _) => self.family_tables.append(&mut incoming.family_tables),
            (false, true) => *self = incoming,
            (false, false) => {
                let newcomers = incoming.registry.records.into_iter().skip(1);
                self.registry.records.extend(newcomers);
                self.family_tables.append(&mut incoming.family_tables);
                self.reindex();
            }
        }
    }

    pub fn set_scope_tag(&mut self, space: crate::AssetCategoryScope) {
        self.weapons_mut().for_each(|row| row.source_ns = space);
        self.reindex();
    }

    pub fn resolve_combat_fx(
        &mut self,
        effects: &crate::EffectRoster,
        tracers: &crate::TracerRoster,
    ) {
        for row in self.weapons_mut() {
            let space = row.source_ns;
            relink_combat_fx(&mut row.fx, space, effects, tracers);
        }
    }

    pub fn link_hud_materials(&mut self, defs: &crate::MaterialDefinitions) {
        for row in &mut self.registry.records {
            let space = row.source_ns;
            let hud = &mut row.hud;
            if hud.scope_image.is_none() {
                let found = hud
                    .scope
                    .caption
                    .as_deref()
                    .and_then(|text| defs.material_index_by_ns(space, text));
                hud.scope_image = found
                    .and_then(|at| defs.materials.get(at.order()))
                    .and_then(|material| defs.overlay_picture_name(material))
                    .map(str::to_owned);
            }
            let sight = &mut row.sight;
            let center = Slot::new(sight.middle_surface.as_deref(), space);
            sight.center_edge = center.material(defs, sight.center_authored);
            let side = Slot::new(sight.flank_surface.as_deref(), space);
            sight.side_edge = side.material(defs, sight.side_authored);
            for link in [&mut hud.scope, &mut hud.icon, &mut hud.elimination_emblem] {
                link.resolved = Slot::new(link.caption.as_deref(), space).material(defs, false);
            }
        }
    }

    pub fn link_projectile_effects(&mut self, effects: &crate::EffectRoster) {
        for row in &mut self.registry.records {
            let space = row.source_ns;
            for link in [&mut row.trail, &mut row.ignition] {
                link.resolved = effects.hint_edge(false, link.caption.as_deref(), space);
            }
        }
    }

    pub fn link_anim_clips(&mut self, anims: &crate::MotionClipRoster) {
        for row in &mut self.registry.records {
            let space = row.source_ns;
            row.anims.edges = row
                .anims
                .names
                .each_ref()
                .map(|names| xanim_edges_of(names, space, anims));
        }
    }

    /// Read each animation's inline `sndnt#` and `rmbnt#` notetracks into what
    /// the weapon plays when they fire. Returns how many actions were linked.
    pub fn resolve_notetrack_actions(&mut self, anims: &crate::MotionClipRoster) -> usize {
        self.registry
            .records
            .iter_mut()
            .map(|rec| {
                rec.notetracks = notetrack_table(rec, anims);
                rec.notetracks.len()
            })
            .sum()
    }

    pub fn link_fpv_meshes(&mut self, meshes: &crate::FirstPersonMeshRoster) {
        for row in &mut self.registry.records {
            let space = row.source_ns;
            let view = &mut row.view;
            for link in [&mut view.gun, &mut view.missile] {
                link.resolved = Slot::new(link.caption.as_deref(), space).fpv(meshes);
            }
            for set in [&mut view.hip, &mut view.aimed] {
                set.edges = set
                    .models
                    .iter()
                    .map(|text| Slot::named(text, space).fpv(meshes))
                    .collect();
            }
            let plan_for = |set: &Attached<FpvMeshRange>| {
                mount_plan(
                    meshes,
                    view.gun.resolved,
                    view.missile.resolved,
                    &set.edges,
                    &set.offsets,
                )
            };
            // The aimed plan exists only when a sight swaps its model.
            let (hip_plan, aimed_plan) = (plan_for(&view.hip), plan_for(&view.aimed));
            view.plan_hip = hip_plan;
            view.plan_aimed = if view.aimed.edges.is_empty() {
                None
            } else {
                aimed_plan
            };
        }
    }

    pub fn resolve_fpv_hands(
        &mut self,
        meshes: &crate::FirstPersonMeshRoster,
        bodies: &asset_model::CharacterMeshRoster,
    ) -> String {
        // How many rows drew each pair of arms (or none), for the load report.
        let mut picked: BTreeMap<String, usize> = BTreeMap::new();
        for row in &mut self.registry.records {
            let map_ns = meshes.level_scope.unwrap_or(row.source_ns);
            let pick = |left_handed: bool| {
                let kit = bodies.kits().kit(left_handed);
                let choice =
                    asset_model::FirstPersonArms::choose(meshes, map_ns, kit, None, row.source_ns);
                let (space, name) = choice.key()?;
                let at = meshes.row_of(space, name)?;
                let bone_rig = &meshes.at_row(at)?.bone_rig;
                let holds_gun = bone_rig.joint_labels.iter().any(|bone| bone == "tag_weapon");
                (bone_rig.pose.is_some() && holds_gun)
                    .then(|| (choice, crate::FpvMeshSlot::from_sequence(at)))
            };
            row.view.hands = [pick(false), pick(true)];
            let label = match &row.view.hands[0] {
                Some((choice, _)) => format!("{choice:?}"),
                None => "none".to_owned(),
            };
            *picked.entry(label).or_default() += 1;
        }
        let picked: Vec<String> = picked.iter().map(|(k, n)| format!("{k} x{n}")).collect();
        format!("FPV hands (side 0): {}", picked.join(", "))
    }

    /// Put each weapon's gun, attachments and hands together for both sides,
    /// and bind every animation it plays to the parts.
    pub fn resolve_fpv_assemblies(
        &mut self,
        meshes: &crate::FirstPersonMeshRoster,
        anims: &crate::MotionClipRoster,
    ) -> FpvAssemblyCensus {
        let mut assembler = Assembler {
            view_roster: meshes,
            cache: HashMap::new(),
            built: 0,
        };
        let mut tracks = crate::FpvClipTracks::default();
        let mut census = FpvAssemblyCensus::default();
        self.registry.alternate_fpv.clear();

        // Every weapon as itself, then each weapon that is another's alternate
        // and sits in the inventory as a held item as it looks in that
        // parent's hands, which hides different tags.
        let count = self.registry.records.len() as u32;
        let as_held_alternate = |base: u32| {
            let alternate = self.registry.alternate_of(base);
            let held_item = |facts: WeaponChassisSummary| facts.carry_slot_kind == 3;
            let eligible = alternate != 0 && self.registry.summary_for(alternate).is_some_and(held_item);
            eligible.then_some((alternate, base))
        };
        let alt_pairs: Vec<(u32, u32)> = (0..count)
            .map(|id| (id, 0))
            .chain((1..count).filter_map(as_held_alternate))
            .collect();

        for (id, base) in alt_pairs {
            let concealed_bones =
                crate::effective_hide_tags(&self.registry, if base == 0 { id } else { base });
            let row = &mut self.registry.records[id as usize];
            if base == 0 {
                row.view.sides = [None, None];
            }
            let Some(Ok(hip)) = &row.view.plan_hip else {
                continue;
            };
            let aimed = row
                .view
                .plan_aimed
                .as_ref()
                .and_then(|plan| plan.as_ref().ok());
            let sides: [Option<Result<crate::FpvSideAssemblies, String>>; 2] = row.view.hands.each_ref().map(|hands| {
                let (_, hands) = hands.as_ref()?;
                Some(assembler.side(*hands, hip, aimed, &concealed_bones))
            });
            bind_clips_to_parts(&mut tracks, meshes, anims, &row.bound_clips(), &sides);
            for side_hit in sides.iter().flatten() {
                match side_hit {
                    Ok(_) => census.linked += 1,
                    Err(_) => census.refused += 1,
                }
            }
            if base == 0 {
                row.view.sides = sides;
            } else {
                self.registry.alternate_fpv.insert((id, base), sides);
            }
        }
        census.built = assembler.built;
        census.clip_tables = tracks.len();
        self.registry.fpv_clip_tracks = Arc::new(tracks);
        census
    }

    pub fn link_world_meshes(&mut self, guns: &crate::LevelWeaponRoster) {
        self.registry.world_catalog_identity = guns.identity();
        for row in &mut self.registry.records {
            let space = row.source_ns;
            let world = &mut row.world;
            world.gun.resolved = Slot::new(world.gun.caption.as_deref(), space).world_gun(guns);
            world.attached.edges = world
                .attached
                .models
                .iter()
                .map(|text| Slot::named(text, space).world_gun(guns))
                .collect();

            let gun_bones = world
                .gun
                .resolved
                .extent_position()
                .and_then(|at| guns.at_row(at))
                .map(|gun| &gun.bone_rig.joint_labels);
            let find_bone = |wanted: &str| {
                let bones = gun_bones?;
                bones
                    .iter()
                    .find(|bone| bone.eq_ignore_ascii_case(wanted))
                    .cloned()
            };
            // The gun bone an attachment sits on: the tag it names, else the
            // gun's bone that matches the attachment model's own root.
            let bone_for = |edge: &AssetDependencyLink<WorldWeaponRange>, tag: Option<&String>| match tag {
                Some(tag) if !tag.is_empty() => find_bone(tag),
                _ => {
                    let model = guns.at_row(edge.extent_position()?)?;
                    find_bone(model.bone_rig.joint_labels.first()?)
                }
            };
            world.mounts = world
                .attached
                .edges
                .iter()
                .zip(world.tags.iter().map(Some).chain(std::iter::repeat(None)))
                .map(|(edge, tag)| bone_for(edge, tag))
                .collect();
        }
    }
}
