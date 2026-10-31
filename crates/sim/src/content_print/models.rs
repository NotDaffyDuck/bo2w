//! The script-model part of the content digest: which models scripts have put in the world,
//! their hit boxes, and the brushes linked to them.

use super::digest::Fnv1a;
use crate::{ServerModelSource, EntityHitAbilities};

/// Counts here are four bytes, unlike the eight of every other list in the digest.
pub(super) fn script_models(h: &mut Fnv1a, models: &[EntityHitAbilities]) {
    h.put(models.len() as u32);
    for caps in models {
        match caps.owner {
            ServerModelSource::ScriptModel(id) => {
                h.put(1_u32);
                h.put(id.0);
            }
        }
        h.put(caps.epoch().fingerprint_code());

        match &caps.dobj {
            Some(dobj) => {
                h.put(1_u32);
                h.put_text(&dobj.active_mesh);
                h.put(dobj.model_version_stamp);
                h.put(dobj.pose_rev);
                let bones = dobj.live_hitbox.as_ref().map(|c| c.bones.as_slice());
                let bones = bones.unwrap_or_default();
                h.put(bones.len() as u32);
                for joint in bones {
                    h.put(u32::from(joint.bone));
                    h.put(u32::from(joint.body_part_category));
                    h.put_all(joint.center);
                    h.put_all(joint.axes.into_iter().flatten());
                    h.put_all(joint.half_ext);
                }
            }
            None => h.put(0_u32),
        }

        h.put(caps.attached_solids.len() as u32);
        for brush in &caps.attached_solids {
            h.put(brush.collision_model_ref);
            h.put_all(brush.origin);
            h.put_all(brush.turn);
        }
    }
}
