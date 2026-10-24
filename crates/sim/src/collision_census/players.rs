use crate::bullet_collision::PlayerCollisionPose;

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
    pub materialize_error: Option<String>,
}

/// Counts the bodies in `poses`; `materialize_error` is carried along as it came.
pub(crate) fn player_clip_census(poses: &[PlayerCollisionPose], materialize_error: Option<String>) -> PlayerClipCensus {
    let bone_counts = poses.iter().map(|pose| pose.bones.len() as u32);
    let has_head = |pose: &PlayerCollisionPose| {
        pose.bones
            .iter()
            .any(|bone| hud_t6::gamemsg::obituary_is_headshot(bone.part_classification))
    };
    PlayerClipCensus {
        poses: poses.len() as u32,
        with_bones: bone_counts.clone().filter(|n| *n > 0).count() as u32,
        aabb_only: bone_counts.clone().filter(|n| *n == 0).count() as u32,
        min_bones: bone_counts.clone().min().unwrap_or(0),
        max_bones: bone_counts.max().unwrap_or(0),
        with_head_bone: poses.iter().filter(|pose| has_head(pose)).count() as u32,
        materialize_error,
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
                materialize_error: Some("no model".to_owned()),
                ..Default::default()
            }
        );
    }
}
