//! The effects a build knows, found by name, and the passes that tie their
//! materials, models and child effects to the catalogs they name.

use std::collections::{BTreeMap, HashMap, HashSet};

use asset_core::{AssetDependencyLink, DependencyCause, FxRange, ZoneCustodian};
use asset_material::MaterialDefinitions;
use fx_t6::runtime::EmitterRecipeLens;

use super::elem_look::{MaterialSlot, SelfContainedEffectElement, SelfContainedEffectLook};
use super::FxName;
use crate::{AssetCategoryScope, AssetPointer, EffectMeshRoster};

/// The additive flare the Thunder Gun's flash sprites draw with (its own
/// power cell effect loads it, so it is in every zone that has the gun).
pub const THUNDER_FLASH_MATERIAL: &str = "gfx_fxt_light_flare_halogen_vm";

#[derive(Clone, Debug)]
pub struct SelfContainedEffectDef {
    pub namespace: AssetCategoryScope,
    pub name: String,
    pub view: EmitterRecipeLens,
    pub elems: Vec<SelfContainedEffectElement>,
}

/// The effects a build finished with, found by namespace and name (case does
/// not matter).
#[derive(Clone, Debug, Default)]
pub struct FxDefinitions {
    effects: Vec<SelfContainedEffectDef>,
    tables: HashMap<AssetCategoryScope, HashMap<String, usize>>,
    home: AssetCategoryScope,
}

/// The effects being collected. It reads as the finished set; `publish` hands
/// that set on.
#[derive(Clone, Debug, Default)]
pub struct EffectRoster(FxDefinitions);

impl std::ops::Deref for EffectRoster {
    type Target = FxDefinitions;

    fn deref(&self) -> &FxDefinitions {
        &self.0
    }
}

impl std::ops::DerefMut for EffectRoster {
    fn deref_mut(&mut self) -> &mut FxDefinitions {
        &mut self.0
    }
}

impl EffectRoster {
    /// The edge to the effect a weapon row names. A row that has the slot but
    /// no name, or a name nobody defines, is an open edge; a row with neither
    /// has no edge at all.
    pub fn hint_edge(&self, designer_slot: bool, caption: Option<&str>, namespace: AssetCategoryScope) -> AssetDependencyLink<FxRange> {
        let named = caption.filter(|text| !text.is_empty());
        if named.is_none() && !designer_slot {
            return AssetDependencyLink::NothingThere;
        }
        let found = named.and_then(|text| self.slot_in(namespace, text));
        found.map_or(AssetDependencyLink::Unresolved(DependencyCause::IndexLookupFailure), |pos| {
            AssetDependencyLink::bind_order(pos, ZoneCustodian::default())
        })
    }

    pub fn publish(self) -> FxDefinitions {
        self.0
    }

    /// Takes every effect of another catalog; one of the same name is replaced.
    pub fn merge_in(&mut self, incoming: EffectRoster) {
        incoming.0.effects.into_iter().for_each(|fxdef| self.add_owned_fx(fxdef));
    }
}

impl FxDefinitions {
    pub fn len(&self) -> usize {
        self.effects.len()
    }

    pub fn is_empty(&self) -> bool {
        self.effects.is_empty()
    }

    pub fn slot_in(&self, namespace: AssetCategoryScope, name: &str) -> Option<usize> {
        let table = self.tables.get(&namespace)?;
        table.get(&name.to_ascii_lowercase()).copied()
    }

    pub fn get_in(&self, namespace: AssetCategoryScope, name: &str) -> Option<&SelfContainedEffectDef> {
        self.slot_in(namespace, name).and_then(|pos| self.recipe_at(pos))
    }

    /// The catalog order of an effect that sits in this catalog.
    pub fn index_of(&self, fxdef: &SelfContainedEffectDef) -> Option<usize> {
        let pos = self.slot_in(fxdef.namespace, &fxdef.name)?;
        std::ptr::eq(&self.effects[pos], fxdef).then_some(pos)
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.effects().map(|fxdef| fxdef.name.as_str())
    }

    pub fn recipe_at(&self, pos: usize) -> Option<&SelfContainedEffectDef> {
        self.effects.get(pos)
    }

    pub fn effects(&self) -> impl Iterator<Item = &SelfContainedEffectDef> {
        self.effects.iter()
    }

    /// Adds an effect, or replaces the one of the same name where it sits.
    pub fn add_owned_fx(&mut self, fxdef: SelfContainedEffectDef) {
        let table = self.tables.entry(fxdef.namespace).or_default();
        let next = self.effects.len();
        let pos = *table.entry(fxdef.name.to_ascii_lowercase()).or_insert(next);
        if pos == next {
            self.effects.push(fxdef);
        } else {
            self.effects[pos] = fxdef;
        }
    }

    /// The namespace a map's own effects live in.
    pub fn set_map_namespace(&mut self, namespace: AssetCategoryScope) {
        self.home = namespace;
    }

    pub fn level_scope(&self) -> AssetCategoryScope {
        self.home
    }

    /// An effect a map's script names by the id its effect list gave it.
    pub fn find_createfx_id(&self, fxid: &str) -> Option<&SelfContainedEffectDef> {
        let in_map_or_base = self.resolve_def_for_map(fxid);
        in_map_or_base.or_else(|| self.get_in(self.home, fxid))
    }

    /// The name of a map's effect together with the namespace that defines it.
    pub fn map_fx_name<'a>(&self, name: &'a str) -> FxName<'a> {
        let defined_in = self.resolve_def_for_map(name).map(|fxdef| fxdef.namespace);
        FxName::new(defined_in.unwrap_or(self.home), name)
    }

    /// The effect of that name that has parts to play. A name that carries a
    /// reference mark is looked up again without it.
    pub fn resolve_def_in(&self, namespace: AssetCategoryScope, name: &str) -> Option<&SelfContainedEffectDef> {
        let bare = AssetPointer::bare_name(name);
        let spellings = [Some(name), Some(bare).filter(|plain| *plain != name)];
        spellings
            .into_iter()
            .flatten()
            .filter_map(|spelling| self.get_in(namespace, spelling))
            .find(|fxdef| !fxdef.elems.is_empty())
    }

    /// The same for a map: its own namespace first, then the base game's.
    pub fn resolve_def_for_map(&self, name: &str) -> Option<&SelfContainedEffectDef> {
        let base = (self.home != AssetCategoryScope::T6).then_some(AssetCategoryScope::T6);
        std::iter::once(self.home)
            .chain(base)
            .find_map(|namespace| self.resolve_def_in(namespace, name))
    }

    /// Every drawn piece with the namespace its effect was authored in.
    fn pieces(&self) -> impl Iterator<Item = (AssetCategoryScope, &SelfContainedEffectLook)> {
        self.effects().flat_map(|fxdef| {
            let namespace = fxdef.namespace;
            fxdef.elems.iter().flat_map(|part| &part.visuals).map(move |piece| (namespace, piece))
        })
    }

    fn for_each_piece_mut(&mut self, mut apply: impl FnMut(AssetCategoryScope, &mut SelfContainedEffectLook)) {
        for fxdef in &mut self.effects {
            let namespace = fxdef.namespace;
            fxdef
                .elems
                .iter_mut()
                .flat_map(|part| part.visuals.iter_mut())
                .for_each(|piece| apply(namespace, piece));
        }
    }

    /// Ties every material an effect draws with to the material catalog.
    pub fn link_surfaces(&mut self, mat_cat: &MaterialDefinitions) {
        self.swap_thunder_flash_material();
        self.for_each_piece_mut(|namespace, piece| piece.resolve_materials(mat_cat, namespace));
    }

    /// bo2zm 10-14: the Thunder Gun's shot (Declassified port of BO1) builds its flash from
    /// heat-haze sprites (a white 300 core, seven violet 144,144,255 puffs). BO2's material of
    /// that name is a pure warp, so the shot showed only grey rings, where BO1 bursts
    /// white-violet. Those sprites draw with the gun's own additive flare picture, in their
    /// colours.
    fn swap_thunder_flash_material(&mut self) {
        let flashes = self
            .effects
            .iter_mut()
            .filter(|fxdef| fxdef.name.starts_with("weapon/muzzleflashes/fx_thundergun_view"));
        for fxdef in flashes {
            for piece in fxdef.elems.iter_mut().flat_map(|part| part.visuals.iter_mut()) {
                if let SelfContainedEffectLook::Material(slot) = piece
                    && slot.hint.as_deref().map(|n| n.trim_start_matches(','))
                        == Some("gfx_fxt_fx_distortion_heat")
                {
                    slot.hint = Some(THUNDER_FLASH_MATERIAL.to_owned());
                }
            }
        }
    }

    /// Ties the effects an effect spawns or runs to the effects here. Only an
    /// effect that has parts can be one.
    pub fn flatten_child_edges(&mut self) {
        let playable: Vec<bool> = self.effects.iter().map(|fxdef| !fxdef.elems.is_empty()).collect();
        let tables = &self.tables;
        for fxdef in &mut self.effects {
            if let Some(table) = tables.get(&fxdef.namespace) {
                let find = |name: &str| playable_position(table, &playable, name);
                for part in &mut fxdef.elems {
                    part.bind_children(&find);
                }
            }
        }
    }

    /// Ties every model an effect shows to the effect-model catalog.
    pub fn link_effect_meshes(&mut self, model_cat: &EffectMeshRoster) {
        self.for_each_piece_mut(|namespace, piece| {
            let SelfContainedEffectLook::Mesh { edge, hint } = piece else {
                return;
            };
            *edge = model_cat.slot_in(namespace, hint).map_or(
                AssetDependencyLink::Unresolved(DependencyCause::IndexLookupFailure),
                |pos| AssetDependencyLink::bind_order(pos, model_cat.source_zone(pos)),
            );
        });
    }

    /// The models the effects show, each by the namespace it was authored in
    /// and its name.
    pub fn model_hints(&self) -> HashSet<(AssetCategoryScope, String)> {
        self.pieces()
            .filter_map(|(scope, piece)| {
                let SelfContainedEffectLook::Mesh { hint: hint_text, .. } = piece else {
                    return None;
                };
                Some((scope, hint_text.to_owned()))
            })
            .collect()
    }

    /// The matched material slots of the pieces `wanted` accepts, once each.
    fn bound_slots_of(&self, wanted: fn(&SelfContainedEffectLook) -> bool) -> Vec<(usize, &str)> {
        let slots = self
            .pieces()
            .map(|(_, piece)| piece)
            .filter(|piece| wanted(piece))
            .flat_map(|piece| piece.slots());
        unique_bound(slots)
    }

    /// Each material the effects draw with that is matched, by catalog order,
    /// with the name it was authored under.
    pub fn distinct_binding_clues(&self) -> Vec<(usize, &str)> {
        self.bound_slots_of(draws_material)
    }

    pub fn surface_look_distinct_binding_total(&self) -> usize {
        self.bound_slots_of(draws_material).len()
    }

    /// The same for the materials of decals.
    pub fn distinct_decal_hints(&self) -> Vec<(usize, &str)> {
        self.bound_slots_of(|piece| matches!(piece, SelfContainedEffectLook::Mark(_)))
    }

    /// The names of the sounds the effects play, each once.
    pub fn distinct_tenth_kind_audio_clues(&self) -> Vec<&str> {
        let mut heard: Vec<&str> = self
            .pieces()
            .filter_map(|(_, piece)| match piece {
                SelfContainedEffectLook::Sound { hint } => Some(hint.as_str()),
                _ => None,
            })
            .collect();
        heard.sort_unstable();
        heard.dedup();
        heard
    }
}

/// Whether a visual draws with a single material (a decal has two, counted apart).
fn draws_material(piece: &SelfContainedEffectLook) -> bool {
    matches!(piece, SelfContainedEffectLook::Material(_))
}

/// The catalog position of the first spelling of `name` (as written, then
/// without its reference mark) that names an effect with parts.
fn playable_position(table: &HashMap<String, usize>, playable: &[bool], wanted: &str) -> Option<usize> {
    [wanted, AssetPointer::bare_name(wanted)]
        .iter()
        .find_map(|spelling| table.get(&spelling.to_ascii_lowercase()).copied().filter(|&pos| playable[pos]))
}

/// Each matched slot once, by catalog order; the name is the first one seen,
/// or "-" for a slot that has none.
fn unique_bound<'a>(slots: impl Iterator<Item = &'a MaterialSlot>) -> Vec<(usize, &'a str)> {
    let firsts = slots
        .filter_map(|slot| Some((slot.edge.extent_position()?, slot)))
        .fold(BTreeMap::new(), |mut seen, (pos, slot)| {
            seen.entry(pos).or_insert(slot);
            seen
        });
    firsts
        .into_iter()
        .map(|(pos, slot)| match slot.hint.as_deref() {
            Some(text) if !text.is_empty() => (pos, text),
            _ => (pos, "-"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fx::look_kind;
    use crate::fx::elem_look::pending;
    use fx_t6::runtime::ParticleRecipeLens;

    const T6: AssetCategoryScope = AssetCategoryScope::T6;

    /// An element view that is all zero apart from its type.
    fn blank_view(elem_type: u8) -> ParticleRecipeLens {
        let (nil, span, boxes) = (0.0_f32, [0.0_f32; 2], [[0.0_f32; 2]; 3]);
        let (look_keyframe_total, collide_with_pickups, paint_rank) = Default::default();
        ParticleRecipeLens {
            // Identity and timing.
            shape_kind: elem_type,
            look_variant_total: 1,
            flags: 0,
            birth_corner_a: 0,
            birth_corner_b: 0,
            birth_wait_floor_ms: 0,
            birth_wait_swing_ms: 0,
            lifespan_window_floor: 0,
            lifespan_window_swing: 0,
            // Where and how it appears.
            birth_span_floor: nil,
            birth_span_swing: nil,
            birth_view_reject_reach: nil,
            appear_blend_reach: span,
            vanish_blend_reach: span,
            birth_point: boxes,
            birth_orientation: boxes,
            birth_ring_reach_floor: nil,
            birth_ring_reach_swing: nil,
            birth_lift_floor: nil,
            birth_lift_swing: nil,
            // Motion.
            twirl_rate: boxes,
            birth_spin_angle: span,
            fall_pull_floor: nil,
            fall_pull_swing: nil,
            mirror_strength: span,
            bounce_box_bottom: [nil; 3],
            bounce_box_top: [nil; 3],
            emission_spacing: span,
            emission_spacing_spread: span,
            // Graph sizes and draw state.
            speed_keyframe_total: 0,
            look_keyframe_total,
            shading_blend: 0,
            collide_with_pickups,
            paint_rank,
            line: false,
        }
    }

    /// A part that spawns nothing and draws `visuals`.
    fn bare_part(visuals: Vec<SelfContainedEffectLook>) -> SelfContainedEffectElement {
        SelfContainedEffectElement {
            visuals,
            view: blank_view(look_kind::ALWAYS_FACING),
            atlas: [0; 8],
            speed_readings: Vec::new(),
            vel_graph_local: Vec::new(),
            vel_graph_world: Vec::new(),
            visibility_readings: Vec::new(),
            effect_upon_strike: AssetDependencyLink::NothingThere,
            effect_upon_demise: AssetDependencyLink::NothingThere,
            effect_spawned: AssetDependencyLink::NothingThere,
            on_impact_name: None,
            on_death_name: None,
            emitted_name: None,
            trail_kind: None,
            cloud_density: None,
        }
    }

    /// A part that spawns `on_death` when it ends and draws `visuals`.
    fn part_spawning(on_death: &str, visuals: Vec<SelfContainedEffectLook>) -> SelfContainedEffectElement {
        let mut part = bare_part(visuals);
        (part.effect_upon_demise, part.on_death_name) = pending(on_death);
        part
    }

    fn plain_part() -> SelfContainedEffectElement {
        part_spawning("", Vec::new())
    }

    fn effect_of(fx_name: &str, pieces: Vec<SelfContainedEffectElement>) -> SelfContainedEffectDef {
        let zero = 0;
        let view = EmitterRecipeLens {
            spawn_total: zero,
            single_play_total: zero,
            repeat_total: zero,
            loop_lifetime_millis: zero,
            flags: zero,
        };
        SelfContainedEffectDef { namespace: T6, name: fx_name.to_owned(), view, elems: pieces }
    }

    fn catalog_of(effects: impl IntoIterator<Item = SelfContainedEffectDef>) -> EffectRoster {
        let mut catalog = EffectRoster::default();
        effects.into_iter().for_each(|fxdef| catalog.add_owned_fx(fxdef));
        catalog
    }

    #[test]
    fn effects_are_found_whatever_the_case() {
        let fx = catalog_of([effect_of("Impacts/Dirt", vec![plain_part()])]);
        let found = ["impacts/dirt", "IMPACTS/DIRT"].map(|name| fx.slot_in(T6, name));
        assert_eq!(found, [Some(0), Some(0)]);
        assert_eq!(fx.slot_in(AssetCategoryScope::T5, "impacts/dirt"), None);
    }

    #[test]
    fn a_second_effect_of_the_same_name_replaces_the_first_in_place() {
        let mut fx = catalog_of([effect_of("a", vec![]), effect_of("b", vec![])]);
        fx.add_owned_fx(effect_of("A", vec![plain_part()]));
        assert_eq!((fx.len(), fx.recipe_at(0).map(|d| d.elems.len())), (2, Some(1)));
        assert_eq!(fx.recipe_at(0).map(|d| d.name.as_str()), Some("A"));
    }

    #[test]
    fn a_name_with_the_reference_mark_finds_the_effect_without_it() {
        let fx = catalog_of([effect_of("blast", vec![plain_part()])]);
        assert!(fx.resolve_def_in(T6, ",blast").is_some());
        assert!(fx.resolve_def_in(T6, "missing").is_none());
    }

    #[test]
    fn an_effect_with_no_parts_cannot_be_played() {
        let fx = catalog_of([effect_of("empty", vec![])]);
        assert!(fx.get_in(T6, "empty").is_some());
        assert!(fx.resolve_def_in(T6, "empty").is_none());
    }

    #[test]
    fn a_spawned_effect_binds_to_an_effect_that_has_parts() {
        let spawner = vec![part_spawning("Child", vec![]), part_spawning("hollow", vec![]), plain_part()];
        let mut fx = catalog_of([
            effect_of("parent", spawner),
            effect_of("child", vec![plain_part()]),
            effect_of("hollow", vec![]),
        ]);
        fx.flatten_child_edges();
        let spawned: Vec<_> = fx.get_in(T6, "parent").unwrap().elems.iter().map(|p| p.effect_upon_demise.clone()).collect();
        assert_eq!(spawned[0].extent_position(), Some(1));
        assert_eq!(spawned[1], AssetDependencyLink::Unresolved(DependencyCause::IndexLookupFailure));
        assert!(spawned[2].lacks_entry());
    }

    #[test]
    fn a_weapon_row_edge_is_absent_only_when_it_has_neither_slot_nor_name() {
        let fx = catalog_of([effect_of("flash", vec![])]);
        let edge = |slot, label| fx.hint_edge(slot, label, T6);
        for (slot, label) in [(false, None), (false, Some(""))] {
            assert!(edge(slot, label).lacks_entry());
        }
        for (slot, label) in [(true, None), (false, Some("nothing"))] {
            assert!(edge(slot, label).is_unlinked_ref());
        }
        assert_eq!(edge(false, Some("Flash")).extent_position(), Some(0));
    }

    #[test]
    fn a_map_looks_in_its_own_namespace_before_the_base_games() {
        let mut fx = catalog_of([effect_of("steam", vec![plain_part()])]);
        fx.set_map_namespace(AssetCategoryScope::T5);
        assert!(fx.find_createfx_id("steam").is_some());
        let homes = ["steam", "unknown"].map(|name| fx.map_fx_name(name).namespace);
        assert_eq!(homes, [T6, AssetCategoryScope::T5]);
    }

    #[test]
    fn material_and_sound_names_are_listed_once() {
        let playing = |names: &[&str]| {
            let parts = names.iter().map(|n| part_spawning("", vec![SelfContainedEffectLook::sound(n)]));
            parts.collect::<Vec<_>>()
        };
        let fx = catalog_of([effect_of("a", playing(&["boom", "crack"])), effect_of("b", playing(&["boom"]))]);
        assert_eq!(fx.distinct_tenth_kind_audio_clues(), ["boom", "crack"]);
        assert!(fx.distinct_binding_clues().is_empty());
    }

    #[test]
    fn a_matched_material_is_listed_once_under_the_first_name_seen() {
        let matched = |label: &str, order: usize| {
            let mut visual = SelfContainedEffectLook::material(label);
            if let SelfContainedEffectLook::Material(slot) = &mut visual {
                slot.edge = AssetDependencyLink::bind_order(order, ZoneCustodian::default());
            }
            visual
        };
        let parts = vec![
            part_spawning("", vec![matched("late", 5), matched("first", 2)]),
            part_spawning("", vec![matched("again", 2), SelfContainedEffectLook::material("unmatched")]),
        ];
        let fx = catalog_of([effect_of("a", parts)]);
        assert_eq!(fx.distinct_binding_clues(), [(2, "first"), (5, "late")]);
        assert_eq!(fx.surface_look_distinct_binding_total(), 2);
        assert!(fx.distinct_decal_hints().is_empty());
        assert!(!fx.is_empty());
    }
}
