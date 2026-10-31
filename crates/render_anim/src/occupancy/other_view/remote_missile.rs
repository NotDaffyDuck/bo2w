use super::*;

/// The nose camera of the missile the local player is steering, if any.
pub fn remote_missile_camera(
    presented: &ShownWorldFrame,
    own_client: ClientId,
    at_time: i32,
) -> Option<WorldEyePlacement> {
    let link = presented
        .snapshot()?
        .meta
        .for_client(own_client)?
        .remote_missile
        .filter(|link| link.unlink_at_ms.is_none())?;
    let missile = presented
        .presented_projectiles()
        .iter()
        .find(|p| p.host_assigned_id() == Some(link.projectile))?;
    Some(WorldEyePlacement {
        origin: missile.point_at(at_time),
        turn: link.turn,
    })
}
