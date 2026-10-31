//! The entity events a bouncing or sticking item sends to the clients, and the note it leaves for the script host.

use super::scene::{SURF_WATER, in_water, required_facts};
use super::flying::{FlyingProjectile, WeaponNote};
use crate::step_view::StepWorldView;
use crate::player_ledger::EventRecipients;
use crate::{SeatNo, HappeningBody, StepNo};
use entity_t6::EventCode;

/// Weapon file `projExplosion` of a bolt (the crossbow's bolt, the ballistic knife's blade).
pub(super) const PROJ_EXPLOSION_BOLT: i32 = 9;

/// A bounce off a surface of material `surface`, facing `normal`, at `origin`.
pub(super) fn push_bounce_event(
    world: &mut StepWorldView,
    tick: StepNo,
    missile: &FlyingProjectile,
    spot: [f32; 3],
    face: [f32; 3],
    surface: u8,
) {
    let note = HappeningBody {
        number: missile.ent_no,
        weapon: missile.gun,
        correlation: missile.id.0,
        origin: spot,
        direction: face,
        event_parm: i32::from(surface),
        surf_type: surface,
        ..Default::default()
    };
    world.post_entity_event(tick, EventRecipients::All, EventCode::BOMB_HOP, note);
}

/// What a missile stuck to: nothing special (the map), a player (and whether it is his shield), with the hit location.
#[derive(Clone, Copy, Default)]
pub(super) struct StuckTo {
    pub player: Option<SeatNo>,
    pub shield: bool,
    /// Player struck and where, for a bolt's impact event.
    pub struck: Option<(SeatNo, u8)>,
}

/// A missile that stuck where it hit, facing `normal`, on material `surface` (flesh for a player, riot shield for his
/// shield; water if it is in water). A bolt sends its impact event, which plays the weapon's impact effect and sound
/// for the surface, with the player and hit location it struck; any other item sends a bounce event on the surface.
pub(super) fn push_stick_event(
    world: &mut StepWorldView,
    tick: StepNo,
    missile: &FlyingProjectile,
    stuck: StuckTo,
    face: [f32; 3],
    surface: u8,
) {
    world.weapon_notes.push(WeaponNote::Stuck { id: missile.id, player: stuck.player, shield: stuck.shield });
    let surface = if in_water(world, missile.origin) { SURF_WATER } else { surface };
    let base = HappeningBody {
        number: missile.ent_no,
        weapon: missile.gun,
        correlation: missile.id.0,
        origin: missile.origin,
        direction: face,
        surf_type: surface,
        ..Default::default()
    };
    let is_bolt = required_facts(world, missile.gun).shell_blast_kind == PROJ_EXPLOSION_BOLT;
    if is_bolt {
        // The hit normal rides in `direction`, so the event parameter carries the hit location instead.
        let note = HappeningBody {
            other_entity_num: match stuck.struck {
                Some((client, _)) => client.0 as i32,
                None => i32::from(trace_t6::WORLD_ENTITY_SLOT),
            },
            event_parm: stuck.struck.map_or(0, |(_, hitloc)| i32::from(hitloc)),
            ..base
        };
        world.post_entity_event(tick, EventRecipients::All, EventCode::BOLT_IMPACT, note);
    } else {
        let note = HappeningBody { event_parm: i32::from(surface), ..base };
        world.post_entity_event(tick, EventRecipients::All, EventCode::BOMB_HOP, note);
    }
}
