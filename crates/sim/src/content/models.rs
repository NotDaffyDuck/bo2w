//! The script-model part of the content digest: which models scripts have put in the world,
//! their hit boxes, and the brushes linked to them.

use super::digest::Fnv1a;
use crate::{AuthorityModelOwner, EntityCollisionCapabilities};

/// Counts here are four bytes, unlike the eight of every other list in the digest.
pub(super) fn script_models(h: &mut Fnv1a, models: &[EntityCollisionCapabilities]) {
    h.put(models.len() as u32);
    for caps in models {
        match caps.owner {
            AuthorityModelOwner::ScriptModel(id) => {
                h.put(1_u32);
                h.put(id.0);
            }
        }
        h.put(caps.epoch().digest_tag());

        match &caps.dobj {
            Some(dobj) => {
                h.put(1_u32);
                h.put_text(&dobj.current_model);
                h.put(dobj.model_revision);
                h.put(dobj.pose_revision);
                let bones = dobj.current_collision.as_ref().map(|c| c.bones.as_slice());
                let bones = bones.unwrap_or_default();
                h.put(bones.len() as u32);
                for bone in bones {
                    h.put(u32::from(bone.bone));
                    h.put(u32::from(bone.part_classification));
                    h.put_all(bone.center);
                    h.put_all(bone.axes.into_iter().flatten());
                    h.put_all(bone.half_size);
                }
            }
            None => h.put(0_u32),
        }

        h.put(caps.linked_brushes.len() as u32);
        for brush in &caps.linked_brushes {
            h.put(brush.cmodel_handle);
            h.put_all(brush.origin);
            h.put_all(brush.angles);
        }
    }
}
