//! The effects a build knows, found by name, and the passes that tie their
//! materials, models and child effects to the catalogs they name.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use asset_core::{AssetEdge, AssetEdgeReason, FxSpace, ZoneOwner};
use asset_material::MaterialDefinitions;
use fx_t6::runtime::FxEffectDefView;

use super::visual::{MaterialSlot, OwnedFxElemDef, OwnedFxVisual};
use super::FxName;
use crate::{AssetNamespace, AssetRef, FxModelCatalog};

#[derive(Clone, Debug)]
pub struct OwnedFxEffectDef {
    pub namespace: AssetNamespace,
    pub name: String,
    pub view: FxEffectDefView,
    pub elems: Vec<OwnedFxElemDef>,
}

/// The effects a build finished with, found by namespace and name (case does
/// not matter).
#[derive(Clone, Debug, Default)]
pub struct FxDefinitions {
    by_key: HashMap<(AssetNamespace, String), usize>,
    defs: Vec<OwnedFxEffectDef>,
    map_namespace: AssetNamespace,
}

/// The effects being collected. It reads as the finished set; `publish` hands
/// that set on.
#[derive(Clone, Debug, Default)]
pub struct FxCatalog {
    published: FxDefinitions,
}

impl std::ops::Deref for FxCatalog {
    type Target = FxDefinitions;

    fn deref(&self) -> &Self::Target {
        &self.published
    }
}

impl std::ops::DerefMut for FxCatalog {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.published
    }
}

impl FxCatalog {
    /// The edge to the effect a weapon row names. A row that has the slot but
    /// no name, or a name nobody defines, is an open edge; a row with neither
    /// has no edge at all.
    pub fn hint_edge(&self, authored_slot: bool, hint: Option<&str>, namespace: AssetNamespace) -> AssetEdge<FxSpace> {
        let hint = hint.filter(|name| !name.is_empty());
        if hint.is_none() && !authored_slot {
            return AssetEdge::Absent;
        }
        match hint.and_then(|name| self.index_in(namespace, name)) {
            Some(index) => AssetEdge::bind_order(index, ZoneOwner::default()),
            None => AssetEdge::Unresolved(AssetEdgeReason::CatalogMiss),
        }
    }

    pub fn publish(self) -> FxDefinitions {
        self.published
    }

    /// Takes every effect of another catalog; one of the same name is replaced.
    pub fn absorb(&mut self, other: FxCatalog) {
        for def in other.published.defs {
            self.insert_owned(def);
        }
    }
}

impl FxDefinitions {
    pub fn len(&self) -> usize {
        self.defs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }

    pub fn index_in(&self, namespace: AssetNamespace, name: &str) -> Option<usize> {
        self.by_key.get(&(namespace, name.to_ascii_lowercase())).copied()
    }

    pub fn get_in(&self, namespace: AssetNamespace, name: &str) -> Option<&OwnedFxEffectDef> {
        self.defs.get(self.index_in(namespace, name)?)
    }

    /// The catalog order of an effect that sits in this catalog.
    pub fn index_of(&self, def: &OwnedFxEffectDef) -> Option<usize> {
        self.index_in(def.namespace, &def.name)
            .filter(|&index| std::ptr::eq(&self.defs[index], def))
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.defs.iter().map(|def| def.name.as_str())
    }

    pub fn def_at(&self, index: usize) -> Option<&OwnedFxEffectDef> {
        self.defs.get(index)
    }

    pub fn effects(&self) -> impl Iterator<Item = &OwnedFxEffectDef> {
        self.defs.iter()
    }

    /// Adds an effect, or replaces the one of the same name where it sits.
    pub fn insert_owned(&mut self, def: OwnedFxEffectDef) {
        let key = (def.namespace, def.name.to_ascii_lowercase());
        match self.by_key.get(&key) {
            Some(&index) => self.defs[index] = def,
            None => {
                self.by_key.insert(key, self.defs.len());
                self.defs.push(def);
            }
        }
    }

    /// The namespace a map's own effects live in.
    pub fn set_map_namespace(&mut self, namespace: AssetNamespace) {
        self.map_namespace = namespace;
    }

    pub fn map_namespace(&self) -> AssetNamespace {
        self.map_namespace
    }

    /// An effect a map's script names by the id its effect list gave it.
    pub fn resolve_createfx_id(&self, fxid: &str) -> Option<&OwnedFxEffectDef> {
        self.resolve_def_for_map(fxid)
            .or_else(|| self.get_in(self.map_namespace, fxid))
    }

    /// The name of a map's effect together with the namespace that defines it.
    pub fn map_fx_name<'a>(&self, name: &'a str) -> FxName<'a> {
        let namespace = self
            .resolve_def_for_map(name)
            .map_or(self.map_namespace, |def| def.namespace);
        FxName::new(namespace, name)
    }

    /// The effect of that name that has parts to play. A name that carries a
    /// reference mark is looked up again without it.
    pub fn resolve_def_in(&self, namespace: AssetNamespace, name: &str) -> Option<&OwnedFxEffectDef> {
        let with_parts = |name: &str| self.get_in(namespace, name).filter(|def| !def.elems.is_empty());
        with_parts(name).or_else(|| {
            let bare = AssetRef::bare_name(name);
            (bare != name).then(|| with_parts(bare)).flatten()
        })
    }

    /// The same for a map: its own namespace first, then the base game's.
    pub fn resolve_def_for_map(&self, name: &str) -> Option<&OwnedFxEffectDef> {
        self.resolve_def_in(self.map_namespace, name).or_else(|| {
            (self.map_namespace != AssetNamespace::T6)
                .then(|| self.resolve_def_in(AssetNamespace::T6, name))
                .flatten()
        })
    }

    /// Ties every material an effect draws with to the material catalog.
    pub fn resolve_materials(&mut self, materials: &MaterialDefinitions) {
        for effect in &mut self.defs {
            let namespace = effect.namespace;
            for visual in effect.elems.iter_mut().flat_map(|elem| elem.visuals.iter_mut()) {
                visual.resolve_materials(materials, namespace);
            }
        }
    }

    /// Ties the effects an effect spawns or runs to the effects here. Only an
    /// effect that has parts can be one.
    pub fn resolve_nested_edges(&mut self) {
        let Self { by_key, defs, .. } = self;
        let has_parts: Vec<bool> = defs.iter().map(|def| !def.elems.is_empty()).collect();
        for effect in defs.iter_mut() {
            let namespace = effect.namespace;
            let find = |name: &str| {
                let lookup = |key: String| {
                    by_key
                        .get(&(namespace, key))
                        .copied()
                        .filter(|&index| has_parts[index])
                };
                lookup(name.to_ascii_lowercase())
                    .or_else(|| lookup(AssetRef::bare_name(name).to_ascii_lowercase()))
            };
            for elem in &mut effect.elems {
                elem.bind_children(&find);
            }
        }
    }

    /// Ties every model an effect shows to the effect-model catalog.
    pub fn resolve_model_edges(&mut self, models: &FxModelCatalog) {
        for effect in &mut self.defs {
            let namespace = effect.namespace;
            for visual in effect.elems.iter_mut().flat_map(|elem| elem.visuals.iter_mut()) {
                if let OwnedFxVisual::Model { edge, hint } = visual {
                    *edge = match models.index_in(namespace, hint) {
                        Some(index) => AssetEdge::bind_order(index, models.zone_of(index)),
                        None => AssetEdge::Unresolved(AssetEdgeReason::CatalogMiss),
                    };
                }
            }
        }
    }

    /// The models the effects show, each by the namespace it was authored in
    /// and its name.
    pub fn model_hints(&self) -> HashSet<(AssetNamespace, String)> {
        let mut hints = HashSet::new();
        for effect in &self.defs {
            for visual in effect.elems.iter().flat_map(|elem| &elem.visuals) {
                if let OwnedFxVisual::Model { hint, .. } = visual {
                    hints.insert((effect.namespace, hint.clone()));
                }
            }
        }
        hints
    }

    fn visuals(&self) -> impl Iterator<Item = &OwnedFxVisual> {
        self.defs
            .iter()
            .flat_map(|effect| &effect.elems)
            .flat_map(|elem| &elem.visuals)
    }

    /// Each material the effects draw with that is matched, by catalog order,
    /// with the name it was authored under.
    pub fn unique_bound_hints(&self) -> Vec<(usize, &str)> {
        let drawn = self
            .visuals()
            .filter(|visual| matches!(visual, OwnedFxVisual::Material(_)));
        unique_bound(drawn.flat_map(|visual| visual.slots()))
    }

    pub fn material_visual_unique_bound_count(&self) -> usize {
        self.unique_bound_hints().len()
    }

    /// The same for the materials of decals.
    pub fn unique_decal_mark_hints(&self) -> Vec<(usize, &str)> {
        let marks = self
            .visuals()
            .filter(|visual| matches!(visual, OwnedFxVisual::Mark(_)));
        unique_bound(marks.flat_map(|visual| visual.slots()))
    }

    /// The names of the sounds the effects play, each once.
    pub fn unique_type10_sound_hints(&self) -> Vec<&str> {
        let names: BTreeSet<&str> = self
            .visuals()
            .filter_map(|visual| match visual {
                OwnedFxVisual::Sound { hint } => Some(hint.as_str()),
                _ => None,
            })
            .collect();
        names.into_iter().collect()
    }
}

/// Each matched slot once, by catalog order; the name is the first one seen,
/// or "-" for a slot that has none.
fn unique_bound<'a>(slots: impl Iterator<Item = &'a MaterialSlot>) -> Vec<(usize, &'a str)> {
    let mut seen = BTreeMap::new();
    for slot in slots {
        if let Some(index) = slot.edge.bound_index() {
            seen.entry(index)
                .or_insert(slot.hint.as_deref().filter(|name| !name.is_empty()).unwrap_or("-"));
        }
    }
    seen.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fx::elem_type;
    use crate::fx::visual::pending_child;
    use fx_t6::runtime::FxElemDefView;

    const T6: AssetNamespace = AssetNamespace::T6;

    fn view(elem_type: u8) -> FxElemDefView {
        FxElemDefView {
            flags: 0,
            spawn_a: 0,
            spawn_b: 0,
            spawn_range_base: 0.0,
            spawn_range_amplitude: 0.0,
            fade_in_range: [0.0; 2],
            fade_out_range: [0.0; 2],
            spawn_frustum_cull_radius: 0.0,
            spawn_delay_msec_base: 0,
            spawn_delay_msec_amplitude: 0,
            life_span_msec_base: 0,
            life_span_msec_amplitude: 0,
            spawn_origin: [[0.0; 2]; 3],
            spawn_offset_radius_base: 0.0,
            spawn_offset_radius_amplitude: 0.0,
            spawn_offset_height_base: 0.0,
            spawn_offset_height_amplitude: 0.0,
            spawn_angles: [[0.0; 2]; 3],
            angular_velocity: [[0.0; 2]; 3],
            initial_rotation: [0.0; 2],
            gravity_base: 0.0,
            gravity_amplitude: 0.0,
            reflection_factor: [0.0; 2],
            coll_mins: [0.0; 3],
            coll_maxs: [0.0; 3],
            elem_type,
            visual_count: 1,
            vel_interval_count: 0,
            vis_state_interval_count: 0,
            lighting_frac: 0,
            use_item_clip: 0,
            line: false,
            sort_order: 0,
            emit_dist: [0.0; 2],
            emit_dist_variance: [0.0; 2],
        }
    }

    fn elem(on_death: &str, visuals: Vec<OwnedFxVisual>) -> OwnedFxElemDef {
        let (effect_on_death, on_death_name) = pending_child(on_death);
        OwnedFxElemDef {
            view: view(elem_type::BILLBOARD),
            atlas: [0; 8],
            vel_samples: Vec::new(),
            vel_graph_local: Vec::new(),
            vel_graph_world: Vec::new(),
            vis_samples: Vec::new(),
            visuals,
            effect_on_impact: AssetEdge::Absent,
            effect_on_death,
            effect_emitted: AssetEdge::Absent,
            on_impact_name: None,
            on_death_name,
            emitted_name: None,
            trail_def: None,
            cloud_density: None,
        }
    }

    fn effect(name: &str, elems: Vec<OwnedFxElemDef>) -> OwnedFxEffectDef {
        OwnedFxEffectDef {
            namespace: T6,
            name: name.to_owned(),
            view: FxEffectDefView {
                flags: 0,
                msec_looping_life: 0,
                looping_count: 0,
                one_shot_count: 0,
                emission_count: 0,
            },
            elems,
        }
    }

    fn catalog(effects: Vec<OwnedFxEffectDef>) -> FxCatalog {
        let mut catalog = FxCatalog::default();
        for def in effects {
            catalog.insert_owned(def);
        }
        catalog
    }

    #[test]
    fn effects_are_found_whatever_the_case() {
        let fx = catalog(vec![effect("Impacts/Dirt", vec![elem("", vec![])])]);
        assert_eq!(fx.index_in(T6, "impacts/dirt"), Some(0));
        assert_eq!(fx.index_in(T6, "IMPACTS/DIRT"), Some(0));
        assert_eq!(fx.index_in(AssetNamespace::T5, "impacts/dirt"), None);
    }

    #[test]
    fn a_second_effect_of_the_same_name_replaces_the_first_in_place() {
        let mut fx = catalog(vec![effect("a", vec![]), effect("b", vec![])]);
        fx.insert_owned(effect("A", vec![elem("", vec![])]));
        assert_eq!(fx.len(), 2);
        assert_eq!(fx.def_at(0).unwrap().elems.len(), 1);
    }

    #[test]
    fn a_name_with_the_reference_mark_finds_the_effect_without_it() {
        let fx = catalog(vec![effect("blast", vec![elem("", vec![])])]);
        assert!(fx.resolve_def_in(T6, ",blast").is_some());
        assert!(fx.resolve_def_in(T6, "missing").is_none());
    }

    #[test]
    fn an_effect_with_no_parts_cannot_be_played() {
        let fx = catalog(vec![effect("empty", vec![])]);
        assert!(fx.get_in(T6, "empty").is_some());
        assert!(fx.resolve_def_in(T6, "empty").is_none());
    }

    #[test]
    fn a_spawned_effect_binds_to_an_effect_that_has_parts() {
        let mut fx = catalog(vec![
            effect("parent", vec![elem("Child", vec![]), elem("hollow", vec![]), elem("", vec![])]),
            effect("child", vec![elem("", vec![])]),
            effect("hollow", vec![]),
        ]);
        fx.resolve_nested_edges();
        let parent = fx.get_in(T6, "parent").unwrap();
        assert_eq!(parent.elems[0].effect_on_death.bound_index(), Some(1));
        assert_eq!(
            parent.elems[1].effect_on_death,
            AssetEdge::Unresolved(AssetEdgeReason::CatalogMiss)
        );
        assert!(parent.elems[2].effect_on_death.is_absent());
    }

    #[test]
    fn a_weapon_row_edge_is_absent_only_when_it_has_neither_slot_nor_name() {
        let fx = catalog(vec![effect("flash", vec![])]);
        assert!(fx.hint_edge(false, None, T6).is_absent());
        assert!(fx.hint_edge(false, Some(""), T6).is_absent());
        assert!(fx.hint_edge(true, None, T6).is_unresolved());
        assert!(fx.hint_edge(false, Some("nothing"), T6).is_unresolved());
        assert_eq!(fx.hint_edge(false, Some("Flash"), T6).bound_index(), Some(0));
    }

    #[test]
    fn a_map_looks_in_its_own_namespace_before_the_base_games() {
        let mut fx = catalog(vec![effect("steam", vec![elem("", vec![])])]);
        fx.set_map_namespace(AssetNamespace::T5);
        assert!(fx.resolve_createfx_id("steam").is_some());
        assert_eq!(fx.map_fx_name("steam").namespace, T6);
        assert_eq!(fx.map_fx_name("unknown").namespace, AssetNamespace::T5);
    }

    #[test]
    fn material_and_sound_names_are_listed_once() {
        let sound = |name: &str| elem("", vec![OwnedFxVisual::sound(name)]);
        let fx = catalog(vec![
            effect("a", vec![sound("boom"), sound("crack")]),
            effect("b", vec![sound("boom")]),
        ]);
        assert_eq!(fx.unique_type10_sound_hints(), ["boom", "crack"]);
        assert!(fx.unique_bound_hints().is_empty());
    }
}
