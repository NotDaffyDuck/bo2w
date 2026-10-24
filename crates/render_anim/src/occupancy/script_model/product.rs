//! What posing hands to planning: the posed assets of the frame and the
//! owners that draw them.

use bevy::prelude::*;
use xmodel_runtime::DObjSemanticState;

use crate::anim::xmodel_pose::PosedModelSurface;

/// One owner's place in the frame: which posed asset it draws, and where.
pub(super) struct ScriptOwnerRow {
    pub(super) entity: Entity,
    pub(super) asset_index: usize,
    pub(super) focused_owner_id: Option<u32>,
    pub(super) model: asset_world::MapXModelAssetKey,
    pub(super) world_from_local: Mat4,
    pub(super) entnum: Option<u32>,
    pub(super) camera_hidden: bool,
    pub(super) lighting_origin: [f32; 3],
    pub(super) lookup_fallback: u8,
    pub(super) caster_bound: render_scene::XModelCasterBound,
}

/// A model posed one way: the skinned surfaces, and the material the map
/// authored for each.
pub(super) struct ScriptPosedAsset {
    pub(super) key: asset_world::MapXModelAssetKey,
    pub(super) dobj_state: DObjSemanticState,
    pub(super) camera_lods: Vec<Option<u8>>,
    pub(super) surfaces: Vec<PosedModelSurface>,
    pub(super) authored: Vec<Option<assets::MaterialIndex>>,
}

#[derive(Resource, Default)]
pub(super) struct ScriptModelPoseProduct {
    pub(super) assets: Vec<ScriptPosedAsset>,
    pub(super) rows: Vec<ScriptOwnerRow>,
    pub(super) producer_unavailable: bool,
    pub(super) surface_material_unbound: usize,
    pub(super) surface_material_first: Option<String>,
}

impl ScriptModelPoseProduct {
    /// The asset already posed this way.
    pub(super) fn asset_index(
        &self,
        key: &asset_world::MapXModelAssetKey,
        dobj_state: &DObjSemanticState,
        camera_lods: &[Option<u8>],
    ) -> Option<usize> {
        self.assets.iter().position(|asset| {
            &asset.key == key && &asset.dobj_state == dobj_state && asset.camera_lods == camera_lods
        })
    }

    /// Forgets the rows and counts of the last frame (the assets stay, to be
    /// found again).
    pub(super) fn clear_frame(&mut self) {
        self.rows.clear();
        self.producer_unavailable = false;
        self.surface_material_unbound = 0;
        self.surface_material_first = None;
    }

    /// Counts the surfaces of `model` the map authored no material for.
    pub(super) fn note_unbound(
        &mut self,
        model: &str,
        surfaces: &[PosedModelSurface],
        authored: &[Option<assets::MaterialIndex>],
    ) {
        for (surface, _) in surfaces
            .iter()
            .zip(authored)
            .filter(|(_, authored)| authored.is_none())
        {
            self.surface_material_unbound += 1;
            self.surface_material_first
                .get_or_insert_with(|| format!("{model} surface={}", surface.surface_index));
        }
    }

    /// Keeps `posed`, in the place of an asset this frame does not use if one
    /// has the same shape, else after the rest. Returns its index.
    pub(super) fn store(&mut self, live: &[usize], posed: ScriptPosedAsset) -> usize {
        let slot = recyclable_slot(&self.assets, live, |old| {
            old.key == posed.key
                && old.camera_lods == posed.camera_lods
                && topology_matches(&old.dobj_state, &posed.dobj_state)
        });
        match slot {
            Some(index) => {
                self.assets[index] = posed;
                index
            }
            None => {
                self.assets.push(posed);
                self.assets.len() - 1
            }
        }
    }
}

/// The first of `items` that `fits` and is not `live`.
pub(super) fn recyclable_slot<T>(
    items: &[T],
    live: &[usize],
    fits: impl Fn(&T) -> bool,
) -> Option<usize> {
    items
        .iter()
        .enumerate()
        .position(|(index, item)| !live.contains(&index) && fits(item))
}

/// Whether a pose made for `retained` can be redone in place for `incoming`:
/// the same models, parts, hidden parts and animation tree shape, whatever
/// the times.
pub(super) fn topology_matches(retained: &DObjSemanticState, incoming: &DObjSemanticState) -> bool {
    retained.composition == incoming.composition
        && retained.requested_parts == incoming.requested_parts
        && retained.hide_part_bits == incoming.hide_part_bits
        && match (&retained.tree, &incoming.tree) {
            (None, None) => true,
            (Some(retained), Some(incoming)) => {
                retained.definition_revision == incoming.definition_revision
                    && retained.nodes.len() == incoming.nodes.len()
                    && retained.nodes.iter().zip(&incoming.nodes).all(|(a, b)| {
                        (&a.parent, &a.kind, &a.clip, &a.parts)
                            == (&b.parent, &b.kind, &b.clip, &b.parts)
                    })
            }
            _ => false,
        }
}

/// Which of a list's entries are in use, and where each goes when the rest
/// are dropped.
#[derive(Default)]
pub(super) struct Compaction {
    keep: Vec<bool>,
    new_index: Vec<Option<usize>>,
}

impl Compaction {
    /// Marks the entries of a list of `len` named by `live`.
    pub(super) fn plan(&mut self, len: usize, live: &[usize]) {
        self.keep.clear();
        self.keep.resize(len, false);
        for &index in live {
            if let Some(used) = self.keep.get_mut(index) {
                *used = true;
            }
        }
        let mut next = 0;
        self.new_index.clear();
        self.new_index.extend(self.keep.iter().map(|&used| {
            used.then(|| {
                next += 1;
                next - 1
            })
        }));
    }

    pub(super) fn keep(&self) -> &[bool] {
        &self.keep
    }

    /// Where the entry that stood at `old` stands now, if it stays.
    pub(super) fn new_index(&self, old: usize) -> Option<usize> {
        self.new_index.get(old).copied().flatten()
    }

    /// `items` without the dropped entries.
    pub(super) fn apply<T>(&self, items: Vec<T>) -> Vec<T> {
        items
            .into_iter()
            .zip(&self.keep)
            .filter_map(|(item, &used)| used.then_some(item))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn standing(clip_time: f32) -> DObjSemanticState {
        DObjSemanticState::one_leaf("zombie".to_owned(), "idle".to_owned(), 1, 1, clip_time)
    }

    #[test]
    fn a_pose_is_redone_in_place_when_only_the_time_differs() {
        assert!(topology_matches(&standing(0.1), &standing(0.7)));
    }

    #[test]
    fn another_clip_or_another_tree_needs_a_new_pose() {
        let other = DObjSemanticState::one_leaf("zombie".to_owned(), "run".to_owned(), 1, 1, 0.1);
        assert!(!topology_matches(&standing(0.1), &other));
        let bind = DObjSemanticState::bind_pose("zombie".to_owned(), 1, 1);
        assert!(!topology_matches(&standing(0.1), &bind));
        assert!(topology_matches(&bind, &bind.clone()));
    }

    #[test]
    fn hiding_a_part_needs_a_new_pose() {
        let mut hidden = standing(0.1);
        hidden.hide_part_bits.set(3);
        assert!(!topology_matches(&standing(0.1), &hidden));
    }

    #[test]
    fn a_slot_in_use_is_not_recycled() {
        let items = [1, 2, 2, 3];
        assert_eq!(recyclable_slot(&items, &[], |&n| n == 2), Some(1));
        assert_eq!(recyclable_slot(&items, &[1], |&n| n == 2), Some(2));
        assert_eq!(recyclable_slot(&items, &[1, 2], |&n| n == 2), None);
    }

    #[test]
    fn compaction_numbers_what_stays_in_order() {
        let mut compaction = Compaction::default();
        compaction.plan(5, &[3, 1, 9]);
        assert_eq!(compaction.keep(), [false, true, false, true, false]);
        assert_eq!(compaction.new_index(1), Some(0));
        assert_eq!(compaction.new_index(3), Some(1));
        assert_eq!(compaction.new_index(2), None);
        assert_eq!(compaction.new_index(9), None);
        assert_eq!(compaction.apply(vec!['a', 'b', 'c', 'd', 'e']), vec!['b', 'd']);
    }

    #[test]
    fn a_new_plan_forgets_the_old_one() {
        let mut compaction = Compaction::default();
        compaction.plan(3, &[0, 1, 2]);
        compaction.plan(2, &[1]);
        assert_eq!(compaction.keep(), [false, true]);
        assert_eq!(compaction.new_index(2), None);
    }
}
