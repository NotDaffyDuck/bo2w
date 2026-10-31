//! The master volume applied to the sounds already playing.

use std::collections::HashMap;

use bevy::{audio::AudioSink, audio::AudioSinkPlayback, audio::GlobalVolume};
use bevy::prelude::*;
use frame::PlayerPrefsBundle;

/// A playing sound's volume before the master gain was applied to it. With
/// the old gain at zero the sink holds no information, so the volume noted
/// when it was muted stands in, or failing that the one it started with.
fn unscaled(current: f32, previous_gain: f32, remembered: Option<f32>, started_at: f32) -> f32 {
    if previous_gain > f32::EPSILON {
        current / previous_gain
    } else {
        remembered.unwrap_or(started_at)
    }
}

pub(crate) fn set_overall_loudness(
    settings: Res<PlayerPrefsBundle>,
    mut overall: ResMut<GlobalVolume>,
    // A sound with a menu volume is rescaled by the audio crate instead.
    mut sinks: Query<(Entity, &mut AudioSink, &PlaybackSettings), Without<audio::MenuVolume>>,
    mut applied_gain: Local<Option<f32>>,
    mut muted: Local<HashMap<Entity, f32>>,
) {
    let changed = settings.is_changed();
    if !changed {
        return;
    }
    let previous = applied_gain.unwrap_or(1.0);
    let gain = settings.master_gain();
    overall.volume = bevy::audio::Volume::Linear(gain);
    muted.retain(|entity, _| sinks.contains(*entity));
    for (sound, mut player, playback) in &mut sinks {
        let base = unscaled(
            player.volume().to_linear(),
            previous,
            muted.get(&sound).copied(),
            playback.volume.to_linear(),
        );
        if gain == 0.0 {
            muted.insert(sound, base);
        }
        player.set_volume(bevy::audio::Volume::Linear(base * gain));
    }
    if gain > 0.0 {
        muted.clear();
    }
    *applied_gain = Some(gain);
}

#[cfg(test)]
mod tests {
    use crate::prefs::sound::*;
    use bevy::app::Update;
    use bevy::audio::Volume;

    fn sound_app() -> App {
        let mut app = App::default();
        app.init_resource::<PlayerPrefsBundle>()
            .init_resource::<GlobalVolume>()
            .add_systems(Update, set_overall_loudness);
        app
    }

    fn global(app: &App) -> f32 {
        app.world().resource::<GlobalVolume>().volume.to_linear()
    }

    fn set_master(app: &mut App, volume: f32) {
        app.world_mut().resource_mut::<PlayerPrefsBundle>().main_level = volume;
    }

    #[test]
    fn a_sound_is_taken_back_to_its_own_volume() {
        // Playing at 0.5 of a 0.25 gain: the sound itself is 2.0.
        assert_eq!(unscaled(0.5, 0.25, None, 9.0), 2.0);
    }

    #[test]
    fn a_muted_sound_comes_back_at_the_volume_noted_for_it() {
        assert_eq!(unscaled(0.0, 0.0, Some(0.8), 9.0), 0.8);
    }

    #[test]
    fn a_sound_first_seen_muted_comes_back_at_its_starting_volume() {
        assert_eq!(unscaled(0.0, 0.0, None, 0.6), 0.6);
    }

    #[test]
    fn the_global_volume_follows_the_master_slider_squared() {
        let mut app = sound_app();
        set_master(&mut app, 0.5);
        app.update();
        assert_eq!(global(&app), 0.25);
    }

    #[test]
    fn nothing_is_touched_while_the_settings_stand_still() {
        let mut app = sound_app();
        set_master(&mut app, 0.5);
        app.update();
        app.world_mut().resource_mut::<GlobalVolume>().volume = Volume::Linear(0.9);
        app.update();
        assert_eq!(global(&app), 0.9);
        set_master(&mut app, 1.0);
        app.update();
        assert_eq!(global(&app), 1.0);
    }

    #[test]
    fn the_slider_at_zero_silences_the_global_volume() {
        let mut app = sound_app();
        set_master(&mut app, 0.0);
        app.update();
        assert_eq!(global(&app), 0.0);
    }
}
