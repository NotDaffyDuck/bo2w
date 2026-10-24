//! What one effect element draws, plays or spawns.

use asset_core::{AssetEdge, AssetEdgeReason, FxModelSpace, FxSpace, MaterialSpace, ZoneOwner};
use asset_material::MaterialDefinitions;
use fx_t6::runtime::{FxElemDefView, FxElemVec3Range, FxTrailVertex, elem_visual_index};

use super::elem_type;
use crate::{AssetNamespace, SoundCatalog};

/// What a sound element plays out of a sound bank.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FxBankSound<'a> {
    /// The element is not a sound, or the bank has no such sound.
    Gap,
    /// The element plays nothing.
    Silent,
    Play {
        namespace: AssetNamespace,
        alias: &'a str,
    },
}

/// One material an element draws with: the name the zone gave it, and the
/// catalog entry that name was matched to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterialSlot {
    pub edge: AssetEdge<MaterialSpace>,
    pub hint: Option<String>,
}

impl MaterialSlot {
    /// A slot the zone left empty.
    fn empty() -> Self {
        Self {
            edge: AssetEdge::Absent,
            hint: None,
        }
    }

    /// A slot with a name, not yet looked up.
    fn named(name: &str) -> Self {
        if name.is_empty() {
            return Self::empty();
        }
        Self {
            edge: AssetEdge::Unresolved(AssetEdgeReason::CatalogMiss),
            hint: Some(name.to_owned()),
        }
    }

    /// The material's name, once it is matched.
    fn bound_name(&self) -> Option<&str> {
        if !self.edge.is_bound() {
            return None;
        }
        self.hint.as_deref().filter(|name| !name.is_empty())
    }

    /// Looks the slot's name up among the materials. A name that is not found
    /// leaves the slot open; a slot that never had a name stays absent.
    fn resolve(&mut self, materials: &MaterialDefinitions, namespace: AssetNamespace) {
        let found = self
            .hint
            .as_deref()
            .and_then(|name| materials.material_index_by_ns(namespace, name))
            .filter(|index| {
                materials
                    .materials
                    .get(index.order())
                    .is_some_and(|material| material.name.is_real() && !material.name.is_empty())
            });
        self.edge = match found {
            Some(index) => AssetEdge::bind(index, materials.zone_of(index.order())),
            None => AssetEdge::unresolved_presence(self.hint.is_some(), false),
        };
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OwnedFxVisual {
    /// The material of a sprite, tail, trail or cloud.
    Material(MaterialSlot),
    /// The two materials of a decal.
    Mark([MaterialSlot; 2]),
    Model {
        edge: AssetEdge<FxModelSpace>,
        hint: String,
    },
    /// Plays another effect.
    Runner {
        edge: AssetEdge<FxSpace>,
        hint: Option<String>,
    },
    Sound {
        hint: String,
    },
    /// Nothing to draw or play: a light, or a visual the zone left unnamed.
    None,
}

impl OwnedFxVisual {
    pub(super) fn material(name: &str) -> Self {
        if name.is_empty() {
            Self::None
        } else {
            Self::Material(MaterialSlot::named(name))
        }
    }

    pub(super) fn mark(first: &str, second: &str) -> Self {
        Self::Mark([MaterialSlot::named(first), MaterialSlot::named(second)])
    }

    pub(super) fn model(name: &str) -> Self {
        if name.is_empty() {
            return Self::None;
        }
        Self::Model {
            edge: AssetEdge::Unresolved(AssetEdgeReason::CatalogMiss),
            hint: name.to_owned(),
        }
    }

    pub(super) fn runner(name: &str) -> Self {
        let (edge, hint) = pending_child(name);
        Self::Runner { edge, hint }
    }

    pub(super) fn sound(name: &str) -> Self {
        if name.is_empty() {
            Self::None
        } else {
            Self::Sound { hint: name.to_owned() }
        }
    }

    /// The catalog order of the material, for a material visual that is matched.
    pub fn bound_index(&self) -> Option<usize> {
        match self {
            Self::Material(slot) => slot.edge.bound_index(),
            _ => None,
        }
    }

    /// The name of one of a decal's materials, once it is matched.
    pub fn mark_bound_name(&self, slot: usize) -> Option<&str> {
        match self {
            Self::Mark(slots) => slots.get(slot)?.bound_name(),
            _ => None,
        }
    }

    /// How one of a decal's materials stands, for the report.
    pub fn mark_edge_kind(&self, slot: usize) -> Option<&'static str> {
        match self {
            Self::Mark(slots) => Some(slots.get(slot)?.edge.edge_kind()),
            _ => None,
        }
    }

    pub(super) fn resolve_materials(&mut self, materials: &MaterialDefinitions, namespace: AssetNamespace) {
        match self {
            Self::Material(slot) => slot.resolve(materials, namespace),
            Self::Mark(slots) => {
                for slot in slots {
                    slot.resolve(materials, namespace);
                }
            }
            _ => {}
        }
    }

    /// The material slots a visual draws with.
    pub(super) fn slots(&self) -> &[MaterialSlot] {
        match self {
            Self::Material(slot) => std::slice::from_ref(slot),
            Self::Mark(slots) => slots,
            _ => &[],
        }
    }
}

/// An effect that another effect names, before the catalog has been searched.
pub(super) fn pending_child(name: &str) -> (AssetEdge<FxSpace>, Option<String>) {
    if name.is_empty() {
        (AssetEdge::Absent, None)
    } else {
        (
            AssetEdge::Unresolved(AssetEdgeReason::CatalogMiss),
            Some(name.to_owned()),
        )
    }
}

/// The edge to an effect found by name; `find` gives its catalog order.
pub(super) fn bind_child(name: Option<&str>, find: impl Fn(&str) -> Option<usize>) -> AssetEdge<FxSpace> {
    let Some(name) = name.filter(|name| !name.is_empty()) else {
        return AssetEdge::Absent;
    };
    match find(name) {
        Some(order) => AssetEdge::bind_order(order, ZoneOwner::default()),
        None => AssetEdge::Unresolved(AssetEdgeReason::CatalogMiss),
    }
}

/// The ribbon a trail element is drawn as.
#[derive(Clone, Debug, PartialEq)]
pub struct OwnedFxTrailDef {
    pub scroll_time_msec: i32,
    pub repeat_dist: i32,
    pub inv_split_dist: f32,
    pub inv_split_arc_dist: f32,
    pub inv_split_time: f32,
    pub verts: Vec<FxTrailVertex>,
    pub inds: Vec<u16>,
}

#[derive(Clone, Debug)]
pub struct OwnedFxElemDef {
    pub view: FxElemDefView,
    /// The atlas bytes, the entry count already widened to 16 bits.
    pub atlas: [u8; 8],
    pub vel_samples: Vec<u8>,
    pub vel_graph_local: Vec<FxElemVec3Range>,
    pub vel_graph_world: Vec<FxElemVec3Range>,
    pub vis_samples: Vec<u8>,
    pub visuals: Vec<OwnedFxVisual>,

    /// The effects spawned where a particle hits something, dies, or is
    /// emitted; the names they were authored under sit beside them.
    pub effect_on_impact: AssetEdge<FxSpace>,
    pub effect_on_death: AssetEdge<FxSpace>,
    pub effect_emitted: AssetEdge<FxSpace>,
    pub on_impact_name: Option<String>,
    pub on_death_name: Option<String>,
    pub emitted_name: Option<String>,

    pub trail_def: Option<OwnedFxTrailDef>,
    /// A particle cloud's particle count: the base, and the random extra.
    pub cloud_density: Option<[i32; 2]>,
}

impl OwnedFxElemDef {
    /// The visual this particle shows; each particle picks one of the
    /// element's visuals by its random seed.
    fn picked(&self, random_seed: u32) -> Option<&OwnedFxVisual> {
        self.visuals
            .get(elem_visual_index(self.view.visual_count, random_seed))
    }

    pub fn runner_child_edge(&self, random_seed: u32) -> Option<AssetEdge<FxSpace>> {
        if self.view.elem_type != elem_type::RUNNER {
            return None;
        }
        match self.picked(random_seed)? {
            OwnedFxVisual::Runner { edge, .. } => Some(*edge),
            _ => None,
        }
    }

    pub fn sound_in_bank<'a>(&self, random_seed: u32, sounds: &'a SoundCatalog) -> FxBankSound<'a> {
        if self.view.elem_type != elem_type::SOUND {
            return FxBankSound::Gap;
        }
        let name = match self.picked(random_seed) {
            Some(OwnedFxVisual::Sound { hint }) => hint,
            Some(OwnedFxVisual::None) => return FxBankSound::Silent,
            _ => return FxBankSound::Gap,
        };
        let found = sounds
            .index_in(AssetNamespace::T6, name)
            .or_else(|| sounds.index_unique(name))
            .and_then(|index| Some((index, sounds.name_at(index)?)));
        match found {
            Some((index, alias)) => FxBankSound::Play {
                namespace: sounds.namespace_of_alias(index),
                alias,
            },
            None => FxBankSound::Gap,
        }
    }

    pub fn model_edge(&self, random_seed: u32) -> Option<AssetEdge<FxModelSpace>> {
        if self.view.elem_type != elem_type::MODEL {
            return None;
        }
        match self.picked(random_seed)? {
            OwnedFxVisual::Model { edge, .. } => Some(*edge),
            OwnedFxVisual::None => Some(AssetEdge::Absent),
            _ => None,
        }
    }

    /// The decal's visual, for the report of which materials it drew with.
    pub fn decal_mark_pair(&self, random_seed: u32) -> Option<&OwnedFxVisual> {
        if self.view.elem_type != elem_type::DECAL {
            return None;
        }
        self.picked(random_seed)
    }

    /// The first material the element draws with that is matched, and its name.
    pub fn primary_material(&self) -> Option<(usize, &str)> {
        self.visuals.iter().find_map(|visual| match visual {
            OwnedFxVisual::Material(slot) => Some((slot.edge.bound_index()?, slot.bound_name()?)),
            _ => None,
        })
    }

    /// Binds the effects this element spawns; `find` gives an effect's
    /// catalog order by name.
    pub(super) fn bind_children(&mut self, find: impl Fn(&str) -> Option<usize>) {
        self.effect_on_impact = bind_child(self.on_impact_name.as_deref(), &find);
        self.effect_on_death = bind_child(self.on_death_name.as_deref(), &find);
        self.effect_emitted = bind_child(self.emitted_name.as_deref(), &find);
        for visual in &mut self.visuals {
            if let OwnedFxVisual::Runner { edge, hint } = visual {
                *edge = bind_child(hint.as_deref(), &find);
            }
        }
    }
}
