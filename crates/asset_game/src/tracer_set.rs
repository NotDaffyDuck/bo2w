//! The bullet tracers the loaded zones define, found by name.

use std::collections::HashMap;
use std::ops::{Deref, DerefMut};

use asset_core::{AssetEdge, AssetEdgeReason, MaterialKey, MaterialSpace, ZoneOwner};

use crate::{AssetNamespace, MaterialDefinitions};

/// The material a tracer's beam is drawn with.
pub type TracerMaterial = AssetEdge<MaterialSpace>;

/// One tracer as a zone defines it.
#[derive(Clone, Debug, PartialEq)]
pub struct OwnedTracerDef {
    pub namespace: AssetNamespace,
    pub name: String,
    /// The material row the beam draws with, once the materials are loaded.
    pub material: TracerMaterial,
    /// The beam material's name, when the zone gave one.
    pub material_name: Option<String>,
    pub draw_interval: u32,
    pub speed: f32,
    pub beam_length: f32,
    pub beam_width: f32,
    pub screw_radius: f32,
    pub screw_dist: f32,
    pub colors: [[f32; 4]; 5],
    /// How long a smoke tracer (type 1) takes to fade out along the bullet's
    /// whole path, in milliseconds. 0 for a tracer that flies.
    pub fade_msec: u32,
}

impl OwnedTracerDef {
    /// The beam material's name, when that material is loaded.
    pub fn present_name(&self) -> Option<&str> {
        if self.material.is_bound() {
            self.material_name.as_deref()
        } else {
            None
        }
    }
}

/// Every tracer, numbered in the order it first arrived. A tracer defined
/// again under the same name replaces the earlier one where it stands.
#[derive(Clone, Debug, Default)]
pub struct TracerDefinitions {
    rows: Vec<OwnedTracerDef>,
    by_key: HashMap<(AssetNamespace, String), usize>,
}

impl TracerDefinitions {
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn def_at(&self, index: usize) -> Option<&OwnedTracerDef> {
        self.rows.get(index)
    }

    /// Every tracer, in the order it first arrived.
    pub fn defs(&self) -> impl Iterator<Item = &OwnedTracerDef> {
        self.rows.iter()
    }

    pub fn index_of(&self, namespace: AssetNamespace, name: &str) -> Option<usize> {
        self.by_key.get(&(namespace, name.to_owned())).copied()
    }

    /// Tracers are gathered with all their zones merged, so none of them
    /// has a zone of its own.
    pub fn zone_of(&self, _index: usize) -> ZoneOwner {
        ZoneOwner::default()
    }

    /// The materials the beams name.
    pub fn material_keys(&self) -> impl Iterator<Item = MaterialKey> + '_ {
        self.rows.iter().filter_map(|def| {
            let name = def.material_name.as_ref()?;
            Some(MaterialKey {
                namespace: def.namespace,
                name: name.clone(),
            })
        })
    }

    /// Binds each beam to the material row that has since been loaded. A beam
    /// that named a material nobody has loaded stays unresolved.
    pub fn resolve_materials(&mut self, materials: &MaterialDefinitions) {
        for def in &mut self.rows {
            let found = def
                .material_name
                .as_deref()
                .and_then(|name| materials.material_index_by_ns(def.namespace, name))
                .filter(|index| {
                    materials
                        .materials
                        .get(index.order())
                        .is_some_and(|row| row.name.is_real() && !row.name.is_empty())
                });
            def.material = match found {
                Some(index) => TracerMaterial::bind(index, materials.zone_of(index.order())),
                None => AssetEdge::unresolved_presence(def.material_name.is_some(), false),
            };
        }
    }

    fn put(&mut self, def: OwnedTracerDef) {
        let key = (def.namespace, def.name.clone());
        match self.by_key.get(&key) {
            Some(&at) => self.rows[at] = def,
            None => {
                self.by_key.insert(key, self.rows.len());
                self.rows.push(def);
            }
        }
    }
}

/// The tracers still being gathered. It reads as the set it will become.
#[derive(Clone, Debug, Default)]
pub struct TracerCatalog {
    defs: TracerDefinitions,
}

impl Deref for TracerCatalog {
    type Target = TracerDefinitions;

    fn deref(&self) -> &TracerDefinitions {
        &self.defs
    }
}

impl DerefMut for TracerCatalog {
    fn deref_mut(&mut self) -> &mut TracerDefinitions {
        &mut self.defs
    }
}

impl TracerCatalog {
    /// Adds a zone's tracer. False when the zone only points at a tracer
    /// another zone defines (a name starting with a comma) or names none.
    pub fn capture_t6(&mut self, tracer: &asset_t6::TracerRef) -> bool {
        if tracer.name.is_empty() || tracer.name.starts_with(',') {
            return false;
        }
        let smoke = tracer.ty == 1;
        self.defs.put(OwnedTracerDef {
            namespace: AssetNamespace::T6,
            name: tracer.name.clone(),
            material: AssetEdge::Unresolved(AssetEdgeReason::CatalogMiss),
            material_name: Some(tracer.material.clone()).filter(|name| !name.is_empty()),
            draw_interval: u32::try_from(tracer.draw_interval).unwrap_or(0),
            speed: tracer.speed,
            beam_length: tracer.beam_length,
            beam_width: tracer.beam_width,
            screw_radius: tracer.screw_radius,
            screw_dist: tracer.screw_dist,
            colors: tracer.colors,
            fade_msec: if smoke {
                (tracer.fade_time.max(0.0) * 1000.0) as u32
            } else {
                0
            },
        });
        true
    }

    /// Takes in the tracers a map zone defines; one already here under the
    /// same name is replaced.
    pub fn absorb(&mut self, other: TracerCatalog) {
        for def in other.defs.rows {
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
        asset_t6::TracerRef {
            name: name.to_owned(),
            material: material.to_owned(),
            ..Default::default()
        }
    }

    #[test]
    fn stubs_and_unnamed_tracers_are_left_out() {
        let mut catalog = TracerCatalog::default();
        assert!(!catalog.capture_t6(&tracer("", "m")));
        assert!(!catalog.capture_t6(&tracer(",elsewhere", "m")));
        assert!(catalog.capture_t6(&tracer("bullet", "m")));
        assert_eq!(catalog.len(), 1);
    }

    #[test]
    fn a_later_definition_replaces_the_earlier_one_in_place() {
        let mut catalog = TracerCatalog::default();
        catalog.capture_t6(&tracer("a", "one"));
        catalog.capture_t6(&tracer("b", "two"));
        catalog.capture_t6(&tracer("a", "three"));
        assert_eq!(catalog.len(), 2);
        let at = catalog.index_of(AssetNamespace::T6, "a").expect("a");
        assert_eq!(at, 0);
        assert_eq!(catalog.def_at(at).and_then(|d| d.material_name.as_deref()), Some("three"));
    }

    #[test]
    fn absorbing_a_map_zone_replaces_by_name_and_appends_the_rest() {
        let mut common = TracerCatalog::default();
        common.capture_t6(&tracer("a", "one"));
        let mut map = TracerCatalog::default();
        map.capture_t6(&tracer("a", "map"));
        map.capture_t6(&tracer("c", "extra"));
        common.absorb(map);
        assert_eq!(common.len(), 2);
        let names: Vec<_> = common.material_keys().map(|key| key.name).collect();
        assert_eq!(names, ["map", "extra"]);
    }

    #[test]
    fn only_smoke_tracers_fade() {
        let mut catalog = TracerCatalog::default();
        let mut smoke = tracer("smoke", "");
        smoke.ty = 1;
        smoke.fade_time = 1.5;
        let mut flying = tracer("flying", "");
        flying.fade_time = 1.5;
        catalog.capture_t6(&smoke);
        catalog.capture_t6(&flying);
        assert_eq!(catalog.def_at(0).map(|d| d.fade_msec), Some(1500));
        assert_eq!(catalog.def_at(1).map(|d| d.fade_msec), Some(0));
    }

    #[test]
    fn a_beam_with_no_material_name_names_nothing() {
        let mut catalog = TracerCatalog::default();
        catalog.capture_t6(&tracer("plain", ""));
        assert_eq!(catalog.material_keys().count(), 0);
        assert_eq!(catalog.def_at(0).and_then(OwnedTracerDef::present_name), None);
    }
}
