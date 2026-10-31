//! Which bone of a joined view each track of an animation clip drives.

use std::collections::HashMap;
use std::sync::Arc;

use asset_model::FirstPersonMeshRoster;
use xmodel_runtime::SkeletalClip;

use crate::FpvMeshSlot;
use crate::fpv_view_types::FpvAssembly;

/// For each clip and model, the model's bone for every track of the clip
/// (an index into that model's own bones), or [`FpvClipTracks::NONE`].
#[derive(Clone, Debug, Default)]
pub struct FpvClipTracks {
    tables: HashMap<(usize, FpvMeshSlot), Arc<[u16]>>,
}

impl FpvClipTracks {
    /// A track the model has no bone for.
    pub const NONE: u16 = u16::MAX;

    pub fn table(&self, reel: usize, mdl: FpvMeshSlot) -> Option<&[u16]> {
        self.tables.get(&(reel, mdl)).map(|table| &table[..])
    }

    pub fn len(&self) -> usize {
        self.tables.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tables.is_empty()
    }

    /// Matches the clip's tracks to the model's bones by name, once per
    /// clip and model. When a name repeats, the first bone with it wins.
    pub fn bind(
        &mut self,
        catalog: &FirstPersonMeshRoster,
        motion_clip_slot: usize,
        reel: &SkeletalClip,
        mdl: FpvMeshSlot,
    ) {
        let key = (motion_clip_slot, mdl);
        if self.tables.contains_key(&key) {
            return;
        }
        let Some(pose) = catalog
            .at_row(mdl.order())
            .and_then(|entry| entry.bone_rig.pose.as_ref())
        else {
            return;
        };
        let mut bone_of: HashMap<&str, u16> = HashMap::with_capacity(pose.skeleton_joint_total);
        for (bone, name) in pose.joint_labels.iter().take(pose.skeleton_joint_total).enumerate() {
            if let Ok(bone) = u16::try_from(bone) {
                bone_of.entry(name.as_str()).or_insert(bone);
            }
        }
        let table = reel
            .tracks
            .iter()
            .map(|track| bone_of.get(track.name.as_str()).copied().unwrap_or(Self::NONE))
            .collect();
        self.tables.insert(key, table);
    }
}

impl FpvAssembly {
    /// For each track of a clip, the joined skeleton's bone that plays it: the
    /// first part that has a bone for the track decides. `None` when no part
    /// has a table for the clip.
    pub fn compose_tracks(
        &self,
        reel: usize,
        tracks: &FpvClipTracks,
    ) -> Option<Vec<Option<usize>>> {
        let tables: Vec<Option<&[u16]>> = self
            .pieces
            .iter()
            .map(|part| tracks.table(reel, part.mdl))
            .collect();
        let track_count = tables.iter().flatten().next()?.len();
        let bone_for = |track: usize| {
            self.pieces.iter().zip(&tables).find_map(|(part, table)| {
                let local = *table.as_ref()?.get(track)?;
                (local != FpvClipTracks::NONE).then(|| part.joint_start_offset + usize::from(local))
            })
        };
        Some((0..track_count).map(bone_for).collect())
    }
}
