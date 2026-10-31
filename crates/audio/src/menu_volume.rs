//! bo2mp: Black Ops II's menu volumes (Options > Sound: effects, music, voice and
//! cinematic; interface sounds follow the effects slider). A Black Ops II sound
//! carries its alias's category: it starts at its volume times that category's
//! gain (the slider squared, as the master), and moving a slider rescales the sounds
//! already playing.

use bevy::{
    audio::{AudioSink, AudioSinkPlayback, PlaybackSettings, Volume},
    prelude::*,
    transform::TransformSystems,
};

use crate::play_core::SpatialVoice;

/// A playing sound's menu category (`SndMenuCategory`) and its volume before the
/// master and menu volumes. A 3D sound's volume follows its distance every frame
/// (`Channel3d`), so only its start is scaled here.
#[derive(Component, Clone, Copy, Debug)]
pub struct MenuVolume {
    pub cat_ix: u8,
    pub base: f32,
}

/// A new sound is scaled before its sink opens: Bevy opens sinks in PostUpdate after
/// transform propagation.
pub(crate) fn wire_up(app: &mut App) {
    app.add_systems(
        PostUpdate,
        scale_new_sounds.before(TransformSystems::Propagate),
    )
    .add_systems(Update, rescale_on_settings_change.after(crate::duck::DuckSet));
}

/// A sound starts at its volume times its category's volume (the master volume
/// is applied by the global volume when its sink opens).
fn scale_new_sounds(
    settings: Res<frame::PlayerPrefsBundle>,
    ducks: Res<crate::DuckMixer>,
    mut sounds: Query<
        (&MenuVolume, &mut PlaybackSettings, Option<&crate::DuckGroup>),
        Added<MenuVolume>,
    >,
) {
    for (menu, mut playback, group) in &mut sounds {
        let scale = settings.menu_volume(Some(menu.cat_ix)) * ducks.linear(group.map(|g| g.0));
        playback.volume = Volume::Linear((playback.volume.to_linear() * scale).max(0.0));
    }
}

fn rescale_on_settings_change(
    settings: Res<frame::PlayerPrefsBundle>,
    ducks: Res<crate::DuckMixer>,
    mut sinks: Query<(&MenuVolume, &mut AudioSink, Option<&crate::DuckGroup>), Without<SpatialVoice>>,
) {
    // bo2mp: a Black Ops II duck moving its group's volume rescales them too.
    if !settings.is_changed() && !ducks.dirty {
        return;
    }
    for (menu, mut sink, group) in &mut sinks {
        let volume = menu.base
            * settings.master_gain()
            * settings.menu_volume(Some(menu.cat_ix))
            * ducks.linear(group.map(|g| g.0));
        sink.set_volume(Volume::Linear(volume.max(0.0)));
    }
}
