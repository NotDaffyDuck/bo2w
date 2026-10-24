//! Which bone of a joined view each track of an animation clip drives.

use std::collections::HashMap;
use std::sync::Arc;

use asset_model::FpvMeshCatalog;
use xmodel_runtime::AnimClip;

use crate::FpvMeshIndex;
use crate::fpv_view_types::FpvAssembly;

/// For each clip and model, the model's bone for every track of the clip
/// (an index into that model's own bones), or [`FpvClipTracks::NONE`].
#[derive(Clone, Debug, Default)]
pub struct FpvClipTracks {
    tables: HashMap<(usize, FpvMeshIndex), Arc<[u16]>>,
}

impl FpvClipTracks {
    /// A track the model has no bone for.
    pub const NONE: u16 = u16::MAX;

    pub fn table(&self, clip: usize, model: FpvMeshIndex) -> Option<&[u16]> {
        self.tables.get(&(clip, model)).map(|table| &table[..])
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
        catalog: &FpvMeshCatalog,
        clip_index: usize,
        clip: &AnimClip,
        model: FpvMeshIndex,
    ) {
        let key = (clip_index, model);
        if self.tables.contains_key(&key) {
            return;
        }
        let Some(pose) = catalog
            .get_at(model.order())
            .and_then(|entry| entry.skel.pose.as_ref())
        else {
            return;
        };
        let mut bone_of: HashMap<&str, u16> = HashMap::with_capacity(pose.num_bones);
        for (bone, name) in pose.bone_names.iter().take(pose.num_bones).enumerate() {
            if let Ok(bone) = u16::try_from(bone) {
                bone_of.entry(name.as_str()).or_insert(bone);
            }
        }
        let table = clip
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
        clip: usize,
        tracks: &FpvClipTracks,
    ) -> Option<Vec<Option<usize>>> {
        let tables: Vec<Option<&[u16]>> = self
            .parts
            .iter()
            .map(|part| tracks.table(clip, part.model))
            .collect();
        let track_count = tables.iter().flatten().next()?.len();
        let bone_for = |track: usize| {
            self.parts.iter().zip(&tables).find_map(|(part, table)| {
                let local = *table.as_ref()?.get(track)?;
                (local != FpvClipTracks::NONE).then(|| part.bone_base + usize::from(local))
            })
        };
        Some((0..track_count).map(bone_for).collect())
    }
}
