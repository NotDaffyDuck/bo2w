//! Weapons: what the zones say about each gun, and the table every other crate
//! reads them from.
//!
//! The zone walk fills a [`ArmoryRoster`] with one [`RosterWeapon`] per gun.
//! `into_build` turns that into a [`WeaponBuild`], the later passes link its
//! names to meshes, clips, materials and fx entries, and `publish` hands out
//! the finished [`WeaponLedger`]. Weapon id 0 is "no weapon".

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
    AssetDependencyLink, DependencyCause, FpvMeshRange, FxRange, SurfaceShaderSpace, WorldWeaponRange, XAnimRange,
};
use weapon_t6::asset_slots::WEAPON_ANIM_SLOTS;
use weapon_t6::asset_slots::weap_anim_slot;

pub use combat_fx::WeaponBattleEffects;
pub use facts::*;
pub use registry::overlay_name_is_hud_iris;
pub use sounds::{WeaponAudioNicknames, WeaponAudioRole};
pub use t6::{T6_SIGHT_ATTACHMENTS, t6_attachment_combo_name};

/// A fresh revision number: no two registries, or two states of one, share one.
fn fresh_epoch() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// The key weapons are looked up by: lower case, without BO2's `_mp` suffix.
fn weapon_key(raw: &str) -> String {
    let mut key = raw.to_ascii_lowercase();
    if key.ends_with("_mp") {
        key.truncate(key.len() - 3);
    }
    key
}

/// The name a script calls a weapon by, from its catalog name.
pub fn gsc_weapon_script_name(index_unprefixed: &str) -> String {
    // A gun with a sight keeps its suffix in the middle (`mp7_mp+reflex`).
    let suffixed = index_unprefixed.ends_with("_mp") || index_unprefixed.contains("_mp+");
    match (index_unprefixed.is_empty(), suffixed) {
        (true, _) => String::new(),
        (false, true) => String::from(index_unprefixed),
        (false, false) => [index_unprefixed, "_mp"].concat(),
    }
}

/// A weapon as the zone walk captured it, before anything is linked.
#[derive(Clone, Debug)]
pub struct RosterWeapon {
    pub namespace: crate::AssetCategoryScope,
    pub name: String,
    pub alternate_weapon: Option<String>,
    pub shown_title_token: Option<String>,
    pub reticle: WeaponCrosshairResources,
    pub hud_layer_surface: Option<String>,
    pub badge_pic: Option<String>,
    pub elimination_emblem: Option<String>,
    pub shell_wake: Option<String>,
    pub shell_ignite: Option<String>,
    pub weapon_mesh: Option<String>,
    pub third_person_mesh: Option<String>,
    pub shell_mesh: Option<String>,
    pub missile_mesh: Option<String>,
    pub clip_names: [Option<String>; WEAPON_ANIM_SLOTS],
    pub right_clip_names: [Option<String>; WEAPON_ANIM_SLOTS],
    pub left_clip_names: [Option<String>; WEAPON_ANIM_SLOTS],
    pub concealed_bones: Vec<String>,
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
    /// bo2zm: the parts of the gun everyone else sees, on a wall or in the
    /// box (`attachWorldModel`), and where each sits: an empty tag means the
    /// gun's `tag_weapon`.
    pub world_parts: Vec<(String, asset_model::FpvMountOffset)>,
    pub sounds: WeaponAudioNicknames,
    pub combat_fx: WeaponBattleEffects,
    pub facts: WeaponChassisSummary,
}

/// A scope's reticle pictures.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WeaponCrosshairResources {
    pub middle_surface: Option<String>,
    pub flank_surface: Option<String>,
    pub center_edge: AssetDependencyLink<SurfaceShaderSpace>,
    pub side_edge: AssetDependencyLink<SurfaceShaderSpace>,
    pub middle_texture: Option<String>,
    pub flank_texture: Option<String>,
    pub middle_extent: i32,
    pub flank_extent: i32,
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
pub struct UnrecognizedArmoryLabel {
    pub name: String,
}

impl core::fmt::Display for UnrecognizedArmoryLabel {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_fmt(format_args!("unknown weapon name `{}`", self.name))
    }
}

impl std::error::Error for UnrecognizedArmoryLabel {}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FpvAssemblyCensus {
    pub built: usize,
    pub linked: usize,
    pub refused: usize,
    pub clip_tables: usize,
}

/// The weapons one zone walk captured.
#[derive(Clone, Debug, Default)]
pub struct ArmoryRoster {
    captured: Vec<RosterWeapon>,
    into_ns: crate::AssetCategoryScope,
}

/// A name a weapon carries and the catalog link made from it.
#[derive(Clone, Debug)]
struct NamedLink<S: asset_core::KeyDomain> {
    caption: Option<String>,
    resolved: AssetDependencyLink<S>,
}

impl<S: asset_core::KeyDomain> NamedLink<S> {
    /// A link that has a name but is not looked up yet.
    fn named(caption: Option<String>) -> Self {
        Self { caption, resolved: AssetDependencyLink::NothingThere }
    }
}

impl<S: asset_core::KeyDomain> Default for NamedLink<S> {
    fn default() -> Self {
        Self::named(None)
    }
}

/// A list of attached models with where each one sits and what it linked to.
#[derive(Clone, Debug)]
struct Attached<S: asset_core::KeyDomain> {
    models: Vec<String>,
    offsets: Vec<Option<asset_model::FpvMountOffset>>,
    edges: Vec<AssetDependencyLink<S>>,
}

impl<S: asset_core::KeyDomain> Default for Attached<S> {
    fn default() -> Self {
        let (mesh_names, offsets, edges) = (vec![], vec![], vec![]);
        Self { models: mesh_names, offsets, edges }
    }
}

/// Everything the first-person view of a weapon is built from.
#[derive(Clone, Debug, Default)]
struct ViewLinks {
    gun: NamedLink<FpvMeshRange>,
    missile: NamedLink<FpvMeshRange>,
    /// Attachments drawn while hip-firing.
    hip: Attached<FpvMeshRange>,
    /// Attachments drawn while aimed (a sight's stripped model).
    aimed: Attached<FpvMeshRange>,
    hands: [Option<(asset_model::FirstPersonArms, crate::FpvMeshSlot)>; 2],
    plan_hip: Option<Result<asset_model::FpvMountPlan, asset_model::FpvMountError>>,
    plan_aimed: Option<Result<asset_model::FpvMountPlan, asset_model::FpvMountError>>,
    sides: [Option<Result<crate::FpvSideAssemblies, String>>; 2],
}

/// The third-person gun and what rides on it.
#[derive(Clone, Debug, Default)]
struct WorldLinks {
    gun: NamedLink<WorldWeaponRange>,
    attached: Attached<WorldWeaponRange>,
    /// The bone each attachment is found on, once resolved.
    mounts: Vec<Option<String>>,
    /// The gun tag each attachment world model names; empty: found by the
    /// model's root bone.
    tags: Vec<String>,
    shot: Option<String>,
    /// bo2zm: see `RosterWeapon::world_parts`.
    pieces: Vec<(String, asset_model::FpvMountOffset)>,
}

const ANIM_SZ: usize = 0;
const ANIM_RIGHT: usize = 1;
const ANIM_LEFT: usize = 2;

/// The three animation sets of a weapon (single, and the dual-wield pair),
/// as names and as the links made from them.
#[derive(Clone, Debug)]
struct AnimLinks {
    names: [[Option<String>; WEAPON_ANIM_SLOTS]; 3],
    edges: [[AssetDependencyLink<XAnimRange>; WEAPON_ANIM_SLOTS]; 3],
}

impl Default for AnimLinks {
    fn default() -> Self {
        Self {
            names: std::array::from_fn(|_| std::array::from_fn(|_| None)),
            edges: [[AssetDependencyLink::NothingThere; WEAPON_ANIM_SLOTS]; 3],
        }
    }
}

/// The 2D assets a weapon shows: scope, hud and kill-feed icons.
#[derive(Clone, Debug, Default)]
struct HudLinks {
    scope: NamedLink<SurfaceShaderSpace>,
    scope_image: Option<String>,
    icon: NamedLink<SurfaceShaderSpace>,
    elimination_emblem: NamedLink<SurfaceShaderSpace>,
}

/// One weapon in the registry: the capture plus every link made since.
#[derive(Clone, Debug, Default)]
struct WeaponEntry {
    name: String,
    source_ns: crate::AssetCategoryScope,
    body: WeaponChassisSummary,
    /// The alternate (mode-switch) weapon: its name, then its id once known.
    alt: (Option<String>, u32),
    dual_wield: Option<String>,
    spawned_grenade: Option<String>,

    view: ViewLinks,
    world: WorldLinks,
    anims: AnimLinks,
    /// Notetrack name (lower case) to what it triggers.
    notetracks: HashMap<String, LinkedNotetrackAction>,
    hidden: Vec<String>,

    aliases: WeaponAudioNicknames,
    fx: WeaponBattleEffects,
    sight: WeaponCrosshairResources,
    display_key: Option<String>,
    hud: HudLinks,
    trail: NamedLink<FxRange>,
    ignition: NamedLink<FxRange>,
}

/// The finished weapon table. Ids are row numbers; id 0 is "no weapon".
#[derive(Clone, Debug, Default)]
pub struct WeaponLedger {
    records: Vec<WeaponEntry>,
    world_catalog_identity: u64,
    slot_for_label: HashMap<String, u32>,
    index_by_ns_name: HashMap<(crate::AssetCategoryScope, String), u32>,
    families: crate::WeaponFamilies,
    alternate_fpv: HashMap<(u32, u32), [Option<Result<crate::FpvSideAssemblies, String>>; 2]>,
    fpv_clip_tracks: Arc<crate::FpvClipTracks>,
    epoch_no: u64,
}

/// A registry still being linked. It reads as the registry it will become.
#[derive(Clone, Debug, Default)]
pub struct WeaponBuild {
    registry: WeaponLedger,
    family_tables: Vec<(crate::AssetCategoryScope, crate::SnapshotStringTable)>,
}

impl std::ops::Deref for WeaponBuild {
    type Target = WeaponLedger;

    fn deref(&self) -> &Self::Target {
        &self.registry
    }
}
