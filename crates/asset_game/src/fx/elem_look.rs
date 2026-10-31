//! What one effect element draws, plays or spawns.

use asset_core::{AssetDependencyLink, DependencyCause, FxModelRange, FxRange, KeyDomain, SurfaceShaderSpace, ZoneCustodian};
use asset_material::MaterialDefinitions;
use fx_t6::runtime::{ParticleRecipeLens, Vec3Spread, StreakCorner, elem_visual_index};

use super::look_kind;
use crate::{AssetCategoryScope, AudioRoster};

/// What a sound element plays out of a sound bank.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FxBankSound<'a> {
    /// The element is not a sound, or the bank has no such sound.
    Gap,
    /// The element plays nothing.
    Silent,
    Play {
        namespace: AssetCategoryScope,
        alias: &'a str,
    },
}

/// One material an element draws with: the name the zone gave it, and the
/// catalog entry that name was matched to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterialSlot {
    pub edge: AssetDependencyLink<SurfaceShaderSpace>,
    pub hint: Option<String>,
}

impl MaterialSlot {
    /// A slot with a name, not yet looked up; no name leaves it empty.
    fn named(caption: &str) -> Self {
        let (edge, hint) = pending::<SurfaceShaderSpace>(caption);
        MaterialSlot { edge, hint }
    }

    /// The material's name, once it is matched.
    fn bound_name(&self) -> Option<&str> {
        let label = self.hint.as_deref()?;
        (self.edge.is_attached() && !label.is_empty()).then_some(label)
    }

    /// Looks the slot's name up among the materials. A name that is not found
    /// leaves the slot open; a slot that never had a name stays absent.
    fn settle(&mut self, mat_cat: &MaterialDefinitions, space: AssetCategoryScope) {
        let real = |pos: &asset_material::SurfaceMaterialId| {
            mat_cat.materials.get(pos.order()).is_some_and(|def| !def.name.is_empty() && def.name.is_genuine())
        };
        let found = self
            .hint
            .as_deref()
            .and_then(|label| mat_cat.material_index_by_ns(space, label))
            .filter(real);
        self.edge = match found {
            None => AssetDependencyLink::unresolved_presence(self.hint.is_some(), false),
            Some(pos) => AssetDependencyLink::bind(pos, mat_cat.owning_zone(pos.order())),
        };
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SelfContainedEffectLook {
    /// The material of a sprite, tail, trail or cloud.
    Material(MaterialSlot),
    /// The two materials of a decal.
    Mark([MaterialSlot; 2]),
    Mesh {
        edge: AssetDependencyLink<FxModelRange>,
        hint: String,
    },
    /// Plays another effect.
    ScenarioDriver {
        edge: AssetDependencyLink<FxRange>,
        driver_hint: Option<String>,
    },
    Sound {
        hint: String,
    },
    /// Nothing to draw or play: a light, or a visual the zone left unnamed.
    None,
}

impl SelfContainedEffectLook {
    /// Wraps a named thing as `build` says, or `None` for an unnamed one.
    fn when_named(caption: &str, build: impl FnOnce() -> Self) -> Self {
        if caption.is_empty() { Self::None } else { build() }
    }

    pub(super) fn material(caption: &str) -> Self {
        Self::when_named(caption, || Self::Material(MaterialSlot::named(caption)))
    }

    pub(super) fn mark(first: &str, second: &str) -> Self {
        Self::Mark([first, second].map(MaterialSlot::named))
    }

    pub(super) fn mdl(caption: &str) -> Self {
        let pending = AssetDependencyLink::Unresolved(DependencyCause::IndexLookupFailure);
        Self::when_named(caption, || Self::Mesh { edge: pending, hint: caption.into() })
    }

    pub(super) fn runner(caption: &str) -> Self {
        let (edge, hint) = pending(caption);
        Self::ScenarioDriver { edge, driver_hint: hint }
    }

    pub(super) fn sound(caption: &str) -> Self {
        Self::when_named(caption, || Self::Sound { hint: caption.to_string() })
    }

    /// The catalog order of the material, for a material visual that is matched.
    pub fn extent_position(&self) -> Option<usize> {
        let Self::Material(slot) = self else {
            return None;
        };
        slot.edge.extent_position()
    }

    /// The edge of a model visual; an unnamed visual stands for no model at all.
    fn as_model_edge(&self) -> Option<AssetDependencyLink<FxModelRange>> {
        match self {
            Self::Mesh { edge: link, .. } => Some(*link),
            Self::None => Some(AssetDependencyLink::NothingThere),
            _ => None,
        }
    }

    /// One of a decal's materials.
    fn mark_slot(&self, which: usize) -> Option<&MaterialSlot> {
        match self {
            Self::Mark(pair) => pair.get(which),
            _ => None,
        }
    }

    /// The name of one of a decal's materials, once it is matched.
    pub fn decal_bind_name(&self, mat_slot: usize) -> Option<&str> {
        self.mark_slot(mat_slot).and_then(MaterialSlot::bound_name)
    }

    /// How one of a decal's materials stands, for the report.
    pub fn decal_edge_kind(&self, mat_slot: usize) -> Option<&'static str> {
        self.mark_slot(mat_slot).map(|slot| slot.edge.dependency_cause_of())
    }

    pub(super) fn resolve_materials(&mut self, mat_cat: &MaterialDefinitions, space: AssetCategoryScope) {
        let slots: &mut [MaterialSlot] = match self {
            Self::Material(single) => std::slice::from_mut(single),
            Self::Mark(pair) => pair,
            _ => return,
        };
        slots.iter_mut().for_each(|slot| slot.settle(mat_cat, space));
    }

    /// The material slots a visual draws with.
    pub(super) fn slots(&self) -> &[MaterialSlot] {
        match self {
            Self::Material(single) => std::slice::from_ref(single),
            Self::Mark(pair) => pair,
            _ => &[],
        }
    }
}

/// An edge and name for something authored by name and not yet looked up.
pub(super) fn pending<S: KeyDomain>(label: &str) -> (AssetDependencyLink<S>, Option<String>) {
    let miss = (AssetDependencyLink::Unresolved(DependencyCause::IndexLookupFailure), Some(label.to_owned()));
    if label.is_empty() { (AssetDependencyLink::NothingThere, None) } else { miss }
}

/// The edge to an effect found by name; `find` gives its catalog order.
pub(super) fn bind_child(caption: Option<&str>, find: impl Fn(&str) -> Option<usize>) -> AssetDependencyLink<FxRange> {
    let Some(name) = caption.filter(|text| !text.is_empty()) else {
        return AssetDependencyLink::NothingThere;
    };
    find(name).map_or(AssetDependencyLink::Unresolved(DependencyCause::IndexLookupFailure), |order| {
        AssetDependencyLink::bind_order(order, ZoneCustodian::default())
    })
}

/// The ribbon a trail element is drawn as.
#[derive(Clone, Debug, PartialEq)]
pub struct SelfContainedTrailDef {
    pub scroll_period_millis: i32,
    pub recurrence_distance: i32,
    pub reciprocal_split_distance: f32,
    pub reciprocal_split_arc_length: f32,
    pub reciprocal_split_duration: f32,
    pub vertex_points: Vec<StreakCorner>,
    pub index_run_values: Vec<u16>,
}

#[derive(Clone, Debug)]
pub struct SelfContainedEffectElement {
    pub view: ParticleRecipeLens,
    /// The atlas bytes, the entry count already widened to 16 bits.
    pub atlas: [u8; 8],
    pub speed_readings: Vec<u8>,
    pub vel_graph_local: Vec<Vec3Spread>,
    pub vel_graph_world: Vec<Vec3Spread>,
    pub visibility_readings: Vec<u8>,
    pub visuals: Vec<SelfContainedEffectLook>,

    /// The effects spawned where a particle hits something, dies, or is
    /// emitted; the names they were authored under sit beside them.
    pub effect_upon_strike: AssetDependencyLink<FxRange>,
    pub effect_upon_demise: AssetDependencyLink<FxRange>,
    pub effect_spawned: AssetDependencyLink<FxRange>,
    pub on_impact_name: Option<String>,
    pub on_death_name: Option<String>,
    pub emitted_name: Option<String>,

    pub trail_kind: Option<SelfContainedTrailDef>,
    /// A particle cloud's particle count: the base, and the random extra.
    pub cloud_density: Option<[i32; 2]>,
}

impl SelfContainedEffectElement {
    /// The visual a particle of the given element type shows. Each particle
    /// picks one of the element's visuals by its random seed; an element of
    /// another type shows none.
    fn shown_as(&self, kind: u8, roll: u32) -> Option<&SelfContainedEffectLook> {
        if self.view.shape_kind != kind {
            return None;
        }
        self.visuals.get(elem_visual_index(self.view.look_variant_total, roll))
    }

    pub fn runner_kid_link(&self, roll: u32) -> Option<AssetDependencyLink<FxRange>> {
        match self.shown_as(look_kind::SPRINTER, roll) {
            Some(SelfContainedEffectLook::ScenarioDriver { edge, .. }) => Some(*edge),
            _ => None,
        }
    }

    pub fn sound_in_bank<'a>(&self, roll: u32, bank: &'a AudioRoster) -> FxBankSound<'a> {
        let label = match self.shown_as(look_kind::AUDIO_KIND, roll) {
            Some(SelfContainedEffectLook::Sound { hint }) => hint,
            Some(SelfContainedEffectLook::None) => return FxBankSound::Silent,
            Some(_) | None => return FxBankSound::Gap,
        };
        let pos = bank.slot_in(AssetCategoryScope::T6, label).or_else(|| bank.slot_unique(label));
        match pos.and_then(|pos| bank.label_at(pos).map(|alias| (pos, alias))) {
            None => FxBankSound::Gap,
            Some((pos, alias)) => FxBankSound::Play { namespace: bank.space_of_slot(pos), alias },
        }
    }

    pub fn model_link(&self, roll: u32) -> Option<AssetDependencyLink<FxModelRange>> {
        self.shown_as(look_kind::MESH_KIND, roll)?.as_model_edge()
    }

    /// The decal's visual, for the report of which materials it drew with.
    pub fn decal_pair_look(&self, roll: u32) -> Option<&SelfContainedEffectLook> {
        self.shown_as(look_kind::SURFACE_STICKER, roll)
    }

    /// The first material the element draws with that is matched, and its name.
    pub fn main_surface(&self) -> Option<(usize, &str)> {
        let mut drawn = self.visuals.iter().filter_map(|piece| match piece {
            SelfContainedEffectLook::Material(slot) => Some(slot),
            _ => None,
        });
        drawn.find_map(|slot| slot.edge.extent_position().zip(slot.bound_name()))
    }

    /// Binds the effects this element spawns; `find` gives an effect's
    /// catalog order by name.
    pub(super) fn bind_children(&mut self, find: impl Fn(&str) -> Option<usize>) {
        let bind = |name: &Option<String>| bind_child(name.as_deref(), &find);
        self.effect_upon_strike = bind(&self.on_impact_name);
        self.effect_upon_demise = bind(&self.on_death_name);
        self.effect_spawned = bind(&self.emitted_name);
        self.visuals
            .iter_mut()
            .filter_map(|piece| match piece {
                SelfContainedEffectLook::ScenarioDriver { edge, driver_hint: hint } => Some((edge, &*hint)),
                _ => None,
            })
            .for_each(|(edge, hint)| *edge = bind(hint));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unnamed_thing_is_absent_and_a_named_one_waits_for_the_catalog() {
        let (edge, hint) = pending::<FxRange>("");
        assert!(edge.lacks_entry() && hint.is_none());
        let (edge, hint) = pending::<SurfaceShaderSpace>("mtl_glow");
        assert_eq!(edge, AssetDependencyLink::Unresolved(DependencyCause::IndexLookupFailure));
        assert_eq!(hint.as_deref(), Some("mtl_glow"));
    }

    #[test]
    fn a_runner_is_kept_even_without_a_name() {
        assert!(matches!(SelfContainedEffectLook::runner(""), SelfContainedEffectLook::ScenarioDriver { driver_hint: None, .. }));
        let SelfContainedEffectLook::ScenarioDriver { edge, driver_hint: hint } = SelfContainedEffectLook::runner("child") else { panic!("not a runner") };
        assert!(edge.is_unlinked_ref());
        assert_eq!(hint.as_deref(), Some("child"));
    }

    #[test]
    fn only_model_and_unnamed_visuals_give_a_model_edge() {
        let model = SelfContainedEffectLook::mdl("xm_brass");
        assert_eq!(model.as_model_edge(), Some(AssetDependencyLink::Unresolved(DependencyCause::IndexLookupFailure)));
        assert_eq!(SelfContainedEffectLook::mdl("").as_model_edge(), Some(AssetDependencyLink::NothingThere));
        assert_eq!(SelfContainedEffectLook::material("mtl").as_model_edge(), None);
        assert_eq!(SelfContainedEffectLook::sound("boom").as_model_edge(), None);
    }

    #[test]
    fn a_material_slot_names_itself_only_once_matched() {
        let mut slot = MaterialSlot::named("mtl_spark");
        assert!(slot.bound_name().is_none());
        slot.edge = AssetDependencyLink::bind_order(2, ZoneCustodian::default());
        assert_eq!(slot.bound_name(), Some("mtl_spark"));
        assert!(MaterialSlot::named("").edge.lacks_entry());
    }
}
