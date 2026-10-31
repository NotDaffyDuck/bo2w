//! A flashbang going off.

use crate::step_view::StepWorldView;
use crate::sim_parts::SeatNo;

/// A flashbang at `origin` thrown by `attacker`; the map's own scripts decide who is blinded and by how much.
pub(crate) fn blind_players_in_flash(
    world: &mut StepWorldView,
    spot: [f32; 3],
    far_radius: f32,
    near_radius: f32,
    attacker: SeatNo,
) {
    crate::zombie_rt::flashbang::blast(world.ecs(), spot, far_radius, near_radius, attacker);
}
