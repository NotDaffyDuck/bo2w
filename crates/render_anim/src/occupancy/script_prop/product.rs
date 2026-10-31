//! What posing hands to planning: the posed assets of the frame and the
//! owners that draw them.

use bevy::prelude::*;
use xmodel_runtime::RigPoseRecord;

use crate::motion::figure_pose::SkinnedMeshFacet;

/// One owner's place in the frame: which posed asset it draws, and where.
pub(super) struct ScriptOwnerRow {
    pub(super) ent: Entity,
    pub(super) draw_asset_slot: usize,
    pub(super) spectated_entity_id: Option<u32>,
    pub(super) mdl: asset_world::LevelModelAssetId,
    pub(super) world_from_local: Mat4,
    pub(super) entnum: Option<u32>,
    pub(super) hidden_from_viewer: bool,
    pub(super) illum_origin: [f32; 3],
    pub(super) lookup_backup: u8,
    pub(super) caster_bound: render_scene::XModelCasterBound,
}

/// A model posed one way: the skinned surfaces, and the material the map
/// authored for each.
pub(super) struct ScriptPosedAsset {
    pub(super) key: asset_world::LevelModelAssetId,
    pub(super) puppet_state: RigPoseRecord,
    pub(super) viewer_detail_levels: Vec<Option<u8>>,
    pub(super) surfaces: Vec<SkinnedMeshFacet>,
    pub(super) authored: Vec<Option<assets::SurfaceMaterialId>>,
}

#[derive(Resource, Default)]
pub(super) struct ScriptPropPoseOutput {
    pub(super) assets: Vec<ScriptPosedAsset>,
    pub(super) rows: Vec<ScriptOwnerRow>,
    pub(super) unavailable_producer: bool,
    pub(super) surface_material_unbound: usize,
    pub(super) surface_material_first: Option<String>,
}

impl ScriptPropPoseOutput {
    /// The asset already posed this way.
    pub(super) fn draw_asset_slot(
        &self,
        wanted: &asset_world::LevelModelAssetId,
        state: &RigPoseRecord,
        detail_tiers: &[Option<u8>],
    ) -> Option<usize> {
        let same = |have: &ScriptPosedAsset| {
            have.viewer_detail_levels == detail_tiers && have.key == *wanted && have.puppet_state == *state
        };
        self.assets.iter().position(same)
    }

    /// Forgets the rows and counts of the last frame (the assets stay, to be
    /// found again).
    pub(super) fn wipe_frame(&mut self) {
        self.rows.truncate(0);
        (self.unavailable_producer, self.surface_material_unbound) = (false, 0);
        self.surface_material_first.take();
    }

    /// Counts the surfaces of `model` the map authored no material for.
    pub(super) fn note_unbound(
        &mut self,
        xmodel: &str,
        surfaces: &[SkinnedMeshFacet],
        authored: &[Option<assets::SurfaceMaterialId>],
    ) {
        let bare = surfaces.iter().zip(authored).filter(|(_, slot)| slot.is_none());
        for (surf, _) in bare {
            self.surface_material_unbound += 1;
            if self.surface_material_first.is_none() {
                self.surface_material_first = Some(format!("{xmodel} surface={}", surf.surface_slot_number));
            }
        }
    }

    /// Keeps `posed`, in the place of an asset this frame does not use if one
    /// has the same shape, else after the rest. Returns its index.
    pub(super) fn store(&mut self, live: &[usize], posed: ScriptPosedAsset) -> usize {
        let reusable = recyclable_slot(&self.assets, live, |old| {
            topology_matches(&old.puppet_state, &posed.puppet_state)
                && old.viewer_detail_levels == posed.viewer_detail_levels
                && old.key == posed.key
        });
        if let Some(index) = reusable {
            self.assets[index] = posed;
            return index;
        }
        self.assets.push(posed);
        self.assets.len() - 1
    }
}

/// The first of `items` that `fits` and is not `live`.
pub(super) fn recyclable_slot<T>(items: &[T], live: &[usize], fits: impl Fn(&T) -> bool) -> Option<usize> {
    (0..items.len()).find(|index| !live.contains(index) && fits(&items[*index]))
}

/// Whether a pose made for `retained` can be redone in place for `incoming`:
/// the same models, parts, hidden parts and animation tree shape, whatever
/// the times.
pub(super) fn topology_matches(old: &RigPoseRecord, new: &RigPoseRecord) -> bool {
    let same_shape = (&old.composition, &old.wanted_mask, &old.hidden_mask)
        == (&new.composition, &new.wanted_mask, &new.hidden_mask);
    let same_tree = match (&old.tree, &new.tree) {
        (None, None) => true,
        (Some(was), Some(is)) => {
            was.shape_epoch == is.shape_epoch
                && was.nodes.len() == is.nodes.len()
                && was.nodes.iter().zip(&is.nodes).all(|(a, b)| {
                    (&a.parent, &a.kind, &a.clip, &a.parts) == (&b.parent, &b.kind, &b.clip, &b.parts)
                })
        }
        _ => false,
    };
    same_shape && same_tree
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
        self.keep.truncate(0);
        self.keep.extend((0..len).map(|index| live.contains(&index)));
        self.new_index.truncate(0);
        let mut placed = 0usize;
        for &stays in &self.keep {
            self.new_index.push(stays.then_some(placed));
            placed += usize::from(stays);
        }
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
        let mut kept = Vec::with_capacity(items.len());
        for (item, stays) in items.into_iter().zip(self.keep.iter().copied()) {
            if stays {
                kept.push(item);
            }
        }
        kept
    }
}

mod tests {
    #![cfg(test)]
    use super::*;

    fn standing(clip_secs: f32) -> RigPoseRecord {
        RigPoseRecord::single_clip("zombie".to_owned(), "idle".to_owned(), 1, 1, clip_secs)
    }

    #[test]
    fn a_pose_is_redone_in_place_when_only_the_time_differs() {
        assert!(topology_matches(&standing(0.1), &standing(0.7)));
    }

    #[test]
    fn another_clip_or_another_tree_needs_a_new_pose() {
        let other = RigPoseRecord::single_clip("zombie".to_owned(), "run".to_owned(), 1, 1, 0.1);
        assert!(!topology_matches(&standing(0.1), &other));
        let bind = RigPoseRecord::neutral_pose("zombie".to_owned(), 1, 1);
        assert!(!topology_matches(&standing(0.1), &bind));
        assert!(topology_matches(&bind, &bind.clone()));
    }

    #[test]
    fn hiding_a_part_needs_a_new_pose() {
        let mut hidden = standing(0.1);
        hidden.hidden_mask.set(3);
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
