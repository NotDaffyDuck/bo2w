//! Weapons: what the zones say about each gun, and the table every other crate
//! reads them from.
//!
//! The zone walk fills a [`WeaponCatalog`] with one [`CatalogWeapon`] per gun.
//! `into_build` turns that into a [`WeaponBuild`], the later passes link its
//! names to models, animations, materials and effects, and `publish` hands out
//! the finished [`WeaponRegistry`]. Weapon id 0 is "no weapon".

mod build;
mod catalog;
mod combat_fx;
mod facts;
mod registry;
mod sounds;
mod t6;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use asset_core::{
    AssetEdge, AssetEdgeReason, FpvMeshSpace, FxSpace, MaterialSpace,
    WorldWeaponSpace, XAnimSpace,
};
use weapon_t6::asset_slots::WEAPON_ANIM_SLOTS;
use weapon_t6::asset_slots::weap_anim_slot as weap_anim;

pub use combat_fx::WeaponCombatFx;
pub use facts::*;
pub use registry::overlay_name_is_hud_iris;
pub use sounds::{WeaponSoundAliases, WeaponSoundSlot};
pub use t6::{T6_SIGHT_ATTACHMENTS, t6_attachment_combo_name};

/// A fresh revision number: no two registries, or two states of one, share one.
fn mint_weapon_revision() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// The key weapons are looked up by: lower case, without BO2's `_mp` suffix.
fn normalize_weapon_name(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    lower.strip_suffix("_mp").unwrap_or(&lower).to_owned()
}

/// The name a script calls a weapon by, from its catalog name.
pub fn gsc_weapon_script_name(catalog_bare: &str) -> String {
    if catalog_bare.is_empty() {
        return String::new();
    }
    // A gun with a sight keeps its suffix in the middle (`mp7_mp+reflex`).
    if catalog_bare.ends_with("_mp") || catalog_bare.contains("_mp+") {
        catalog_bare.to_owned()
    } else {
        format!("{catalog_bare}_mp")
    }
}

/// A weapon as the zone walk captured it, before anything is linked.
#[derive(Clone, Debug)]
pub struct CatalogWeapon {
    pub namespace: crate::AssetNamespace,
    pub name: String,
    pub alternate_weapon: Option<String>,
    pub display_name_key: Option<String>,
    pub reticle: WeaponReticleAssets,
    pub overlay_material: Option<String>,
    pub hud_icon: Option<String>,
    pub kill_icon: Option<String>,
    pub proj_trail: Option<String>,
    pub proj_ignition: Option<String>,
    pub gun_xmodel: Option<String>,
    pub world_model: Option<String>,
    pub projectile_model: Option<String>,
    pub rocket_model: Option<String>,
    pub sz_xanims: [Option<String>; WEAPON_ANIM_SLOTS],
    pub sz_xanims_right: [Option<String>; WEAPON_ANIM_SLOTS],
    pub sz_xanims_left: [Option<String>; WEAPON_ANIM_SLOTS],
    pub hide_tags: Vec<String>,
    /// View-model attachments (magazine, scope) and where each sits on the gun.
    pub view_attachments: Vec<(String, asset_model::FpvMountOffset)>,
    /// The same list while aimed, when a sight swaps its model for a stripped
    /// one; empty: no swap.
    pub ads_view_attachments: Vec<(String, asset_model::FpvMountOffset)>,
    /// A dual-wield weapon's left-hand weapon.
    pub dual_wield_weapon: Option<String>,
    /// The grenade a bolt leaves where it sticks.
    pub spawned_grenade_weapon: Option<String>,
    /// Attachment world models (third person, on the floor) and the gun tag
    /// each goes on.
    pub world_attachments: Vec<(String, String)>,
    pub sounds: WeaponSoundAliases,
    pub combat_fx: WeaponCombatFx,
    pub facts: WeaponBodyFacts,
}

/// A scope's reticle pictures.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WeaponReticleAssets {
    pub center_material: Option<String>,
    pub side_material: Option<String>,
    pub center_edge: AssetEdge<MaterialSpace>,
    pub side_edge: AssetEdge<MaterialSpace>,
    pub center_image: Option<String>,
    pub side_image: Option<String>,
    pub center_size: i32,
    pub side_size: i32,
    /// Whether the zone authored the material at all.
    pub center_authored: bool,
    pub side_authored: bool,
    /// The picture the grenade warning shows for this weapon; none: the frag's.
    pub indicator_icon: Option<String>,
}

/// What a notetrack on one of a weapon's animations does when it fires.
#[derive(Clone, Debug, Default)]
pub struct LinkedNotetrackAction {
    pub sound_alias: Option<String>,
    pub rumble_alias: Option<String>,
}

/// Something a weapon needs that the loaded zones do not have.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WeaponDependencyGap {
    pub id: u32,
    pub kind: &'static str,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownWeaponName {
    pub name: String,
}

impl core::fmt::Display for UnknownWeaponName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "unknown weapon name `{}`", self.name)
    }
}

impl std::error::Error for UnknownWeaponName {}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FpvAssemblyCensus {
    pub built: usize,
    pub linked: usize,
    pub refused: usize,
    pub clip_tables: usize,
}

/// The weapons one zone walk captured.
#[derive(Clone, Debug, Default)]
pub struct WeaponCatalog {
    entries: Vec<CatalogWeapon>,
    capture_ns: crate::AssetNamespace,
}

/// One weapon in the registry: the capture plus every link made since.
#[derive(Clone, Debug)]
struct WeaponRow {
    name: String,
    alternate_weapon: Option<String>,
    alternate_index: u32,
    namespace: crate::AssetNamespace,
    facts: WeaponBodyFacts,

    gun_xmodel: Option<String>,
    gun_xmodel_edge: AssetEdge<FpvMeshSpace>,
    rocket_model_edge: AssetEdge<FpvMeshSpace>,
    attachment_view_model_edges: Vec<AssetEdge<FpvMeshSpace>>,
    fpv_hands: [Option<(asset_model::FpvHands, crate::FpvMeshIndex)>; 2],
    fpv_mount_plan: Option<Result<asset_model::FpvMountPlan, asset_model::FpvMountError>>,
    /// The mount plan while aimed (a sight's stripped model).
    ads_view_models: Vec<String>,
    ads_view_offsets: Vec<Option<asset_model::FpvMountOffset>>,
    fpv_mount_plan_ads: Option<Result<asset_model::FpvMountPlan, asset_model::FpvMountError>>,
    fpv_assemblies: [Option<Result<crate::FpvSideAssemblies, String>>; 2],

    world_model: Option<String>,
    world_model_edge: AssetEdge<WorldWeaponSpace>,
    attachment_world_model_edges: Vec<AssetEdge<WorldWeaponSpace>>,
    attachment_world_mounts: Vec<Option<String>>,
    /// The gun tag each attachment world model names; empty: found by the
    /// model's root bone.
    attachment_world_tags: Vec<String>,
    projectile_model: Option<String>,
    rocket_model: Option<String>,

    sz_xanims: [Option<String>; WEAPON_ANIM_SLOTS],
    sz_xanims_right: [Option<String>; WEAPON_ANIM_SLOTS],
    sz_xanims_left: [Option<String>; WEAPON_ANIM_SLOTS],
    sz_xanim_edges: [AssetEdge<XAnimSpace>; WEAPON_ANIM_SLOTS],
    sz_xanim_right_edges: [AssetEdge<XAnimSpace>; WEAPON_ANIM_SLOTS],
    sz_xanim_left_edges: [AssetEdge<XAnimSpace>; WEAPON_ANIM_SLOTS],
    /// Notetrack name (lower case) to what it triggers.
    notetrack_actions: HashMap<String, LinkedNotetrackAction>,

    hide_tags: Vec<String>,
    attachment_view_models: Vec<String>,
    /// Each attachment view model's authored placement; none: the mount plan
    /// places it itself.
    attachment_view_offsets: Vec<Option<asset_model::FpvMountOffset>>,
    attachment_world_models: Vec<String>,
    dual_wield_weapon: Option<String>,
    spawned_grenade_weapon: Option<String>,

    sounds: WeaponSoundAliases,
    combat_fx: WeaponCombatFx,
    reticle: WeaponReticleAssets,
    display_name_key: Option<String>,

    overlay_material: Option<String>,
    overlay_image: Option<String>,
    overlay_edge: AssetEdge<MaterialSpace>,
    hud_icon: Option<String>,
    hud_icon_edge: AssetEdge<MaterialSpace>,
    kill_icon: Option<String>,
    kill_icon_edge: AssetEdge<MaterialSpace>,
    proj_trail: Option<String>,
    trail_edge: AssetEdge<FxSpace>,
    proj_ignition: Option<String>,
    ignition_edge: AssetEdge<FxSpace>,
}

impl Default for WeaponRow {
    fn default() -> Self {
        Self {
            name: String::new(),
            alternate_weapon: None,
            alternate_index: 0,
            namespace: crate::AssetNamespace::T6,
            facts: WeaponBodyFacts::default(),
            gun_xmodel: None,
            gun_xmodel_edge: AssetEdge::Absent,
            rocket_model_edge: AssetEdge::Absent,
            attachment_view_model_edges: Vec::new(),
            fpv_hands: [None, None],
            fpv_mount_plan: None,
            ads_view_models: Vec::new(),
            ads_view_offsets: Vec::new(),
            fpv_mount_plan_ads: None,
            fpv_assemblies: [None, None],
            world_model: None,
            world_model_edge: AssetEdge::Absent,
            attachment_world_model_edges: Vec::new(),
            attachment_world_mounts: Vec::new(),
            attachment_world_tags: Vec::new(),
            projectile_model: None,
            rocket_model: None,
            sz_xanims: [const { None }; WEAPON_ANIM_SLOTS],
            sz_xanims_right: [const { None }; WEAPON_ANIM_SLOTS],
            sz_xanims_left: [const { None }; WEAPON_ANIM_SLOTS],
            sz_xanim_edges: [AssetEdge::Absent; WEAPON_ANIM_SLOTS],
            sz_xanim_right_edges: [AssetEdge::Absent; WEAPON_ANIM_SLOTS],
            sz_xanim_left_edges: [AssetEdge::Absent; WEAPON_ANIM_SLOTS],
            notetrack_actions: HashMap::new(),
            hide_tags: Vec::new(),
            attachment_view_models: Vec::new(),
            attachment_view_offsets: Vec::new(),
            attachment_world_models: Vec::new(),
            dual_wield_weapon: None,
            spawned_grenade_weapon: None,
            sounds: WeaponSoundAliases::default(),
            combat_fx: WeaponCombatFx::default(),
            reticle: WeaponReticleAssets::default(),
            display_name_key: None,
            overlay_material: None,
            overlay_image: None,
            overlay_edge: AssetEdge::Absent,
            hud_icon: None,
            hud_icon_edge: AssetEdge::Absent,
            kill_icon: None,
            kill_icon_edge: AssetEdge::Absent,
            proj_trail: None,
            trail_edge: AssetEdge::Absent,
            proj_ignition: None,
            ignition_edge: AssetEdge::Absent,
        }
    }
}

/// The finished weapon table. Ids are row numbers; id 0 is "no weapon".
#[derive(Clone, Debug, Default)]
pub struct WeaponRegistry {
    rows: Vec<WeaponRow>,
    world_catalog_identity: u64,
    by_name: HashMap<String, u32>,
    by_namespaced: HashMap<(crate::AssetNamespace, String), u32>,
    families: crate::WeaponFamilies,
    alternate_fpv: HashMap<(u32, u32), [Option<Result<crate::FpvSideAssemblies, String>>; 2]>,
    fpv_clip_tracks: Arc<crate::FpvClipTracks>,
    revision: u64,
}

/// A registry still being linked. It reads as the registry it will become.
#[derive(Clone, Debug, Default)]
pub struct WeaponBuild {
    registry: WeaponRegistry,
    family_tables: Vec<(crate::AssetNamespace, crate::CapturedStringTable)>,
}

impl std::ops::Deref for WeaponBuild {
    type Target = WeaponRegistry;

    fn deref(&self) -> &Self::Target {
        &self.registry
    }
}
