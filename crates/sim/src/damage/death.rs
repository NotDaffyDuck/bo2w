//! What dying does to the victim's view: he drops, and his camera turns to face his killer.

use playerstate_t6::game::{PM_TYPE_DEAD, look_at_killer_yaw};

use crate::frame::FrameWorld;
use crate::world::ClientId;

/// The link flag that puts a player's view on something other than himself (the Lodestar's camera).
const LINK_WEAPONVIEW_ONLY: u32 = 4;

/// A player who drives a remote vehicle or whose view is linked away keeps his own view when he dies; everyone else
/// is turned to look at his killer.
fn keeps_own_view(link_flags: u32, in_remote_vehicle: bool) -> bool {
    link_flags & LINK_WEAPONVIEW_ONLY != 0 || in_remote_vehicle
}

/// Puts the victim in the dead state. Unless his view is not his to turn, he is also leveled out and set to look at
/// `attacker` (nobody, if he killed himself or the killer is gone).
pub(crate) fn play_death(world: &mut FrameWorld, victim: ClientId, attacker: Option<ClientId>) {
    let Some(before) = world.player(victim).copied() else {
        return;
    };
    let own_view = keeps_own_view(before.link_flags, crate::t6::in_remote_vehicle(world.ecs(), victim.0));
    let toward_killer = (!own_view).then(|| {
        let killer_at = attacker.filter(|&id| id != victim).and_then(|id| world.player(id)).map(|ps| ps.origin);
        look_at_killer_yaw(killer_at, before.origin, before.viewangles[1])
    });

    if let Some(ps) = world.player_mut(victim) {
        ps.pm_type = PM_TYPE_DEAD;
        if !own_view {
            ps.viewangles = [0.0, before.viewangles[1], 0.0];
        }
    }
    if let Some(yaw) = toward_killer {
        world.client_meta_mut(victim).look_at_killer_yaw = yaw;
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
