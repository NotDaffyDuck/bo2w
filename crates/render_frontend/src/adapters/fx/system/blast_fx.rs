//! Things that blow up or bounce: a weapon's explosion (its effect, its
//! shake on nearby glass and its report), a grenade hitting a surface, and
//! the killcam switch that takes a remote missile's blast back out.

use asset_game::{FxName, SelfContainedImpactFxTable, WeaponChassisSummary, WeaponAudioRole};
use assets::StagedWeapons;
use bevy::prelude::*;
use entity_t6::event;
use fx::basis_off_contact_facing;
use net::ClientGuessState;
use render_fx::{JournalCount, ReadyHitFx};
use render_fx::battle_fx::{UNTURNED_BASIS, explosion_effect_names};

use super::stage::{FxLedger, FxStage, Pose};
use super::weapon_audio::WeaponVoices;

/// Where an effect sits on a surface: turned to the surface normal the event
/// carried, or left upright when the event carried none.
fn surface_pose(origin: [f32; 3], nrm: [f32; 3]) -> Pose {
    let axis = if nrm == [0.0; 3] {
        UNTURNED_BASIS
    } else {
        basis_off_contact_facing(nrm)
    };
    Pose { anchor: origin, turn: axis }
}

/// A blast knocks loose the window pieces within its radius, with the
/// strengths its weapon authored.
fn shake_glass(host: &mut fx::EffectSystemHost, pos: [f32; 3], facts: &WeaponChassisSummary) {
    let glass = &mut host.glass.t6;
    if facts.blast_outer_reach == 0 || glass.shards.is_empty() {
        return;
    }
    glass.explosion(
        pos,
        facts.blast_core_damage as f32,
        facts.blast_rim_damage as f32,
        facts.blast_outer_reach as f32,
    );
}

pub(super) fn explosion(
    blast: On<net::BlastAt>,
    weapons: Option<Res<StagedWeapons>>,
    impact_effect: Option<Res<ReadyHitFx>>,
    mut stage: FxStage,
    mut voices: WeaponVoices,
    mut ledger: FxLedger,
) {
    let body_data = blast.event.payload;
    let registry = weapons.as_deref().map(|prepared| &*prepared.0);
    let facts = registry.and_then(|registry| registry.summary_for(body_data.weapon));
    if let Some(facts) = &facts {
        shake_glass(stage.host(), body_data.origin, facts);
    }

    let names = explosion_effect_names(
        facts.as_ref().map(|facts| facts.impact_kind_id),
        body_data.surf_type,
        impact_effect.as_deref().and_then(|fx| fx.0.as_ref()),
        registry
            .and_then(|registry| registry.effects_for(body_data.weapon))
            .and_then(|fx| fx.explosion_present()),
    );
    let at = surface_pose(body_data.origin, body_data.direction);
    let booms = ledger.cursor.slot(JournalCount::BlastPlayed);
    let started = stage.playback().is_some_and(|mut play| {
        let on_surface = play.at_origin(names.table, at, booms);
        let on_weapon = play.at_origin(names.slot, at, booms);
        on_surface || on_weapon
    });
    if !started {
        ledger.cursor.bump(JournalCount::BlastMissed);
    }

    let report = WeaponAudioRole::ProjectileExplosion;
    let ent = audio::ent_from_number(body_data.number);
    let heard = voices
        .alias(body_data.weapon, report)
        .is_some_and(|alias| voices.say(body_data.weapon, alias, Some(body_data.origin), ent));
    if !heard {
        ledger.cursor.bump(JournalCount::BlastSoundMissed);
    }
    ledger.close_out();
}

/// The impact table row Black Ops II keeps for a grenade bouncing.
const BOUNCE_ROW: usize = 11;

/// The effect a grenade makes bouncing off a surface, if the impact table
/// names one for it.
fn bounce_effect(table: &SelfContainedImpactFxTable, surf_type: u8) -> Option<FxName<'_>> {
    let name = table
        .entries
        .get(BOUNCE_ROW)?
        .nonflesh_name(usize::from(surf_type))?;
    (!name.is_empty()).then(|| FxName::new(table.namespace(), name))
}

/// A Black Ops II grenade bouncing (or a missile splashing into water) also
/// plays the impact table's bounce effect for the surface it struck, turned
/// to the event's direction. Older games' bounces only sound.
pub(super) fn grenade_bounce(
    touch: On<net::GrenadeBounceAt>,
    impact_effect: Option<Res<ReadyHitFx>>,
    mut stage: FxStage,
) {
    if touch.event.event.0 != i32::from(event::BOMB_HOP) {
        return;
    }
    let payload = touch.event.payload;
    let Some(table) = impact_effect
        .as_deref()
        .and_then(|fx| fx.0.as_ref())
        .filter(|table| table.t6)
    else {
        return;
    };
    let Some(effect) = bounce_effect(table, payload.surf_type) else {
        return;
    };
    let Some(mut play) = stage.playback() else {
        return;
    };
    let mut began = 0;
    let at = surface_pose(payload.origin, payload.direction);
    play.at_origin(Some(effect), at, &mut began);
}

/// Weapons whose blast the killcam camera must not replay when it cuts back
/// to the shooter.
const KILLCAM_REPLAYED_BLASTS: [&str; 1] = ["remotemissile_projectile_mp"];

pub(super) fn stop_killcam_explosion_fx(
    _transition: On<net::KillcamFxTransition>,
    weapons: Option<Res<StagedWeapons>>,
    prediction: Res<ClientGuessState>,
    mut stage: FxStage,
) {
    let Some(registry) = weapons.as_deref().map(|prepared| &*prepared.0) else {
        return;
    };
    let time_slice = prediction.0.predicted_local().map_or(0, |ps| ps.time_slice);
    let Some(mut play) = stage.playback() else {
        return;
    };
    let cutoff = play.now().wrapping_sub(time_slice);
    for name in KILLCAM_REPLAYED_BLASTS {
        let Ok(Some(weapon)) = registry.follow_slot_link(name) else {
            continue;
        };
        let blast = registry
            .effects_for(weapon)
            .and_then(|fx| fx.explosion_present())
            .and_then(|authored| authored.settle(play.defs()));
        if let Some(effect) = blast {
            play.host().kill_def_newer_than(&effect.name, cutoff);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asset_game::SelfContainedImpactFxRecord;

    #[test]
    fn a_blast_without_a_surface_stands_upright() {
        assert_eq!(surface_pose([0.0; 3], [0.0; 3]).turn, UNTURNED_BASIS);
    }

    #[test]
    fn a_blast_on_a_surface_turns_to_its_normal() {
        let axis = surface_pose([0.0; 3], [0.0, 1.0, 0.0]).turn;
        assert_eq!(axis[0], [0.0, 1.0, 0.0]);
        assert_ne!(axis, UNTURNED_BASIS);
    }

    fn table_with_bounce(surf: usize, name: &str) -> SelfContainedImpactFxTable {
        let mut table = SelfContainedImpactFxTable {
            t6: true,
            ..Default::default()
        };
        table.entries = vec![SelfContainedImpactFxRecord::default(); BOUNCE_ROW + 1];
        table.entries[BOUNCE_ROW].hard_hit[surf] = name.to_owned();
        table
    }

    #[test]
    fn a_bounce_plays_the_name_its_surface_has_in_the_bounce_row() {
        let table = table_with_bounce(5, "impacts/grenade_bounce_dirt");
        let effect = bounce_effect(&table, 5).expect("named");
        assert_eq!(effect.name, "impacts/grenade_bounce_dirt");
        assert_eq!(effect.namespace, table.namespace());
    }

    #[test]
    fn a_surface_with_no_bounce_effect_stays_quiet() {
        let table = table_with_bounce(5, "impacts/grenade_bounce_dirt");
        assert!(bounce_effect(&table, 6).is_none());
    }

    #[test]
    fn a_table_too_short_for_the_bounce_row_stays_quiet() {
        let table = SelfContainedImpactFxTable {
            t6: true,
            entries: vec![SelfContainedImpactFxRecord::default(); BOUNCE_ROW],
            ..Default::default()
        };
        assert!(bounce_effect(&table, 0).is_none());
    }

    #[test]
    fn a_surface_number_past_the_table_stays_quiet() {
        let table = table_with_bounce(0, "x");
        assert!(bounce_effect(&table, u8::MAX).is_none());
    }
}
