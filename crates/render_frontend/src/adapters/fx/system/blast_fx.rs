//! Things that blow up or bounce: a weapon's explosion (its effect, its
//! shake on nearby glass and its report), a grenade hitting a surface, and
//! the killcam switch that takes a remote missile's blast back out.

use asset_game::{FxName, OwnedFxImpactTable, WeaponSoundSlot};
use assets::PreparedWeapons;
use bevy::prelude::*;
use entity_t6::event as ev;
use fx::axis_from_hit_normal;
use net::ClientPredictionState;
use render_fx::PreparedImpactFx;
use render_fx::combat::{IDENTITY_AXIS, explosion_fx_names};

use super::stage::{FxLedger, FxStage};
use super::weapon_audio::WeaponVoices;

/// The orientation an effect takes on a surface: turned to the surface
/// normal the event carried, or left upright when the event carried none.
fn surface_axis(normal: [f32; 3]) -> [[f32; 3]; 3] {
    if normal == [0.0; 3] {
        IDENTITY_AXIS
    } else {
        axis_from_hit_normal(normal)
    }
}

pub(super) fn explosion(
    blast: On<net::EntityExplosion>,
    weapons: Option<Res<PreparedWeapons>>,
    impact_fx: Option<Res<PreparedImpactFx>>,
    mut stage: FxStage,
    mut voices: WeaponVoices,
    mut ledger: FxLedger,
) {
    let payload = blast.event.payload;
    let registry = weapons.as_deref().map(|prepared| &*prepared.0);
    let facts = registry.and_then(|registry| registry.facts_of(payload.weapon));

    // A blast knocks loose the window pieces within its radius, with the
    // strengths its weapon authored.
    if let Some(facts) = &facts
        && facts.explosion_radius > 0
    {
        let glass = &mut stage.host().glass.t6;
        if !glass.shards.is_empty() {
            glass.explosion(
                payload.origin,
                facts.explosion_inner_damage as f32,
                facts.explosion_outer_damage as f32,
                facts.explosion_radius as f32,
            );
        }
    }

    let names = explosion_fx_names(
        facts.as_ref().map(|facts| facts.impact_type),
        payload.surf_type,
        impact_fx.as_deref().and_then(|fx| fx.0.as_ref()),
        registry
            .and_then(|registry| registry.combat_fx_of(payload.weapon))
            .and_then(|fx| fx.explosion_present()),
    );
    let started = match stage.playback() {
        Some(mut play) => {
            let axis = surface_axis(payload.direction);
            let mut boom = ledger.cursor.boom_played;
            let on_surface = play.at_origin(names.table, payload.origin, axis, &mut boom);
            let on_weapon = play.at_origin(names.slot, payload.origin, axis, &mut boom);
            ledger.cursor.boom_played = boom;
            on_surface || on_weapon
        }
        None => false,
    };
    if !started {
        ledger.cursor.explosion_gap = ledger.cursor.explosion_gap.saturating_add(1);
    }

    let heard = voices
        .alias(payload.weapon, WeaponSoundSlot::ProjectileExplosion)
        .is_some_and(|alias| {
            voices.say(
                payload.weapon,
                alias,
                Some(payload.origin),
                audio::ent_from_number(payload.number),
            )
        });
    if !heard {
        ledger.cursor.explosion_sound_gap = ledger.cursor.explosion_sound_gap.saturating_add(1);
    }
    ledger.close_out();
}

/// The impact table row Black Ops II keeps for a grenade bouncing.
const BOUNCE_ROW: usize = 11;

/// The effect a grenade makes bouncing off a surface, if the impact table
/// names one for it.
fn bounce_effect(table: &OwnedFxImpactTable, surf_type: u8) -> Option<FxName<'_>> {
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
    contact: On<net::EntityGrenadeContact>,
    impact_fx: Option<Res<PreparedImpactFx>>,
    mut stage: FxStage,
) {
    if contact.event.event.0 != i32::from(ev::GRENADE_BOUNCE) {
        return;
    }
    let payload = contact.event.payload;
    let Some(table) = impact_fx
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
    let mut started = 0;
    play.at_origin(
        Some(effect),
        payload.origin,
        surface_axis(payload.direction),
        &mut started,
    );
}

/// Weapons whose blast the killcam camera must not replay when it cuts back
/// to the shooter.
const KILLCAM_REPLAYED_BLASTS: [&str; 1] = ["remotemissile_projectile_mp"];

pub(super) fn stop_killcam_explosion_fx(
    _transition: On<net::KillcamFxTransition>,
    weapons: Option<Res<PreparedWeapons>>,
    prediction: Res<ClientPredictionState>,
    mut stage: FxStage,
) {
    let Some(registry) = weapons.as_deref().map(|prepared| &*prepared.0) else {
        return;
    };
    let delta_time = prediction.0.predicted_local().map_or(0, |ps| ps.delta_time);
    let Some(mut play) = stage.playback() else {
        return;
    };
    let cutoff = play.now().wrapping_sub(delta_time);
    for name in KILLCAM_REPLAYED_BLASTS {
        let Ok(Some(weapon)) = registry.resolve_index(name) else {
            continue;
        };
        let blast = registry
            .combat_fx_of(weapon)
            .and_then(|fx| fx.explosion_present())
            .and_then(|authored| authored.resolve(play.defs()));
        if let Some(effect) = blast {
            play.host().kill_def_newer_than(&effect.name, cutoff);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asset_game::OwnedFxImpactEntry;

    #[test]
    fn a_blast_without_a_surface_stands_upright() {
        assert_eq!(surface_axis([0.0; 3]), IDENTITY_AXIS);
    }

    #[test]
    fn a_blast_on_a_surface_turns_to_its_normal() {
        let axis = surface_axis([0.0, 1.0, 0.0]);
        assert_eq!(axis[0], [0.0, 1.0, 0.0]);
        assert_ne!(axis, IDENTITY_AXIS);
    }

    fn table_with_bounce(surf: usize, name: &str) -> OwnedFxImpactTable {
        let mut table = OwnedFxImpactTable {
            t6: true,
            ..Default::default()
        };
        table.entries = vec![OwnedFxImpactEntry::default(); BOUNCE_ROW + 1];
        table.entries[BOUNCE_ROW].nonflesh[surf] = name.to_owned();
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
        let table = OwnedFxImpactTable {
            t6: true,
            entries: vec![OwnedFxImpactEntry::default(); BOUNCE_ROW],
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
