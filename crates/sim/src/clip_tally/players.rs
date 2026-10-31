use crate::shot_trace::PlayerHitboxPose;

/// How the player bodies shots can hit are built.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlayerClipCensus {
    pub poses: u32,
    /// Bodies with bone boxes.
    pub with_bones: u32,
    /// Bodies that are only the plain player box.
    pub aabb_only: u32,
    pub min_bones: u32,
    pub max_bones: u32,
    /// Bodies with a bone box that counts as a head.
    pub with_head_bone: u32,
    /// Why the poses could not be built, if they could not.
    pub bake_failure: Option<String>,
}

/// Counts the bodies in `poses`; `bake_failure` is carried along as it came.
pub(crate) fn player_clip_census(poses: &[PlayerHitboxPose], bake_failure: Option<String>) -> PlayerClipCensus {
    let bone_counts = poses.iter().map(|pose| pose.joints.len() as u32);
    let has_head = |pose: &PlayerHitboxPose| {
        pose.joints
            .iter()
            .any(|bone| hud_t6::gamemsg::kill_notice_was_skull(bone.body_part_category))
    };
    PlayerClipCensus {
        poses: poses.len() as u32,
        with_bones: bone_counts.clone().filter(|n| *n > 0).count() as u32,
        aabb_only: bone_counts.clone().filter(|n| *n == 0).count() as u32,
        min_bones: bone_counts.clone().min().unwrap_or(0),
        max_bones: bone_counts.max().unwrap_or(0),
        with_head_bone: poses.iter().filter(|pose| has_head(pose)).count() as u32,
        bake_failure,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_bodies_count_as_zero_everywhere() {
        let census = player_clip_census(&[], Some("no model".to_owned()));
        assert_eq!(
            census,
            PlayerClipCensus {
                bake_failure: Some("no model".to_owned()),
                ..Default::default()
            }
        );
    }
}
