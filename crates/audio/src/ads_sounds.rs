//! bo2mp: Black Ops II's aim-down-sight sounds for his own gun: the gear
//! rustle as the sights come up (`adsRaiseSoundPlayer`) and go down
//! (`adsLowerSoundPlayer`), and a scope's zoom click (`adsZoomSound`,
//! `fly_scope_zoom`) the moment its overlay snaps on.

use asset_game::WeaponAudioRole;
use assets::StagedWeapons;
use bevy::prelude::*;
use net::{HostViewClient, ShownWorldFrame};

use crate::{SOUND_SOURCE_SELF, AudioCatalogRes, GunNoiseMsg};

#[derive(Default)]
pub(crate) struct AdsSoundState {
    weapon_ix: u32,
    frac: f32,
    /// Which way the sights last moved (true: up).
    raising: bool,
}

/// What the sights did this frame: (raise started, lower started, reached
/// full ADS).
fn ads_edges(state: &mut AdsSoundState, wpn: u32, frac: f32) -> (bool, bool, bool) {
    if wpn != state.weapon_ix {
        *state = AdsSoundState {
            weapon_ix: wpn,
            frac,
            raising: frac > 0.0,
        };
        return (false, false, false);
    }
    let (mut up, mut down) = (false, false);
    if frac > state.frac + 1e-4 && !state.raising {
        state.raising = true;
        up = true;
    } else if frac + 1e-4 < state.frac && state.raising {
        state.raising = false;
        down = true;
    }
    let full = state.frac < 1.0 && frac >= 1.0;
    state.frac = frac;
    (up, down, full)
}

pub(crate) fn play_ads_sounds(
    mut state: Local<AdsSoundState>,
    shown: Res<ShownWorldFrame>,
    local: Res<HostViewClient>,
    weapons: Option<Res<StagedWeapons>>,
    bank: Option<Res<AudioCatalogRes>>,
    mut out: MessageWriter<GunNoiseMsg>,
) {
    let Some(pstate) = shown.player(local.0) else {
        *state = AdsSoundState::default();
        return;
    };
    let (up, down, full) = ads_edges(&mut state, pstate.weapon, pstate.weapon_raise_fraction);
    let (Some(weapons), Some(bank)) = (weapons.as_deref(), bank.as_deref()) else {
        return;
    };
    // Only Black Ops II's guns name these.
    if weapons.0.scope_for(pstate.weapon) != Some(asset_core::AssetCategoryScope::T6) {
        return;
    }
    let scoped = weapons.0.scope_is_iris(pstate.weapon);
    for (fire, slot) in [
        (up, WeaponAudioRole::AdsRaisePlayer),
        (down, WeaponAudioRole::AdsLowerPlayer),
        (full && scoped, WeaponAudioRole::AdsZoom),
    ] {
        if !fire {
            continue;
        }
        if let Some((cue_ns, cue_id)) = weapons.0.weapon_sound_key(pstate.weapon, slot, &bank.0) {
            out.write(GunNoiseMsg {
                cue_ns,
                alias: cue_id.to_owned(),
                position_in_inches: None,
                source_entity_no: Some(SOUND_SOURCE_SELF),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raise_lower_and_full_fire_once_each() {
        let mut s = AdsSoundState::default();
        assert_eq!(ads_edges(&mut s, 5, 0.0), (false, false, false));
        assert_eq!(ads_edges(&mut s, 5, 0.2), (true, false, false));
        assert_eq!(ads_edges(&mut s, 5, 0.6), (false, false, false));
        assert_eq!(ads_edges(&mut s, 5, 1.0), (false, false, true));
        assert_eq!(ads_edges(&mut s, 5, 1.0), (false, false, false));
        assert_eq!(ads_edges(&mut s, 5, 0.7), (false, true, false));
        assert_eq!(ads_edges(&mut s, 5, 0.0), (false, false, false));
        // A new gun is never a raise.
        assert_eq!(ads_edges(&mut s, 6, 0.5), (false, false, false));
    }
}
