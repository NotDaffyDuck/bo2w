//! Looking effects up in the loaded catalog, and the per-effect element tables the
//! simulation reads while an effect plays.

use std::cell::Cell;
use std::sync::Arc;

use asset_game::{FxDefinitions, FxName, OwnedFxEffectDef};
use fx::{FX_CATALOG_INDEX_NONE, FxEffectDefInfo, FxElemDefInfo, FxPlayPose, FxPlayRequest};

/// The effect stored at `index`, or `None` for the "no catalog entry" marker and for
/// an index past the end of the catalog.
pub fn catalog_lookup(catalog: &FxDefinitions, index: u16) -> Option<&OwnedFxEffectDef> {
    if index == FX_CATALOG_INDEX_NONE {
        return None;
    }
    catalog.def_at(usize::from(index))
}

/// The index the running effect system uses for `effect`, or the "none" marker when the
/// effect is not one of the catalog's own entries (or the catalog outgrew `u16`).
pub(super) fn runtime_index(catalog: &FxDefinitions, effect: &OwnedFxEffectDef) -> u16 {
    catalog
        .index_of(effect)
        .and_then(|slot| u16::try_from(slot).ok())
        .unwrap_or(FX_CATALOG_INDEX_NONE)
}

/// What the simulation needs to know about each element of `effect`, in element order.
pub(super) fn elem_table(effect: &OwnedFxEffectDef) -> Arc<[FxElemDefInfo]> {
    effect.elems.iter().map(elem_info).collect()
}

fn elem_info(elem: &asset_game::OwnedFxElemDef) -> FxElemDefInfo {
    let v = &elem.view;
    // Only trail elements carry split distances; everything else reports zero.
    let (split_dist, split_arc_dist, split_time) = match &elem.trail_def {
        Some(trail) => (
            trail.inv_split_dist,
            trail.inv_split_arc_dist,
            trail.inv_split_time,
        ),
        None => (0.0, 0.0, 0.0),
    };
    FxElemDefInfo {
        elem_type: v.elem_type,
        spawn_a: v.spawn_a,
        spawn_b: v.spawn_b,
        delay_base: v.spawn_delay_msec_base,
        delay_amp: v.spawn_delay_msec_amplitude,
        life_base: v.life_span_msec_base,
        life_amp: v.life_span_msec_amplitude,
        flags: v.flags,
        visual_count: v.visual_count,
        vis_state_interval_count: v.vis_state_interval_count,
        spawn_range_base: v.spawn_range_base,
        spawn_range_amp: v.spawn_range_amplitude,
        spawn_origin: v.spawn_origin,
        spawn_offset_radius_base: v.spawn_offset_radius_base,
        spawn_offset_radius_amp: v.spawn_offset_radius_amplitude,
        spawn_offset_height_base: v.spawn_offset_height_base,
        spawn_offset_height_amp: v.spawn_offset_height_amplitude,
        has_effect_on_impact: !elem.effect_on_impact.is_absent(),
        has_effect_on_death: !elem.effect_on_death.is_absent(),
        has_effect_emitted: !elem.effect_emitted.is_absent(),
        sort_order: v.sort_order,
        inv_split_dist: split_dist,
        inv_split_arc_dist: split_arc_dist,
        inv_split_time: split_time,
        gravity_base: v.gravity_base,
        gravity_amp: v.gravity_amplitude,
        reflection_base: v.reflection_factor[0],
        reflection_amp: v.reflection_factor[1],
        coll_mins: v.coll_mins,
        coll_maxs: v.coll_maxs,
        use_item_clip: v.use_item_clip,
        spawn_angles: v.spawn_angles,
        angular_velocity: v.angular_velocity,
    }
}

/// One element table per catalog effect, rebuilt only when the catalog changes shape.
///
/// The catalog is only ever appended to, so the entry count and the first name are
/// enough to tell a reload from an unchanged catalog.
#[derive(Clone, Debug, Default)]
pub struct FxElemInfoCache {
    tables: Vec<Arc<[FxElemDefInfo]>>,
    built_for: Option<(usize, Option<String>)>,
}

impl FxElemInfoCache {
    /// Rebuild the tables when `catalog` differs from the one they were built for.
    pub fn sync(&mut self, catalog: &FxDefinitions) {
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
    pub fn arc_for(
        &self,
        catalog: &FxDefinitions,
        effect: &OwnedFxEffectDef,
    ) -> Arc<[FxElemDefInfo]> {
        catalog
            .index_of(effect)
            .and_then(|slot| self.tables.get(slot))
            .map(Arc::clone)
            .unwrap_or_else(|| elem_table(effect))
    }
}

/// An effect together with everything needed to hand it to the effect system.
pub(super) struct Launchable<'a> {
    pub effect: &'a OwnedFxEffectDef,
    index: u16,
    table: Arc<[FxElemDefInfo]>,
}

impl<'a> Launchable<'a> {
    pub fn new(
        catalog: &FxDefinitions,
        cache: &FxElemInfoCache,
        effect: &'a OwnedFxEffectDef,
    ) -> Self {
        Self::at_index(catalog, cache, effect, runtime_index(catalog, effect))
    }

    /// Like `new`, for callers that already know the effect's catalog index.
    pub fn at_index(
        catalog: &FxDefinitions,
        cache: &FxElemInfoCache,
        effect: &'a OwnedFxEffectDef,
        index: u16,
    ) -> Self {
        Self {
            effect,
            index,
            table: cache.arc_for(catalog, effect),
        }
    }

    /// The effect a name stands for, or `None` when the catalog has no such effect.
    pub fn named(
        catalog: &'a FxDefinitions,
        cache: &FxElemInfoCache,
        name: FxName<'_>,
    ) -> Option<Self> {
        // Not `FxName::resolve`, which would tie the result to the name's lifetime too.
        let effect = catalog.resolve_def_in(name.namespace, name.name)?;
        Some(Self::new(catalog, cache, effect))
    }

    pub fn def_info(&self) -> FxEffectDefInfo<'_> {
        let view = &self.effect.view;
        FxEffectDefInfo {
            msec_looping_life: view.msec_looping_life,
            looping_count: view.looping_count,
            one_shot_count: view.one_shot_count,
            emission_count: view.emission_count,
            elems: &self.table,
        }
    }

    /// A request to play this effect at `pose`; never asks for a spotlight.
    pub fn request(&self, pose: FxPlayPose) -> FxPlayRequest<'_> {
        FxPlayRequest {
            def_name: self.effect.name.as_str(),
            pose,
            wants_spotlight: false,
            def: Some(self.def_info()),
            catalog_index: self.index,
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

    pub fn find(&self, index: u16) -> Option<&'a OwnedFxEffectDef> {
        if index == FX_CATALOG_INDEX_NONE {
            self.by_name.set(self.by_name.get().saturating_add(1));
            return None;
        }
        self.by_index.set(self.by_index.get().saturating_add(1));
        self.catalog.def_at(usize::from(index))
    }

    /// `(addressed by index, addressed by name only)`.
    pub fn counts(&self) -> (u32, u32) {
        (self.by_index.get(), self.by_name.get())
    }
}
