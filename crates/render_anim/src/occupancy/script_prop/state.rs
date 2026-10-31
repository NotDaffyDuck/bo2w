//! What the script-model systems keep from one frame to the next.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::prelude::*;
use xmodel_runtime::{PosedSkeleton, SkeletonCacheKey, RigPoseRecord};

use super::skels::gather_shown_models;
use super::wheel_rig::{WheelRig, WheelTrack, wheel_rig};
use super::{HeliRotors, TurretGun, VehicleWheels, has_wheels};
use crate::motion::model_materials::PreparedModelMaterials;

/// The DObj built for one owner, and the key it was built under.
pub(super) struct LongLivedScriptSkeleton {
    pub(super) dobj: Arc<PosedSkeleton>,
    pub(super) reuse_key: SkeletonCacheKey,
    /// bo2zm: the wheels of a model with wheel tags that `VehicleWheels`
    /// does not drive, and how far they have turned.
    pub(super) rig: Option<WheelRig>,
    pub(super) track: WheelTrack,
}

/// Per-owner state, keyed by the owner's wire id (`by_id` and the vehicle
/// drivers) or by its map-entity ordinal (`mover_angles`).
#[derive(Resource, Default)]
pub(super) struct ScriptPropSkeletons {
    pub(super) by_id: HashMap<u32, LongLivedScriptSkeleton>,
    /// Angles each mover was placed at this frame; a mover absent here had
    /// no centity to be placed by.
    pub(super) mover_angles: HashMap<u32, [f32; 3]>,
    pub(super) wheels: HashMap<u32, VehicleWheels>,
    pub(super) rotors: HashMap<u32, HeliRotors>,
    pub(super) turrets: HashMap<u32, TurretGun>,
}

/// Two states build the same DObj when they stack the same models on the
/// same tags.
pub(super) fn reuse_key_for(state: &RigPoseRecord) -> SkeletonCacheKey {
    // bo2zm: a weapon part's place on its tag counts too.
    let offsets: Vec<String> = state
        .composition
        .models
        .iter()
        .map(|model| {
            model.tag_offset.map_or_else(String::new, |at| {
                format!("{:?}{:?}", at.offset(), at.rotation())
            })
        })
        .collect::<_>();
    let parts: Vec<&str> = state
        .composition
        .models
        .iter()
        .zip(&offsets)
        .flat_map(|(model, offset)| {
            [
                model.model.as_str(),
                model.mount_tag.as_deref().unwrap_or(""),
                offset.as_str(),
            ]
        })
        .collect::<_>();
    SkeletonCacheKey {
        entity_kind: i32::from(entity_t6::et::SCRIPTMOVER),
        model: xmodel_runtime::model_token(&parts),
    }
}

impl ScriptPropSkeletons {
    /// The DObj for owner `id` under `state`: the one already held if it was
    /// built for the same models, else a new one (a lone model's comes
    /// prebuilt from `prepared`). `None` when the models are not all known.
    pub(super) fn dobj_for(
        &mut self,
        id: u32,
        catalog: &asset_world::LevelModelPlacementRoster,
        state: &RigPoseRecord,
        prepared: &PreparedModelMaterials,
    ) -> Option<Arc<PosedSkeleton>> {
        let wanted = reuse_key_for(state);
        if let Some(held) = self
            .by_id
            .get(&id)
            .filter(|held| xmodel_runtime::reuse_matches(held.reuse_key, wanted))
        {
            return Some(Arc::clone(&held.dobj));
        }
        let (specs, _) = gather_shown_models(catalog, state)?;
        let prebuilt = match state.composition.models.as_slice() {
            [only] => prepared.stage_puppet(&only.model).cloned(),
            _ => None,
        };
        // bo2zm: weapon parts sit at their offset and turn from their tag.
        let root_binds: Vec<_> = state
            .composition
            .models
            .iter()
            .map(|d| d.tag_offset.map(xmodel_runtime::TagOffset::root_bind))
            .collect::<_>();
        let obj = match prebuilt {
            Some(obj) => obj,
            None => Arc::new(PosedSkeleton::build_with_root_binds(&specs, &root_binds).ok()?),
        };
        let rig = if has_wheels(&obj) { None } else { wheel_rig(&obj) };
        self.by_id.insert(
            id,
            LongLivedScriptSkeleton {
                dobj: Arc::clone(&obj),
                reuse_key: wanted,
                rig,
                track: WheelTrack::default(),
            },
        );
        Some(obj)
    }

    /// Forgets everything about owners that were not posed this frame.
    pub(super) fn keep_only(&mut self, live_in: &std::collections::HashSet<u32>) {
        self.by_id.retain(|id, _| live_in.contains(id));
        let Self {
            by_id,
            wheels,
            rotors,
            turrets,
            ..
        } = self;
        wheels.retain(|id, _| by_id.contains_key(id));
        rotors.retain(|id, _| by_id.contains_key(id));
        turrets.retain(|id, _| by_id.contains_key(id));
    }
}

mod tests {
    #![cfg(test)]
    use super::*;
    use xmodel_runtime::{RigMeshEntry, RigPoseRecord};

    fn stacked(models: &[(&str, Option<&str>)]) -> RigPoseRecord {
        let mut state = RigPoseRecord::neutral_pose(models[0].0.to_owned(), 1, 1);
        state.composition.models = models
            .iter()
            .enumerate()
            .map(|(ix, (m, tag))| RigMeshEntry {
                model: (*m).to_owned(),
                host_index: tag.map(|_| 0).filter(|_| ix > 0),
                mount_tag: tag.map(str::to_owned),
                no_hits: false,
                tag_offset: None,
            })
            .collect::<_>();
        state
    }

    fn same(a: &RigPoseRecord, b: &RigPoseRecord) -> bool {
        xmodel_runtime::reuse_matches(reuse_key_for(a), reuse_key_for(b))
    }

    #[test]
    fn the_same_models_on_the_same_tags_share_a_dobj() {
        let a = stacked(&[("body", None), ("gun", Some("tag_weapon"))]);
        let mut b = a.clone();
        b.pose_epoch += 7;
        assert!(same(&a, &b));
    }

    #[test]
    fn another_model_or_another_tag_needs_its_own_dobj() {
        let base = stacked(&[("body", None), ("gun", Some("tag_weapon"))]);
        assert!(!same(&base, &stacked(&[("body", None), ("rifle", Some("tag_weapon"))])));
        assert!(!same(&base, &stacked(&[("body", None), ("gun", Some("tag_flash"))])));
        assert!(!same(&base, &stacked(&[("body", None)])));
    }

    #[test]
    fn keeping_only_the_live_owners_drops_their_drivers_too() {
        let mut dobjs = ScriptPropSkeletons::default();
        for id in [1, 2] {
            dobjs.by_id.insert(
                id,
                LongLivedScriptSkeleton {
                    dobj: Arc::new(PosedSkeleton::build(&[]).expect("empty dobj")),
                    reuse_key: reuse_key_for(&stacked(&[("body", None)])),
                    rig: None,
                    track: WheelTrack::default(),
                },
            );
            dobjs.turrets.insert(id, TurretGun::new());
        }
        dobjs.turrets.insert(9, TurretGun::new());
        dobjs.keep_only(&[2].into_iter().collect());
        assert_eq!(dobjs.by_id.keys().copied().collect::<Vec<_>>(), vec![2]);
        assert_eq!(dobjs.turrets.keys().copied().collect::<Vec<_>>(), vec![2]);
    }
}
