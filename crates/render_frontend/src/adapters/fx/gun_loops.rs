//! bo2mp: a spinning gun's sounds (the Death Machine). Black Ops II gives it
//! no sound per shot (`fireSound` is empty); while the trigger is down it
//! plays `startSpinSound` once, then holds `spinLoopSound` and
//! `fireLoopSound`, and on letting go it stops both and plays
//! `stopSpinSound`. The `Player` forms are for the shooter's own ears.
//!
//! Firing is seen here as the stream of fire events: the first starts the
//! loops, and when none has come for a little over the gun's fire time the
//! trigger is taken as released.

use std::collections::HashMap;

use asset_game::WeaponSoundSlot;
use assets::PreparedWeapons;
use bevy::prelude::*;
use frame::ViewSubject;
use net::LocalPresentClient;

/// Slack past the gun's own fire time before the trigger counts as
/// released (fire events come in with the server's frames).
const RELEASE_SLACK_MS: f64 = 120.0;

struct Firing {
    weapon: u32,
    own: bool,
    origin: [f32; 3],
    last_ms: f64,
    hold_ms: f64,
}

#[derive(Resource, Default)]
pub(crate) struct GunLoopState(HashMap<i32, Firing>);

fn slot_alias(
    weapons: &PreparedWeapons,
    bank: &audio::SoundBank,
    weapon: u32,
    own: bool,
    npc: WeaponSoundSlot,
    plr: WeaponSoundSlot,
) -> Option<String> {
    let pick = |slot| weapons.0.weapon_sound_alias(weapon, slot, &bank.0);
    if own { pick(plr).or_else(|| pick(npc)) } else { pick(npc) }.map(str::to_owned)
}

fn spins(weapons: &PreparedWeapons, bank: &audio::SoundBank, weapon: u32) -> bool {
    [
        WeaponSoundSlot::FireLoop,
        WeaponSoundSlot::FireLoopPlayer,
        WeaponSoundSlot::SpinLoop,
        WeaponSoundSlot::SpinLoopPlayer,
    ]
    .into_iter()
    .any(|slot| weapons.0.weapon_sound_alias(weapon, slot, &bank.0).is_some())
}

fn play_once(
    out: &mut MessageWriter<audio::AliasCommand>,
    weapons: &PreparedWeapons,
    bank: &audio::SoundBank,
    number: i32,
    f: &Firing,
    npc: WeaponSoundSlot,
    plr: WeaponSoundSlot,
) {
    let Some(alias) = slot_alias(weapons, bank, f.weapon, f.own, npc, plr) else {
        return;
    };
    out.write(audio::AliasCommand::Play(audio::PlayAlias {
        namespace: weapons
            .0
            .namespace_of(f.weapon)
            .unwrap_or(asset_core::AssetNamespace::T6),
        alias,
        fallback: None,
        origin_inches: (!f.own).then_some(f.origin),
        snd_ent: audio::ent_from_number(number),
    }));
}

pub(crate) fn note_fire(
    fire: On<net::EntityWeaponFire>,
    time: Res<Time>,
    local: Res<LocalPresentClient>,
    view: Res<ViewSubject>,
    weapons: Option<Res<PreparedWeapons>>,
    bank: Option<Res<audio::SoundBank>>,
    mut state: ResMut<GunLoopState>,
    mut out: MessageWriter<audio::AliasCommand>,
) {
    let (Some(weapons), Some(bank)) = (weapons, bank) else {
        return;
    };
    let p = &fire.event.payload;
    if !spins(&weapons, &bank, p.weapon) {
        return;
    }
    let eyes = match *view {
        ViewSubject::Seat {
            focus: Some(focus), ..
        } => i32::try_from(focus).unwrap_or(-1),
        _ => i32::try_from(local.0.0).unwrap_or(-1),
    };
    let now = time.elapsed_secs_f64() * 1000.0;
    let hold_ms = weapons
        .0
        .facts_of(p.weapon)
        .map_or(100.0, |f| f64::from(f.fire_time_ms.max(0)))
        + RELEASE_SLACK_MS;
    let firing = Firing {
        weapon: p.weapon,
        own: p.number == eyes,
        origin: p.origin,
        last_ms: now,
        hold_ms,
    };
    match state.0.get_mut(&p.number) {
        Some(f) if f.weapon == p.weapon => {
            f.last_ms = now;
            f.origin = p.origin;
            f.own = firing.own;
        }
        _ => {
            play_once(
                &mut out,
                &weapons,
                &bank,
                p.number,
                &firing,
                WeaponSoundSlot::StartSpin,
                WeaponSoundSlot::StartSpinPlayer,
            );
            state.0.insert(p.number, firing);
        }
    }
}

pub(crate) fn tick(
    time: Res<Time>,
    weapons: Option<Res<PreparedWeapons>>,
    bank: Option<Res<audio::SoundBank>>,
    mut state: ResMut<GunLoopState>,
    mut loops: ResMut<audio::WeaponLoops>,
    mut out: MessageWriter<audio::AliasCommand>,
) {
    loops.0.clear();
    let (Some(weapons), Some(bank)) = (weapons, bank) else {
        state.0.clear();
        return;
    };
    let now = time.elapsed_secs_f64() * 1000.0;
    state.0.retain(|&number, f| {
        if now - f.last_ms > f.hold_ms {
            play_once(
                &mut out,
                &weapons,
                &bank,
                number,
                f,
                WeaponSoundSlot::StopSpin,
                WeaponSoundSlot::StopSpinPlayer,
            );
            return false;
        }
        true
    });
    for (&number, f) in &state.0 {
        let namespace = weapons
            .0
            .namespace_of(f.weapon)
            .unwrap_or(asset_core::AssetNamespace::T6);
        for (npc, plr) in [
            (WeaponSoundSlot::SpinLoop, WeaponSoundSlot::SpinLoopPlayer),
            (WeaponSoundSlot::FireLoop, WeaponSoundSlot::FireLoopPlayer),
        ] {
            if let Some(alias) = slot_alias(&weapons, &bank, f.weapon, f.own, npc, plr) {
                loops.0.push(audio::WeaponLoopRow {
                    owner: u32::try_from(number).unwrap_or(0),
                    namespace,
                    alias,
                    origin: (!f.own).then_some(f.origin),
                });
            }
        }
    }
}
