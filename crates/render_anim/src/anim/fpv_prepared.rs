//! Preparing every first-person weapon before the first one is shown.
//!
//! A world load hands over the weapon registry, the first-person mesh catalog,
//! the animation catalog and the session's materials. `prepare_fpv_compositions`
//! walks them in 6 ms slices a frame:
//!
//! 1. **Admit**: every mesh surface gets a material verdict,
//! 2. **Layout**: every (mesh, hidden parts) pair gets a skin layout,
//! 3. **Weapons**: every weapon, on each side, gets its compositions and rigs.
//!
//! The result is an `FpvWeaponTable` the presenter reads without waiting.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use asset_anim::XAnimCatalog;
use asset_core::AssetNamespace;
use asset_game::{
    FpvAssembly, FpvSideAssemblies, WEAPON_ANIM_SLOTS, WeaponAnimSlot, WeaponAnimations,
    WeaponBodyFacts, WeaponRegistry,
};
use asset_material::TS_COLOR_MAP;
use asset_model::{FpvHands, FpvMeshCatalog, FpvMeshEntry};
use assets::{AssetEdge, FpvMeshIndex, PreparedFpvMeshes, PreparedWeapons, PreparedXAnims};
use bevy::prelude::*;
use bevy::render::render_resource::Face;
use render_material::{RuntimeMaterial, RuntimeMaterialCatalog, RuntimeSortedMaterialTable};
use render_scene::{SmodelPassMaterial, TessMaterials, WorldModelLightingAtlas};

use crate::anim::fpv_rig::{
    FpvMaterialAdmission, FpvSurfaceVerdict, PreparedFpvComposition, PreparedFpvModel,
    PreparedFpvRig, compose_clip_tracks,
};
use crate::gaps::RenderGapCause;

/// What one frame may spend on preparation.
const PREPARE_FRAME_BUDGET: Duration = Duration::from_millis(6);

/// The inapplicable verdict for a surface whose technique has no colour map.
const NO_COLOUR_MAP: &str = "technique samples no colour map";

/// Most material names the per-gun census keeps.
const HINTS_KEPT: usize = 12;
const SKIPS_KEPT: usize = 8;

/// Most refused weapons the preparation log lists.
const REFUSALS_LOGGED: usize = 16;

// ---------------------------------------------------------------------------
// The rigs of one weapon side
// ---------------------------------------------------------------------------

/// Which assembly of a weapon a rig was built for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RigVariant {
    Bare,
    Rocket,
    Melee,
    Ads,
}

/// One value per (variant, hand).
#[derive(Debug)]
struct VariantSlots<T> {
    bare: [Option<T>; 2],
    rocket: [Option<T>; 2],
    melee: [Option<T>; 2],
    /// bo2mp: the gun aimed, its sight swapped for the stripped model.
    ads: [Option<T>; 2],
}

impl<T> Default for VariantSlots<T> {
    fn default() -> Self {
        Self {
            bare: [None, None],
            rocket: [None, None],
            melee: [None, None],
            ads: [None, None],
        }
    }
}

impl<T> VariantSlots<T> {
    fn of(&self, variant: RigVariant) -> &[Option<T>; 2] {
        match variant {
            RigVariant::Bare => &self.bare,
            RigVariant::Rocket => &self.rocket,
            RigVariant::Melee => &self.melee,
            RigVariant::Ads => &self.ads,
        }
    }

    fn of_mut(&mut self, variant: RigVariant) -> &mut [Option<T>; 2] {
        match variant {
            RigVariant::Bare => &mut self.bare,
            RigVariant::Rocket => &mut self.rocket,
            RigVariant::Melee => &mut self.melee,
            RigVariant::Ads => &mut self.ads,
        }
    }

    /// The value the frame's state asks for. A gun's rocket shows only if it
    /// has a rocket rig (`rocket` is true for any gun with a round in it), and
    /// the aimed sight only on a gun that does not.
    fn pick(&self, combined: bool, rocket: bool, dual: bool, melee: bool, ads: bool) -> Option<&T> {
        let hand = usize::from(dual && !combined);
        let held = |variant| self.of(variant)[hand].as_ref();
        if melee && let Some(found) = held(RigVariant::Melee) {
            return Some(found);
        }
        if ads
            && held(RigVariant::Rocket).is_none()
            && let Some(found) = held(RigVariant::Ads)
        {
            return Some(found);
        }
        if rocket && let Some(found) = held(RigVariant::Rocket) {
            return Some(found);
        }
        held(RigVariant::Bare)
    }
}

/// The rigs of one weapon on one side.
#[derive(Default)]
pub struct FpvRigSet {
    rigs: VariantSlots<Arc<PreparedFpvRig>>,
    /// bo2zm: Black Ops II dual wield: one rig holds both guns and poses
    /// both hands' clips together (`PreparedFpvRig::pose_combined`).
    pub combined: bool,
}

impl FpvRigSet {
    pub fn pick(
        &self,
        rocket: bool,
        dual: bool,
        melee: bool,
        ads: bool,
    ) -> Option<&Arc<PreparedFpvRig>> {
        self.rigs.pick(self.combined, rocket, dual, melee, ads)
    }
}

/// A rig to build for each assembly of a weapon: the hand slot it fills,
/// whether it carries the left gun too, and whether the left clips are bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RigSpec {
    hand: usize,
    dual: bool,
    with_left_clips: bool,
}

/// bo2zm: both guns on one set of arms (Black Ops II) take one rig that binds
/// both hands' clips. Otherwise the right hand has a rig of its own, and, if
/// the weapon has a left gun, a second rig with the left one laid out beside it.
fn rig_specs(combined: bool, has_left: bool) -> Vec<RigSpec> {
    if combined {
        return vec![RigSpec {
            hand: 0,
            dual: false,
            with_left_clips: true,
        }];
    }
    let mut specs = vec![RigSpec {
        hand: 0,
        dual: false,
        with_left_clips: false,
    }];
    if has_left {
        specs.push(RigSpec {
            hand: 1,
            dual: true,
            with_left_clips: true,
        });
    }
    specs
}

// ---------------------------------------------------------------------------
// Weapon views and the table
// ---------------------------------------------------------------------------

/// What the presenter can say about a gun's surfaces without looking again.
#[derive(Clone, Debug, Default)]
pub struct FpvViewCensus {
    pub gun_colormap_skip_n: u32,
    pub gun_colormap_skip_names: Vec<String>,
    pub mat_hints: Vec<String>,
}

fn push_name_capped(names: &mut Vec<String>, name: &str, cap: usize) {
    if names.len() < cap && !names.iter().any(|seen| seen == name) {
        names.push(name.to_owned());
    }
}

/// Counts a gun's surfaces by verdict: names of what draws (a hint at what
/// the gun is made of) and of what the technique left without a colour map.
fn census_of_verdicts<'a>(
    surfaces: impl IntoIterator<Item = (&'a FpvSurfaceVerdict, Option<&'a str>)>,
) -> FpvViewCensus {
    let mut census = FpvViewCensus::default();
    for (verdict, name) in surfaces {
        match verdict {
            FpvSurfaceVerdict::Admitted(_) => {
                if let Some(name) = name {
                    push_name_capped(&mut census.mat_hints, name, HINTS_KEPT);
                }
            }
            FpvSurfaceVerdict::Inapplicable(why) if *why == NO_COLOUR_MAP => {
                census.gun_colormap_skip_n = census.gun_colormap_skip_n.saturating_add(1);
                if let Some(name) = name {
                    push_name_capped(&mut census.gun_colormap_skip_names, name, SKIPS_KEPT);
                }
            }
            _ => {}
        }
    }
    census
}

pub struct FpvWeaponView {
    pub gun_name: String,
    pub gun_index: FpvMeshIndex,
    pub hands: FpvHands,
    pub hands_index: FpvMeshIndex,
    pub namespace: AssetNamespace,
    pub assemblies: FpvSideAssemblies,
    pub right: WeaponAnimations,
    pub left: Option<WeaponAnimations>,
    pub idle_name: Option<String>,
    pub rigs: FpvRigSet,
    pub census: FpvViewCensus,
}

pub enum FpvWeaponSlot {
    Absent,
    Ready(Arc<FpvWeaponView>),
    Refused(RenderGapCause),
}

static ABSENT_SLOT: FpvWeaponSlot = FpvWeaponSlot::Absent;

#[derive(Clone, Copy, Debug, Default)]
pub struct FpvPreparationCensus {
    pub models: usize,
    pub layouts: usize,
    pub compositions: usize,
    pub rigs: usize,
    pub materials: usize,
    pub elapsed_ms: f64,
}

/// Inventory type of a weapon that is swapped in for another (an alternate
/// fire mode), which has slots of its own per parent.
const ALTERNATE_INVENTORY_TYPE: i32 = 3;

type SidePair = [FpvWeaponSlot; 2];

pub struct FpvWeaponTable {
    material_catalog: Arc<RuntimeMaterialCatalog>,
    catalog_id: u64,
    facts: Vec<Option<WeaponBodyFacts>>,
    hud_iris: Vec<bool>,
    melee: Vec<u32>,
    guns: Vec<Option<FpvMeshIndex>>,
    slots: Vec<SidePair>,
    alternate_slots: HashMap<(u32, u32), SidePair>,
    census: FpvPreparationCensus,
}

impl FpvWeaponTable {
    pub fn census(&self) -> FpvPreparationCensus {
        self.census
    }

    pub fn material_catalog(&self) -> &Arc<RuntimeMaterialCatalog> {
        &self.material_catalog
    }

    pub fn catalog_id(&self) -> u64 {
        self.catalog_id
    }

    pub fn facts_of(&self, weapon: u32) -> Option<WeaponBodyFacts> {
        self.facts.get(weapon as usize).copied().flatten()
    }

    pub fn overlay_is_hud_iris(&self, weapon: u32) -> bool {
        self.hud_iris.get(weapon as usize).copied().unwrap_or(false)
    }

    pub fn melee_weapon_of(&self, weapon: u32) -> u32 {
        self.melee.get(weapon as usize).copied().unwrap_or(weapon)
    }

    pub fn gun_index(&self, weapon: u32) -> Option<FpvMeshIndex> {
        self.guns.get(weapon as usize).copied().flatten()
    }

    pub fn slot(&self, weapon: u32, parent: u32, axis: bool) -> &FpvWeaponSlot {
        let side = usize::from(axis);
        let alternate = parent != 0
            && self
                .facts_of(weapon)
                .is_some_and(|facts| facts.inventory_type == ALTERNATE_INVENTORY_TYPE);
        let sides = if alternate {
            self.alternate_slots.get(&(weapon, parent))
        } else {
            self.slots.get(weapon as usize)
        };
        sides.map_or(&ABSENT_SLOT, |sides| &sides[side])
    }
}

// ---------------------------------------------------------------------------
// Preparation state
// ---------------------------------------------------------------------------

/// What a preparation was made from. A table stays valid while every part is
/// the same one.
struct FpvPreparationOwner {
    materials: Arc<RuntimeMaterialCatalog>,
    images: Arc<Vec<Option<Handle<Image>>>>,
    weapons: Arc<WeaponRegistry>,
    catalog_id: u64,
    atlas: Option<bevy::asset::AssetId<Image>>,
}

impl FpvPreparationOwner {
    fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.materials, &other.materials)
            && Arc::ptr_eq(&self.images, &other.images)
            && Arc::ptr_eq(&self.weapons, &other.weapons)
            && self.catalog_id == other.catalog_id
            && self.atlas == other.atlas
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FpvPreparationStage {
    Admit,
    Layout,
    Weapons,
    Done,
}

/// A mesh and the parts it hides.
type LayoutKey = (usize, Option<[u32; 5]>);

/// Two rigs are the same when they share a composition, a dual flag and the
/// clips bound for each hand.
type RigKey = (usize, bool, Vec<Option<usize>>, Vec<Option<usize>>);

struct FpvPreparationJob {
    owner: FpvPreparationOwner,
    stage: FpvPreparationStage,
    started: Instant,
    progress: Option<asset_transport::StageHandle>,
    work_total: u64,
    work_done: u64,

    models: Vec<usize>,
    next_model: usize,
    admission: FpvMaterialAdmission,
    image_cache: HashMap<u32, Handle<Image>>,

    layouts_queue: Vec<LayoutKey>,
    next_layout: usize,
    layouts: HashMap<LayoutKey, Result<Arc<PreparedFpvModel>, String>>,

    next_weapon: u32,
    alternate_queue: Vec<(u32, u32)>,
    compositions: HashMap<usize, Result<Arc<PreparedFpvComposition>, String>>,
    tracks: HashMap<(usize, usize), Option<Arc<[u16]>>>,
    rigs: HashMap<RigKey, Arc<PreparedFpvRig>>,
    facts: Vec<Option<WeaponBodyFacts>>,
    hud_iris: Vec<bool>,
    melee: Vec<u32>,
    guns: Vec<Option<FpvMeshIndex>>,
    slots: Vec<SidePair>,
    alternate_slots: HashMap<(u32, u32), SidePair>,
    refused: Vec<String>,
}

#[derive(Resource, Default)]
pub struct PreparedFpv {
    job: Option<FpvPreparationJob>,
    table: Option<(FpvPreparationOwner, Arc<FpvWeaponTable>)>,
    /// bo2zm: the match has no weapon, first-person mesh or xanim catalog at
    /// all (a Black Ops II world before weapons are read), so there is
    /// nothing to prepare and nothing for the world spawn to wait on.
    inputs_absent: bool,
}

impl PreparedFpv {
    /// bo2zm: see `inputs_absent`.
    pub fn inputs_absent(&self) -> bool {
        self.inputs_absent && self.job.is_none() && self.table.is_none()
    }

    pub fn table(&self) -> Option<&Arc<FpvWeaponTable>> {
        self.table.as_ref().map(|(_, table)| table)
    }

    pub fn settled_for(&self, materials: &Arc<RuntimeMaterialCatalog>) -> bool {
        self.job.is_none()
            && self
                .table()
                .is_some_and(|table| Arc::ptr_eq(&table.material_catalog, materials))
    }

    pub fn clear(&mut self) {
        if let Some(stage) = self.job.take().and_then(|job| job.progress) {
            stage.cancel();
        }
        self.table = None;
    }
}

fn assembly_key(assembly: &Arc<FpvAssembly>) -> usize {
    Arc::as_ptr(assembly) as usize
}

fn composition_key(composition: &Arc<PreparedFpvComposition>) -> usize {
    Arc::as_ptr(composition) as usize
}

// ---------------------------------------------------------------------------
// Material admission
// ---------------------------------------------------------------------------

/// The authored material a surface is bound to.
struct SurfaceBinding<'a> {
    /// Index into the session's material catalog.
    catalog: usize,
    /// Name the surface's material is presented under.
    name: &'a str,
    /// Index of the material within the model, which is what surfaces of one
    /// model share a draw material by.
    authored: usize,
}

/// Follows a surface to its material, or says why it has none.
fn bind_surface(
    entry: &FpvMeshEntry,
    surface: usize,
) -> Result<SurfaceBinding<'_>, FpvSurfaceVerdict> {
    let edge = entry
        .material_edges
        .get(surface)
        .copied()
        .unwrap_or(AssetEdge::Absent);
    let leftover = entry
        .material_keys
        .get(surface)
        .and_then(|key| Some(key.as_ref()?.name.as_str()))
        .unwrap_or("-");
    let refuse = |material: &str, cause: &'static str| FpvSurfaceVerdict::Refused {
        material: material.to_owned(),
        cause,
    };
    let catalog = match edge {
        AssetEdge::Absent => return Err(FpvSurfaceVerdict::Inapplicable("no material")),
        AssetEdge::Unresolved(_) => return Err(refuse(leftover, edge.edge_kind())),
        AssetEdge::Bound(_) => edge
            .bound_index()
            .ok_or_else(|| refuse(leftover, "bound edge without an index"))?,
    };
    let name = entry
        .material_present_name(surface)
        .ok_or_else(|| refuse(leftover, "bound material has no name"))?;
    let authored = entry
        .skel
        .surface_materials
        .get(surface)
        .copied()
        .flatten()
        .map(|index| index.get())
        .ok_or_else(|| refuse(name, "surface carries no authored material"))?;
    Ok(SurfaceBinding {
        catalog,
        name,
        authored,
    })
}

/// The session's uploaded images, remembering the ones a model asked for.
struct ImageLookup<'a> {
    uploaded: &'a [Option<Handle<Image>>],
    asked: &'a mut HashMap<u32, Handle<Image>>,
}

impl ImageLookup<'_> {
    fn get(&mut self, index: u32) -> Option<Handle<Image>> {
        if let Some(known) = self.asked.get(&index) {
            return Some(known.clone());
        }
        let handle = self.uploaded.get(index as usize)?.clone()?;
        self.asked.insert(index, handle.clone());
        Some(handle)
    }
}

/// The image a material binds to a texture slot of this meaning.
fn image_of(material: &RuntimeMaterial, semantic: u8) -> Option<u32> {
    material.textures.iter().find_map(|(_, texture)| {
        texture
            .filter(|binding| binding.semantic == semantic)
            .map(|binding| binding.image.0)
    })
}

/// Hash of the constant that carries a material's environment-map parameters.
const ENV_MAP_PARMS: u32 = 1_033_475_292;

fn env_map_parms_of(constants: &[(u32, [u32; 4])]) -> [f32; 4] {
    constants
        .iter()
        .find(|(hash, _)| *hash == ENV_MAP_PARMS)
        .map_or([0.0; 4], |(_, words)| words.map(f32::from_bits))
}

/// The face the material's first state culls.
fn cull_face_of(state_bits_table: &[[u32; 2]]) -> Option<Face> {
    let bits = state_bits_table.first()?;
    match asset_material::cull_face_from_state_bits(*bits) {
        asset_material::MaterialCullFace::Back => Some(Face::Back),
        asset_material::MaterialCullFace::Front => Some(Face::Front),
        asset_material::MaterialCullFace::None => None,
    }
}

/// Gives one surface its verdict, admitting its material the first time it is
/// seen in its model.
fn admit_surface(
    global: &RuntimeMaterialCatalog,
    images: &[Option<Handle<Image>>],
    entry: &FpvMeshEntry,
    surface: usize,
    lighting: &WorldModelLightingAtlas,
    admission: &mut FpvMaterialAdmission,
    image_cache: &mut HashMap<u32, Handle<Image>>,
) -> FpvSurfaceVerdict {
    use lighting_t6::{
        MODEL_LIGHTING_INV_ATLAS_WIDTH, MODEL_LIGHTING_VOLUME_W, model_lighting_inv_image_height,
        model_lighting_lookup_scale,
    };

    let binding = match bind_surface(entry, surface) {
        Ok(binding) => binding,
        Err(verdict) => return verdict,
    };
    if let Some(&row) = admission.by_authored.get(&binding.authored) {
        return FpvSurfaceVerdict::Admitted(row);
    }
    let refuse = |cause: &'static str| FpvSurfaceVerdict::Refused {
        material: binding.name.to_owned(),
        cause,
    };
    let Some(authored) = global.materials.get(binding.catalog) else {
        return refuse("material outside the session catalog");
    };
    let ordinal = global.ordinal_for_asset_id(assets::MaterialIndex::from_order(binding.catalog));
    // bo2zm: Black Ops II materials have no technique set and so no sorted
    // ordinal; the fallback pass draws them by catalog index.
    if ordinal.is_none() && authored.t6_draw.is_none() {
        return refuse("material has no sorted ordinal");
    }
    let Some(color_index) = image_of(authored, TS_COLOR_MAP) else {
        return FpvSurfaceVerdict::Inapplicable(NO_COLOUR_MAP);
    };
    let mut lookup = ImageLookup {
        uploaded: images,
        asked: image_cache,
    };
    let Some(color) = lookup.get(color_index) else {
        return refuse("colour map not uploaded");
    };
    let specular =
        image_of(authored, asset_material::TS_SPECULAR_MAP).and_then(|index| lookup.get(index));
    let Some(inv_height) = model_lighting_inv_image_height(lighting.dims.image_height) else {
        return refuse("model lighting atlas has no rows");
    };
    let scale = model_lighting_lookup_scale(inv_height);

    let row = admission.materials.len() as u32;
    admission.materials.push(SmodelPassMaterial {
        model_lighting_required: true,
        color: Some(color),
        specular,
        probe: None,
        atlas: Some(lighting.image.clone()),
        alpha_mode: AlphaMode::Opaque,
        draw_mode: None,
        cull_mode: cull_face_of(&authored.state_bits_table),
        env_map_parms: env_map_parms_of(&authored.constants),
        lighting_lookup_scale: [scale.u, scale.v, scale.w, scale.q],
        atlas_lookup: [
            MODEL_LIGHTING_INV_ATLAS_WIDTH as f32,
            inv_height,
            MODEL_LIGHTING_VOLUME_W,
            0.0,
        ],
        sort_key: authored.sort_key,
        material_sorted_index: ordinal.map(|o| o.get()),
        // bo2zm: see `SmodelPassMaterial::t6_material`.
        t6_material: authored.t6_draw.map(|_| binding.catalog as u32),
    });
    admission.by_authored.insert(binding.authored, row);
    FpvSurfaceVerdict::Admitted(row)
}

// ---------------------------------------------------------------------------
// Which meshes and layouts a registry needs
// ---------------------------------------------------------------------------

/// The meshes (in first-seen order) and the (mesh, hidden parts) layouts the
/// registry's assemblies call for.
fn work_lists(registry: &WeaponRegistry, fpv: &FpvMeshCatalog) -> (Vec<usize>, Vec<LayoutKey>) {
    let weapon_n = registry.len() as u32;
    let mut models = Vec::new();
    let mut layouts = Vec::new();
    let mut seen_models = HashSet::new();
    let mut seen_layouts = HashSet::new();
    let mut seen_assemblies = HashSet::new();

    let weapons = (1..=weapon_n)
        .map(|id| (id, 0))
        .chain(registry.alternate_fpv_pairs());
    for (id, parent) in weapons {
        for axis in [false, true] {
            let Some(sides) = registry.fpv_assemblies_for(id, parent, axis) else {
                continue;
            };
            let assemblies = std::iter::once(&sides.bare)
                .chain(&sides.rocket)
                .chain(&sides.melee)
                .chain(&sides.ads);
            for assembly in assemblies {
                if !seen_assemblies.insert(assembly_key(assembly)) {
                    continue;
                }
                for part in &assembly.parts {
                    let order = part.model.order();
                    if fpv.get_at(order).is_none() {
                        continue;
                    }
                    if seen_models.insert(order) {
                        models.push(order);
                    }
                    if seen_layouts.insert((order, part.hide)) {
                        layouts.push((order, part.hide));
                    }
                }
            }
        }
    }
    (models, layouts)
}

/// The gun mesh of a weapon, when the catalog has it.
fn gun_mesh_of(
    registry: &WeaponRegistry,
    fpv: &FpvMeshCatalog,
    weapon: u32,
) -> Option<FpvMeshIndex> {
    registry
        .gun_xmodel_edge_of(weapon)
        .and_then(|edge| edge.bound_index())
        .filter(|&order| fpv.get_at(order).is_some())
        .map(FpvMeshIndex::from_order)
}

fn unresolved(weapon_id: u32, role: &'static str, name: String) -> RenderGapCause {
    RenderGapCause::FpvDependencyUnresolved {
        weapon_id,
        role,
        name,
    }
}

/// Whether a weapon gets a left-hand set of clips: dual wield is allowed and
/// the right-hand set has an idle clip to pair it with.
fn has_paired_hands(no_dual_wield: Option<bool>, right_idle_bound: bool) -> bool {
    no_dual_wield == Some(false) && right_idle_bound
}

// ---------------------------------------------------------------------------
// What a weapon's two sides share
// ---------------------------------------------------------------------------

/// The part of a weapon's preparation that does not depend on the side.
struct WeaponShared {
    id: u32,
    parent: u32,
    gun_index: FpvMeshIndex,
    gun_name: String,
    namespace: AssetNamespace,
    right: WeaponAnimations,
    left: Option<WeaponAnimations>,
    idle_name: Option<String>,
    census: FpvViewCensus,
    right_orders: Vec<Option<usize>>,
    left_orders: Vec<Option<usize>>,
}

/// An assembly group's compositions, or why one is unusable.
type Composed = Result<Vec<Option<Arc<PreparedFpvComposition>>>, RenderGapCause>;

impl FpvPreparationJob {
    fn new(
        owner: FpvPreparationOwner,
        fpv: &FpvMeshCatalog,
        progress: Option<asset_transport::StageHandle>,
    ) -> Self {
        let registry = Arc::clone(&owner.weapons);
        let weapon_n = registry.len() as u32;
        let (models, layouts_queue) = work_lists(&registry, fpv);
        let mut alternates: Vec<_> = registry.alternate_fpv_pairs().collect();
        let work_total =
            (models.len() + layouts_queue.len() + alternates.len()) as u64 + u64::from(weapon_n);
        if let Some(stage) = &progress {
            stage.set_total(work_total);
        }
        // Popped from the back, so the lowest pair goes first.
        alternates.sort_unstable_by(|a, b| b.cmp(a));
        let room = weapon_n as usize + 1;
        Self {
            owner,
            stage: FpvPreparationStage::Admit,
            started: Instant::now(),
            progress,
            work_total,
            work_done: 0,
            models,
            next_model: 0,
            admission: FpvMaterialAdmission::default(),
            image_cache: HashMap::new(),
            layouts_queue,
            next_layout: 0,
            layouts: HashMap::new(),
            next_weapon: 0,
            alternate_queue: alternates,
            compositions: HashMap::new(),
            tracks: HashMap::new(),
            rigs: HashMap::new(),
            facts: Vec::with_capacity(room),
            hud_iris: Vec::with_capacity(room),
            melee: Vec::with_capacity(room),
            guns: Vec::with_capacity(room),
            slots: Vec::with_capacity(room),
            alternate_slots: HashMap::new(),
            refused: Vec::new(),
        }
    }

    fn finish_unit(&mut self) {
        self.work_done = self.work_done.saturating_add(1);
        if let Some(stage) = &self.progress {
            stage.set_completed(self.work_done.min(self.work_total));
        }
    }

    fn advance(
        &mut self,
        deadline: Instant,
        fpv: &FpvMeshCatalog,
        xanims: &XAnimCatalog,
        tess: &TessMaterials,
        lighting: &WorldModelLightingAtlas,
    ) {
        while self.stage != FpvPreparationStage::Done && Instant::now() < deadline {
            match self.stage {
                FpvPreparationStage::Admit => self.admit_next(fpv, tess, lighting),
                FpvPreparationStage::Layout => self.lay_out_next(fpv),
                FpvPreparationStage::Weapons => self.prepare_next_weapon(fpv, xanims),
                FpvPreparationStage::Done => {}
            }
        }
    }

    fn admit_next(
        &mut self,
        fpv: &FpvMeshCatalog,
        tess: &TessMaterials,
        lighting: &WorldModelLightingAtlas,
    ) {
        let Some(&order) = self.models.get(self.next_model) else {
            self.stage = FpvPreparationStage::Layout;
            return;
        };
        self.next_model += 1;
        if let Some(entry) = fpv.get_at(order) {
            let verdicts = entry
                .skel
                .surfaces_for_lod(0)
                .map(|surface| {
                    let verdict = admit_surface(
                        &tess.catalog,
                        tess.material_images.as_ref(),
                        entry,
                        surface,
                        lighting,
                        &mut self.admission,
                        &mut self.image_cache,
                    );
                    (surface, verdict)
                })
                .collect();
            self.admission.record(order, verdicts);
        }
        self.finish_unit();
    }

    fn lay_out_next(&mut self, fpv: &FpvMeshCatalog) {
        let Some(&(order, hide)) = self.layouts_queue.get(self.next_layout) else {
            self.stage = FpvPreparationStage::Weapons;
            self.next_weapon = 0;
            return;
        };
        self.next_layout += 1;
        let model = PreparedFpvModel::build(fpv, order, hide.as_ref(), &self.admission)
            .map(Arc::new)
            .map_err(|error| error.to_string());
        self.layouts.insert((order, hide), model);
        self.finish_unit();
    }

    /// The composition of an assembly, made once.
    fn composition(
        &mut self,
        assembly: &Arc<FpvAssembly>,
    ) -> Result<Arc<PreparedFpvComposition>, String> {
        let key = assembly_key(assembly);
        if let Some(done) = self.compositions.get(&key) {
            return done.clone();
        }
        let layouts = &self.layouts;
        let composed = PreparedFpvComposition::compose(Arc::clone(assembly), |order, hide| {
            layouts
                .get(&(order, hide))
                .cloned()
                .unwrap_or_else(|| Err("model missing from the first-person catalog".to_owned()))
        })
        .map(Arc::new);
        self.compositions.insert(key, composed.clone());
        composed
    }

    /// Composes a group of assemblies the same way: any that cannot be
    /// composed fails the group with `layout_role`, then any that cannot draw
    /// fails it with `material_role`.
    fn compose_group(
        &mut self,
        weapon: u32,
        assemblies: &[Option<&Arc<FpvAssembly>>],
        layout_role: &'static str,
        material_role: &'static str,
    ) -> Composed {
        let mut composed = Vec::with_capacity(assemblies.len());
        let mut first_error = None;
        for assembly in assemblies {
            let one = assembly.map(|assembly| self.composition(assembly));
            match one {
                Some(Ok(done)) => composed.push(Some(done)),
                Some(Err(error)) => {
                    first_error.get_or_insert(error);
                    composed.push(None);
                }
                None => composed.push(None),
            }
        }
        if let Some(error) = first_error {
            return Err(unresolved(weapon, layout_role, error));
        }
        let refusal = composed
            .iter()
            .flatten()
            .find_map(|composition| composition.refusal());
        if let Some(refusal) = refusal {
            return Err(unresolved(weapon, material_role, refusal.to_string()));
        }
        Ok(composed)
    }

    /// For each of a weapon's clips, the bone each of its tracks drives within
    /// this composition.
    fn clip_tracks(
        &mut self,
        composition: &PreparedFpvComposition,
        orders: &[Option<usize>],
    ) -> Vec<Option<Arc<[u16]>>> {
        let registry = Arc::clone(&self.owner.weapons);
        let assembly = composition.assembly();
        let key = assembly_key(assembly);
        orders
            .iter()
            .map(|order| {
                let order = (*order)?;
                self.tracks
                    .entry((key, order))
                    .or_insert_with(|| {
                        compose_clip_tracks(assembly, order, registry.fpv_clip_tracks())
                    })
                    .clone()
            })
            .collect()
    }

    /// A rig for a composition, made once per distinct set of clips.
    fn rig(
        &mut self,
        composition: &Arc<PreparedFpvComposition>,
        spec: RigSpec,
        right: &[Option<usize>],
        left: &[Option<usize>],
    ) -> Arc<PreparedFpvRig> {
        let left = if spec.with_left_clips { left } else { &[] };
        let key: RigKey = (
            composition_key(composition),
            spec.dual,
            right.to_vec(),
            left.to_vec(),
        );
        if let Some(rig) = self.rigs.get(&key) {
            return Arc::clone(rig);
        }
        let tracks = [
            self.clip_tracks(composition, right),
            self.clip_tracks(composition, left),
        ];
        let rig = Arc::new(PreparedFpvRig::build(
            Arc::clone(composition),
            spec.dual,
            &self.admission,
            tracks,
        ));
        self.rigs.insert(key, Arc::clone(&rig));
        rig
    }

    fn census_of(&self, fpv: &FpvMeshCatalog, gun: FpvMeshIndex) -> FpvViewCensus {
        let order = gun.order();
        let Some(entry) = fpv.get_at(order) else {
            return FpvViewCensus::default();
        };
        census_of_verdicts(
            self.admission
                .verdicts_of(order)
                .iter()
                .map(|(surface, verdict)| {
                    let name = entry
                        .material_keys
                        .get(*surface)
                        .and_then(|key| Some(key.as_ref()?.name.as_str()));
                    (verdict, name)
                }),
        )
    }

    /// The next weapon to prepare: every id in order, then the alternate
    /// (weapon, parent) pairs. `None` when there is no more.
    fn next_target(&mut self, registry: &WeaponRegistry) -> Option<(u32, u32)> {
        if self.next_weapon as usize > registry.len() {
            return self.alternate_queue.pop();
        }
        let id = self.next_weapon;
        self.next_weapon += 1;
        self.facts.push(registry.facts_of(id));
        self.hud_iris.push(registry.overlay_is_hud_iris(id));
        self.melee.push(registry.melee_weapon_of(id));
        Some((id, 0))
    }

    fn store_sides(&mut self, id: u32, parent: u32, sides: SidePair) {
        if parent == 0 {
            self.slots.push(sides);
        } else {
            self.alternate_slots.insert((id, parent), sides);
        }
    }

    /// The animations of a weapon: the right hand's, the left hand's when the
    /// weapon pairs, and the name of the idle clip.
    fn weapon_anims(
        registry: &Arc<WeaponRegistry>,
        id: u32,
        xanims: &XAnimCatalog,
    ) -> (WeaponAnimations, Option<WeaponAnimations>, Option<String>) {
        let idle = WeaponAnimSlot::Idle.index();
        let right_edges = registry.sz_xanim_right_edges_of(id);
        let right_idle_bound = right_edges.is_some_and(|edges| edges[idle].is_bound());
        let no_dual_wield = registry.facts_of(id).map(|facts| facts.no_dual_wield);
        let paired = has_paired_hands(no_dual_wield, right_idle_bound);
        let right = if paired {
            WeaponAnimations::from_registry_edges(registry, id, right_edges, xanims)
        } else {
            WeaponAnimations::from_registry(registry, id, xanims)
        };
        let left_edges = registry.sz_xanim_left_edges_of(id);
        let left_idle_bound = left_edges.is_some_and(|edges| edges[idle].is_bound());
        let left = (paired && left_idle_bound)
            .then(|| WeaponAnimations::from_registry_edges(registry, id, left_edges, xanims));
        let idle_name = registry
            .sz_xanim_edges_of(id)
            .and_then(|row| row[idle].bound_index())
            .and_then(|order| xanims.clip_at(order))
            .map(|clip| clip.name.clone());
        (right, left, idle_name)
    }

    fn prepare_next_weapon(&mut self, fpv: &FpvMeshCatalog, xanims: &XAnimCatalog) {
        let registry = Arc::clone(&self.owner.weapons);
        let Some((id, parent)) = self.next_target(&registry) else {
            self.stage = FpvPreparationStage::Done;
            return;
        };
        let gun = gun_mesh_of(&registry, fpv, id);
        if parent == 0 {
            self.guns.push(gun);
        }
        if id == 0 {
            self.slots
                .push([FpvWeaponSlot::Absent, FpvWeaponSlot::Absent]);
            return;
        }
        let Some(gun_index) = gun else {
            let cause = RenderGapCause::FpvGunXModelUnresolved { weapon_id: id };
            self.store_sides(
                id,
                parent,
                [
                    FpvWeaponSlot::Refused(cause.clone()),
                    FpvWeaponSlot::Refused(cause),
                ],
            );
            self.finish_unit();
            return;
        };

        let (right, left, idle_name) = Self::weapon_anims(&registry, id, xanims);
        let right_orders = right.clip_orders().to_vec();
        debug_assert_eq!(right_orders.len(), WEAPON_ANIM_SLOTS);
        let left_orders = left
            .as_ref()
            .map_or_else(Vec::new, |left| left.clip_orders().to_vec());
        let shared = WeaponShared {
            id,
            parent,
            gun_index,
            gun_name: fpv
                .get_at(gun_index.order())
                .map_or_else(String::new, |entry| entry.skel.name.clone()),
            namespace: registry.namespace_of(id).unwrap_or(AssetNamespace::T6),
            census: self.census_of(fpv, gun_index),
            right,
            left,
            idle_name,
            right_orders,
            left_orders,
        };

        let sides = [false, true].map(|axis| self.prepare_side(&registry, &shared, axis));
        for (axis, side) in sides.iter().enumerate() {
            if let FpvWeaponSlot::Refused(cause) = side
                && self.refused.len() < REFUSALS_LOGGED
            {
                self.refused.push(format!(
                    "{} ({}): {cause}",
                    registry.name_of(id),
                    if axis == 0 { "allies" } else { "axis" }
                ));
            }
        }
        self.store_sides(id, parent, sides);
        self.finish_unit();
    }

    /// One side of a weapon: its compositions, then a rig for each of them.
    fn prepare_side(
        &mut self,
        registry: &WeaponRegistry,
        shared: &WeaponShared,
        axis: bool,
    ) -> FpvWeaponSlot {
        let id = shared.id;
        let (Some((hands, hands_index)), Some(assemblies)) = (
            registry.fpv_hands_of(id, axis),
            registry.fpv_assemblies_for(id, shared.parent, axis),
        ) else {
            return FpvWeaponSlot::Refused(unresolved(
                id,
                "FPV skeleton",
                registry.fpv_assembly_gap_of(id, axis),
            ));
        };

        let bare_and_rocket = match self.compose_group(
            id,
            &[Some(&assemblies.bare), assemblies.rocket.as_ref()],
            "FPV layout",
            "FPV material",
        ) {
            Ok(group) => group,
            Err(cause) => return FpvWeaponSlot::Refused(cause),
        };
        let melee = match self.compose_group(
            id,
            &[assemblies.melee.as_ref()],
            "melee FPV layout",
            "melee FPV material",
        ) {
            Ok(mut group) => group.remove(0),
            Err(cause) => return FpvWeaponSlot::Refused(cause),
        };
        let ads = match self.compose_group(
            id,
            &[assemblies.ads.as_ref()],
            "aimed FPV layout",
            "aimed FPV material",
        ) {
            Ok(mut group) => group.remove(0),
            Err(cause) => return FpvWeaponSlot::Refused(cause),
        };

        let composed = [
            (RigVariant::Bare, bare_and_rocket[0].clone()),
            (RigVariant::Rocket, bare_and_rocket[1].clone()),
            (RigVariant::Melee, melee),
            (RigVariant::Ads, ads),
        ];
        let combined = assemblies.bare.arms_gun && shared.left.is_some();
        let specs = rig_specs(combined, shared.left.is_some());
        let mut rigs = FpvRigSet {
            combined,
            ..FpvRigSet::default()
        };
        for (variant, composition) in composed {
            let Some(composition) = composition else {
                continue;
            };
            for spec in &specs {
                let rig = self.rig(
                    &composition,
                    *spec,
                    &shared.right_orders,
                    &shared.left_orders,
                );
                rigs.rigs.of_mut(variant)[spec.hand] = Some(rig);
            }
        }
        FpvWeaponSlot::Ready(Arc::new(FpvWeaponView {
            gun_name: shared.gun_name.clone(),
            gun_index: shared.gun_index,
            hands: hands.clone(),
            hands_index,
            namespace: shared.namespace,
            assemblies: assemblies.clone(),
            right: shared.right.clone(),
            left: shared.left.clone(),
            idle_name: shared.idle_name.clone(),
            rigs,
            census: shared.census.clone(),
        }))
    }

    fn finish(self) -> (FpvPreparationOwner, FpvWeaponTable) {
        let all = || {
            self.slots
                .iter()
                .chain(self.alternate_slots.values())
                .flatten()
        };
        let ready = all()
            .filter(|slot| matches!(slot, FpvWeaponSlot::Ready(_)))
            .count();
        let refused_n = all()
            .filter(|slot| matches!(slot, FpvWeaponSlot::Refused(_)))
            .count();
        let inapplicable = self
            .models
            .iter()
            .flat_map(|&order| self.admission.verdicts_of(order))
            .filter(|(_, verdict)| matches!(verdict, FpvSurfaceVerdict::Inapplicable(_)))
            .count();
        let census = FpvPreparationCensus {
            models: self.models.len(),
            layouts: self.layouts.len(),
            compositions: self.compositions.len(),
            rigs: self.rigs.len(),
            materials: self.admission.materials.len(),
            elapsed_ms: self.started.elapsed().as_secs_f64() * 1000.0,
        };
        diag::info!(
            Fpv,
            "fpv prepared before Ready: models={} layouts={} compositions={} rigs={} materials={} inapplicable_surfaces={inapplicable} sides ready={ready} refused={refused_n} elapsed={:.1}ms",
            census.models,
            census.layouts,
            census.compositions,
            census.rigs,
            census.materials,
            census.elapsed_ms,
        );
        for line in &self.refused {
            diag::info!(Fpv, "fpv refused before use: {line}");
        }
        if let Some(stage) = self.progress {
            stage.done();
        }
        let table = FpvWeaponTable {
            material_catalog: Arc::clone(&self.owner.materials),
            catalog_id: self.owner.catalog_id,
            facts: self.facts,
            hud_iris: self.hud_iris,
            melee: self.melee,
            guns: self.guns,
            slots: self.slots,
            alternate_slots: self.alternate_slots,
            census,
        };
        (self.owner, table)
    }
}

/// A table in which every weapon is refused for one shared reason.
fn refused_table(
    owner: &FpvPreparationOwner,
    fpv: &FpvMeshCatalog,
    cause: RenderGapCause,
) -> FpvWeaponTable {
    let registry = &owner.weapons;
    let ids = 0..=registry.len() as u32;
    let slot_of = |id: u32| -> SidePair {
        if id == 0 {
            [FpvWeaponSlot::Absent, FpvWeaponSlot::Absent]
        } else {
            [
                FpvWeaponSlot::Refused(cause.clone()),
                FpvWeaponSlot::Refused(cause.clone()),
            ]
        }
    };
    FpvWeaponTable {
        material_catalog: Arc::clone(&owner.materials),
        catalog_id: owner.catalog_id,
        facts: ids.clone().map(|id| registry.facts_of(id)).collect(),
        hud_iris: ids
            .clone()
            .map(|id| registry.overlay_is_hud_iris(id))
            .collect(),
        melee: ids.clone().map(|id| registry.melee_weapon_of(id)).collect(),
        guns: ids
            .clone()
            .map(|id| gun_mesh_of(registry, fpv, id))
            .collect(),
        slots: ids.map(slot_of).collect(),
        alternate_slots: HashMap::new(),
        census: FpvPreparationCensus::default(),
    }
}

/// Why a world cannot have a first-person weapon table at all, if it cannot.
fn shared_refusal(
    sorted_ready: bool,
    has_lighting_atlas: bool,
) -> Option<(RenderGapCause, &'static str)> {
    if !sorted_ready {
        Some((
            RenderGapCause::FpvCatalogMissing,
            "the material generation has no sorted table",
        ))
    } else if !has_lighting_atlas {
        Some((
            RenderGapCause::FpvNoLightingAtlas,
            "no model lighting atlas in this world",
        ))
    } else {
        None
    }
}

#[derive(bevy::ecs::system::SystemParam)]
pub struct PrepareFpvInputs<'w> {
    weapons: Option<Res<'w, PreparedWeapons>>,
    fpv_meshes: Option<Res<'w, PreparedFpvMeshes>>,
    xanims: Option<Res<'w, PreparedXAnims>>,
    tess: Res<'w, TessMaterials>,
    lighting: Option<Res<'w, WorldModelLightingAtlas>>,
    load: Option<Res<'w, assets::MapLoadProcess>>,
}

pub fn prepare_fpv_compositions(inputs: PrepareFpvInputs, mut prepared: ResMut<PreparedFpv>) {
    let PrepareFpvInputs {
        weapons,
        fpv_meshes,
        xanims,
        tess,
        lighting,
        load,
    } = inputs;
    let (Some(weapons), Some(fpv_meshes), Some(xanims)) = (weapons, fpv_meshes, xanims) else {
        if prepared.table.is_some() || prepared.job.is_some() {
            prepared.clear();
        }
        if !prepared.inputs_absent {
            prepared.inputs_absent = true;
        }
        return;
    };
    if prepared.inputs_absent {
        prepared.inputs_absent = false;
    }
    if tess.material_images.is_empty() {
        return;
    }
    let sorted_ready = matches!(
        tess.catalog.sorted_materials,
        RuntimeSortedMaterialTable::Ready { .. }
    );
    let owner = FpvPreparationOwner {
        materials: Arc::clone(&tess.catalog),
        images: Arc::clone(&tess.material_images),
        weapons: Arc::clone(&weapons.0),
        catalog_id: fpv_meshes.0.identity(),
        atlas: lighting.as_ref().map(|atlas| atlas.image.id()),
    };
    let settled = prepared
        .table
        .as_ref()
        .is_some_and(|(settled, _)| settled.same(&owner));
    if prepared.job.is_none() && settled {
        return;
    }
    let restart = prepared
        .job
        .as_ref()
        .is_none_or(|job| !job.owner.same(&owner));
    if restart {
        prepared.clear();
        if let Some((cause, why)) = shared_refusal(sorted_ready, lighting.is_some()) {
            diag::info!(
                Fpv,
                "fpv: {why} — every first-person weapon refused before Ready"
            );
            let table = Arc::new(refused_table(&owner, &fpv_meshes.0, cause));
            prepared.table = Some((owner, table));
            return;
        }
        let progress = load
            .as_deref()
            .filter(|process| !process.is_complete())
            .map(|process| {
                process
                    .progress
                    .begin(asset_transport::StageId::FirstPerson, None)
            });
        prepared.job = Some(FpvPreparationJob::new(owner, &fpv_meshes.0, progress));
    }
    let Some(lighting) = lighting.as_deref() else {
        return;
    };
    let deadline = Instant::now() + PREPARE_FRAME_BUDGET;
    let Some(job) = prepared.job.as_mut() else {
        return;
    };
    job.advance(deadline, &fpv_meshes.0, &xanims.0, &tess, lighting);
    if job.stage != FpvPreparationStage::Done {
        return;
    }
    if let Some(job) = prepared.job.take() {
        let (owner, table) = job.finish();
        prepared.table = Some((owner, Arc::new(table)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slots_of(
        bare: [Option<u8>; 2],
        rocket: [Option<u8>; 2],
        melee: [Option<u8>; 2],
        ads: [Option<u8>; 2],
    ) -> VariantSlots<u8> {
        VariantSlots {
            bare,
            rocket,
            melee,
            ads,
        }
    }

    fn all_variants() -> VariantSlots<u8> {
        slots_of(
            [Some(1), Some(11)],
            [Some(2), Some(12)],
            [Some(3), Some(13)],
            [Some(4), Some(14)],
        )
    }

    #[test]
    fn a_plain_frame_picks_the_bare_rig() {
        assert_eq!(
            all_variants().pick(false, false, false, false, false),
            Some(&1)
        );
    }

    #[test]
    fn melee_beats_everything_when_there_is_a_melee_rig() {
        assert_eq!(
            all_variants().pick(false, false, false, true, true),
            Some(&3)
        );
        let none = slots_of([Some(1), None], [None; 2], [None; 2], [None; 2]);
        assert_eq!(none.pick(false, false, false, true, false), Some(&1));
    }

    #[test]
    fn a_rocket_shows_only_for_a_gun_with_a_rocket_rig() {
        assert_eq!(
            all_variants().pick(false, true, false, false, false),
            Some(&2)
        );
        let no_rocket = slots_of([Some(1), None], [None; 2], [None; 2], [None; 2]);
        assert_eq!(no_rocket.pick(false, true, false, false, false), Some(&1));
    }

    #[test]
    fn the_aimed_sight_shows_only_on_a_gun_without_a_rocket_rig() {
        let aimed_only = slots_of([Some(1), None], [None; 2], [None; 2], [Some(4), None]);
        assert_eq!(aimed_only.pick(false, false, false, false, true), Some(&4));
        assert_eq!(
            all_variants().pick(false, false, false, false, true),
            Some(&1),
            "a rocket rig keeps the bare one when aimed"
        );
    }

    #[test]
    fn dual_wield_reads_the_left_hand_unless_the_rig_is_combined() {
        assert_eq!(
            all_variants().pick(false, false, true, false, false),
            Some(&11)
        );
        assert_eq!(
            all_variants()
                .pick(true, false, true, false, false)
                .copied(),
            Some(1),
            "(combined, dual)"
        );
        assert_eq!(
            all_variants().pick(false, true, true, false, false),
            Some(&12)
        );
        let no_left = slots_of([Some(1), None], [None; 2], [None; 2], [None; 2]);
        assert_eq!(no_left.pick(false, false, true, false, false), None);
    }

    #[test]
    fn an_empty_set_picks_nothing() {
        assert!(
            VariantSlots::<u8>::default()
                .pick(true, true, false, true, true)
                .is_none()
        );
        assert!(
            FpvRigSet::default()
                .pick(false, false, false, false)
                .is_none()
        );
    }

    #[test]
    fn variant_slots_address_their_own_variant() {
        let mut slots = VariantSlots::<u8>::default();
        slots.of_mut(RigVariant::Ads)[1] = Some(9);
        assert_eq!(slots.of(RigVariant::Ads)[1], Some(9));
        assert!(slots.of(RigVariant::Bare).iter().all(Option::is_none));
    }

    #[test]
    fn a_single_gun_has_one_right_hand_rig() {
        assert_eq!(
            rig_specs(false, false),
            [RigSpec {
                hand: 0,
                dual: false,
                with_left_clips: false
            }]
        );
    }

    #[test]
    fn two_guns_add_a_second_rig_with_the_left_clips() {
        assert_eq!(
            rig_specs(false, true),
            [
                RigSpec {
                    hand: 0,
                    dual: false,
                    with_left_clips: false
                },
                RigSpec {
                    hand: 1,
                    dual: true,
                    with_left_clips: true
                },
            ]
        );
    }

    #[test]
    fn arms_holding_both_guns_take_one_combined_rig() {
        assert_eq!(
            rig_specs(true, true),
            [RigSpec {
                hand: 0,
                dual: false,
                with_left_clips: true
            }]
        );
    }

    #[test]
    fn a_weapon_pairs_hands_only_when_dual_wield_is_allowed_with_an_idle_clip() {
        assert!(has_paired_hands(Some(false), true));
        assert!(!has_paired_hands(Some(true), true));
        assert!(!has_paired_hands(Some(false), false));
        assert!(!has_paired_hands(None, true), "no facts: not dual");
    }

    #[test]
    fn the_census_names_what_draws_and_what_lacks_a_colour_map() {
        let admitted = FpvSurfaceVerdict::Admitted(0);
        let skipped = FpvSurfaceVerdict::Inapplicable(NO_COLOUR_MAP);
        let other = FpvSurfaceVerdict::Inapplicable("no material");
        let refused = FpvSurfaceVerdict::Refused {
            material: "m".into(),
            cause: "c",
        };
        let census = census_of_verdicts([
            (&admitted, Some("gun")),
            (&admitted, Some("gun")),
            (&admitted, None),
            (&skipped, Some("glass")),
            (&skipped, Some("glass")),
            (&skipped, None),
            (&other, Some("ignored")),
            (&refused, Some("ignored")),
        ]);
        assert_eq!(census.mat_hints, ["gun"]);
        assert_eq!(
            census.gun_colormap_skip_n, 3,
            "the count includes unnamed surfaces"
        );
        assert_eq!(census.gun_colormap_skip_names, ["glass"]);
    }

    #[test]
    fn the_census_keeps_a_few_names_at_most() {
        let admitted = FpvSurfaceVerdict::Admitted(0);
        let names: Vec<String> = (0..30).map(|n| format!("mat{n}")).collect();
        let census = census_of_verdicts(names.iter().map(|name| (&admitted, Some(name.as_str()))));
        assert_eq!(census.mat_hints.len(), HINTS_KEPT);
        assert_eq!(census.mat_hints[0], "mat0");

        let skipped = FpvSurfaceVerdict::Inapplicable(NO_COLOUR_MAP);
        let census = census_of_verdicts(names.iter().map(|name| (&skipped, Some(name.as_str()))));
        assert_eq!(census.gun_colormap_skip_names.len(), SKIPS_KEPT);
        assert_eq!(census.gun_colormap_skip_n, 30);
    }

    #[test]
    fn environment_parameters_come_from_their_constant() {
        let one = 1.0f32.to_bits();
        let constants = [(7, [9; 4]), (ENV_MAP_PARMS, [one, 0, one, 0])];
        assert_eq!(env_map_parms_of(&constants), [1.0, 0.0, 1.0, 0.0]);
        assert_eq!(env_map_parms_of(&[(7, [one; 4])]), [0.0; 4]);
        assert_eq!(env_map_parms_of(&[]), [0.0; 4]);
    }

    #[test]
    fn a_material_without_states_culls_nothing() {
        assert_eq!(cull_face_of(&[]), None);
    }

    #[test]
    fn a_world_without_sorted_materials_or_an_atlas_refuses_every_weapon() {
        assert!(matches!(
            shared_refusal(false, false),
            Some((RenderGapCause::FpvCatalogMissing, _))
        ));
        assert!(matches!(
            shared_refusal(true, false),
            Some((RenderGapCause::FpvNoLightingAtlas, _))
        ));
        assert!(shared_refusal(true, true).is_none());
    }

    #[test]
    fn an_unresolved_dependency_names_weapon_role_and_what_is_missing() {
        match unresolved(5, "FPV layout", "gone".into()) {
            RenderGapCause::FpvDependencyUnresolved {
                weapon_id,
                role,
                name,
            } => assert_eq!((weapon_id, role, name.as_str()), (5, "FPV layout", "gone")),
            other => panic!("unexpected cause {other:?}"),
        }
    }

    #[test]
    fn a_fresh_preparation_is_not_settled_and_has_no_table() {
        let prepared = PreparedFpv::default();
        assert!(prepared.table().is_none());
        assert!(!prepared.inputs_absent());
        let mut absent = PreparedFpv {
            inputs_absent: true,
            ..PreparedFpv::default()
        };
        assert!(absent.inputs_absent());
        absent.clear();
        assert!(absent.table().is_none());
    }
}
