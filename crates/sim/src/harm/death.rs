//! What dying does to the victim's view: he drops, and his camera turns to face his killer.

use playerstate_t6::game::{MOVE_KIND_DEAD, look_at_killer_yaw};

use crate::step_view::StepWorldView;
use crate::sim_parts::SeatNo;

/// The link flag that puts a player's view on something other than himself (the Lodestar's camera).
const LINK_WEAPONVIEW_ONLY: u32 = 4;

/// A player who drives a remote vehicle or whose view is linked away keeps his own view when he dies; everyone else
/// is turned to look at his killer.
fn keeps_own_view(attach_bits: u32, in_remote_vehicle: bool) -> bool {
    attach_bits & LINK_WEAPONVIEW_ONLY != 0 || in_remote_vehicle
}

/// Puts the victim in the dead state. Unless his view is not his to turn, he is also leveled out and set to look at
/// `attacker` (nobody, if he killed himself or the killer is gone).
pub(crate) fn play_death(world: &mut StepWorldView, victim: SeatNo, attacker: Option<SeatNo>) {
    let Some(before) = world.avatar(victim).copied() else {
        return;
    };
    let own_view = keeps_own_view(before.attach_bits, crate::zombie_rt::in_remote_vehicle(world.ecs(), victim.0));
    let toward_killer = (!own_view).then(|| {
        let killer_at = attacker.filter(|&id| id != victim).and_then(|id| world.avatar(id)).map(|ps| ps.origin);
        look_at_killer_yaw(killer_at, before.origin, before.look_angles[1])
    });

    if let Some(dying) = world.occupant_mut(victim) {
        dying.movement_mode = MOVE_KIND_DEAD;
        if !own_view {
            dying.look_angles = [0.0, before.look_angles[1], 0.0];
        }
    }
    if let Some(yaw) = toward_killer {
        let record = world.client_meta_mut(victim);
        record.killer_turn = yaw;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ordinary_player_is_turned_to_his_killer() {
        assert!(!keeps_own_view(0, false));
        assert!(!keeps_own_view(!LINK_WEAPONVIEW_ONLY, false));
    }

    #[test]
    fn a_linked_view_or_a_remote_vehicle_keeps_the_players_own_view() {
        assert!(keeps_own_view(LINK_WEAPONVIEW_ONLY, false));
        assert!(keeps_own_view(LINK_WEAPONVIEW_ONLY | 1, false));
        assert!(keeps_own_view(0, true));
        assert!(keeps_own_view(LINK_WEAPONVIEW_ONLY, true));
    }
}
