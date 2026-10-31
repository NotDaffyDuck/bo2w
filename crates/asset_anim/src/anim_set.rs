//! Every animation a session loaded, found by namespace and name. A clip is
//! decoded from its parts the first time somebody asks for it.

use std::collections::HashMap;
use std::ops::Deref;
use std::sync::{Arc, OnceLock};

use crate::AssetCategoryScope;
use asset_core::{AssetDependencyLink, DependencyCause, XAnimRange, ZoneCustodian};
use crate::motion_clip::{SkeletalClip, StoredRootMotion, StoredClipParts};

/// An animation as the zone holds it, before any clip is decoded from it.
#[derive(Clone, Debug)]
pub struct XAnimCapture {
    pub namespace: AssetCategoryScope,
    pub pieces: Arc<StoredClipParts>,
    /// Black Ops II's root motion, kept aside: its clips decode without it
    /// unless asked through [`MotionClipRoster::clip_with_root_motion`].
    pub root_motion: Option<Arc<StoredRootMotion>>,
}

/// An animation's place in the set; names compare without case.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Address {
    namespace: AssetCategoryScope,
    name: String,
}

impl Address {
    fn new(namespace: AssetCategoryScope, anim_key: &str) -> Self {
        Self {
            namespace,
            name: anim_key.to_ascii_lowercase(),
        }
    }
}

/// An animation with the zone that supplied it and, once asked for, its clip.
#[derive(Clone, Debug)]
struct Slot {
    address: Address,
    zone: ZoneCustodian,
    anim: XAnimCapture,
    /// Filled by the first decode; `Some(None)` means the parts do not decode.
    reel: OnceLock<Option<Arc<SkeletalClip>>>,
}

/// The finished set. Slots keep the order they were first added in, and a
/// slot's position in that order is its index.
#[derive(Clone, Debug, Default)]
pub struct MotionClipRoster {
    ordered_caps: Vec<Slot>,
    by_address: HashMap<Address, usize>,
}

impl MotionClipRoster {
    pub fn len(&self) -> usize {
        self.ordered_caps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ordered_caps.is_empty()
    }

    pub fn slot_for_label(&self, realm: AssetCategoryScope, key_name: &str) -> Option<usize> {
        self.by_address.get(&Address::new(realm, key_name)).copied()
    }

    pub fn get(&self, from_ns: AssetCategoryScope, get_name: &str) -> Option<&XAnimCapture> {
        let slot = self.ordered_caps.get(self.slot_for_label(from_ns, get_name)?)?;
        Some(&slot.anim)
    }

    pub fn label_at(&self, index: usize) -> Option<&str> {
        self.ordered_caps.get(index).map(|slot| slot.address.name.as_str())
    }

    pub fn source_zone(&self, index: usize) -> ZoneCustodian {
        self.ordered_caps.get(index).map(|slot| slot.zone).unwrap_or_default()
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.ordered_caps.iter().map(|slot| slot.address.name.as_str())
    }

    /// What a weapon table's animation name points at: nothing when it names
    /// none, the animation's index when it is loaded, a miss otherwise.
    pub fn hint_edge(&self, hint_name: Option<&str>, hint_ns: AssetCategoryScope) -> AssetDependencyLink<XAnimRange> {
        let Some(name) = hint_name.filter(|name| !name.is_empty()) else {
            return AssetDependencyLink::NothingThere;
        };
        match self.slot_for_label(hint_ns, name) {
            Some(index) => AssetDependencyLink::bind_order(index, self.source_zone(index)),
            None => AssetDependencyLink::Unresolved(DependencyCause::IndexLookupFailure),
        }
    }

    /// The decoded clip, kept for the next ask.
    pub fn node_clip(&self, index: usize) -> Option<Arc<SkeletalClip>> {
        let slot = self.ordered_caps.get(index)?;
        slot.reel
            .get_or_init(|| SkeletalClip::from_parts(&slot.anim.pieces).ok().map(Arc::new))
            .clone()
    }

    pub fn clip(&self, clip_ns: AssetCategoryScope, clip_name: &str) -> Option<Arc<SkeletalClip>> {
        self.node_clip(self.slot_for_label(clip_ns, clip_name)?)
    }

    /// A clip decoded afresh, not kept.
    pub fn decode_fresh(&self, fresh_ns: AssetCategoryScope, fresh_name: &str) -> Option<SkeletalClip> {
        SkeletalClip::from_parts(&self.get(fresh_ns, fresh_name)?.pieces).ok()
    }

    /// The clip with its root motion, also where the kept decode goes without
    /// it (Black Ops II's clips: a climb moves the player along it).
    pub fn clip_with_root_motion(&self, root_ns: AssetCategoryScope, root_name: &str) -> Option<SkeletalClip> {
        let anim = self.get(root_ns, root_name)?;
        match anim.root_motion.as_deref() {
            Some(root) if anim.pieces.delta_trans.is_none() => {
                let mut parts = (*anim.pieces).clone();
                parts.delta_trans = Some(root.clone());
                SkeletalClip::from_parts(&parts).ok()
            }
            _ => self.clip(root_ns, root_name).map(|clip| (*clip).clone()),
        }
    }
}

/// A set still being filled. It reads as the set it will become.
#[derive(Clone, Debug, Default)]
pub struct XAnimBuild {
    roster: MotionClipRoster,
    zone_credit: ZoneCustodian,
    snapshot_nanos: AssetCategoryScope,
    /// Animations a capture had to leave out because their data did not add up.
    pub missed_frames: usize,
}

impl Deref for XAnimBuild {
    type Target = MotionClipRoster;

    fn deref(&self) -> &MotionClipRoster {
        &self.roster
    }
}

impl XAnimBuild {
    pub fn publish(self) -> MotionClipRoster {
        self.roster
    }

    /// The namespace `insert_captured` files animations under.
    pub fn file_under(&mut self, next_ns: AssetCategoryScope) {
        self.snapshot_nanos = next_ns;
    }

    /// The zone the animations added from now on are credited to.
    pub fn credit_zone(&mut self, zone: ZoneCustodian) {
        self.zone_credit = zone;
    }

    pub fn add_capture(&mut self, anim: XAnimCapture) {
        self.put_into_namespace(self.snapshot_nanos, anim);
    }

    /// Adds an animation under `into_ns`, replacing the one with the same
    /// namespace and name; a replaced animation loses its decoded clip.
    pub fn put_into_namespace(&mut self, into_ns: AssetCategoryScope, mut anim: XAnimCapture) {
        anim.namespace = into_ns;
        self.put(Slot {
            address: Address::new(into_ns, &anim.pieces.name),
            zone: self.zone_credit,
            anim,
            reel: OnceLock::new(),
        });
    }

    /// Whether the slot added a row, as opposed to replacing one.
    fn put(&mut self, bay: Slot) -> bool {
        let catalog = &mut self.roster;
        if let Some(&at) = catalog.by_address.get(&bay.address) {
            catalog.ordered_caps[at] = bay;
            return false;
        }
        catalog.by_address.insert(bay.address.clone(), catalog.ordered_caps.len());
        catalog.ordered_caps.push(bay);
        true
    }

    /// Takes in another build's animations, each keeping its own zone, and
    /// says how many names were new to this one.
    pub fn merge_in(&mut self, incoming: Self) -> usize {
        self.missed_frames = self.missed_frames.saturating_add(incoming.missed_frames);
        incoming
            .roster
            .ordered_caps
            .into_iter()
            .map(|slot| self.put(slot))
            .filter(|added| *added)
            .count()
    }
}
