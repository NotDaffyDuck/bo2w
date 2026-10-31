//! A gun's animation set: the clip on each animation slot, and the authored
//! times the viewmodel is paced by.

mod slot;

pub use slot::GunMoveRole;

use std::sync::Arc;

use anim_bank::motion_clip::SkeletalClip;
use anim_bank::{ClipPlanner, ClipPlanningFault, MotionClipRoster};
use asset_core::{AssetDependencyLink, XAnimRange};
use weapon_t6::asset_slots::WEAPON_ANIM_SLOTS;

use crate::weapon::WeaponLedger;

/// How the ADS-up and ADS-down clips are driven while aiming.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScopeOverlayStyle {
    /// The ADS-up clip is scrubbed by how far into the aim the player is, and
    /// its weight follows that fraction.
    #[default]
    WeightFractional,
    /// The ADS-up and ADS-down clips play as animations: one is held at full
    /// weight while the other is faded out.
    StartAimMotion,
}

/// The clip on each slot, and the catalog order each one was taken from.
#[derive(Clone)]
struct ClipTable {
    clips: [Option<Arc<SkeletalClip>>; WEAPON_ANIM_SLOTS],
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
pub struct GunClipSet {
    pub name: String,

    pub fire_time_ms: i32,
    /// An offhand's wind-up (`holdFireTime`): the pull-back plays over it.
    pub trigger_hold_millis: i32,
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

    pub run_lift_millis: i32,
    pub run_cycle_millis: i32,
    pub run_lower_millis: i32,

    pub reload_duration_millis: i32,
    pub refill_dry_millis: i32,
    pub refill_begin_millis: i32,
    pub refill_finish_millis: i32,
    pub reload_quick_time_ms: i32,
    pub reload_quick_empty_time_ms: i32,

    /// Take-off, in the air, touch-down.
    pub dive_times_ms: [i32; 3],
    /// In, forward, back, right, left, out-fire, out.
    pub crawl_times_ms: [i32; 7],

    pub ads_overlay: ScopeOverlayStyle,
    pub carries_over_talents: bool,
    /// The fire animation (the recoil) plays to its end once the trigger is
    /// up, instead of settling to idle at once.
    pub fire_plays_out: bool,

    clips: ClipTable,
}

impl std::fmt::Debug for GunClipSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use std::fmt::Debug;
        let resolved = self.linked_total();
        let shown: [(&str, &dyn Debug); 5] = [
            ("name", &self.name),
            ("fire_ms", &self.fire_time_ms),
            ("raise_ms", &self.raise_time_ms),
            ("ads_overlay", &self.ads_overlay),
            ("resolved_clips", &resolved),
        ];
        shown
            .into_iter()
            .fold(f.debug_struct("WeaponAnimations"), |mut out, (label, value)| {
                out.field(label, value);
                out
            })
            .finish()
    }
}

impl GunClipSet {
    /// A gun with no clips and no authored times.
    pub fn blank(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    /// The set for a registered gun, with its clips found through the
    /// animation edges its weapon row carries.
    pub fn from_registry(registry: &WeaponLedger, index: u32, animation_refs: &MotionClipRoster) -> Self {
        Self::from_registry_edges(registry, index, registry.sz_xanim_edges_of(index), animation_refs)
    }

    /// The same, with the clip edges given: a left-hand gun of a dual pair
    /// has its own.
    pub fn from_registry_edges(
        registry: &WeaponLedger,
        index: u32,
        edges: Option<&[AssetDependencyLink<XAnimRange>; WEAPON_ANIM_SLOTS]>,
        animation_refs: &MotionClipRoster,
    ) -> Self {
        let mut set = Self::blank(registry.label_for(index));
        if let Some(facts) = registry.summary_for(index) {
            set.fire_time_ms = facts.shot_gap_ms;
            set.trigger_hold_millis = facts.trigger_hold_millis;
            set.intro_fire_time_ms = facts.intro_fire_time_ms;
            set.melee_time_ms = facts.bash_ms;
            set.melee_charge_time_ms = facts.bash_windup_ms;
            set.raise_time_ms = facts.draw_ms;
            set.alternate_raise_time_ms = facts.alternate_raise_time_ms;
            set.alternate_drop_time_ms = facts.alternate_drop_time_ms;
            set.drop_time_ms = facts.holster_ms;
            set.quick_drop_time_ms = facts.fast_holster_ms;
            set.quick_raise_time_ms = facts.fast_draw_ms;
            set.run_lift_millis = facts.run_lift_millis;
            set.run_cycle_millis = facts.run_cycle_millis;
            set.run_lower_millis = facts.run_lower_millis;
            set.reload_duration_millis = facts.reload_duration_millis;
            set.refill_dry_millis = facts.refill_dry_millis;
            set.refill_begin_millis = facts.refill_begin_millis;
            set.refill_finish_millis = facts.refill_finish_millis;
            if let Some(dual) = facts.dual_mag {
                set.reload_quick_time_ms = dual.reload_ms;
                set.reload_quick_empty_time_ms = dual.reload_empty_ms;
            }
            set.dive_times_ms = facts.dive_times_ms;
            set.crawl_times_ms = facts.crawl_times_ms;
            set.carries_over_talents = facts.carries_over_talents;
        }
        set.ads_overlay = ScopeOverlayStyle::StartAimMotion;
        set.fire_plays_out = registry.scope_for(index).is_some();

        set.bind_clips(edges, |order| animation_refs.node_clip(order));
        set
    }

    /// Puts a clip on every slot whose edge is bound to a catalog order that
    /// `lookup` can turn into a clip; the other slots stay as they were.
    fn bind_clips(
        &mut self,
        edges: Option<&[AssetDependencyLink<XAnimRange>; WEAPON_ANIM_SLOTS]>,
        lookup: impl Fn(usize) -> Option<Arc<SkeletalClip>>,
    ) {
        let found = edges.into_iter().flatten().enumerate().filter_map(|(role, edge)| {
            let order = edge.extent_position()?;
            Some((role, order, lookup(order)?))
        });
        for (slot, order, clip) in found {
            self.clips.clips[slot] = Some(clip);
            self.clips.orders[slot] = Some(order);
        }
    }

    pub fn reel(&self, bay: GunMoveRole) -> Option<&Arc<SkeletalClip>> {
        self.clip_for_node(bay.index())
    }

    pub fn clip_for_node(&self, node: usize) -> Option<&Arc<SkeletalClip>> {
        self.clips.clips.get(node)?.as_ref()
    }

    /// The catalog order each slot's clip came from, for slots that have one.
    pub fn clip_orders(&self) -> &[Option<usize>; WEAPON_ANIM_SLOTS] {
        &self.clips.orders
    }

    /// Puts a clip on a slot by hand (for tests).
    pub fn with_clip(mut self, bay: GunMoveRole, reel: Arc<SkeletalClip>) -> Self {
        self.clips.clips[bay.index()] = Some(reel);
        self
    }

    pub fn linked_total(&self) -> usize {
        self.clips.clips.iter().flatten().count()
    }

    /// Loads every clip onto the node of the same number in a clip tree.
    pub fn install_clips(&self, tree: &mut ClipPlanner) -> Result<(), ClipPlanningFault> {
        self.clips
            .clips
            .iter()
            .enumerate()
            .filter_map(|(node, clip)| Some((node, clip.as_ref()?)))
            .try_for_each(|(node, clip)| tree.load_clip(node, Arc::clone(clip)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anim_bank::motion_clip::RootShift;

    #[test]
    fn a_new_gun_has_no_clips_and_no_times() {
        let gun = GunClipSet::blank("mp7_mp");
        assert_eq!(gun.name, "mp7_mp");
        assert_eq!(gun.linked_total(), 0);
        assert_eq!(gun.fire_time_ms, 0);
        assert_eq!(gun.ads_overlay, ScopeOverlayStyle::WeightFractional);
        assert!(!gun.fire_plays_out);
        assert!(gun.clip_orders().iter().all(Option::is_none));
    }

    fn sample_clip(name: &str) -> Arc<SkeletalClip> {
        Arc::new(SkeletalClip {
            name: name.into(),
            fps: 30.0,
            frame_total: 2,
            looping: false,
            tracks: Vec::new(),
            notifies: Vec::new(),
            root_motion_shift: RootShift::Default,
        })
    }

    #[test]
    fn bound_edges_pick_up_clips_and_others_stay_empty() {
        use asset_core::ZoneCustodian;
        let mut edges: [AssetDependencyLink<XAnimRange>; WEAPON_ANIM_SLOTS] = std::array::from_fn(|_| AssetDependencyLink::NothingThere);
        edges[0] = AssetDependencyLink::bind_order(7, ZoneCustodian::default());
        edges[1] = AssetDependencyLink::bind_order(9, ZoneCustodian::default());
        let mut gun = GunClipSet::blank("mp7_mp");
        let clip = sample_clip("seven");
        gun.bind_clips(Some(&edges), |order| (order == 7).then(|| Arc::clone(&clip)));
        assert_eq!(gun.linked_total(), 1);
        assert_eq!(gun.clip_orders()[0], Some(7));
        assert_eq!(gun.clip_orders()[1], None);
        assert!(Arc::ptr_eq(gun.clip_for_node(0).unwrap(), &clip));
        let mut untouched = GunClipSet::blank("none");
        untouched.bind_clips(None, |_| Some(Arc::clone(&clip)));
        assert_eq!(untouched.linked_total(), 0);
    }

    #[test]
    fn install_clips_loads_each_resolved_slot_and_reports_a_short_tree() {
        let gun = GunClipSet::blank("mp7_mp")
            .with_clip(GunMoveRole::Fire, sample_clip("fire"))
            .with_clip(GunMoveRole::Idle, sample_clip("idle"));
        let mut roomy = ClipPlanner::new(WEAPON_ANIM_SLOTS);
        assert!(gun.install_clips(&mut roomy).is_ok());
        let mut empty_tree = ClipPlanner::new(0);
        assert!(gun.install_clips(&mut empty_tree).is_err());
        assert!(GunClipSet::blank("x").install_clips(&mut ClipPlanner::new(0)).is_ok());
    }

    #[test]
    fn debug_lists_the_summary_fields_and_the_resolved_count() {
        let gun = GunClipSet::blank("mp7_mp").with_clip(GunMoveRole::Fire, sample_clip("fire"));
        let shown = format!("{gun:?}");
        assert_eq!(
            shown,
            "WeaponAnimations { name: \"mp7_mp\", fire_ms: 0, raise_ms: 0, ads_overlay: WeightFractional, resolved_clips: 1 }"
        );
    }

    #[test]
    fn a_clip_is_found_by_slot_and_by_node() {
        let fire_clip = Arc::new(SkeletalClip {
            name: "mp7_fire".into(),
            fps: 30.0,
            frame_total: 2,
            looping: false,
            tracks: Vec::new(),
            notifies: Vec::new(),
            root_motion_shift: RootShift::Default,
        });
        let gun = GunClipSet::blank("mp7_mp").with_clip(GunMoveRole::Fire, Arc::clone(&fire_clip));
        assert_eq!(gun.linked_total(), 1);
        assert!(Arc::ptr_eq(gun.reel(GunMoveRole::Fire).unwrap(), &fire_clip));
        assert!(Arc::ptr_eq(gun.clip_for_node(GunMoveRole::Fire.index()).unwrap(), &fire_clip));
        assert!(gun.reel(GunMoveRole::Idle).is_none());
        assert!(gun.clip_for_node(WEAPON_ANIM_SLOTS).is_none());
    }
}
