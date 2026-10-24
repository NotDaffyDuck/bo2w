//! Every animation a session loaded, found by namespace and name. A clip is
//! decoded from its parts the first time somebody asks for it.

use std::collections::HashMap;
use std::ops::Deref;
use std::sync::{Arc, OnceLock};

use crate::AssetNamespace;
use asset_core::{AssetEdge, AssetEdgeReason, XAnimSpace, ZoneOwner};
use crate::xanim_clip::{AnimClip, RawDeltaTrans, RawXAnimParts};

/// An animation as the zone holds it, before any clip is decoded from it.
#[derive(Clone, Debug)]
pub struct CapturedXAnim {
    pub namespace: AssetNamespace,
    pub parts: Arc<RawXAnimParts>,
    /// Black Ops II's root motion, kept aside: its clips decode without it
    /// unless asked through [`XAnimCatalog::clip_with_root_motion`].
    pub root_motion: Option<Arc<RawDeltaTrans>>,
}

/// An animation's place in the set; names compare without case.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Address {
    namespace: AssetNamespace,
    name: String,
}

impl Address {
    fn new(namespace: AssetNamespace, name: &str) -> Self {
        Self {
            namespace,
            name: name.to_ascii_lowercase(),
        }
    }
}

/// An animation with the zone that supplied it and, once asked for, its clip.
#[derive(Clone, Debug)]
struct Slot {
    address: Address,
    zone: ZoneOwner,
    anim: CapturedXAnim,
    /// Filled by the first decode; `Some(None)` means the parts do not decode.
    clip: OnceLock<Option<Arc<AnimClip>>>,
}

/// The finished set. Slots keep the order they were first added in, and a
/// slot's position in that order is its index.
#[derive(Clone, Debug, Default)]
pub struct XAnimCatalog {
    slots: Vec<Slot>,
    by_address: HashMap<Address, usize>,
}

impl XAnimCatalog {
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    pub fn index_by_name(&self, ns: AssetNamespace, name: &str) -> Option<usize> {
        self.by_address.get(&Address::new(ns, name)).copied()
    }

    pub fn get(&self, ns: AssetNamespace, name: &str) -> Option<&CapturedXAnim> {
        let slot = self.slots.get(self.index_by_name(ns, name)?)?;
        Some(&slot.anim)
    }

    pub fn name_at(&self, index: usize) -> Option<&str> {
        self.slots.get(index).map(|slot| slot.address.name.as_str())
    }

    pub fn zone_of(&self, index: usize) -> ZoneOwner {
        self.slots.get(index).map(|slot| slot.zone).unwrap_or_default()
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.slots.iter().map(|slot| slot.address.name.as_str())
    }

    /// What a weapon table's animation name points at: nothing when it names
    /// none, the animation's index when it is loaded, a miss otherwise.
    pub fn hint_edge(&self, hint: Option<&str>, ns: AssetNamespace) -> AssetEdge<XAnimSpace> {
        let Some(name) = hint.filter(|name| !name.is_empty()) else {
            return AssetEdge::Absent;
        };
        match self.index_by_name(ns, name) {
            Some(index) => AssetEdge::bind_order(index, self.zone_of(index)),
            None => AssetEdge::Unresolved(AssetEdgeReason::CatalogMiss),
        }
    }

    /// The decoded clip, kept for the next ask.
    pub fn clip_at(&self, index: usize) -> Option<Arc<AnimClip>> {
        let slot = self.slots.get(index)?;
        slot.clip
            .get_or_init(|| AnimClip::from_parts(&slot.anim.parts).ok().map(Arc::new))
            .clone()
    }

    pub fn clip(&self, ns: AssetNamespace, name: &str) -> Option<Arc<AnimClip>> {
        self.clip_at(self.index_by_name(ns, name)?)
    }

    /// A clip decoded afresh, not kept.
    pub fn decode(&self, ns: AssetNamespace, name: &str) -> Option<AnimClip> {
        AnimClip::from_parts(&self.get(ns, name)?.parts).ok()
    }

    /// The clip with its root motion, also where the kept decode goes without
    /// it (Black Ops II's clips: a climb moves the player along it).
    pub fn clip_with_root_motion(&self, ns: AssetNamespace, name: &str) -> Option<AnimClip> {
        let anim = self.get(ns, name)?;
        match anim.root_motion.as_deref() {
            Some(root) if anim.parts.delta_trans.is_none() => {
                let mut parts = (*anim.parts).clone();
                parts.delta_trans = Some(root.clone());
                AnimClip::from_parts(&parts).ok()
            }
            _ => self.clip(ns, name).map(|clip| (*clip).clone()),
        }
    }
}

/// A set still being filled. It reads as the set it will become.
#[derive(Clone, Debug, Default)]
pub struct XAnimBuild {
    catalog: XAnimCatalog,
    capture_zone: ZoneOwner,
    capture_ns: AssetNamespace,
    /// Animations a capture had to leave out because their data did not add up.
    pub capture_gaps: usize,
}

impl Deref for XAnimBuild {
    type Target = XAnimCatalog;

    fn deref(&self) -> &XAnimCatalog {
        &self.catalog
    }
}

impl XAnimBuild {
    pub fn publish(self) -> XAnimCatalog {
        self.catalog
    }

    /// The namespace `insert_captured` files animations under.
    pub fn set_capture_ns(&mut self, ns: AssetNamespace) {
        self.capture_ns = ns;
    }

    /// The zone the animations added from now on are credited to.
    pub fn set_capture_zone(&mut self, zone: ZoneOwner) {
        self.capture_zone = zone;
    }

    pub fn insert_captured(&mut self, anim: CapturedXAnim) {
        self.insert_in(self.capture_ns, anim);
    }

    /// Adds an animation under `ns`, replacing the one with the same
    /// namespace and name; a replaced animation loses its decoded clip.
    pub fn insert_in(&mut self, ns: AssetNamespace, mut anim: CapturedXAnim) {
        anim.namespace = ns;
        self.put(Slot {
            address: Address::new(ns, &anim.parts.name),
            zone: self.capture_zone,
            anim,
            clip: OnceLock::new(),
        });
    }

    /// Whether the slot added a row, as opposed to replacing one.
    fn put(&mut self, slot: Slot) -> bool {
        let catalog = &mut self.catalog;
        if let Some(&at) = catalog.by_address.get(&slot.address) {
            catalog.slots[at] = slot;
            return false;
        }
        catalog.by_address.insert(slot.address.clone(), catalog.slots.len());
        catalog.slots.push(slot);
        true
    }

    /// Takes in another build's animations, each keeping its own zone, and
    /// says how many names were new to this one.
    pub fn absorb(&mut self, other: Self) -> usize {
        self.capture_gaps = self.capture_gaps.saturating_add(other.capture_gaps);
        other
            .catalog
            .slots
            .into_iter()
            .map(|slot| self.put(slot))
            .filter(|added| *added)
            .count()
    }
}
