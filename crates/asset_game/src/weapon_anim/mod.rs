//! A gun's animation set: the clip on each animation slot, and the authored
//! times the viewmodel is paced by.

mod slot;

pub use slot::WeaponAnimSlot;

use std::sync::Arc;

use asset_anim::xanim_clip::AnimClip;
use asset_anim::{ClipScheduler, ClipSchedulerError, XAnimCatalog};
use asset_core::{AssetEdge, XAnimSpace};
use weapon_t6::asset_slots::WEAPON_ANIM_SLOTS;

use crate::weapon::WeaponRegistry;

/// How the ADS-up and ADS-down clips are driven while aiming.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AdsOverlayConvention {
    /// The ADS-up clip is scrubbed by how far into the aim the player is, and
    /// its weight follows that fraction.
    #[default]
    WeightIsFrac,
    /// The ADS-up and ADS-down clips play as animations: one is held at full
    /// weight while the other is faded out.
    PlayAdsAnim,
}

/// The clip on each slot, and the catalog order each one was taken from.
#[derive(Clone)]
struct ClipTable {
    clips: [Option<Arc<AnimClip>>; WEAPON_ANIM_SLOTS],
    orders: [Option<usize>; WEAPON_ANIM_SLOTS],
}

impl Default for ClipTable {
    fn default() -> Self {
        Self {
            clips: [const { None }; WEAPON_ANIM_SLOTS],
            orders: [None; WEAPON_ANIM_SLOTS],
        }
    }
}

/// Everything the viewmodel needs to animate one gun. A time of 0 means the
/// gun authors none, and its clip plays at its own speed.
#[derive(Clone, Default)]
pub struct WeaponAnimations {
    pub name: String,

    pub fire_time_ms: i32,
    /// How long the first shots of a burst take to fire.
    pub intro_fire_time_ms: i32,
    pub melee_time_ms: i32,
    pub melee_charge_time_ms: i32,

    pub raise_time_ms: i32,
    pub alternate_raise_time_ms: i32,
    pub alternate_drop_time_ms: i32,
    pub drop_time_ms: i32,
    pub quick_drop_time_ms: i32,
    pub quick_raise_time_ms: i32,

    pub sprint_raise_time_ms: i32,
    pub sprint_loop_time_ms: i32,
    pub sprint_drop_time_ms: i32,

    pub reload_time_ms: i32,
    pub reload_empty_time_ms: i32,
    pub reload_start_time_ms: i32,
    pub reload_end_time_ms: i32,
    pub reload_quick_time_ms: i32,
    pub reload_quick_empty_time_ms: i32,

    /// Take-off, in the air, touch-down.
    pub dive_times_ms: [i32; 3],
    /// In, forward, back, right, left, out-fire, out.
    pub crawl_times_ms: [i32; 7],

    pub ads_overlay: AdsOverlayConvention,
    pub inherits_perks: bool,
    /// The fire animation (the recoil) plays to its end once the trigger is
    /// up, instead of settling to idle at once.
    pub fire_plays_out: bool,

    clips: ClipTable,
}

impl std::fmt::Debug for WeaponAnimations {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WeaponAnimations")
            .field("name", &self.name)
            .field("fire_time_ms", &self.fire_time_ms)
            .field("raise_time_ms", &self.raise_time_ms)
            .field("ads_overlay", &self.ads_overlay)
            .field("resolved_clips", &self.resolved_count())
            .finish()
    }
}

impl WeaponAnimations {
    /// A gun with no clips and no authored times.
    pub fn empty(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    /// The set for a registered gun, with its clips found through the
    /// animation edges its weapon row carries.
    pub fn from_registry(registry: &WeaponRegistry, index: u32, xanims: &XAnimCatalog) -> Self {
        Self::from_registry_edges(registry, index, registry.sz_xanim_edges_of(index), xanims)
    }

    /// The same, with the clip edges given: a left-hand gun of a dual pair
    /// has its own.
    pub fn from_registry_edges(
        registry: &WeaponRegistry,
        index: u32,
        edges: Option<&[AssetEdge<XAnimSpace>; WEAPON_ANIM_SLOTS]>,
        xanims: &XAnimCatalog,
    ) -> Self {
        let mut set = Self::empty(registry.name_of(index));
        if let Some(facts) = registry.facts_of(index) {
            set.fire_time_ms = facts.fire_time_ms;
            set.intro_fire_time_ms = facts.intro_fire_time_ms;
            set.melee_time_ms = facts.melee_time_ms;
            set.melee_charge_time_ms = facts.melee_charge_time_ms;
            set.raise_time_ms = facts.raise_time_ms;
            set.alternate_raise_time_ms = facts.alternate_raise_time_ms;
            set.alternate_drop_time_ms = facts.alternate_drop_time_ms;
            set.drop_time_ms = facts.drop_time_ms;
            set.quick_drop_time_ms = facts.quick_drop_time_ms;
            set.quick_raise_time_ms = facts.quick_raise_time_ms;
            set.sprint_raise_time_ms = facts.sprint_raise_time_ms;
            set.sprint_loop_time_ms = facts.sprint_loop_time_ms;
            set.sprint_drop_time_ms = facts.sprint_drop_time_ms;
            set.reload_time_ms = facts.reload_time_ms;
            set.reload_empty_time_ms = facts.reload_empty_time_ms;
            set.reload_start_time_ms = facts.reload_start_time_ms;
            set.reload_end_time_ms = facts.reload_end_time_ms;
            if let Some(dual) = facts.dual_mag {
                set.reload_quick_time_ms = dual.reload_ms;
                set.reload_quick_empty_time_ms = dual.reload_empty_ms;
            }
            set.dive_times_ms = facts.dive_times_ms;
            set.crawl_times_ms = facts.crawl_times_ms;
            set.inherits_perks = facts.inherits_perks;
        }
        set.ads_overlay = AdsOverlayConvention::PlayAdsAnim;
        set.fire_plays_out = registry.namespace_of(index).is_some();

        for (slot, edge) in edges.into_iter().flatten().enumerate() {
            let Some(order) = edge.bound_index() else { continue };
            if let Some(clip) = xanims.clip_at(order) {
                set.clips.clips[slot] = Some(clip);
                set.clips.orders[slot] = Some(order);
            }
        }
        set
    }

    pub fn clip(&self, slot: WeaponAnimSlot) -> Option<&Arc<AnimClip>> {
        self.clip_at(slot.index())
    }

    pub fn clip_at(&self, node: usize) -> Option<&Arc<AnimClip>> {
        self.clips.clips.get(node)?.as_ref()
    }

    /// The catalog order each slot's clip came from, for slots that have one.
    pub fn clip_orders(&self) -> &[Option<usize>; WEAPON_ANIM_SLOTS] {
        &self.clips.orders
    }

    /// Puts a clip on a slot by hand (for tests).
    pub fn with_clip(mut self, slot: WeaponAnimSlot, clip: Arc<AnimClip>) -> Self {
        self.clips.clips[slot.index()] = Some(clip);
        self
    }

    pub fn resolved_count(&self) -> usize {
        self.clips.clips.iter().flatten().count()
    }

    /// Loads every clip onto the node of the same number in a clip tree.
    pub fn install_clips(&self, tree: &mut ClipScheduler) -> Result<(), ClipSchedulerError> {
        for (node, clip) in self.clips.clips.iter().enumerate() {
            if let Some(clip) = clip {
                tree.set_clip(node, Arc::clone(clip))?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asset_anim::xanim_clip::Translation;

    #[test]
    fn a_new_gun_has_no_clips_and_no_times() {
        let gun = WeaponAnimations::empty("mp7_mp");
        assert_eq!(gun.name, "mp7_mp");
        assert_eq!(gun.resolved_count(), 0);
        assert_eq!(gun.fire_time_ms, 0);
        assert_eq!(gun.ads_overlay, AdsOverlayConvention::WeightIsFrac);
        assert!(!gun.fire_plays_out);
        assert!(gun.clip_orders().iter().all(Option::is_none));
    }

    #[test]
    fn a_clip_is_found_by_slot_and_by_node() {
        let clip = Arc::new(AnimClip {
            name: "mp7_fire".into(),
            framerate: 30.0,
            numframes: 2,
            looping: false,
            tracks: Vec::new(),
            notifies: Vec::new(),
            delta_translation: Translation::Default,
        });
        let gun = WeaponAnimations::empty("mp7_mp").with_clip(WeaponAnimSlot::Fire, Arc::clone(&clip));
        assert_eq!(gun.resolved_count(), 1);
        assert!(Arc::ptr_eq(gun.clip(WeaponAnimSlot::Fire).unwrap(), &clip));
        assert!(Arc::ptr_eq(gun.clip_at(WeaponAnimSlot::Fire.index()).unwrap(), &clip));
        assert!(gun.clip(WeaponAnimSlot::Idle).is_none());
        assert!(gun.clip_at(WEAPON_ANIM_SLOTS).is_none());
    }
}
