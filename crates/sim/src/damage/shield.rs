//! A riot shield that took part of a blast: the event that shakes its bearer's view.

use entity_t6::EntityEventKind;

use crate::frame::FrameWorld;
use crate::world::{ClientId, Tick};
use crate::{EntityEventPayload, EventAudience};

/// The event carries the unshielded damage in 26 bits.
const PARM_MAX: i32 = 0x3ff_ffff;

/// A shield bearer, where he stands, and what the blast would have done to him without the shield.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ShieldSplash {
    pub(super) id: ClientId,
    pub(super) origin: [f32; 3],
    pub(super) unshielded: i32,
}

impl ShieldSplash {
    pub(super) fn announce(self, world: &mut FrameWorld, tick: Tick) {
        shield_splash_event(world, tick, self.id, self.origin, self.unshielded);
    }
}

/// Tells everyone that the shield `id` carries took a share of a blast; the bearer's view shakes harder the more
/// damage (`unshielded`) the shield saved him.
pub(crate) fn shield_splash_event(world: &mut FrameWorld, tick: Tick, id: ClientId, origin: [f32; 3], unshielded: i32) {
    if std::env::var_os("BO2MP_SHIELDLOG").is_some() {
        diag::info!(Sim, "bo2mp shield: splash event for {} unshielded {unshielded}", id.0);
    }
    world.push_entity_event(
        tick,
        EventAudience::All,
        EntityEventKind::EXPLOSIVE_SPLASH_ON_SHIELD,
        splash_payload(id, origin, unshielded),
    );
}

fn splash_payload(id: ClientId, origin: [f32; 3], unshielded: i32) -> EntityEventPayload {
    EntityEventPayload {
        number: id.0 as i32,
        event_parm: unshielded.clamp(0, PARM_MAX),
        origin,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_payload_names_the_bearer_where_he_stands_and_what_was_saved() {
        let payload = splash_payload(ClientId(3), [1.0, 2.0, 3.0], 120);
        assert_eq!(payload.number, 3);
        assert_eq!(payload.event_parm, 120);
        assert_eq!(payload.origin, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn saved_damage_is_held_between_zero_and_what_26_bits_can_say() {
        let parm = |unshielded| splash_payload(ClientId(0), [0.0; 3], unshielded).event_parm;
        assert_eq!(parm(-5), 0);
        assert_eq!(parm(0), 0);
        assert_eq!(parm(PARM_MAX), PARM_MAX);
        assert_eq!(parm(PARM_MAX + 1), PARM_MAX);
        assert_eq!(parm(i32::MAX), PARM_MAX);
        assert_eq!(PARM_MAX, (1 << 26) - 1);
    }
}
