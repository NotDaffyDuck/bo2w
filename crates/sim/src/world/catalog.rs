//! What a loaded map and weapon set hand the simulation: the collision data, the weapon tables, the player body kits.
//!
//! A [`SimContentBuilder`] is filled in once at load time and frozen into an `Arc<SimContent>`; every `SimState`
//! (the server's and each client's prediction copy) shares the same frozen content.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use weapon_t6::combat_facts::WeaponCombatFacts;

use super::state::SimState;
use crate::equipment::EquipmentRuntimeFacts;

/// One convex brush of the fallback clip data, with the surface flags per plane.
#[derive(Clone, Debug)]
pub struct SimBrush {
    pub planes: Vec<[f32; 4]>,
    pub contents: u32,
    pub plane_surface_flags: Vec<u32>,
    /// Zero for solid world; otherwise the glass pane's index plus one.
    pub glass_encoded: u16,
    pub verts: Vec<[f32; 3]>,
    pub hit_surface_flags: u32,
}

/// The BSP side of the clip data.
#[derive(Clone, Debug, Default)]
pub struct SimClipBsp {
    pub leafbrushes: Vec<u16>,
    pub t6: Option<&'static clipmap_t6::ClipMap<'static>>,
    pub t6_glass: Vec<SimBrush>,
}

impl SimClipBsp {
    /// The glass panes among `brushes`, copied out so they can be switched on and off per trace.
    #[must_use]
    pub fn glass_of(brushes: &[SimBrush]) -> Vec<SimBrush> {
        brushes.iter().filter(|b| b.glass_encoded != 0).cloned().collect()
    }
}

/// The static-model side of the clip data.
#[derive(Clone, Debug, Default)]
pub struct SimClipMesh {
    pub t6_static: Option<&'static clipmap_t6::StaticWorld>,
}

/// The clip models (doors, movers, triggers) and the names that find them.
#[derive(Clone, Debug, Default)]
pub struct SimClipCmodels {
    pub models: Vec<asset_world::ClipCmodel>,
    pub triggers: Vec<Vec<SimTriggerHull>>,
    pub model_cmodels: HashMap<String, u32>,
}

/// A trigger volume: a box that bounds it and the slabs that shape it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SimTriggerHull {
    pub mid: [f32; 3],
    pub half: [f32; 3],
    pub slabs: Vec<([f32; 3], f32, f32)>,
}

/// The models a team's players are built from, as far as collision is concerned.
#[derive(Clone, Debug, Default)]
pub struct PlayerKitCollision {
    pub body_key: String,
    pub body: Option<Arc<xmodel_runtime::RetainedModelCapability>>,
    pub head_key: String,
    pub head: Option<Arc<xmodel_runtime::RetainedModelCapability>>,
}

impl PlayerKitCollision {
    /// The key a built player skeleton is filed under: two kits that name the same models share one.
    pub(super) fn reuse_key(&self) -> xmodel_runtime::DObjReuseKey {
        xmodel_runtime::DObjReuseKey {
            e_type: i32::from(entity_t6::et::PLAYER),
            model: xmodel_runtime::model_token(&[self.body_key.as_str(), self.head_key.as_str()]),
        }
    }
}

/// The weapon a script name stands for: which realm it belongs to, its base name and its attachments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WeaponSetup {
    pub realm: crate::script::Realm,
    pub base: String,
    pub attachments: Vec<String>,
}

/// The sound aliases a weapon's script asks for by name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WeaponScriptSounds {
    pub fire: Option<String>,
    pub fire_player: Option<String>,
    pub pickup: Option<String>,
    pub pickup_player: Option<String>,
    pub proj_explosion: Option<String>,
    pub proj_explosion_fx: Option<String>,
}

/// Everything the loaders supply. Fields are private; the `set_*` calls below fill them.
#[derive(Debug, Default)]
pub struct SimContentBuilder {
    clip_brushes: Vec<SimBrush>,
    clip_bsp: SimClipBsp,
    clip_mesh: SimClipMesh,
    clip_cmodels: SimClipCmodels,

    weapon_def_scales: Vec<(f32, f32, f32)>,
    weapon_combat: Vec<WeaponCombatFacts>,
    weapon_runnable: Vec<bool>,
    weapon_transition_groups: Vec<u32>,
    gun_facts: Arc<weapon_t6::facts::FactsTable>,
    equipment_runtime: Vec<EquipmentRuntimeFacts>,

    bullet_pen: Vec<weapon_t6::penetration::BulletPenFacts>,
    pen_table: weapon_t6::penetration::PenetrationDepthTable,
    pen_table_loaded: bool,

    player_kits: [PlayerKitCollision; 2],
    script_model_anims: BTreeMap<String, crate::ScriptModelPlayAnim>,
    xanims: Arc<crate::MantleXAnimBind>,

    weapon_script_names: Arc<[String]>,
    weapon_script_index: Arc<HashMap<String, u32>>,
    weapon_script_aliases: BTreeMap<String, u32>,
    weapon_setups: Arc<[Option<WeaponSetup>]>,
    weapon_world_models: Vec<(String, Vec<String>)>,
    weapon_projectile_models: Vec<String>,
    weapon_melee_only: Vec<bool>,
    weapon_script_sounds: Vec<WeaponScriptSounds>,

    vehicle_turrets: BTreeMap<String, String>,
    vehicle_compass: BTreeMap<String, ([String; 2], [i32; 2])>,
    shocks: BTreeMap<String, hud_t6::flashbang::ShellshockParms>,
}

impl SimContentBuilder {
    /// Freezes the content. When only brushes were given, the fallback clip map is built from them here, and a bad
    /// one stops the load rather than letting traces run on broken data.
    pub fn finish(mut self) -> Arc<SimContent> {
        let needs_fallback_map = self.clip_bsp.t6.is_none() && !self.clip_brushes.is_empty();
        if needs_fallback_map {
            let store = crate::clip_fallback::clip_store(
                &self.clip_brushes,
                &self.clip_cmodels.models,
                &self.clip_bsp.leafbrushes,
            );
            if let Err(e) = store.check() {
                panic!("the clip map built from the sim's brushes is bad: {e}");
            }
            self.clip_bsp.t6 = Some(store.leak());
            if self.clip_bsp.t6_glass.is_empty() {
                self.clip_bsp.t6_glass = SimClipBsp::glass_of(&self.clip_brushes);
            }
        }
        Arc::new(SimContent { data: self })
    }

    pub fn set_clip_brushes(&mut self, brushes: Vec<SimBrush>) {
        self.set_clip_map(brushes, SimClipBsp::default(), SimClipMesh::default(), SimClipCmodels::default());
    }

    pub fn set_clip_map(&mut self, brushes: Vec<SimBrush>, bsp: SimClipBsp, mesh: SimClipMesh, cmodels: SimClipCmodels) {
        self.clip_brushes = brushes;
        self.clip_bsp = bsp;
        self.clip_mesh = mesh;
        self.clip_cmodels = cmodels;
    }

    pub fn set_weapon_def_scales(&mut self, scales: Vec<(f32, f32, f32)>) {
        self.weapon_def_scales = scales;
    }

    pub fn set_weapon_combat_table(&mut self, rows: Vec<WeaponCombatFacts>) {
        self.weapon_combat = rows;
    }

    pub fn set_weapon_runnable_table(&mut self, runnable: Vec<bool>) {
        self.weapon_runnable = runnable;
    }

    pub fn set_weapon_transition_groups(&mut self, groups: Vec<u32>) {
        self.weapon_transition_groups = groups;
    }

    pub fn set_gun_facts(&mut self, facts: weapon_t6::facts::FactsTable) {
        self.gun_facts = Arc::new(facts);
    }

    pub fn set_equipment_runtime_table(&mut self, rows: Vec<EquipmentRuntimeFacts>) {
        self.equipment_runtime = rows;
    }

    pub fn set_bullet_pen_facts(&mut self, rows: Vec<weapon_t6::penetration::BulletPenFacts>) {
        self.bullet_pen = rows;
    }

    pub fn set_penetration_table(&mut self, table: weapon_t6::penetration::PenetrationDepthTable) {
        self.pen_table = table;
    }

    pub fn set_pen_table_loaded(&mut self, loaded: bool) {
        self.pen_table_loaded = loaded;
    }

    pub fn set_player_kit_collisions(&mut self, allies: PlayerKitCollision, axis: PlayerKitCollision) {
        self.player_kits = [allies, axis];
    }

    pub fn set_script_model_anims(&mut self, anims: impl IntoIterator<Item = (String, crate::ScriptModelPlayAnim)>) {
        self.script_model_anims = anims.into_iter().map(|(name, facts)| (name.to_ascii_lowercase(), facts)).collect();
    }

    pub fn set_mantle_xanims(&mut self, bind: crate::MantleXAnimBind) {
        self.xanims = Arc::new(bind);
    }

    /// Slot 0 is the empty weapon and never gets a name; the first holder of a duplicate name keeps it.
    pub fn set_weapon_script_names(&mut self, names: Vec<String>) {
        let mut by_name = HashMap::with_capacity(names.len());
        for (slot, name) in names.iter().enumerate().skip(1) {
            if let Ok(slot) = u32::try_from(slot) {
                by_name.entry(name.clone()).or_insert(slot);
            }
        }
        self.weapon_script_index = Arc::new(by_name);
        self.weapon_script_names = names.into();
    }

    pub fn set_weapon_script_aliases(&mut self, aliases: Vec<(String, u32)>) {
        self.weapon_script_aliases = aliases.into_iter().collect();
    }

    pub fn set_weapon_setups(&mut self, setups: Vec<Option<WeaponSetup>>) {
        self.weapon_setups = setups.into();
    }

    pub fn set_weapon_world_models(&mut self, models: Vec<(String, Vec<String>)>) {
        self.weapon_world_models = models;
    }

    pub fn set_weapon_projectile_models(&mut self, models: Vec<String>) {
        self.weapon_projectile_models = models;
    }

    pub fn set_weapon_melee_only(&mut self, rows: Vec<bool>) {
        self.weapon_melee_only = rows;
    }

    pub fn set_weapon_script_sounds(&mut self, rows: Vec<WeaponScriptSounds>) {
        self.weapon_script_sounds = rows;
    }

    pub fn set_vehicle_compass(&mut self, rows: impl IntoIterator<Item = (String, ([String; 2], [i32; 2]))>) {
        self.vehicle_compass = rows.into_iter().collect();
    }

    pub fn set_vehicle_turrets(&mut self, turrets: Vec<(String, String)>) {
        self.vehicle_turrets = turrets.into_iter().collect();
    }

    pub fn set_shocks(&mut self, shocks: BTreeMap<String, hud_t6::flashbang::ShellshockParms>) {
        self.shocks = shocks;
    }
}

/// Frozen content, shared by every state that plays the same map.
#[derive(Debug)]
pub struct SimContent {
    pub(super) data: SimContentBuilder,
}

impl SimContent {
    #[must_use]
    pub fn clip_brushes(&self) -> &[SimBrush] {
        &self.data.clip_brushes
    }

    #[must_use]
    pub fn clip_bsp(&self) -> &SimClipBsp {
        &self.data.clip_bsp
    }

    #[must_use]
    pub fn clip_mesh(&self) -> &SimClipMesh {
        &self.data.clip_mesh
    }

    #[must_use]
    pub fn clip_cmodels(&self) -> &SimClipCmodels {
        &self.data.clip_cmodels
    }

    pub(super) fn player_kits(&self) -> &[PlayerKitCollision; 2] {
        &self.data.player_kits
    }

    pub(super) fn pen_table_loaded(&self) -> bool {
        self.data.pen_table_loaded
    }

    pub(super) fn weapon_combat(&self) -> &[WeaponCombatFacts] {
        &self.data.weapon_combat
    }

    pub(super) fn weapon_runnable_table(&self) -> &[bool] {
        &self.data.weapon_runnable
    }

    pub(super) fn weapon_transition_groups(&self) -> &[u32] {
        &self.data.weapon_transition_groups
    }

    pub(super) fn equipment_table(&self) -> &[EquipmentRuntimeFacts] {
        &self.data.equipment_runtime
    }
}

/// `rows[index]`, or nothing when the table is shorter than the weapon number.
fn table_row<T>(rows: &[T], index: u32) -> Option<&T> {
    rows.get(usize::try_from(index).ok()?)
}

impl SimState {
    /// The frozen content this state plays on.
    #[must_use]
    pub fn content(&self) -> Arc<SimContent> {
        Arc::clone(&self.content)
    }

    #[must_use]
    pub fn clip_brushes(&self) -> &[SimBrush] {
        self.content.clip_brushes()
    }

    #[must_use]
    pub fn clip_bsp(&self) -> &SimClipBsp {
        self.content.clip_bsp()
    }

    #[must_use]
    pub fn clip_mesh(&self) -> &SimClipMesh {
        self.content.clip_mesh()
    }

    #[must_use]
    pub fn clip_cmodels(&self) -> &SimClipCmodels {
        self.content.clip_cmodels()
    }

    #[must_use]
    pub fn clip_brush_count(&self) -> usize {
        self.content.clip_brushes().len()
    }

    #[must_use]
    pub fn pen_table_loaded(&self) -> bool {
        self.content.pen_table_loaded()
    }

    #[must_use]
    pub fn penetration_table(&self) -> &weapon_t6::penetration::PenetrationDepthTable {
        &self.content.data.pen_table
    }

    /// The bullet penetration row for a weapon (a default row when it has none), with the gun brain's "explodes on
    /// impact" flag copied in.
    #[must_use]
    pub(crate) fn bullet_pen_facts_for(&self, weapon: u32) -> weapon_t6::penetration::BulletPenFacts {
        let mut facts = table_row(&self.content.data.bullet_pen, weapon).copied().unwrap_or_default();
        facts.impact_explode = self.gun_facts().get(weapon).core.bullet_impact_explode;
        facts
    }

    // ---- player body kits -------------------------------------------------------------------------------------

    /// Which of the two kits a player wears: axis players and the second FFA team wear the second.
    pub(super) fn collision_kit_index(&self, id: super::ClientId) -> usize {
        let Some(meta) = self.client_meta(id) else {
            return 0;
        };
        let axis = match meta.client_state_team {
            entity_t6::team::AXIS => true,
            entity_t6::team::ALLIES => false,
            _ => meta.ffa_team == Some(1),
        };
        usize::from(axis)
    }

    pub(super) fn collision_kit(&self, id: super::ClientId) -> &PlayerKitCollision {
        &self.content.data.player_kits[self.collision_kit_index(id)]
    }

    /// `"none"` when no kit carries a body model, otherwise `"bind"`: players are posed in their bind pose.
    #[must_use]
    pub fn player_body_pose_kind(&self) -> &'static str {
        if self.content.data.player_kits.iter().all(|kit| kit.body.is_none()) { "none" } else { "bind" }
    }

    #[must_use]
    pub fn xanims(&self) -> Arc<crate::MantleXAnimBind> {
        Arc::clone(&self.content.data.xanims)
    }

    // ---- weapon names -----------------------------------------------------------------------------------------

    #[must_use]
    pub fn weapon_script_names(&self) -> Arc<[String]> {
        Arc::clone(&self.content.data.weapon_script_names)
    }

    #[must_use]
    pub(crate) fn weapon_setup(&self, weapon: u32) -> Option<&WeaponSetup> {
        table_row(&self.content.data.weapon_setups, weapon)?.as_ref()
    }

    #[must_use]
    pub fn weapon_script_name(&self, weapon: u32) -> &str {
        table_row(&self.content.data.weapon_script_names, weapon).map_or("", String::as_str)
    }

    /// The weapon number for a script name; the alias table is consulted before the names.
    #[must_use]
    pub fn weapon_index_by_script_name(&self, name: &str) -> Option<u32> {
        let data = &self.content.data;
        data.weapon_script_aliases.get(name).or_else(|| data.weapon_script_index.get(name)).copied()
    }

    #[must_use]
    pub(crate) fn weapon_projectile_model(&self, weapon: u32) -> &str {
        table_row(&self.content.data.weapon_projectile_models, weapon).map_or("", String::as_str)
    }

    #[must_use]
    pub(crate) fn weapon_script_sounds(&self, weapon: u32) -> Option<&WeaponScriptSounds> {
        table_row(&self.content.data.weapon_script_sounds, weapon)
    }

    #[must_use]
    pub(crate) fn weapon_is_melee_only(&self, weapon: u32) -> bool {
        table_row(&self.content.data.weapon_melee_only, weapon).copied().unwrap_or(false)
    }

    #[must_use]
    pub(crate) fn weapon_world_model(&self, weapon: u32) -> Option<(&str, &[String])> {
        let (model, attachments) = table_row(&self.content.data.weapon_world_models, weapon)?;
        Some((model.as_str(), attachments.as_slice()))
    }

    #[must_use]
    pub(crate) fn scales_for(&self, weapon: u32) -> (f32, f32, f32) {
        table_row(&self.content.data.weapon_def_scales, weapon).copied().unwrap_or((0.0, 0.0, 1.0))
    }

    // ---- vehicles, shocks, model animations -------------------------------------------------------------------

    #[must_use]
    pub fn vehicle_compass(&self, name: &str) -> Option<&([String; 2], [i32; 2])> {
        self.content.data.vehicle_compass.get(name)
    }

    /// The weapon a vehicle's turret fires: its turret file's base name looked up as a weapon script name.
    #[must_use]
    pub fn vehicle_turret_weapon(&self, vehicle: &str) -> Option<u32> {
        let path = self.content.data.vehicle_turrets.get(vehicle)?;
        let file = path.rsplit('/').next().unwrap_or(path);
        self.weapon_index_by_script_name(file)
    }

    #[must_use]
    pub(crate) fn shock(&self, name: &str) -> Option<&hud_t6::flashbang::ShellshockParms> {
        self.content.data.shocks.get(&name.to_ascii_lowercase())
    }

    /// Per-map custom strings; this build carries none.
    #[must_use]
    pub(crate) fn map_custom(&self, _key: &str) -> &str {
        ""
    }

    #[must_use]
    pub(crate) fn script_model_anim(&self, name: &str) -> Option<crate::ScriptModelPlayAnim> {
        self.content.data.script_model_anims.get(&name.to_ascii_lowercase()).copied()
    }

    // ---- combat and equipment rows ----------------------------------------------------------------------------

    #[must_use]
    pub fn weapon_combat_len(&self) -> usize {
        self.content.data.weapon_combat.len()
    }

    #[must_use]
    pub fn weapon_combat_row(&self, weapon: u32) -> Option<WeaponCombatFacts> {
        table_row(&self.content.data.weapon_combat, weapon).copied()
    }

    /// The combat row, but only when the weapon can actually be used.
    #[must_use]
    pub(crate) fn combat_facts_for(&self, weapon: u32) -> Option<WeaponCombatFacts> {
        self.weapon_combat_row(weapon).filter(|row| row.is_usable())
    }

    #[must_use]
    pub(crate) fn equipment_row_raw(&self, weapon: u32) -> Option<EquipmentRuntimeFacts> {
        table_row(&self.content.data.equipment_runtime, weapon).copied()
    }

    #[must_use]
    pub(crate) fn equipment_facts_for(&self, weapon: u32) -> Option<EquipmentRuntimeFacts> {
        self.equipment_row_raw(weapon).filter(|row| row.is_usable())
    }

    #[must_use]
    pub(crate) fn offhand_loadout_row(&self, weapon: u32) -> Option<EquipmentRuntimeFacts> {
        self.equipment_row_raw(weapon).filter(|row| row.is_offhand())
    }

    #[must_use]
    pub(crate) fn missile_launch_facts(&self, weapon: u32) -> Option<EquipmentRuntimeFacts> {
        self.equipment_row_raw(weapon).filter(|facts| facts.projectile_speed > 0)
    }

    /// True when `from` may be swapped to `to` mid-animation: both sit in the same non-zero transition group.
    #[must_use]
    pub(crate) fn can_transition_weapon(&self, from: u32, to: u32) -> bool {
        if from == 0 || from == to {
            return false;
        }
        let groups = &self.content.data.weapon_transition_groups;
        let Some(&group) = table_row(groups, from) else {
            return false;
        };
        group != 0 && table_row(groups, to) == Some(&group)
    }

    #[must_use]
    pub fn gun_facts(&self) -> &weapon_t6::facts::FactsTable {
        &self.content.data.gun_facts
    }

    #[must_use]
    pub(crate) fn gun_facts_shared(&self) -> Arc<weapon_t6::facts::FactsTable> {
        Arc::clone(&self.content.data.gun_facts)
    }

    #[must_use]
    pub(crate) fn weapon_runnable(&self, weapon: u32) -> bool {
        table_row(&self.content.data.weapon_runnable, weapon).copied().unwrap_or(false)
    }
}
