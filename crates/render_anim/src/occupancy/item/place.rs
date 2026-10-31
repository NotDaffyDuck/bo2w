use super::*;

pub fn item_light_probe_point(org: [f32; 3]) -> [f32; 3] {
    [org[0], org[1], org[2] + ITEM_LIGHT_LIFT]
}

/// The weapon number of an item entity; zero means none.
pub(super) fn item_weapon_index(weapon: u32) -> Option<u32> {
    (weapon != 0).then_some(weapon)
}

pub(super) fn item_world_from_local(origin: [f32; 3], angs: [f32; 3]) -> Mat4 {
    let [pitch, yaw, roll] = angs.map(f32::to_radians);
    Mat4::from_rotation_translation(
        Quat::from_euler(EulerRot::ZYX, yaw, pitch, roll),
        Vec3::from_array(origin),
    )
}

pub(super) fn item_origin(ent: &EntitySnapshot, time: i32) -> [f32; 3] {
    evaluate_trajectory(&ent.lerp.pos, time)
}

/// True on the first frame of each second; throttles the debug log.
pub(super) fn first_frame_of_second(time: i32) -> bool {
    time / 1000 != (time - 16) / 1000
}

pub(super) fn is_scavenger_item(snapshot: &sim::Snapshot, entnum_in: i32) -> bool {
    snapshot
        .meta
        .item_ammo
        .iter()
        .any(|ammo| ammo.entnum == entnum_in && ammo.scavenger != 0)
}

pub(super) fn viewer_sees_scavenger_items(
    presented: &net::ShownWorldFrame,
    local: Option<&net::HostViewClient>,
) -> bool {
    local
        .and_then(|client| presented.player(client.0))
        .is_some_and(|ps| ps.has_perk_bit(PERK_SCAVENGER))
}
