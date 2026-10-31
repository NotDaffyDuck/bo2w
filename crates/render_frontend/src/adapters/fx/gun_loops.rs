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

use asset_game::WeaponAudioRole;
use assets::StagedWeapons;
use bevy::prelude::*;
use frame::EyeSubject;
use net::HostViewClient;

/// Slack past the gun's own fire time before the trigger counts as
/// released (fire events come in with the server's frames).
const RELEASE_SLACK_MS: f64 = 120.0;

struct Firing {
    gun_no: u32,
    own: bool,
    pos: [f32; 3],
    prev_ms: f64,
    hold_ms: f64,
}

#[derive(Resource, Default)]
pub(crate) struct GunLoopState(HashMap<i32, Firing>);

/// A sound the gun has for everyone else and, when it is the shooter's own, one for his ears.
#[derive(Clone, Copy)]
struct Slots {
    others: WeaponAudioRole,
    own: WeaponAudioRole,
}

const START: Slots = Slots {
    others: WeaponAudioRole::StartSpin,
    own: WeaponAudioRole::StartSpinPlayer,
};
const STOP: Slots = Slots {
    others: WeaponAudioRole::StopSpin,
    own: WeaponAudioRole::StopSpinPlayer,
};
const LOOPS: [Slots; 2] = [
    Slots {
        others: WeaponAudioRole::SpinLoop,
        own: WeaponAudioRole::SpinLoopPlayer,
    },
    Slots {
        others: WeaponAudioRole::FireLoop,
        own: WeaponAudioRole::FireLoopPlayer,
    },
];

/// The weapon table and the sound bank the gun's aliases are looked up in.
struct Library<'a> {
    weapons: &'a StagedWeapons,
    bank: &'a audio::AudioCatalogRes,
}

impl Library<'_> {
    fn alias_of(&self, gun_no: u32, slot: WeaponAudioRole) -> Option<&str> {
        self.weapons.0.weapon_sound_alias(gun_no, slot, &self.bank.0)
    }

    fn slot_alias(&self, firing: &Firing, slots: Slots) -> Option<String> {
        let others = self.alias_of(firing.gun_no, slots.others);
        let heard = match firing.own {
            true => self.alias_of(firing.gun_no, slots.own).or(others),
            false => others,
        };
        heard.map(str::to_owned)
    }

    fn spins(&self, weapon_id: u32) -> bool {
        let loops = LOOPS.iter().flat_map(|slots| [slots.others, slots.own]);
        loops.into_iter().any(|slot| self.alias_of(weapon_id, slot).is_some())
    }

    fn namespace_of(&self, gun: u32) -> asset_core::AssetCategoryScope {
        self.weapons
            .0
            .scope_for(gun)
            .unwrap_or(asset_core::AssetCategoryScope::T6)
    }

    fn play_once(
        &self,
        out: &mut MessageWriter<audio::AliasCommand>,
        num: i32,
        firing: &Firing,
        slots: Slots,
    ) {
        let Some(alias) = self.slot_alias(firing, slots) else {
            return;
        };
        let fallback = None;
        out.write(audio::AliasCommand::Play(audio::TriggerCueMsg {
            cue_ns: self.namespace_of(firing.gun_no),
            alias,
            fallback,
            position_in_inches: (!firing.own).then_some(firing.pos),
            source_entity_no: audio::ent_from_number(num),
        }));
    }
}

/// The player number whose eyes the camera looks through.
fn eyes_of(view: &EyeSubject, local: &HostViewClient) -> i32 {
    let number = match *view {
        EyeSubject::ChairSlot {
            focus: Some(seen), ..
        } => seen,
        _ => local.0.0,
    };
    i32::try_from(number).unwrap_or(-1)
}

pub(crate) fn note_fire(
    shoot: On<net::GunFiredAt>,
    time: Res<Time>,
    local: Res<HostViewClient>,
    view: Res<EyeSubject>,
    weapons: Option<Res<StagedWeapons>>,
    bank: Option<Res<audio::AudioCatalogRes>>,
    mut state: ResMut<GunLoopState>,
    mut out: MessageWriter<audio::AliasCommand>,
) {
    let (Some(weapons), Some(bank)) = (weapons.as_deref(), bank.as_deref()) else {
        return;
    };
    let library = Library { weapons, bank };
    let shot = &shoot.event.payload;
    if !library.spins(shot.weapon) {
        return;
    }
    let now = time.elapsed_secs_f64() * 1000.0;
    let fire_time = weapons
        .0
        .summary_for(shot.weapon)
        .map_or(100.0, |facts| f64::from(facts.shot_gap_ms.max(0)));
    let firing = Firing {
        gun_no: shot.weapon,
        own: shot.number == eyes_of(&view, &local),
        pos: shot.origin,
        prev_ms: now,
        hold_ms: fire_time + RELEASE_SLACK_MS,
    };
    match state.0.get_mut(&shot.number) {
        Some(held) if held.gun_no == shot.weapon => {
            held.prev_ms = now;
            held.pos = shot.origin;
            held.own = firing.own;
        }
        _ => {
            library.play_once(&mut out, shot.number, &firing, START);
            state.0.insert(shot.number, firing);
        }
    }
}

pub(crate) fn tick(
    time: Res<Time>,
    weapons: Option<Res<StagedWeapons>>,
    bank: Option<Res<audio::AudioCatalogRes>>,
    mut state: ResMut<GunLoopState>,
    mut loops: ResMut<audio::WeaponLoops>,
    mut out: MessageWriter<audio::AliasCommand>,
) {
    loops.0.clear();
    let (Some(weapons), Some(bank)) = (weapons.as_deref(), bank.as_deref()) else {
        state.0.clear();
        return;
    };
    let library = Library { weapons, bank };
    let now = time.elapsed_secs_f64() * 1000.0;
    state.0.retain(|&number, firing| {
        let released = now - firing.prev_ms > firing.hold_ms;
        if released {
            library.play_once(&mut out, number, firing, STOP);
        }
        !released
    });
    for (&number, firing) in &state.0 {
        let namespace = library.namespace_of(firing.gun_no);
        for slots in LOOPS {
            let Some(alias) = library.slot_alias(firing, slots) else {
                continue;
            };
            loops.0.push(audio::WeaponLoopRow {
                owner: u32::try_from(number).unwrap_or(0),
                namespace,
                alias,
                origin: (!firing.own).then_some(firing.pos),
            });
        }
    }
}
