//! A gun going off: the muzzle flash, the brass it throws and the report.
//! The shot arrives as an `EntityWeaponFire` event; a brass-only event
//! (`EntityEjectBrass`) covers weapons that throw their shell later.

use asset_game::{FxDefinitions, FxName};
use assets::StagedWeapons;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use entity_t6::event;
use fx::EffectBoltTarget;
use net::ClientEntityRecord;
use render_fx::JournalCount;

use super::stage::{FxLedger, FxStage, Playback, Pose};
use super::viewpoint::Viewpoint;
use super::weapon_audio::WeaponVoices;
use crate::adapters::motion::gun_present::AttachTagGoals;
use crate::adapters::motion::peer_body::OtherBodyEffectTags;

/// What the event code says about the shot.
#[derive(Debug, PartialEq)]
struct Trigger {
    prior_shot: bool,
    /// 1 when the left gun of a pair fired.
    hand_side: usize,
}

fn decode_trigger(code: i32) -> Trigger {
    let code = u8::try_from(code).ok();
    Trigger {
        prior_shot: code.is_some_and(event::is_last_shot_fire),
        hand_side: usize::from(code.is_some_and(event::is_left_gun_fire)),
    }
}

/// Where a shooter's flash and brass bolts sit: the viewmodel's for the
/// player looking through it, the body's for everyone else still in the
/// snapshot.
#[derive(SystemParam)]
pub(super) struct ShooterBolts<'w, 's> {
    others: Query<'w, 's, (&'static net::ClientEntityPoseState, &'static OtherBodyEffectTags)>,
    own: Res<'w, AttachTagGoals>,
}

impl ShooterBolts<'_, '_> {
    /// The bolt a body carries, when the body is still in the snapshot.
    fn on_body(
        &self,
        shooter: Entity,
        pick: impl FnOnce(&OtherBodyEffectTags) -> Option<EffectBoltTarget>,
    ) -> Option<EffectBoltTarget> {
        let (epoch_rt, bolts) = self.others.get(shooter).ok()?;
        epoch_rt.present_in_newer_frame().then(|| pick(bolts)).flatten()
    }

    fn flash(&self, shooter: Entity, viewed: bool, hand: usize) -> Option<EffectBoltTarget> {
        match viewed {
            true => self.own.flash[hand],
            false => self.on_body(shooter, |bolts| bolts.flash),
        }
    }

    fn brass(&self, shooter: Entity, viewed: bool, hand: usize) -> Option<EffectBoltTarget> {
        match viewed {
            true => self.own.brass[hand],
            false => self.on_body(shooter, |bolts| bolts.brass),
        }
    }
}

/// The combat effects a weapon authored, if the registry has the weapon.
fn authored_fx(weapons: Option<&StagedWeapons>, weapon: u32) -> Option<&asset_game::WeaponBattleEffects> {
    weapons.and_then(|weapons| weapons.0.effects_for(weapon))
}

/// On the player's own gun, a far-view flash plays its first-person copy,
/// when the catalog has one (`asset_game::T6_FIRST_PERSON_FLASH_SCALE`).
fn first_person_flash(authored: FxName<'_>, defs: &FxDefinitions) -> Option<String> {
    let copy = asset_game::t6_first_person_flash_name(authored.name);
    defs.slot_in(authored.namespace, &copy).map(|_| copy)
}

/// Plays the muzzle flash of one shot: on the flash bolt, or, for a gun on no player (a
/// scorestreak's helicopter or turret, which has no flash bolt), where the server put the muzzle,
/// turned along the shot. Books the miss on the ledger when neither played.
fn play_muzzle(
    play: &mut Playback<'_>,
    ledger: &mut FxLedger,
    authored: Option<FxName<'_>>,
    own_gun: bool,
    flash_at: Option<EffectBoltTarget>,
    on_machine: Option<Pose>,
) {
    let own_copy = authored
        .filter(|_| own_gun)
        .and_then(|name| first_person_flash(name, play.defs()).map(|copy| (name.namespace, copy)));
    let muzzle = match &own_copy {
        Some((ns, copy)) => Some(FxName::new(*ns, copy)),
        None => authored,
    };
    let muzzle_played = ledger.cursor.slot(JournalCount::MuzzlePlayed);
    if play.bolted(muzzle, flash_at, muzzle_played) {
        ledger.cursor.bump(JournalCount::MuzzleBolted);
        return;
    }
    let spot_played = on_machine.is_some_and(|at| play.at_origin(muzzle, at, muzzle_played));
    if !spot_played {
        ledger.cursor.bump(JournalCount::MuzzleMissed);
    }
}

pub(super) fn fire_weapon(
    shoot: On<net::GunFiredAt>,
    identities: Query<&ClientEntityRecord>,
    weapons: Option<Res<StagedWeapons>>,
    view: Viewpoint,
    bolts: ShooterBolts,
    mut stage: FxStage,
    mut voices: WeaponVoices,
    mut ledger: FxLedger,
) {
    let body_data = shoot.event.payload;
    let shot = decode_trigger(shoot.event.event.0);
    let shooter = identities.get(shoot.entity).ok();
    let viewed = shooter.is_some_and(|identity| view.is_viewed_body(identity));
    // The gun of the scorestreak he rides is his own: its first-person flash
    // and its sound.
    let own_gun = viewed || view.rides(body_data.number);
    let combat_fx = authored_fx(weapons.as_deref(), body_data.weapon);
    let flash_at = bolts.flash(shoot.entity, viewed, shot.hand_side);
    let brass_at = bolts.brass(shoot.entity, viewed, shot.hand_side);

    if let Some(mut play) = stage.playback() {
        let on_machine = (flash_at.is_none() && shooter.is_some_and(|who| who.client().is_none()))
            .then(|| Pose {
                anchor: body_data.origin,
                turn: game_math::euler_to_rows(body_data.direction),
            });
        let authored = combat_fx.and_then(|fx| fx.has_muzzle_effect(own_gun));
        play_muzzle(&mut play, &mut ledger, authored, own_gun, flash_at, on_machine);
        play.shell_eject(combat_fx, viewed, shot.prior_shot, brass_at, &mut ledger);
    } else {
        ledger.cursor.bump(JournalCount::MuzzleMissed);
        ledger.cursor.bump(JournalCount::BrassMissed);
    }

    // His own gun is heard from inside his head, not from the muzzle.
    let sound_at = (!own_gun).then(|| flash_at.map_or(body_data.origin, |bolt| bolt.orientation.origin));
    let ent = audio::ent_from_number(body_data.number);
    let heard = shooter.is_some()
        && voices
            .fire_alias(body_data.weapon, shot.prior_shot, own_gun)
            .is_some_and(|alias| voices.say(body_data.weapon, alias, sound_at, ent));
    if !heard {
        ledger.cursor.bump(JournalCount::FireSoundMissed);
    }
    ledger.close_out();
}

pub(super) fn eject_brass(
    shell_casing: On<net::ShellCaseOut>,
    identities: Query<&ClientEntityRecord>,
    weapons: Option<Res<StagedWeapons>>,
    view: Viewpoint,
    bolts: ShooterBolts,
    mut stage: FxStage,
    mut ledger: FxLedger,
) {
    let body_data = shell_casing.event.payload;
    let shot = decode_trigger(shell_casing.event.event.0);
    let own_shooter = identities
        .get(shell_casing.entity)
        .ok()
        .is_some_and(|identity| view.is_local(identity));
    let combat_fx = authored_fx(weapons.as_deref(), body_data.weapon);
    let target = bolts.brass(shell_casing.entity, own_shooter, shot.hand_side);
    match stage.playback() {
        Some(mut play) => {
            play.shell_eject(combat_fx, own_shooter, shot.prior_shot, target, &mut ledger);
        }
        None => ledger.cursor.bump(JournalCount::BrassMissed),
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
                    prior_shot: event::is_last_shot_fire(code),
                    hand_side: usize::from(event::is_left_gun_fire(code)),
                },
                "event code {code}"
            );
        }
    }

    #[test]
    fn a_code_outside_a_byte_is_no_trigger_at_all() {
        let idle = Trigger {
            prior_shot: false,
            hand_side: 0,
        };
        assert_eq!(decode_trigger(-1), idle);
        assert_eq!(decode_trigger(300), idle);
    }

    fn bolt(dobj: u32) -> EffectBoltTarget {
        EffectBoltTarget {
            dobj,
            bone: 0,
            client_actor_teleport: false,
            orientation: fx::AttachPointFrame {
                origin: Vec3::ZERO.to_array(),
                axis: [Vec3::X, Vec3::Y, Vec3::Z].map(|unit| unit.to_array()),
            },
        }
    }

    #[test]
    fn the_viewed_player_shoots_from_his_viewmodel() {
        let mut world = World::new();
        let mut targets = AttachTagGoals::default();
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
        world.init_resource::<AttachTagGoals>();
        let mut state = SystemState::<ShooterBolts>::new(&mut world);
        let bolts = state.get(&world).unwrap();
        assert!(bolts.flash(Entity::PLACEHOLDER, false, 0).is_none());
        assert!(bolts.brass(Entity::PLACEHOLDER, false, 1).is_none());
    }
}
