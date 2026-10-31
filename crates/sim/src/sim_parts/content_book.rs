//! What a loaded map and weapon set hand the simulation: the collision data, the weapon tables, the player body kits.
//!
//! A [`SimContentBuilder`] is filled in once at load time and frozen into an `Arc<SimContent>`; every `MatchSimulation`
//! (the server's and each client's prediction copy) shares the same frozen content.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use weapon_t6::combat_facts::GunBattleProfile;

use super::state::MatchSimulation;
use crate::gear::GadgetLiveStats;

/// One convex brush of the fallback clip data, with the surface flags per plane.
#[derive(Clone, Debug)]
pub struct SolidBrush {
    pub sides: Vec<[f32; 4]>,
    pub solid_bits: u32,
    pub plane_material_flags: Vec<u32>,
    /// Zero for solid world; otherwise the glass pane's index plus one.
    pub glass_hit_code: u16,
    pub vertex_points: Vec<[f32; 3]>,
    pub hit_surface_flags: u32,
}

/// The BSP side of the clip data.
#[derive(Clone, Debug, Default)]
pub struct CollisionTreeData {
    pub brush_leaf_indices: Vec<u16>,
    pub t6: Option<&'static clipmap_t6::ClipMap<'static>>,
    pub t6_glass: Vec<SolidBrush>,
    /// The models' collision-map brushes by brush model handle (`asset_world::CollisionClipData::t6_model_brushes`).
    pub t6_model_brushes: HashMap<u32, &'static [trace_t6::Brush<'static>]>,
}

impl CollisionTreeData {
    /// The glass panes among `brushes`, copied out so they can be switched on and off per trace.
    #[must_use]
    pub fn glass_of(brushes: &[SolidBrush]) -> Vec<SolidBrush> {
        brushes.iter().filter(|b| b.glass_hit_code != 0).cloned().collect()
    }
}

/// The static-model side of the clip data.
#[derive(Clone, Debug, Default)]
pub struct CollisionTriangleData {
    pub t6_static: Option<&'static clipmap_t6::StaticWorld>,
}

/// The clip models (doors, movers, triggers) and the names that find them.
#[derive(Clone, Debug, Default)]
pub struct CollisionSubmodels {
    pub models: Vec<asset_world::CollisionSubmodel>,
    pub triggers: Vec<Vec<SimTriggerHull>>,
    pub model_cmodels: HashMap<String, u32>,
}

/// A trigger volume: a box that bounds it and the slabs that shape it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SimTriggerHull {
    pub mid: [f32; 3],
    pub half: [f32; 3],
    pub planes: Vec<([f32; 3], f32, f32)>,
}

/// The models a team's players are built from, as far as collision is concerned.
#[derive(Clone, Debug, Default)]
pub struct PlayerOutfitHitbox {
    pub torso_model_key: String,
    pub body: Option<Arc<xmodel_runtime::KeptModelAbility>>,
    pub face_model_key: String,
    pub head: Option<Arc<xmodel_runtime::KeptModelAbility>>,
}

impl PlayerOutfitHitbox {
    /// The key a built player skeleton is filed under: two kits that name the same models share one.
    pub(super) fn reuse_key(&self) -> xmodel_runtime::SkeletonCacheKey {
        xmodel_runtime::SkeletonCacheKey {
            entity_kind: i32::from(entity_t6::et::PLAYER),
            model: xmodel_runtime::model_token(&[self.torso_model_key.as_str(), self.face_model_key.as_str()]),
        }
    }
}

/// The weapon a script name stands for: which realm it belongs to, its base name and its attachments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WeaponSetup {
    pub realm: crate::gsc_host::Realm,
    pub base: String,
    pub attachments: Vec<String>,
}

/// A part Black Ops II hangs on a gun's world model (its magazine, scope or rocket, `attachWorldModel`): the model,
/// the gun's bone it sits on (`tag_weapon` when the weapon names none) and its offset and rotation (degrees: pitch,
/// yaw, roll) from that bone.
#[derive(Clone, Debug, PartialEq)]
pub struct WeaponPart {
    pub mesh: String,
    pub joint: String,
    pub offset: [f32; 3],
    pub rotation: [f32; 3],
}

/// A weapon's class and fire type as BO2 numbers them (`weapClass_t`, `weapFireType_t`), which
/// the combat table's remapped numbers cannot give back: `weaponclass` and `weaponissemiauto`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WeaponRawKinds {
    pub gun_kind: i32,
    pub fire_mode: i32,
}

/// The sound aliases a weapon's script asks for by name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WeaponScriptSounds {
    pub shoot: Option<String>,
    pub trigger_sound_name: Option<String>,
    pub grab: Option<String>,
    pub pickup_sound_name: Option<String>,
    pub projectile_blast_fx: Option<String>,
    pub proj_explosion_fx: Option<String>,
}

/// Everything the loaders supply. Fields are private; the `set_*` calls below fill them.
#[derive(Debug, Default)]
pub struct SimContentBuilder {
    collision_solids: Vec<SolidBrush>,
    collision_tree: CollisionTreeData,
    collision_triangles: CollisionTriangleData,
    collision_submodel_set: CollisionSubmodels,

    gun_scales: Vec<(f32, f32, f32)>,
    weapon_combat: Vec<GunBattleProfile>,
    weapon_runnable: Vec<bool>,
    weapon_transition_groups: Vec<u32>,
    gun_facts: Arc<weapon_t6::facts::FactsTable>,
    gadget_live_stats: Vec<GadgetLiveStats>,

    wall_pierce_facts: Vec<weapon_t6::penetration::PierceCapability>,
    pen_table: weapon_t6::penetration::BulletPierceDepthChart,
    wall_pierce_table_ready: bool,

    outfit_hitboxes: [PlayerOutfitHitbox; 2],
    script_model_anims: BTreeMap<String, crate::ScriptedModelAnimPlay>,
    animation_refs: Arc<crate::LedgeClimbAnimSet>,

    weapon_ids_by_script: Arc<[String]>,
    weapon_script_index: Arc<HashMap<String, u32>>,
    weapon_script_aliases: BTreeMap<String, u32>,
    weapon_setups: Arc<[Option<WeaponSetup>]>,
    weapon_world_models: Vec<(String, Vec<String>)>,
    weapon_world_parts: Vec<Vec<WeaponPart>>,
    weapon_display_names: Vec<String>,
    weapon_projectile_models: Vec<String>,
    weapon_melee_only: Vec<bool>,
    weapon_raw_kinds: Vec<WeaponRawKinds>,
    weapon_script_sounds: Vec<WeaponScriptSounds>,

    vehicle_turrets: BTreeMap<String, String>,
    vehicle_compass: BTreeMap<String, ([String; 2], [i32; 2])>,
    shocks: BTreeMap<String, hud_t6::flashbang::ShellshockParms>,
}

impl SimContentBuilder {
    /// Freezes the content. When only brushes were given, the fallback clip map is built from them here, and a bad
    /// one stops the load rather than letting traces run on broken data.
    pub fn finish(mut self) -> Arc<SimContent> {
        let needs_fallback_map = self.collision_tree.t6.is_none() && !self.collision_solids.is_empty();
        if needs_fallback_map {
            let store = crate::brush_clip::clip_store(
                &self.collision_solids,
                &self.collision_submodel_set.models,
                &self.collision_tree.brush_leaf_indices,
            );
            if let Err(e) = store.check() {
                panic!("the clip map built from the sim's brushes is bad: {e}");
            }
            self.collision_tree.t6 = Some(store.leak());
            if self.collision_tree.t6_glass.is_empty() {
                self.collision_tree.t6_glass = CollisionTreeData::glass_of(&self.collision_solids);
            }
        }
        Arc::new(SimContent { data: self })
    }

    pub fn with_solid_brushes(&mut self, brushes: Vec<SolidBrush>) {
        self.set_clip_map(brushes, CollisionTreeData::default(), CollisionTriangleData::default(), CollisionSubmodels::default());
    }

    pub fn set_clip_map(&mut self, brushes: Vec<SolidBrush>, bsp: CollisionTreeData, mesh: CollisionTriangleData, cmodels: CollisionSubmodels) {
        self.collision_solids = brushes;
        self.collision_tree = bsp;
        self.collision_triangles = mesh;
        self.collision_submodel_set = cmodels;
    }

    pub fn with_gun_scales(&mut self, scales: Vec<(f32, f32, f32)>) {
        self.gun_scales = scales;
    }

    pub fn with_gun_fight_table(&mut self, entries: Vec<GunBattleProfile>) {
        self.weapon_combat = entries;
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

    pub fn with_gear_table(&mut self, rows: Vec<GadgetLiveStats>) {
        self.gadget_live_stats = rows;
    }

    pub fn with_pierce_facts(&mut self, rows: Vec<weapon_t6::penetration::PierceCapability>) {
        self.wall_pierce_facts = rows;
    }

    pub fn with_pierce_table(&mut self, grid: weapon_t6::penetration::BulletPierceDepthChart) {
        self.pen_table = grid;
    }

    pub fn mark_pierce_table_ready(&mut self, present: bool) {
        self.wall_pierce_table_ready = present;
    }

    pub fn set_player_kit_collisions(&mut self, allies: PlayerOutfitHitbox, axis: PlayerOutfitHitbox) {
        self.outfit_hitboxes = [allies, axis];
    }

    pub fn set_script_model_anims(&mut self, anims: impl IntoIterator<Item = (String, crate::ScriptedModelAnimPlay)>) {
        self.script_model_anims = anims.into_iter().map(|(name, facts)| (name.to_ascii_lowercase(), facts)).collect();
    }

    pub fn with_climb_clips(&mut self, bind: crate::LedgeClimbAnimSet) {
        self.animation_refs = Arc::new(bind);
    }

    /// Slot 0 is the empty weapon and never gets a name; the first holder of a duplicate name keeps it.
    pub fn store_weapon_script_labels(&mut self, labels: Vec<String>) {
        let mut by_name = HashMap::with_capacity(labels.len());
        for (slot, name) in labels.iter().enumerate().skip(1) {
            if let Ok(slot) = u32::try_from(slot) {
                by_name.entry(name.clone()).or_insert(slot);
            }
        }
        self.weapon_script_index = Arc::new(by_name);
        self.weapon_ids_by_script = labels.into();
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

    pub fn set_weapon_world_parts(&mut self, parts: Vec<Vec<WeaponPart>>) {
        self.weapon_world_parts = parts;
    }

    pub fn set_weapon_display_names(&mut self, labels: Vec<String>) {
        self.weapon_display_names = labels;
    }

    pub fn set_weapon_projectile_models(&mut self, models: Vec<String>) {
        self.weapon_projectile_models = models;
    }

    pub fn set_weapon_melee_only(&mut self, rows: Vec<bool>) {
        self.weapon_melee_only = rows;
    }

    pub fn set_weapon_raw_kinds(&mut self, rows: Vec<WeaponRawKinds>) {
        self.weapon_raw_kinds = rows;
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
    pub fn collision_solids(&self) -> &[SolidBrush] {
        &self.data.collision_solids
    }

    #[must_use]
    pub fn collision_tree(&self) -> &CollisionTreeData {
        &self.data.collision_tree
    }

    #[must_use]
    pub fn collision_triangles(&self) -> &CollisionTriangleData {
        &self.data.collision_triangles
    }

    #[must_use]
    pub fn collision_submodel_set(&self) -> &CollisionSubmodels {
        &self.data.collision_submodel_set
    }

    pub(super) fn outfit_hitboxes(&self) -> &[PlayerOutfitHitbox; 2] {
        &self.data.outfit_hitboxes
    }

    pub(super) fn wall_pierce_table_ready(&self) -> bool {
        self.data.wall_pierce_table_ready
    }

    pub(super) fn weapon_combat(&self) -> &[GunBattleProfile] {
        &self.data.weapon_combat
    }

    pub(super) fn weapon_runnable_table(&self) -> &[bool] {
        &self.data.weapon_runnable
    }

    pub(super) fn weapon_transition_groups(&self) -> &[u32] {
        &self.data.weapon_transition_groups
    }

    pub(super) fn equipment_table(&self) -> &[GadgetLiveStats] {
        &self.data.gadget_live_stats
    }
}

/// `rows[index]`, or nothing when the table is shorter than the weapon number.
fn table_row<T>(rows: &[T], index: u32) -> Option<&T> {
    rows.get(usize::try_from(index).ok()?)
}

impl MatchSimulation {
    /// The frozen content this state plays on.
    #[must_use]
    pub fn content(&self) -> Arc<SimContent> {
        Arc::clone(&self.content)
    }

    #[must_use]
    pub fn collision_solids(&self) -> &[SolidBrush] {
        self.content.collision_solids()
    }

    #[must_use]
    pub fn collision_tree(&self) -> &CollisionTreeData {
        self.content.collision_tree()
    }

    #[must_use]
    pub fn collision_triangles(&self) -> &CollisionTriangleData {
        self.content.collision_triangles()
    }

    #[must_use]
    pub fn collision_submodel_set(&self) -> &CollisionSubmodels {
        self.content.collision_submodel_set()
    }

    #[must_use]
    pub fn solid_brush_total(&self) -> usize {
        self.content.collision_solids().len()
    }

    #[must_use]
    pub fn wall_pierce_table_ready(&self) -> bool {
        self.content.wall_pierce_table_ready()
    }

    #[must_use]
    pub fn wall_pierce_depths(&self) -> &weapon_t6::penetration::BulletPierceDepthChart {
        &self.content.data.pen_table
    }

    /// The bullet penetration row for a weapon (a default row when it has none), with the gun brain's "explodes on
    /// impact" flag copied in.
    #[must_use]
    pub(crate) fn pierce_facts_for(&self, weapon: u32) -> weapon_t6::penetration::PierceCapability {
        let mut facts = table_row(&self.content.data.wall_pierce_facts, weapon).copied().unwrap_or_default();
        facts.impact_explode = self.gun_facts().get(weapon).core.bullet_impact_explode;
        facts
    }

    // ---- player body kits -------------------------------------------------------------------------------------

    /// Which of the two kits a player wears: axis players and the second FFA team wear the second.
    pub(super) fn collision_kit_index(&self, seat: super::SeatNo) -> usize {
        let Some(info) = self.client_meta(seat) else {
            return 0;
        };
        let axis = match info.seat_side {
            entity_t6::team::AXIS => true,
            entity_t6::team::ALLIES => false,
            _ => info.solo_side == Some(1),
        };
        usize::from(axis)
    }

    pub(super) fn collision_kit(&self, id: super::SeatNo) -> &PlayerOutfitHitbox {
        &self.content.data.outfit_hitboxes[self.collision_kit_index(id)]
    }

    /// `"none"` when no kit carries a body model, otherwise `"bind"`: players are posed in their bind pose.
    #[must_use]
    pub fn body_pose_label(&self) -> &'static str {
        if self.content.data.outfit_hitboxes.iter().all(|kit| kit.body.is_none()) { "none" } else { "bind" }
    }

    #[must_use]
    pub fn animation_refs(&self) -> Arc<crate::LedgeClimbAnimSet> {
        Arc::clone(&self.content.data.animation_refs)
    }

    // ---- weapon names -----------------------------------------------------------------------------------------

    #[must_use]
    pub fn weapon_ids_by_script(&self) -> Arc<[String]> {
        Arc::clone(&self.content.data.weapon_ids_by_script)
    }

    #[must_use]
    pub(crate) fn weapon_setup(&self, weapon: u32) -> Option<&WeaponSetup> {
        table_row(&self.content.data.weapon_setups, weapon)?.as_ref()
    }

    #[must_use]
    pub fn weapon_script_label(&self, gun: u32) -> &str {
        table_row(&self.content.data.weapon_ids_by_script, gun).map_or("", String::as_str)
    }

    /// The weapon number for a script name; the alias table is consulted before the names.
    #[must_use]
    pub fn gun_for_script_label(&self, called: &str) -> Option<u32> {
        let data = &self.content.data;
        data.weapon_script_aliases.get(called).or_else(|| data.weapon_script_index.get(called)).copied()
    }

    #[must_use]
    pub(crate) fn weapon_projectile_model(&self, gun: u32) -> &str {
        table_row(&self.content.data.weapon_projectile_models, gun).map_or("", String::as_str)
    }

    #[must_use]
    pub(crate) fn weapon_script_sounds(&self, weapon: u32) -> Option<&WeaponScriptSounds> {
        table_row(&self.content.data.weapon_script_sounds, weapon)
    }

    #[must_use]
    pub(crate) fn weapon_raw_kinds(&self, weapon: u32) -> Option<WeaponRawKinds> {
        table_row(&self.content.data.weapon_raw_kinds, weapon).copied()
    }

    #[must_use]
    pub(crate) fn weapon_is_melee_only(&self, gun: u32) -> bool {
        table_row(&self.content.data.weapon_melee_only, gun).copied().unwrap_or(false)
    }

    #[must_use]
    pub(crate) fn weapon_world_model(&self, weapon: u32) -> Option<(&str, &[String])> {
        let (model, attachments) = table_row(&self.content.data.weapon_world_models, weapon)?;
        Some((model.as_str(), attachments.as_slice()))
    }

    /// The parts Black Ops II hangs on a weapon's world model.
    #[must_use]
    pub(crate) fn weapon_world_parts(&self, weapon: u32) -> &[WeaponPart] {
        table_row(&self.content.data.weapon_world_parts, weapon).map_or(&[], Vec::as_slice)
    }

    /// A weapon's display name key (`getweapondisplayname`).
    #[must_use]
    pub(crate) fn weapon_display_name(&self, weapon: u32) -> Option<&str> {
        table_row(&self.content.data.weapon_display_names, weapon).map(String::as_str).filter(|s| !s.is_empty())
    }

    #[must_use]
    pub(crate) fn damage_scales_of(&self, gun: u32) -> (f32, f32, f32) {
        table_row(&self.content.data.gun_scales, gun).copied().unwrap_or((0.0, 0.0, 1.0))
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
        self.gun_for_script_label(file)
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
    pub(crate) fn script_model_anim(&self, name: &str) -> Option<crate::ScriptedModelAnimPlay> {
        self.content.data.script_model_anims.get(&name.to_ascii_lowercase()).copied()
    }

    // ---- combat and equipment rows ----------------------------------------------------------------------------

    #[must_use]
    pub fn weapon_stats_count(&self) -> usize {
        self.content.data.weapon_combat.len()
    }

    #[must_use]
    pub fn weapon_stats_entry(&self, weapon: u32) -> Option<GunBattleProfile> {
        table_row(&self.content.data.weapon_combat, weapon).copied()
    }

    /// The combat row, but only when the weapon can actually be used.
    #[must_use]
    pub(crate) fn damage_profile_of(&self, weapon: u32) -> Option<GunBattleProfile> {
        self.weapon_stats_entry(weapon).filter(|row| row.ready_for_use())
    }

    #[must_use]
    pub(crate) fn equipment_row_raw(&self, weapon: u32) -> Option<GadgetLiveStats> {
        table_row(&self.content.data.gadget_live_stats, weapon).copied()
    }

    #[must_use]
    pub(crate) fn gadget_stats_lookup(&self, weapon: u32) -> Option<GadgetLiveStats> {
        self.equipment_row_raw(weapon).filter(|row| row.ready_for_use())
    }

    #[must_use]
    pub(crate) fn side_slot_row(&self, weapon: u32) -> Option<GadgetLiveStats> {
        self.equipment_row_raw(weapon).filter(|row| row.is_secondary_gear())
    }

    #[must_use]
    pub(crate) fn rocket_launch_facts(&self, weapon: u32) -> Option<GadgetLiveStats> {
        self.equipment_row_raw(weapon).filter(|facts| facts.missile_velocity > 0)
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
    pub(crate) fn weapon_runnable(&self, gun: u32) -> bool {
        table_row(&self.content.data.weapon_runnable, gun).copied().unwrap_or(false)
    }
}
