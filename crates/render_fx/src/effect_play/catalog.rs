//! Looking effects up in the loaded catalog, and the per-effect element tables the
//! simulation reads while an effect plays.

use std::cell::Cell;
use std::sync::Arc;

use asset_game::{FxDefinitions, FxName, SelfContainedEffectDef};
use fx::{NO_RECIPE_LISTING, EmitterRecipeSummary, ParticleRecipeSummary, EffectStartFrame, EffectStartAsk};

/// The effect stored at `index`, or `None` for the "no catalog entry" marker and for
/// an index past the end of the catalog.
pub fn recipe_lookup(catalog: &FxDefinitions, index: u16) -> Option<&SelfContainedEffectDef> {
    if index == NO_RECIPE_LISTING {
        return None;
    }
    catalog.recipe_at(usize::from(index))
}

/// The index the running effect system uses for `effect`, or the "none" marker when the
/// effect is not one of the catalog's own entries (or the catalog outgrew `u16`).
pub(super) fn runtime_index(catalog: &FxDefinitions, effect: &SelfContainedEffectDef) -> u16 {
    catalog
        .index_of(effect)
        .and_then(|slot| u16::try_from(slot).ok())
        .unwrap_or(NO_RECIPE_LISTING)
}

/// What the simulation needs to know about each element of `effect`, in element order.
pub(super) fn elem_table(effect: &SelfContainedEffectDef) -> Arc<[ParticleRecipeSummary]> {
    effect.elems.iter().map(elem_info).collect()
}

fn elem_info(elem: &asset_game::SelfContainedEffectElement) -> ParticleRecipeSummary {
    let v = &elem.view;
    // Only trail elements carry split distances; everything else reports zero.
    let (split_dist, split_arc_dist, split_time) = match &elem.trail_kind {
        Some(trail) => (
            trail.reciprocal_split_distance,
            trail.reciprocal_split_arc_length,
            trail.reciprocal_split_duration,
        ),
        None => (0.0, 0.0, 0.0),
    };
    ParticleRecipeSummary {
        part_shape: v.shape_kind,
        birth_corner_a: v.birth_corner_a,
        birth_corner_b: v.birth_corner_b,
        wait_floor: v.birth_wait_floor_ms,
        wait_spread: v.birth_wait_swing_ms,
        lifespan_floor: v.lifespan_window_floor,
        lifespan_spread: v.lifespan_window_swing,
        bit_set: v.flags,
        look_variant_total: v.look_variant_total,
        look_keyframe_total: v.look_keyframe_total,
        birth_span_floor: v.birth_span_floor,
        birth_span_spread: v.birth_span_swing,
        birth_point: v.birth_point,
        birth_ring_reach_floor: v.birth_ring_reach_floor,
        birth_ring_reach_spread: v.birth_ring_reach_swing,
        birth_lift_floor: v.birth_lift_floor,
        birth_lift_spread: v.birth_lift_swing,
        spawns_on_contact: !elem.effect_upon_strike.lacks_entry(),
        spawns_on_finish: !elem.effect_upon_demise.lacks_entry(),
        emitted_once: !elem.effect_spawned.lacks_entry(),
        paint_rank: v.paint_rank,
        reciprocal_split_distance: split_dist,
        reciprocal_split_arc_length: split_arc_dist,
        reciprocal_split_duration: split_time,
        fall_pull_floor: v.fall_pull_floor,
        fall_pull_spread: v.fall_pull_swing,
        mirror_floor: v.mirror_strength[0],
        mirror_spread: v.mirror_strength[1],
        bounce_box_bottom: v.bounce_box_bottom,
        bounce_box_top: v.bounce_box_top,
        collide_with_pickups: v.collide_with_pickups,
        birth_orientation: v.birth_orientation,
        twirl_rate: v.twirl_rate,
    }
}

/// One element table per catalog effect, rebuilt only when the catalog changes shape.
///
/// The catalog is only ever appended to, so the entry count and the first name are
/// enough to tell a reload from an unchanged catalog.
#[derive(Clone, Debug, Default)]
pub struct EffectElementInfoMemo {
    tables: Vec<Arc<[ParticleRecipeSummary]>>,
    built_for: Option<(usize, Option<String>)>,
}

impl EffectElementInfoMemo {
    /// Rebuild the tables when `catalog` differs from the one they were built for.
    pub fn restock(&mut self, catalog: &FxDefinitions) {
        let stamp = (catalog.len(), catalog.names().next().map(str::to_owned));
        if self.built_for.as_ref() == Some(&stamp) {
            return;
        }
        self.tables = catalog.effects().map(elem_table).collect();
        self.built_for = Some(stamp);
    }

    /// Forget everything; the next `sync` rebuilds from scratch.
    pub fn clear(&mut self) {
        self.tables.clear();
        self.built_for = None;
    }

    /// The shared table for `effect` when it is one of the catalog's own entries,
    /// otherwise a table built just for this call.
    pub fn shared_handle_for(
        &self,
        catalog: &FxDefinitions,
        effect: &SelfContainedEffectDef,
    ) -> Arc<[ParticleRecipeSummary]> {
        catalog
            .index_of(effect)
            .and_then(|slot| self.tables.get(slot))
            .map(Arc::clone)
            .unwrap_or_else(|| elem_table(effect))
    }
}

/// An effect together with everything needed to hand it to the effect system.
pub(super) struct Launchable<'a> {
    pub effect: &'a SelfContainedEffectDef,
    index: u16,
    table: Arc<[ParticleRecipeSummary]>,
}

impl<'a> Launchable<'a> {
    pub fn new(
        catalog: &FxDefinitions,
        cache: &EffectElementInfoMemo,
        effect: &'a SelfContainedEffectDef,
    ) -> Self {
        Self::at_index(catalog, cache, effect, runtime_index(catalog, effect))
    }

    /// Like `new`, for callers that already know the effect's catalog index.
    pub fn at_index(
        catalog: &FxDefinitions,
        cache: &EffectElementInfoMemo,
        effect: &'a SelfContainedEffectDef,
        index: u16,
    ) -> Self {
        Self {
            effect,
            index,
            table: cache.shared_handle_for(catalog, effect),
        }
    }

    /// The effect a name stands for, or `None` when the catalog has no such effect.
    pub fn named(
        catalog: &'a FxDefinitions,
        cache: &EffectElementInfoMemo,
        name: FxName<'_>,
    ) -> Option<Self> {
        // Not `FxName::resolve`, which would tie the result to the name's lifetime too.
        let effect = catalog.resolve_def_in(name.namespace, name.name)?;
        Some(Self::new(catalog, cache, effect))
    }

    pub fn def_info(&self) -> EmitterRecipeSummary<'_> {
        let view = &self.effect.view;
        EmitterRecipeSummary {
            loop_lifetime_millis: view.loop_lifetime_millis,
            repeat_total: view.repeat_total,
            single_play_total: view.single_play_total,
            spawn_total: view.spawn_total,
            elems: &self.table,
        }
    }

    /// A request to play this effect at `pose`; never asks for a spotlight.
    pub fn request(&self, pose: EffectStartFrame) -> EffectStartAsk<'_> {
        EffectStartAsk {
            definition_name: self.effect.name.as_str(),
            pose,
            needs_cone_glow: false,
            def: Some(self.def_info()),
            ledger_index: self.index,
        }
    }
}

/// Catalog lookups made while drawing, counting how each one was addressed: by catalog
/// index, or only by name (which the draw path cannot resolve).
pub(super) struct DrawLookup<'a> {
    catalog: &'a FxDefinitions,
    by_index: Cell<u32>,
    by_name: Cell<u32>,
}

impl<'a> DrawLookup<'a> {
    pub fn new(catalog: &'a FxDefinitions) -> Self {
        Self {
            catalog,
            by_index: Cell::new(0),
            by_name: Cell::new(0),
        }
    }

    pub fn look_up(&self, index: u16) -> Option<&'a SelfContainedEffectDef> {
        if index == NO_RECIPE_LISTING {
            self.by_name.set(self.by_name.get().saturating_add(1));
            return None;
        }
        self.by_index.set(self.by_index.get().saturating_add(1));
        self.catalog.recipe_at(usize::from(index))
    }

    /// `(addressed by index, addressed by name only)`.
    pub fn counts(&self) -> (u32, u32) {
        (self.by_index.get(), self.by_name.get())
    }
}
