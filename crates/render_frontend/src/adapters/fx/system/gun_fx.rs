//! A gun going off: the muzzle flash, the brass it throws and the report.
//! The shot arrives as an `EntityWeaponFire` event; a brass-only event
//! (`EntityEjectBrass`) covers weapons that throw their shell later.

use asset_game::{FxDefinitions, FxName};
use assets::PreparedWeapons;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use entity_t6::event as ev;
use fx::FxBoltTarget;
use net::CEntity;

use super::stage::{FxLedger, FxStage};
use super::viewpoint::Viewpoint;
use super::weapon_audio::WeaponVoices;
use crate::adapters::anim::fpv_present::FpvBoltTargets;
use crate::adapters::anim::remote_body::RemoteFxBolts;

/// What the event code says about the shot.
#[derive(Debug, PartialEq)]
struct Trigger {
    last_shot: bool,
    /// 1 when the left gun of a pair fired.
    hand: usize,
}

fn decode_trigger(code: i32) -> Trigger {
    let code = u8::try_from(code).ok();
    Trigger {
        last_shot: code.is_some_and(ev::is_last_shot_fire),
        hand: usize::from(code.is_some_and(ev::is_left_gun_fire)),
    }
}

/// Where a shooter's flash and brass bolts sit: the viewmodel's for the
/// player looking through it, the body's for everyone else still in the
/// snapshot.
#[derive(SystemParam)]
pub(super) struct ShooterBolts<'w, 's> {
    others: Query<'w, 's, (&'static net::CEntityRuntime, &'static RemoteFxBolts)>,
    own: Res<'w, FpvBoltTargets>,
}

impl ShooterBolts<'_, '_> {
    fn of_body(&self, shooter: Entity) -> Option<&RemoteFxBolts> {
        self.others
            .get(shooter)
            .ok()
            .filter(|(runtime, _)| runtime.in_next_snap())
            .map(|(_, bolts)| bolts)
    }

    fn flash(&self, shooter: Entity, viewed: bool, hand: usize) -> Option<FxBoltTarget> {
        if viewed {
            self.own.flash[hand]
        } else {
            self.of_body(shooter).and_then(|bolts| bolts.flash)
        }
    }

    fn brass(&self, shooter: Entity, viewed: bool, hand: usize) -> Option<FxBoltTarget> {
        if viewed {
            self.own.brass[hand]
        } else {
            self.of_body(shooter).and_then(|bolts| bolts.brass)
        }
    }
}

/// On the player's own gun, a far-view flash plays its first-person copy,
/// when the catalog has one (`asset_game::T6_FIRST_PERSON_FLASH_SCALE`).
fn first_person_flash(authored: FxName<'_>, defs: &FxDefinitions) -> Option<String> {
    let copy = asset_game::t6_first_person_flash_name(authored.name);
    defs.index_in(authored.namespace, &copy).map(|_| copy)
}

pub(super) fn fire_weapon(
    fire: On<net::EntityWeaponFire>,
    identities: Query<&CEntity>,
    weapons: Option<Res<PreparedWeapons>>,
    view: Viewpoint,
    bolts: ShooterBolts,
    mut stage: FxStage,
    mut voices: WeaponVoices,
    mut ledger: FxLedger,
) {
    let payload = fire.event.payload;
    let shot = decode_trigger(fire.event.event.0);
    let shooter = identities.get(fire.entity).ok();
    let viewed = shooter.is_some_and(|identity| view.is_viewed_body(identity));
    // The gun of the scorestreak he rides is his own: its first-person flash
    // and its sound.
    let own_gun = viewed || view.rides(payload.number);
    let combat_fx = weapons
        .as_deref()
        .and_then(|weapons| weapons.0.combat_fx_of(payload.weapon));
    let flash_at = bolts.flash(fire.entity, viewed, shot.hand);
    let brass_at = bolts.brass(fire.entity, viewed, shot.hand);

    match stage.playback() {
        Some(mut play) => {
            let authored = combat_fx.and_then(|fx| fx.flash_present(own_gun));
            let own_copy = authored
                .filter(|_| own_gun)
                .and_then(|name| first_person_flash(name, play.defs()).map(|c| (name.namespace, c)));
            let muzzle = match &own_copy {
                Some((namespace, copy)) => Some(FxName::new(*namespace, copy)),
                None => authored,
            };
            // A gun on no player (a scorestreak's helicopter or turret) has
            // no flash bolt: its flash plays where the server put the muzzle,
            // turned along the shot.
            let mounted_on_machine =
                flash_at.is_none() && shooter.is_some_and(|identity| identity.client().is_none());

            let mut played = ledger.cursor.muzzle_played;
            let on_bolt = play.bolted(muzzle, flash_at, &mut played);
            let on_spot = !on_bolt
                && mounted_on_machine
                && play.at_origin(
                    muzzle,
                    payload.origin,
                    game_math::angles_to_axis(payload.direction),
                    &mut played,
                );
            ledger.cursor.muzzle_played = played;
            if on_bolt {
                ledger.cursor.muzzle_bolted = ledger.cursor.muzzle_bolted.saturating_add(1);
            } else if !on_spot {
                ledger.cursor.muzzle_gap = ledger.cursor.muzzle_gap.saturating_add(1);
            }
            play.shell_eject(combat_fx, viewed, shot.last_shot, brass_at, &mut ledger);
        }
        None => {
            ledger.cursor.muzzle_gap = ledger.cursor.muzzle_gap.saturating_add(1);
            ledger.cursor.brass_gap = ledger.cursor.brass_gap.saturating_add(1);
        }
    }

    let heard = shooter.is_some()
        && voices
            .fire_alias(payload.weapon, shot.last_shot, own_gun)
            .is_some_and(|alias| {
                // His own gun is heard from inside his head, not from the muzzle.
                let at = (!own_gun)
                    .then(|| flash_at.map_or(payload.origin, |bolt| bolt.orientation.origin));
                voices.say(
                    payload.weapon,
                    alias,
                    at,
                    audio::ent_from_number(payload.number),
                )
            });
    if !heard {
        ledger.cursor.fire_sound_gap = ledger.cursor.fire_sound_gap.saturating_add(1);
    }
    ledger.close_out();
}

pub(super) fn eject_brass(
    brass: On<net::EntityEjectBrass>,
    identities: Query<&CEntity>,
    weapons: Option<Res<PreparedWeapons>>,
    view: Viewpoint,
    bolts: ShooterBolts,
    mut stage: FxStage,
    mut ledger: FxLedger,
) {
    let payload = brass.event.payload;
    let shot = decode_trigger(brass.event.event.0);
    let own_shooter = identities
        .get(brass.entity)
        .ok()
        .is_some_and(|identity| view.is_local(identity));
    let combat_fx = weapons
        .as_deref()
        .and_then(|weapons| weapons.0.combat_fx_of(payload.weapon));
    let target = bolts.brass(brass.entity, own_shooter, shot.hand);
    match stage.playback() {
        Some(mut play) => {
            play.shell_eject(combat_fx, own_shooter, shot.last_shot, target, &mut ledger);
        }
        None => {
            ledger.cursor.brass_gap = ledger.cursor.brass_gap.saturating_add(1);
        }
    }
    ledger.close_out();
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::SystemState;

    #[test]
    fn every_event_code_decodes_as_the_event_helpers_say() {
        for code in 0..=u8::MAX {
            assert_eq!(
                decode_trigger(i32::from(code)),
                Trigger {
                    last_shot: ev::is_last_shot_fire(code),
                    hand: usize::from(ev::is_left_gun_fire(code)),
                },
                "event code {code}"
            );
        }
    }

    #[test]
    fn a_code_outside_a_byte_is_no_trigger_at_all() {
        let idle = Trigger {
            last_shot: false,
            hand: 0,
        };
        assert_eq!(decode_trigger(-1), idle);
        assert_eq!(decode_trigger(300), idle);
    }

    fn bolt(dobj: u32) -> FxBoltTarget {
        FxBoltTarget {
            dobj,
            bone: 0,
            centity_teleport: false,
            orientation: fx::FxBoltOrientation {
                origin: [0.0; 3],
                axis: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            },
        }
    }

    #[test]
    fn the_viewed_player_shoots_from_his_viewmodel() {
        let mut world = World::new();
        let mut targets = FpvBoltTargets::default();
        targets.flash[1] = Some(bolt(11));
        targets.brass[0] = Some(bolt(22));
        world.insert_resource(targets);
        let mut state = SystemState::<ShooterBolts>::new(&mut world);
        let bolts = state.get(&world).unwrap();
        let nobody = Entity::PLACEHOLDER;
        assert_eq!(bolts.flash(nobody, true, 1).map(|t| t.dobj), Some(11));
        assert!(bolts.flash(nobody, true, 0).is_none());
        assert_eq!(bolts.brass(nobody, true, 0).map(|t| t.dobj), Some(22));
    }

    #[test]
    fn a_body_that_left_the_snapshot_has_no_bolts() {
        let mut world = World::new();
        world.init_resource::<FpvBoltTargets>();
        let mut state = SystemState::<ShooterBolts>::new(&mut world);
        let bolts = state.get(&world).unwrap();
        assert!(bolts.flash(Entity::PLACEHOLDER, false, 0).is_none());
        assert!(bolts.brass(Entity::PLACEHOLDER, false, 1).is_none());
    }
}
