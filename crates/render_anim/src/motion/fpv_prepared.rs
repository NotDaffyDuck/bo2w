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

use anim_bank::MotionClipRoster;
use asset_core::AssetCategoryScope;
use asset_game::{
    FpvAssembly, FpvSideAssemblies, WEAPON_ANIM_SLOTS, GunMoveRole, GunClipSet,
    WeaponChassisSummary, WeaponLedger,
};
use asset_material::SEMANTIC_COLOR;
use asset_model::{FirstPersonArms, FirstPersonMeshRoster, FirstPersonMeshRecord};
use assets::{AssetDependencyLink, FpvMeshSlot, ReadiedFpvMeshes, StagedWeapons, StagedXAnims};
use bevy::prelude::*;
use bevy::render::render_resource::Face;
use render_material::{LiveMaterial, LiveMaterialLedger, LiveSortedMaterialTable};
use render_scene::{PropPassMaterial, StitchMaterials, LevelModelIllumAtlas};

use crate::motion::fpv_rig::{
    FpvMaterialAdmission, FpvSurfaceVerdict, PreparedFpvComposition, PreparedFpvModel,
    PreparedFpvRig, compose_clip_tracks,
};
use crate::holes::Cause;

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
    arm: usize,
    dual: bool,
    with_left_clips: bool,
}

/// bo2zm: both guns on one set of arms (Black Ops II) take one rig that binds
/// both hands' clips. Otherwise the right hand has a rig of its own, and, if
/// the weapon has a left gun, a second rig with the left one laid out beside it.
fn rig_specs(combined: bool, has_left: bool) -> Vec<RigSpec> {
    if combined {
        return vec![RigSpec {
            arm: 0,
            dual: false,
            with_left_clips: true,
        }];
    }
    let mut specs = vec![RigSpec {
        arm: 0,
        dual: false,
        with_left_clips: false,
    }];
    if has_left {
        specs.push(RigSpec {
            arm: 1,
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
    pub gun_tint_skipped: u32,
    pub gun_tint_skip_list: Vec<String>,
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
    for (verdict, label) in surfaces {
        let (names, kept) = match verdict {
            FpvSurfaceVerdict::WelcomedIn(_) => (&mut census.mat_hints, HINTS_KEPT),
            FpvSurfaceVerdict::Inapplicable(why) if *why == NO_COLOUR_MAP => {
                census.gun_tint_skipped = census.gun_tint_skipped.saturating_add(1);
                (&mut census.gun_tint_skip_list, SKIPS_KEPT)
            }
            _ => continue,
        };
        if let Some(label) = label {
            push_name_capped(names, label, kept);
        }
    }
    census
}

pub struct FpvWeaponView {
    pub gun_name: String,
    pub gun_index: FpvMeshSlot,
    pub hands: FirstPersonArms,
    pub hands_index: FpvMeshSlot,
    pub namespace: AssetCategoryScope,
    pub assemblies: FpvSideAssemblies,
    pub right: GunClipSet,
    pub left: Option<GunClipSet>,
    pub idle_name: Option<String>,
    pub rigs: FpvRigSet,
    pub census: FpvViewCensus,
    /// bo2zm: the script's hands this view was built with (a change re-spawns
    /// the view).
    pub script_hands: Option<usize>,
}

pub enum FpvWeaponSlot {
    NothingThere,
    Ready(Arc<FpvWeaponView>),
    Declined(Cause),
}

static ABSENT_SLOT: FpvWeaponSlot = FpvWeaponSlot::NothingThere;

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

pub struct FpvWeaponTable {
    material_catalog: Arc<LiveMaterialLedger>,
    catalog_id: u64,
    facts: Vec<Option<WeaponChassisSummary>>,
    hud_iris: Vec<bool>,
    melee: Vec<u32>,
    guns: Vec<Option<FpvMeshSlot>>,
    slots: Vec<[FpvWeaponSlot; 2]>,
    alternate_slots: HashMap<(u32, u32), [FpvWeaponSlot; 2]>,
    tally: FpvPreparationCensus,
}

impl FpvWeaponTable {
    pub fn census(&self) -> FpvPreparationCensus {
        self.tally
    }

    pub fn material_catalog(&self) -> &Arc<LiveMaterialLedger> {
        &self.material_catalog
    }

    pub fn catalog_id(&self) -> u64 {
        self.catalog_id
    }

    pub fn info_of(&self, weapon: u32) -> Option<WeaponChassisSummary> {
        self.facts.get(weapon as usize).copied().flatten()
    }

    pub fn hud_iris_overlay(&self, gun_ix: u32) -> bool {
        self.hud_iris.get(gun_ix as usize).copied().unwrap_or(false)
    }

    pub fn melee_weapon_of(&self, weapon: u32) -> u32 {
        self.melee.get(weapon as usize).copied().unwrap_or(weapon)
    }

    pub fn gun_index(&self, weapon: u32) -> Option<FpvMeshSlot> {
        self.guns.get(weapon as usize).copied().flatten()
    }

    pub fn gun_slot(&self, gun_no: u32, owner: u32, along: bool) -> &FpvWeaponSlot {
        let side = usize::from(along);
        let alternate = owner != 0
            && self
                .info_of(gun_no)
                .is_some_and(|facts| facts.carry_slot_kind == ALTERNATE_INVENTORY_TYPE);
        let sides = if alternate {
            self.alternate_slots.get(&(gun_no, owner))
        } else {
            self.slots.get(gun_no as usize)
        };
        sides.map_or(&ABSENT_SLOT, |sides| &sides[side])
    }
}

// ---------------------------------------------------------------------------
// Preparation state
// ---------------------------------------------------------------------------

/// bo2zm: the hands a Black Ops II script put on (`setviewmodel`, sent as
/// the client dvar `bo2zm_viewhands`; Mob's Afterlife turns on
/// `c_zom_ghost_viewhands`). Hands with the same skeleton as the weapon's
/// own replace them; any other name changes nothing.
#[derive(Resource, Default, Clone, PartialEq, Eq, Debug)]
pub struct FpvHandsOverride(pub Option<String>);

/// What a preparation was made from. A table stays valid while every part is
/// the same one.
struct FpvPreparationOwner {
    mat_list: Arc<LiveMaterialLedger>,
    images: Arc<Vec<Option<Handle<Image>>>>,
    weapons: Arc<WeaponLedger>,
    catalog_id: u64,
    atlas: Option<bevy::asset::AssetId<Image>>,
    /// bo2zm: `FpvHandsOverride`'s model in the first-person catalog.
    hands: Option<usize>,
}

impl FpvPreparationOwner {
    fn same(&self, other_in: &Self) -> bool {
        self.same_but_hands(other_in) && self.hands == other_in.hands
    }

    fn same_but_hands(&self, other_v: &Self) -> bool {
        Arc::ptr_eq(&self.mat_list, &other_v.mat_list)
            && Arc::ptr_eq(&self.images, &other_v.images)
            && Arc::ptr_eq(&self.weapons, &other_v.weapons)
            && self.catalog_id == other_v.catalog_id
            && self.atlas == other_v.atlas
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FpvPreparationStage {
    Admit,
    Layout,
    Weapons,
    Done,
}

struct FpvPreparationJob {
    owner: FpvPreparationOwner,
    stage: FpvPreparationStage,
    progress: Option<asset_transport::StageHandle>,
    work_total: u64,
    work_done: u64,

    models: Vec<usize>,
    next_model: usize,
    admission: FpvMaterialAdmission,
    image_cache: HashMap<u32, Handle<Image>>,

    layouts_queue: Vec<(usize, Option<[u32; 5]>)>,
    next_layout: usize,
    layouts: HashMap<(usize, Option<[u32; 5]>), Result<Arc<PreparedFpvModel>, String>>,
    /// bo2zm: a weapon's hands model -> the script's hands that replace it.
    hands_swap: HashMap<usize, usize>,

    next_weapon: u32,
    alternate_queue: Vec<(u32, u32)>,
    compositions: HashMap<usize, Result<Arc<PreparedFpvComposition>, String>>,
    tracks: HashMap<(usize, usize), Option<Arc<[u16]>>>,
    rigs: HashMap<(usize, bool, Vec<Option<usize>>, Vec<Option<usize>>), Arc<PreparedFpvRig>>,
    facts: Vec<Option<WeaponChassisSummary>>,
    hud_iris: Vec<bool>,
    melee: Vec<u32>,
    guns: Vec<Option<FpvMeshSlot>>,
    slot_list: Vec<[FpvWeaponSlot; 2]>,
    alternate_slots: HashMap<(u32, u32), [FpvWeaponSlot; 2]>,
    refused: Vec<String>,
    /// When preparation began, for the elapsed time in the census.
    began_at: Instant,
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

    pub fn settled_for(&self, mats: &Arc<LiveMaterialLedger>) -> bool {
        self.job.is_none()
            && self
                .table()
                .is_some_and(|table| Arc::ptr_eq(&table.material_catalog, mats))
    }

    pub fn clear(&mut self) {
        self.cancel_job();
        self.table = None;
    }

    fn cancel_job(&mut self) {
        if let Some(stage) = self.job.take().and_then(|job| job.progress) {
            stage.cancel();
        }
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
    /// Name the surface's material is presented under.
    shown_as: &'a str,
    /// Index into the session's material catalog.
    catalog_at: usize,
    /// The material's index within the model; surfaces of one model that share
    /// it share one draw material.
    within_model: usize,
}

/// Follows a surface to its material, or says why it has none.
fn bind_surface(entry: &FirstPersonMeshRecord, bay: usize) -> Result<SurfaceBinding<'_>, FpvSurfaceVerdict> {
    let edge = match entry.surface_links.get(bay) {
        Some(&found) => found,
        None => AssetDependencyLink::NothingThere,
    };
    let leftover = match entry.material_keys.get(bay) {
        Some(Some(key)) => key.name.as_str(),
        _ => "-",
    };
    let refuse = |material_in: &str, cause: &'static str| FpvSurfaceVerdict::Declined {
        mat_name: material_in.to_owned(),
        cause,
    };
    if matches!(edge, AssetDependencyLink::NothingThere) {
        return Err(FpvSurfaceVerdict::Inapplicable("no material"));
    }
    if let AssetDependencyLink::Unresolved(_) = edge {
        return Err(refuse(leftover, edge.dependency_cause_of()));
    }
    let Some(catalog_at) = edge.extent_position() else {
        return Err(refuse(leftover, "bound edge without an index"));
    };
    let Some(shown_as) = entry.material_name_of(bay) else {
        return Err(refuse(leftover, "bound material has no name"));
    };
    let within_model = match entry.bone_rig.facet_materials.get(bay).copied().flatten() {
        Some(index) => index.get(),
        None => return Err(refuse(shown_as, "surface carries no authored material")),
    };
    Ok(SurfaceBinding {
        within_model,
        shown_as,
        catalog_at,
    })
}

/// The session's uploaded images, remembering the ones a model asked for.
struct ImageLookup<'a> {
    uploaded: &'a [Option<Handle<Image>>],
    asked: &'a mut HashMap<u32, Handle<Image>>,
}

impl ImageLookup<'_> {
    fn get(&mut self, index: u32) -> Option<Handle<Image>> {
        use std::collections::hash_map::Entry;
        match self.asked.entry(index) {
            Entry::Occupied(hit) => Some(hit.get().clone()),
            Entry::Vacant(miss) => {
                let fresh = self.uploaded.get(index as usize)?.clone()?;
                Some(miss.insert(fresh).clone())
            }
        }
    }
}

/// The image a material binds to a texture slot of this meaning.
fn image_of(material: &LiveMaterial, semantic: u8) -> Option<u32> {
    let bound = material
        .textures
        .iter()
        .find_map(|(_k, slot)| slot.as_ref().filter(|bound| bound.semantic == semantic))?;
    Some(bound.image.0)
}

/// Hash of the constant that carries a material's environment-map parameters.
const ENV_MAP_PARMS_HASH: u32 = u32::from_be_bytes([0x3d, 0x99, 0x94, 0xdc]);

fn env_map_parms_of(constants: &[(u32, [u32; 4])]) -> [f32; 4] {
    for &(hash, words) in constants {
        if hash == ENV_MAP_PARMS_HASH {
            return words.map(f32::from_bits);
        }
    }
    [0.0; 4]
}

/// The face the material's first state culls.
fn cull_face_of(state_bits_table: &[[u32; 2]]) -> Option<Face> {
    let &bits = state_bits_table.first()?;
    let culled = asset_material::face_culling_of(bits);
    match culled {
        asset_material::FaceCullingMode::None => None,
        asset_material::FaceCullingMode::Front => Some(Face::Front),
        asset_material::FaceCullingMode::Back => Some(Face::Back),
    }
}

/// What admitting surfaces reads and fills in: the session's catalog, the
/// lighting atlas, the rows admitted so far and the images asked for.
struct Admitter<'a> {
    catalog: &'a LiveMaterialLedger,
    atlas: &'a LevelModelIllumAtlas,
    rows: &'a mut FpvMaterialAdmission,
    images: ImageLookup<'a>,
}

/// What the model-lighting shader looks the atlas up with: the lookup scale,
/// then the atlas dimensions.
fn atlas_terms(atlas: &LevelModelIllumAtlas) -> Option<([f32; 4], [f32; 4])> {
    let inv_height = lighting_t6::light_atlas_reciprocal_height(atlas.dims.rows_tall)?;
    let scale = lighting_t6::atlas_lookup_scale(inv_height);
    let dims = [
        lighting_t6::ATLAS_INV_WIDTH as f32,
        inv_height,
        lighting_t6::MODEL_LIGHT_VOLUME_DEPTH_SPAN,
        0.0,
    ];
    Some(([scale.u, scale.v, scale.w, scale.q], dims))
}

impl Admitter<'_> {
    /// Gives one surface its verdict, admitting its material the first time it
    /// is seen in its model.
    fn verdict_for(&mut self, entry: &FirstPersonMeshRecord, bay: usize) -> FpvSurfaceVerdict {
        match self.admit(entry, bay) {
            Ok(row) => FpvSurfaceVerdict::WelcomedIn(row),
            Err(verdict) => verdict,
        }
    }

    fn admit(&mut self, entry: &FirstPersonMeshRecord, bay: usize) -> Result<u32, FpvSurfaceVerdict> {
        let binding = bind_surface(entry, bay)?;
        if let Some(&row) = self.rows.by_authored.get(&binding.within_model) {
            return Ok(row);
        }
        let material = self.pass_material(&binding)?;
        let row = self.rows.mat_list.len() as u32;
        self.rows.mat_list.push(material);
        self.rows.by_authored.insert(binding.within_model, row);
        Ok(row)
    }

    fn pass_material(&mut self, binding: &SurfaceBinding<'_>) -> Result<PropPassMaterial, FpvSurfaceVerdict> {
        let refuse = |cause: &'static str| FpvSurfaceVerdict::Declined {
            mat_name: binding.shown_as.to_owned(),
            cause,
        };
        let cat = self.catalog;
        let Some(src_mat) = cat.materials.get(binding.catalog_at) else {
            return Err(refuse("material outside the session catalog"));
        };
        let ordinal =
            cat.rank_for_asset_id(assets::SurfaceMaterialId::from_sequence(binding.catalog_at));
        // bo2zm: Black Ops II materials have no technique set and so no sorted
        // ordinal; the fallback pass draws them by catalog index.
        if ordinal.is_none() && src_mat.t6_draw.is_none() {
            return Err(refuse("material has no sorted ordinal"));
        }
        let color_index = image_of(src_mat, SEMANTIC_COLOR)
            .ok_or(FpvSurfaceVerdict::Inapplicable(NO_COLOUR_MAP))?;
        let color = self
            .images
            .get(color_index)
            .ok_or_else(|| refuse("colour map not uploaded"))?;
        let specular = image_of(src_mat, asset_material::SEMANTIC_SPECULAR)
            .and_then(|index| self.images.get(index));
        let (illum_lookup_scale, atlas_find) =
            atlas_terms(self.atlas).ok_or_else(|| refuse("model lighting atlas has no rows"))?;
        Ok(PropPassMaterial {
            order_key: src_mat.draw_sort_key,
            color: Some(color),
            specular,
            face_cull: cull_face_of(&src_mat.state_rows),
            env_map_params: env_map_parms_of(&src_mat.constants),
            see_through: AlphaMode::Opaque,
            paint_mode: None,
            probe: None,
            atlas: Some(Handle::clone(&self.atlas.image)),
            model_illum_required: true,
            illum_lookup_scale,
            atlas_find,
            material_draw_rank_slot: ordinal.map(|o| o.get()),
            // bo2zm: see `PropPassMaterial::t6_material`.
            t6_material: src_mat.t6_draw.and(Some(binding.catalog_at as u32)),
        })
    }
}

// ---------------------------------------------------------------------------
// Which meshes and layouts a registry needs
// ---------------------------------------------------------------------------

/// The meshes (in first-seen order) and the (mesh, hidden parts) layouts the
/// registry's assemblies call for.
/// The models and layouts to prepare, and the layouts that are hands (for the
/// script's hands to replace).
fn work_lists(
    registry: &WeaponLedger,
    fpv: &FirstPersonMeshRoster,
) -> (Vec<usize>, Vec<(usize, Option<[u32; 5]>)>, Vec<(usize, Option<[u32; 5]>)>) {
    let weapon_n = registry.len() as u32;
    let mut models = Vec::new();
    let mut layouts = Vec::new();
    let mut hand_parts = Vec::new();
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
                for part in &assembly.pieces {
                    let order = part.mdl.order();
                    if fpv.at_row(order).is_none() {
                        continue;
                    }
                    if seen_models.insert(order) {
                        models.push(order);
                    }
                    if seen_layouts.insert((order, part.hide)) {
                        layouts.push((order, part.hide));
                        if part.role == asset_game::FpvPartRole::Hands {
                            hand_parts.push((order, part.hide));
                        }
                    }
                }
            }
        }
    }
    (models, layouts, hand_parts)
}

/// The gun mesh of a weapon, when the catalog has it.
fn gun_mesh_of(
    registry: &WeaponLedger,
    fpv: &FirstPersonMeshRoster,
    weapon_idx: u32,
) -> Option<FpvMeshSlot> {
    let order = registry.gun_mesh_link_for(weapon_idx)?.extent_position()?;
    fpv.at_row(order)?;
    Some(FpvMeshSlot::from_sequence(order))
}

fn unresolved(gun_catalog_no: u32, purpose: &'static str, name: String) -> Cause {
    Cause::FpvDependencyUnresolved {
        gun_catalog_no,
        part_role: purpose,
        name,
    }
}

/// Whether a weapon gets a left-hand set of clips: dual wield is allowed and
/// the right-hand set has an idle clip to pair it with.
fn has_paired_hands(forbid_twin_grip: Option<bool>, right_idle_bound: bool) -> bool {
    forbid_twin_grip == Some(false) && right_idle_bound
}

// ---------------------------------------------------------------------------
// What a weapon's two sides share
// ---------------------------------------------------------------------------

/// The part of a weapon's preparation that does not depend on the side.
struct WeaponShared {
    id: u32,
    owner: u32,
    gun_index: FpvMeshSlot,
    gun_name: String,
    namespace: AssetCategoryScope,
    right: GunClipSet,
    left: Option<GunClipSet>,
    idle_name: Option<String>,
    census: FpvViewCensus,
    right_orders: Vec<Option<usize>>,
    left_orders: Vec<Option<usize>>,
}

/// What a preparation step reads and never changes.
#[derive(Clone, Copy)]
struct PrepareSources<'a> {
    meshes: &'a FirstPersonMeshRoster,
    clips: &'a MotionClipRoster,
    tess: &'a StitchMaterials,
    atlas: &'a LevelModelIllumAtlas,
}

/// A weapon's animations: the right hand's, the left hand's when the weapon
/// pairs, and the name of the idle clip.
struct HandAnims {
    right: GunClipSet,
    left: Option<GunClipSet>,
    idle_name: Option<String>,
}

impl PrepareSources<'_> {
    fn weapon_anims(&self, registry: &Arc<WeaponLedger>, id: u32) -> HandAnims {
        let clips = self.clips;
        let idle = GunMoveRole::Idle.index();
        let right_edges = registry.sz_xanim_right_edges_of(id);
        let left_edges = registry.sz_xanim_left_edges_of(id);
        let [right_idle_bound, left_idle_bound] =
            [right_edges, left_edges].map(|edges| edges.is_some_and(|row| row[idle].is_attached()));
        let forbid_twin_grip = registry.summary_for(id).map(|facts| facts.forbid_twin_grip);
        let paired = has_paired_hands(forbid_twin_grip, right_idle_bound);
        let right = match paired {
            true => GunClipSet::from_registry_edges(registry, id, right_edges, clips),
            false => GunClipSet::from_registry(registry, id, clips),
        };
        let left = if paired && left_idle_bound {
            Some(GunClipSet::from_registry_edges(
                registry, id, left_edges, clips,
            ))
        } else {
            None
        };
        let idle_clip = registry
            .sz_xanim_edges_of(id)
            .and_then(|row| row[idle].extent_position());
        let idle_name = idle_clip
            .and_then(|order| clips.node_clip(order))
            .map(|clip| clip.name.clone());
        HandAnims { right, left, idle_name }
    }
}

impl FpvPreparationJob {
    fn new(
        owner: FpvPreparationOwner,
        fpv: &FirstPersonMeshRoster,
        progress: Option<asset_transport::StageHandle>,
    ) -> Self {
        let registry = Arc::clone(&owner.weapons);
        let weapon_n = registry.len() as u32;
        let (mut models, mut layouts_queue, hand_parts) = work_lists(&registry, fpv);
        let mut hands_swap: HashMap<usize, usize> = HashMap::new();
        if let Some(over) = owner.hands
            && let Some(over_entry) = fpv.at_row(over)
        {
            for &(order, hide) in &hand_parts {
                let same_bones = fpv
                    .at_row(order)
                    .is_some_and(|own| own.bone_rig.joint_labels == over_entry.bone_rig.joint_labels);
                if order == over || !same_bones {
                    continue;
                }
                hands_swap.insert(order, over);
                if !models.contains(&over) {
                    models.push(over);
                }
                if !layouts_queue.contains(&(over, hide)) {
                    layouts_queue.push((over, hide));
                }
            }
            diag::info!(
                ViewModelPerspective,
                "fpv: script hands {} replace {} hands models",
                over_entry.bone_rig.name,
                hands_swap.len()
            );
        }
        let mut alternates: Vec<_> = registry.alternate_fpv_pairs().collect();
        let work_total =
            (models.len() + layouts_queue.len() + alternates.len()) as u64 + u64::from(weapon_n);
        if let Some(stage) = &progress {
            stage.set_total(work_total);
        }
        // Popped from the back, so the lowest pair goes first.
        alternates.sort_unstable_by(|lhs, rhs| rhs.cmp(lhs));
        let room = weapon_n as usize + 1;
        Self {
            owner,
            stage: FpvPreparationStage::Admit,
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
            hands_swap,
            next_weapon: 0,
            alternate_queue: alternates,
            compositions: HashMap::new(),
            tracks: HashMap::new(),
            rigs: HashMap::new(),
            facts: Vec::with_capacity(room),
            hud_iris: Vec::with_capacity(room),
            melee: Vec::with_capacity(room),
            guns: Vec::with_capacity(room),
            slot_list: Vec::with_capacity(room),
            alternate_slots: HashMap::new(),
            refused: Vec::new(),
            began_at: Instant::now(),
        }
    }

    fn finish_unit(&mut self) {
        self.work_done = self.work_done.saturating_add(1);
        if let Some(stage) = &self.progress {
            stage.set_completed(self.work_done.min(self.work_total));
        }
    }

    fn step_on(&mut self, deadline: Instant, src: PrepareSources<'_>) {
        while Instant::now() < deadline {
            match self.stage {
                FpvPreparationStage::Admit => self.admit_next(src),
                FpvPreparationStage::Layout => self.lay_out_next(src.meshes),
                FpvPreparationStage::Weapons => self.prepare_next_weapon(src),
                FpvPreparationStage::Done => break,
            }
        }
    }

    fn admit_next(&mut self, src: PrepareSources<'_>) {
        let Some(&order) = self.models.get(self.next_model) else {
            self.stage = FpvPreparationStage::Layout;
            return;
        };
        self.next_model += 1;
        if let Some(entry) = src.meshes.at_row(order) {
            let mut admitter = Admitter {
                catalog: &src.tess.catalog,
                atlas: src.atlas,
                rows: &mut self.admission,
                images: ImageLookup {
                    uploaded: src.tess.shader_textures.as_ref(),
                    asked: &mut self.image_cache,
                },
            };
            let verdicts = entry
                .bone_rig
                .facets_at_tier(0)
                .map(|surface| (surface, admitter.verdict_for(entry, surface)))
                .collect::<Vec<_>>();
            self.admission.store(order, verdicts);
        }
        self.finish_unit();
    }

    fn lay_out_next(&mut self, fpv: &FirstPersonMeshRoster) {
        let Some(&(order, hide)) = self.layouts_queue.get(self.next_layout) else {
            self.stage = FpvPreparationStage::Weapons;
            self.next_weapon = 0;
            return;
        };
        self.next_layout += 1;
        let built = PreparedFpvModel::build(fpv, order, hide.as_ref(), &self.admission);
        let model = match built {
            Ok(done) => Ok(Arc::new(done)),
            Err(error) => Err(error.to_string()),
        };
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
        let swap = &self.hands_swap;
        let composed = PreparedFpvComposition::assemble(Arc::clone(assembly), |order, hide| {
            // bo2zm: the script's hands, unless they failed to lay out or
            // their material is refused (then the weapon's own).
            if let Some(over) = swap.get(&order)
                && let Some(Ok(model)) = layouts.get(&(*over, hide))
                && model.refusal().is_none()
            {
                return Ok(Arc::clone(model));
            }
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
        weapon_in: u32,
        assemblies: &[Option<&Arc<FpvAssembly>>],
        layout_role: &'static str,
        material_role: &'static str,
    ) -> Result<Vec<Option<Arc<PreparedFpvComposition>>>, Cause> {
        let attempts: Vec<Result<Option<Arc<PreparedFpvComposition>>, String>> = assemblies
            .iter()
            .map(|assembly| match assembly {
                Some(each) => self.composition(each).map(Some),
                None => Ok(None),
            })
            .collect::<_>();
        let broken = attempts.iter().find_map(|attempt| attempt.as_ref().err());
        if let Some(why) = broken {
            return Err(unresolved(weapon_in, layout_role, why.clone()));
        }
        let composed: Vec<_> = attempts
            .into_iter()
            .map(Result::unwrap_or_default)
            .collect::<_>();
        let refusal = composed
            .iter()
            .flatten()
            .find_map(|composition| composition.refusal());
        if let Some(refusal) = refusal {
            return Err(unresolved(weapon_in, material_role, refusal.to_string()));
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
        let mut per_clip = Vec::with_capacity(orders.len());
        for &order in orders {
            let Some(order) = order else {
                per_clip.push(None);
                continue;
            };
            let compose = || compose_clip_tracks(assembly, order, registry.fpv_clip_tracks());
            per_clip.push(
                self.tracks
                    .entry((key, order))
                    .or_insert_with(compose)
                    .clone(),
            );
        }
        per_clip
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
        let key: (usize, bool, Vec<Option<usize>>, Vec<Option<usize>>) = (
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

    fn census_of(&self, fpv: &FirstPersonMeshRoster, gun: FpvMeshSlot) -> FpvViewCensus {
        let order = gun.order();
        let Some(entry) = fpv.at_row(order) else {
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
    fn next_target(&mut self, registry: &WeaponLedger) -> Option<(u32, u32)> {
        if self.next_weapon as usize > registry.len() {
            return self.alternate_queue.pop();
        }
        let id = self.next_weapon;
        self.next_weapon += 1;
        self.facts.push(registry.summary_for(id));
        self.hud_iris.push(registry.scope_is_iris(id));
        self.melee.push(registry.melee_weapon_of(id));
        Some((id, 0))
    }

    fn store_sides(&mut self, id: u32, owner: u32, sides: [FpvWeaponSlot; 2]) {
        if owner != 0 {
            self.alternate_slots.insert((id, owner), sides);
            return;
        }
        self.slot_list.push(sides);
    }

    fn prepare_next_weapon(&mut self, src: PrepareSources<'_>) {
        let fpv = src.meshes;
        let registry = Arc::clone(&self.owner.weapons);
        let Some((id, parent)) = self.next_target(&registry) else {
            self.stage = FpvPreparationStage::Done;
            return;
        };
        let gun = gun_mesh_of(&registry, fpv, id);
        let is_alternate = parent != 0;
        if !is_alternate {
            self.guns.push(gun);
        }
        if id == 0 {
            self.slot_list
                .push([FpvWeaponSlot::NothingThere, FpvWeaponSlot::NothingThere]);
            return;
        }
        let Some(gun_index) = gun else {
            let cause = Cause::FpvGunXModelUnresolved { gun_catalog_no: id };
            self.store_sides(
                id,
                parent,
                [
                    FpvWeaponSlot::Declined(cause.clone()),
                    FpvWeaponSlot::Declined(cause),
                ],
            );
            self.finish_unit();
            return;
        };

        let HandAnims { right, left, idle_name } = src.weapon_anims(&registry, id);
        let right_orders = right.clip_orders().to_vec();
        debug_assert_eq!(right_orders.len(), WEAPON_ANIM_SLOTS);
        let left_orders = left
            .as_ref()
            .map(|left| left.clip_orders().to_vec()).unwrap_or_default();
        let shared = WeaponShared {
            id,
            owner: parent,
            gun_index,
            gun_name: fpv
                .at_row(gun_index.order())
                .map_or_else(String::new, |entry| entry.bone_rig.name.clone()),
            namespace: registry.scope_for(id).unwrap_or(AssetCategoryScope::T6),
            census: self.census_of(fpv, gun_index),
            right,
            left,
            idle_name,
            right_orders,
            left_orders,
        };

        let sides = [false, true].map(|axis| self.prepare_side(&registry, &shared, axis));
        for (axis, side) in sides.iter().enumerate() {
            if let FpvWeaponSlot::Declined(cause) = side
                && self.refused.len() < REFUSALS_LOGGED
            {
                self.refused.push(format!(
                    "{} ({}): {cause}",
                    registry.label_for(id),
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
        registry: &WeaponLedger,
        shared: &WeaponShared,
        along: bool,
    ) -> FpvWeaponSlot {
        let id = shared.id;
        let script_hands = self.hands_swap.values().next().copied();
        let (Some((hands, hands_index)), Some(assemblies)) = (
            registry.fpv_hands_of(id, along),
            registry.fpv_assemblies_for(id, shared.owner, along),
        ) else {
            return FpvWeaponSlot::Declined(unresolved(
                id,
                "FPV skeleton",
                registry.fpv_assembly_gap_of(id, along),
            ));
        };

        let bare_and_rocket = match self.compose_group(
            id,
            &[Some(&assemblies.bare), assemblies.rocket.as_ref()],
            "FPV layout",
            "FPV material",
        ) {
            Ok(group) => group,
            Err(cause) => return FpvWeaponSlot::Declined(cause),
        };
        let melee = match self.compose_group(
            id,
            &[assemblies.melee.as_ref()],
            "melee FPV layout",
            "melee FPV material",
        ) {
            Ok(mut group) => group.remove(0),
            Err(cause) => return FpvWeaponSlot::Declined(cause),
        };
        let ads = match self.compose_group(
            id,
            &[assemblies.ads.as_ref()],
            "aimed FPV layout",
            "aimed FPV material",
        ) {
            Ok(mut group) => group.remove(0),
            Err(cause) => return FpvWeaponSlot::Declined(cause),
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
                rigs.rigs.of_mut(variant)[spec.arm] = Some(rig);
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
            script_hands,
        }))
    }

    fn finish(self) -> (FpvPreparationOwner, FpvWeaponTable) {
        let all = || {
            self.slot_list
                .iter()
                .chain(self.alternate_slots.values())
                .flatten()
        };
        let ready = all()
            .filter(|slot| matches!(slot, FpvWeaponSlot::Ready(_)))
            .count();
        let refused_n = all()
            .filter(|slot| matches!(slot, FpvWeaponSlot::Declined(_)))
            .count();
        let inapplicable = self
            .models
            .iter()
            .flat_map(|&order| self.admission.verdicts_of(order))
            .filter(|(_k, verdict)| matches!(verdict, FpvSurfaceVerdict::Inapplicable(_)))
            .count();
        let elapsed_millis = self.began_at.elapsed().as_secs_f64() * 1000.0;
        let census = FpvPreparationCensus {
            models: self.models.len(),
            layouts: self.layouts.len(),
            compositions: self.compositions.len(),
            rigs: self.rigs.len(),
            materials: self.admission.mat_list.len(),
            elapsed_ms: elapsed_millis,
        };
        diag::info!(
            ViewModelPerspective,
            "fpv prepared before Ready: models={} layouts={} compositions={} rigs={} materials={} inapplicable_surfaces={inapplicable} sides ready={ready} refused={refused_n} elapsed={:.1}ms",
            census.models,
            census.layouts,
            census.compositions,
            census.rigs,
            census.materials,
            census.elapsed_ms,
        );
        for line in &self.refused {
            diag::info!(ViewModelPerspective, "fpv refused before use: {line}");
        }
        if let Some(stage) = self.progress {
            stage.done();
        }
        let table = FpvWeaponTable {
            material_catalog: Arc::clone(&self.owner.mat_list),
            catalog_id: self.owner.catalog_id,
            facts: self.facts,
            hud_iris: self.hud_iris,
            melee: self.melee,
            guns: self.guns,
            slots: self.slot_list,
            alternate_slots: self.alternate_slots,
            tally: census,
        };
        (self.owner, table)
    }
}

/// A table in which every weapon is refused for one shared reason.
fn refused_table(
    owner: &FpvPreparationOwner,
    fpv: &FirstPersonMeshRoster,
    cause: Cause,
) -> FpvWeaponTable {
    let registry = &owner.weapons;
    let ids = 0..=registry.len() as u32;
    let slot_of = |id: u32| -> [FpvWeaponSlot; 2] {
        if id == 0 {
            [FpvWeaponSlot::NothingThere, FpvWeaponSlot::NothingThere]
        } else {
            [
                FpvWeaponSlot::Declined(cause.clone()),
                FpvWeaponSlot::Declined(cause.clone()),
            ]
        }
    };
    FpvWeaponTable {
        material_catalog: Arc::clone(&owner.mat_list),
        catalog_id: owner.catalog_id,
        facts: ids.clone().map(|id| registry.summary_for(id)).collect(),
        hud_iris: ids
            .clone()
            .map(|id| registry.scope_is_iris(id))
            .collect::<_>(),
        melee: ids.clone().map(|id| registry.melee_weapon_of(id)).collect(),
        guns: ids
            .clone()
            .map(|id| gun_mesh_of(registry, fpv, id))
            .collect::<_>(),
        slots: ids.map(slot_of).collect(),
        alternate_slots: HashMap::new(),
        tally: FpvPreparationCensus::default(),
    }
}

/// Why a world cannot have a first-person weapon table at all, if it cannot.
fn shared_refusal(
    sorted_ready: bool,
    has_lighting_atlas: bool,
) -> Option<(Cause, &'static str)> {
    if !sorted_ready {
        Some((
            Cause::GunTableAbsent,
            "the material generation has no sorted table",
        ))
    } else if !has_lighting_atlas {
        Some((
            Cause::GunNoLightAtlas,
            "no model lighting atlas in this world",
        ))
    } else {
        None
    }
}

#[derive(bevy::ecs::system::SystemParam)]
pub struct PrepareFpvInputs<'w> {
    weapon_table: Option<Res<'w, StagedWeapons>>,
    tessellation: Res<'w, StitchMaterials>,
    clip_table: Option<Res<'w, StagedXAnims>>,
    mesh_table: Option<Res<'w, ReadiedFpvMeshes>>,
    load: Option<Res<'w, assets::MapLoadProcess>>,
    atlas_res: Option<Res<'w, LevelModelIllumAtlas>>,
    hands: Option<Res<'w, FpvHandsOverride>>,
}

pub fn prepare_fpv_compositions(inputs: PrepareFpvInputs, mut prepared: ResMut<PreparedFpv>) {
    let PrepareFpvInputs {
        weapon_table,
        tessellation: tess,
        clip_table,
        mesh_table,
        load,
        atlas_res,
        hands,
    } = inputs;
    let (Some(weapon_table), Some(mesh_table), Some(clip_table)) =
        (weapon_table, mesh_table, clip_table)
    else {
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
    if tess.shader_textures.is_empty() {
        return;
    }
    let sorted_table = &tess.catalog.ranked_materials;
    let sorted_ready = matches!(sorted_table, LiveSortedMaterialTable::Ready { .. });
    let has_atlas = atlas_res.is_some();
    let weapons = Arc::clone(&weapon_table.0);
    let catalog_id = mesh_table.0.identity();
    let atlas_id = atlas_res.as_ref().map(|atlas| atlas.image.id());
    let owner = FpvPreparationOwner {
        mat_list: Arc::clone(&tess.catalog),
        images: Arc::clone(&tess.shader_textures),
        weapons,
        catalog_id,
        atlas: atlas_id,
        hands: hands
            .as_deref()
            .and_then(|script| script.0.as_deref())
            .and_then(|name| mesh_table.0.row_of(AssetCategoryScope::T6, name)),
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
        // bo2zm: only the script's hands changed: the old table draws until the new one is
        // ready.
        let hands_only = prepared
            .table
            .as_ref()
            .is_some_and(|(settled, _)| settled.same_but_hands(&owner));
        match hands_only {
            true => prepared.cancel_job(),
            false => prepared.clear(),
        }
        if let Some((cause, why)) = shared_refusal(sorted_ready, has_atlas) {
            diag::info!(
                ViewModelPerspective,
                "fpv: {why} — every first-person weapon refused before Ready"
            );
            let table = refused_table(&owner, &mesh_table.0, cause);
            prepared.table = Some((owner, Arc::new(table)));
            return;
        }
        let progress = load
            .as_deref()
            .filter(|process| !process.is_done())
            .map(|process| {
                process
                    .progress
                    .begin(asset_transport::StageId::FirstPerson, None)
            });
        let job = FpvPreparationJob::new(owner, &mesh_table.0, progress);
        prepared.job = Some(job);
    }
    let (Some(atlas), Some(job)) = (atlas_res.as_deref(), prepared.job.as_mut()) else {
        return;
    };
    let deadline = Instant::now() + PREPARE_FRAME_BUDGET;
    let sources = PrepareSources {
        meshes: &mesh_table.0,
        clips: &clip_table.0,
        tess: &tess,
        atlas,
    };
    job.step_on(deadline, sources);
    if job.stage != FpvPreparationStage::Done {
        return;
    }
    if let Some(job) = prepared.job.take() {
        let (owner, table) = job.finish();
        prepared.table = Some((owner, Arc::new(table)));
    }
}

mod tests {
    #![cfg(test)]
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
                arm: 0,
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
                    arm: 0,
                    dual: false,
                    with_left_clips: false
                },
                RigSpec {
                    arm: 1,
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
                arm: 0,
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
        let admitted = FpvSurfaceVerdict::WelcomedIn(0);
        let skipped = FpvSurfaceVerdict::Inapplicable(NO_COLOUR_MAP);
        let other = FpvSurfaceVerdict::Inapplicable("no material");
        let refused = FpvSurfaceVerdict::Declined {
            mat_name: "m".into(),
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
            census.gun_tint_skipped, 3,
            "the count includes unnamed surfaces"
        );
        assert_eq!(census.gun_tint_skip_list, ["glass"]);
    }

    #[test]
    fn the_census_keeps_a_few_names_at_most() {
        let admitted = FpvSurfaceVerdict::WelcomedIn(0);
        let names: Vec<String> = (0..30).map(|n| format!("mat{n}")).collect();
        let census = census_of_verdicts(names.iter().map(|name| (&admitted, Some(name.as_str()))));
        assert_eq!(census.mat_hints.len(), HINTS_KEPT);
        assert_eq!(census.mat_hints[0], "mat0");

        let skipped = FpvSurfaceVerdict::Inapplicable(NO_COLOUR_MAP);
        let census = census_of_verdicts(names.iter().map(|name| (&skipped, Some(name.as_str()))));
        assert_eq!(census.gun_tint_skip_list.len(), SKIPS_KEPT);
        assert_eq!(census.gun_tint_skipped, 30);
    }

    #[test]
    fn environment_parameters_come_from_their_constant() {
        let one = 1.0f32.to_bits();
        let carrier = (ENV_MAP_PARMS_HASH, [one, 0, one, 0]);
        let constants = [(7, [9; 4]), carrier];
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
            Some((Cause::GunTableAbsent, _))
        ));
        assert!(matches!(
            shared_refusal(true, false),
            Some((Cause::GunNoLightAtlas, _))
        ));
        assert!(shared_refusal(true, true).is_none());
    }

    #[test]
    fn an_unresolved_dependency_names_weapon_role_and_what_is_missing() {
        match unresolved(5, "FPV layout", "gone".into()) {
            Cause::FpvDependencyUnresolved {
                gun_catalog_no,
                part_role: role,
                name,
            } => assert_eq!((gun_catalog_no, role, name.as_str()), (5, "FPV layout", "gone")),
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
