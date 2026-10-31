//! The bullet tracers the loaded zones define, found by name.

use std::collections::HashMap;
use std::ops::{Deref, DerefMut};

use asset_core::{AssetDependencyLink, DependencyCause, MaterialKey, SurfaceShaderSpace, ZoneCustodian};

use crate::{AssetCategoryScope, MaterialDefinitions};

/// One tracer as a zone defines it.
#[derive(Clone, Debug, PartialEq)]
pub struct SelfContainedTracerDef {
    pub namespace: AssetCategoryScope,
    pub name: String,
    /// The material row the beam draws with, once the materials are loaded.
    pub material: AssetDependencyLink<SurfaceShaderSpace>,
    /// The beam material's name, when the zone gave one.
    pub surface_title: Option<String>,
    pub render_spacing: u32,
    pub speed: f32,
    pub ray_extent: f32,
    pub ray_thickness: f32,
    pub spiral_radius: f32,
    pub spiral_pitch_distance: f32,
    pub colors: [[f32; 4]; 5],
    /// How long a smoke tracer (type 1) takes to fade out along the bullet's
    /// whole path, in milliseconds. 0 for a tracer that flies.
    pub fade_msec: u32,
}

impl SelfContainedTracerDef {
    /// The beam material's name, when that material is loaded.
    pub fn existing_label(&self) -> Option<&str> {
        self.surface_title.as_deref().filter(|_| self.material.is_attached())
    }
}

/// Every tracer, numbered in the order it first arrived. A tracer defined
/// again under the same name replaces the earlier one where it stands.
#[derive(Clone, Debug, Default)]
pub struct TracerDefinitions {
    streak_defs: Vec<SelfContainedTracerDef>,
    slot_by_key: HashMap<(AssetCategoryScope, String), usize>,
}

impl TracerDefinitions {
    pub fn len(&self) -> usize {
        self.streak_defs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.streak_defs.is_empty()
    }

    pub fn recipe_at(&self, index: usize) -> Option<&SelfContainedTracerDef> {
        self.streak_defs.get(index)
    }

    /// Every tracer, in the order it first arrived.
    pub fn defs(&self) -> impl Iterator<Item = &SelfContainedTracerDef> {
        self.streak_defs.iter()
    }

    pub fn position_of(&self, scope: AssetCategoryScope, wanted: &str) -> Option<usize> {
        self.slot_by_key.get(&(scope, wanted.to_owned())).copied()
    }

    /// Tracers are gathered with all their zones merged, so none of them
    /// has a zone of its own.
    pub fn source_zone(&self, _index: usize) -> ZoneCustodian {
        ZoneCustodian::default()
    }

    /// The materials the beams name.
    pub fn material_keys(&self) -> impl Iterator<Item = MaterialKey> + '_ {
        self.streak_defs.iter().filter_map(|def| {
            def.surface_title.clone().map(|name| MaterialKey { namespace: def.namespace, name })
        })
    }

    /// Binds each beam to the material row that has since been loaded. A beam
    /// that named a material nobody has loaded stays unresolved.
    pub fn link_surfaces(&mut self, mat_defs: &MaterialDefinitions) {
        self.streak_defs.iter_mut().for_each(|def| def.material = beam_material(def, mat_defs));
    }

    fn put(&mut self, def: SelfContainedTracerDef) {
        let key = (def.namespace, def.name.clone());
        match self.slot_by_key.get(&key) {
            Some(&at) => self.streak_defs[at] = def,
            None => {
                self.slot_by_key.insert(key, self.streak_defs.len());
                self.streak_defs.push(def);
            }
        }
    }
}

/// Whether the material row at `order` is a real, named one.
fn is_loaded_row(mat_list: &MaterialDefinitions, rank: usize) -> bool {
    matches!(mat_list.materials.get(rank), Some(row) if row.name.is_genuine() && !row.name.is_empty())
}

/// The edge a tracer's beam gets once the materials are loaded.
fn beam_material(def: &SelfContainedTracerDef, mat_list: &MaterialDefinitions) -> AssetDependencyLink<SurfaceShaderSpace> {
    let bound = def
        .surface_title
        .as_deref()
        .and_then(|name| mat_list.material_index_by_ns(def.namespace, name))
        .filter(|index| is_loaded_row(mat_list, index.order()));
    bound.map_or_else(
        || AssetDependencyLink::unresolved_presence(def.surface_title.is_some(), false),
        |index| AssetDependencyLink::bind(index, mat_list.owning_zone(index.order())),
    )
}

/// The tracers still being gathered. It reads as the set it will become.
#[derive(Clone, Debug, Default)]
pub struct TracerRoster {
    defs: TracerDefinitions,
}

impl Deref for TracerRoster {
    type Target = TracerDefinitions;

    fn deref(&self) -> &TracerDefinitions {
        &self.defs
    }
}

impl DerefMut for TracerRoster {
    fn deref_mut(&mut self) -> &mut TracerDefinitions {
        &mut self.defs
    }
}

impl TracerRoster {
    /// Adds a zone's tracer. False when the zone only points at a tracer
    /// another zone defines (a name starting with a comma) or names none.
    pub fn capture_t6(&mut self, tracer: &asset_t6::TracerRef) -> bool {
        if tracer.name.is_empty() || tracer.name.starts_with(',') {
            return false;
        }
        let smoke = tracer.ty == 1;
        self.defs.put(SelfContainedTracerDef {
            namespace: AssetCategoryScope::T6,
            name: tracer.name.clone(),
            material: AssetDependencyLink::Unresolved(DependencyCause::IndexLookupFailure),
            surface_title: Some(tracer.material.clone()).filter(|name| !name.is_empty()),
            render_spacing: u32::try_from(tracer.render_spacing).unwrap_or(0),
            speed: tracer.speed,
            ray_extent: tracer.ray_extent,
            ray_thickness: tracer.ray_thickness,
            spiral_radius: tracer.spiral_radius,
            spiral_pitch_distance: tracer.spiral_pitch_distance,
            colors: tracer.colors,
            fade_msec: if smoke {
                (tracer.fade_secs.max(0.0) * 1000.0) as u32
            } else {
                0
            },
        });
        true
    }

    /// Takes in the tracers a map zone defines; one already here under the
    /// same name is replaced.
    pub fn merge_in(&mut self, other: TracerRoster) {
        for def in other.defs.streak_defs {
            self.defs.put(def);
        }
    }

    pub fn publish(self) -> TracerDefinitions {
        self.defs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracer(name: &str, material: &str) -> asset_t6::TracerRef {
        let [name, material] = [name, material].map(str::to_owned);
        asset_t6::TracerRef { name, material, ..Default::default() }
    }

    fn material_row(name: crate::AssetPointer) -> asset_material::DesignerMaterial {
        let byte = [0_u8; 4];
        asset_material::DesignerMaterial {
            name,
            namespace: AssetCategoryScope::T6,
            method_set: crate::AssetPointer::default(),
            zone: ZoneCustodian::default(),
            draw_order: byte[0],
            game_flag_info: byte[1],
            condition_bits: byte[2],
            camera_zone: byte[3],
            facet_type_bits: Default::default(),
            raster_bits: Default::default(),
            raster_slot: Default::default(),
            method_table: Default::default(),
            route: Default::default(),
            textures: Default::default(),
            constants: Default::default(),
            t6_draw: Default::default(),
        }
    }

    #[test]
    fn a_beam_binds_to_a_loaded_material_and_otherwise_stays_open() {
        let mut materials = MaterialDefinitions::default();
        materials.materials.push(material_row(crate::AssetPointer::Real("mtl_beam".into())));
        let mut catalog = TracerRoster::default();
        catalog.capture_t6(&tracer("hit", "mtl_beam"));
        catalog.capture_t6(&tracer("lost", "mtl_gone"));
        catalog.capture_t6(&tracer("plain", ""));
        catalog.link_surfaces(&materials);
        let edges: Vec<_> = catalog.defs().map(|def| def.material).collect();
        assert_eq!(edges[0].extent_position(), Some(0));
        assert!(edges[1].is_unlinked_ref());
        assert!(edges[2].lacks_entry());
        assert_eq!(catalog.recipe_at(0).and_then(SelfContainedTracerDef::existing_label), Some("mtl_beam"));
        assert_eq!(catalog.recipe_at(1).and_then(SelfContainedTracerDef::existing_label), None);
    }

    #[test]
    fn stubs_and_unnamed_tracers_are_left_out() {
        let mut catalog = TracerRoster::default();
        assert!(!catalog.capture_t6(&tracer("", "m")));
        assert!(!catalog.capture_t6(&tracer(",elsewhere", "m")));
        assert!(catalog.capture_t6(&tracer("bullet", "m")));
        assert_eq!(catalog.len(), 1);
    }

    #[test]
    fn a_later_definition_replaces_the_earlier_one_in_place() {
        let mut catalog = TracerRoster::default();
        catalog.capture_t6(&tracer("a", "one"));
        catalog.capture_t6(&tracer("b", "two"));
        catalog.capture_t6(&tracer("a", "three"));
        assert_eq!(catalog.len(), 2);
        let at = catalog.position_of(AssetCategoryScope::T6, "a").expect("a");
        assert_eq!(at, 0);
        assert_eq!(catalog.recipe_at(at).and_then(|d| d.surface_title.as_deref()), Some("three"));
    }

    #[test]
    fn absorbing_a_map_zone_replaces_by_name_and_appends_the_rest() {
        let mut common = TracerRoster::default();
        common.capture_t6(&tracer("a", "one"));
        let mut map = TracerRoster::default();
        map.capture_t6(&tracer("a", "map"));
        map.capture_t6(&tracer("c", "extra"));
        common.merge_in(map);
        assert_eq!(common.len(), 2);
        let names: Vec<_> = common.material_keys().map(|key| key.name).collect();
        assert_eq!(names, ["map", "extra"]);
    }

    #[test]
    fn only_smoke_tracers_fade() {
        let mut catalog = TracerRoster::default();
        let mut smoke = tracer("smoke", "");
        smoke.ty = 1;
        smoke.fade_secs = 1.5;
        let mut flying = tracer("flying", "");
        flying.fade_secs = 1.5;
        catalog.capture_t6(&smoke);
        catalog.capture_t6(&flying);
        assert_eq!(catalog.recipe_at(0).map(|d| d.fade_msec), Some(1500));
        assert_eq!(catalog.recipe_at(1).map(|d| d.fade_msec), Some(0));
    }

    #[test]
    fn a_beam_with_no_material_name_names_nothing() {
        let mut catalog = TracerRoster::default();
        catalog.capture_t6(&tracer("plain", ""));
        assert_eq!(catalog.material_keys().count(), 0);
        assert_eq!(catalog.recipe_at(0).and_then(SelfContainedTracerDef::existing_label), None);
    }
}
