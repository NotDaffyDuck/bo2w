//! The Special Weapons tab's guest weapons' own sounds on a plain map
//! (Half-Life 2's Gravity Gun, Minecraft's bow and stick): sound files
//! read by the world side, decoded once each and played at the ear or as
//! world one-shots (a linear falloff, MW2's panning).
use std::collections::HashMap;
use std::sync::Arc;

use bevy::audio::{PlaybackSettings, Volume};
use bevy::prelude::*;

use crate::pcm::PcmAudio;
use crate::playback::{AmbientListener, world_oneshot_channel_gains};

/// One sound to play.
pub struct GuestSoundRequest {
    /// The sound file's id, which its decode is cached under.
    pub key: String,
    /// The file's bytes (WAV or Ogg Vorbis); none stops `hold`.
    pub bytes: Arc<[u8]>,
    pub volume: f32,
    pub pitch: f32,
    /// A held sound (the Gravity Gun's hum): it loops under this name, in
    /// place of one already playing under it.
    pub hold: Option<&'static str>,
}

impl GuestSoundRequest {
    /// Stops the held sound `name`.
    pub fn stop(name: &'static str) -> Self {
        Self { key: String::new(), bytes: Arc::from(Vec::new()), volume: 0.0, pitch: 1.0, hold: Some(name) }
    }
}

/// Sounds to play this frame: at the ear, or at a map point (units).
#[derive(Resource, Default)]
pub struct GuestSoundQueue(pub Vec<(GuestSoundRequest, Option<Vec3>)>);

/// A held sound playing, by its name.
#[derive(Component)]
struct GuestHeld(&'static str);

/// The files' level against MW2's much hotter mix.
const MIX_GAIN: f32 = 2.5;
/// A world sound fades to nothing this far off (units; sixteen blocks).
const REACH: f32 = 16.0 * 36.0;

#[derive(Resource, Default)]
struct GuestSoundCache(HashMap<String, Option<PcmAudio>>);

pub(crate) fn register(app: &mut App) {
    app.init_resource::<GuestSoundQueue>()
        .init_resource::<GuestSoundCache>()
        .add_systems(Update, play_guest_sounds);
}

fn play_guest_sounds(
    mut queue: ResMut<GuestSoundQueue>,
    mut cache: ResMut<GuestSoundCache>,
    mut commands: Commands,
    mut pcm_assets: ResMut<Assets<PcmAudio>>,
    listeners: Query<&Transform, With<AmbientListener>>,
    held: Query<(Entity, &GuestHeld)>,
) {
    if queue.0.is_empty() {
        return;
    }
    let pose = listeners.iter().next().map(|t| (t.translation, t.rotation * Vec3::X));
    for (request, position) in std::mem::take(&mut queue.0) {
        if let Some(name) = request.hold {
            for (entity, _) in held.iter().filter(|(_, h)| h.0 == name) {
                commands.entity(entity).despawn();
            }
            if request.bytes.is_empty() {
                continue;
            }
        }
        let pcm = cache.0.entry(request.key.clone()).or_insert_with(|| {
            let decoded = crate::pcm::decode_audio_bytes(&request.bytes);
            if decoded.is_none() {
                diag::warn!(Audio, "guest weapon sound `{}` did not decode", request.key);
            }
            decoded
        });
        let Some(pcm) = pcm.as_ref() else {
            continue;
        };
        let (gain, pan) = match (position, pose) {
            (Some(at), Some((ear, right))) => {
                let falloff = (1.0 - (at - ear).length() / REACH).clamp(0.0, 1.0);
                (request.volume.min(1.0) * falloff, world_oneshot_channel_gains(ear, right, at, 1.0))
            }
            _ => (request.volume.min(1.0), (std::f32::consts::FRAC_1_SQRT_2, std::f32::consts::FRAC_1_SQRT_2)),
        };
        if gain <= 0.001 {
            continue;
        }
        let live = pcm.with_live_pan();
        if let Some(live_pan) = live.live_pan() {
            live_pan.set(pan.0, pan.1);
        }
        let settings = if request.hold.is_some() { PlaybackSettings::LOOP } else { PlaybackSettings::DESPAWN };
        let mut sound = commands.spawn((
            AudioPlayer(pcm_assets.add(live)),
            settings
                .with_volume(Volume::Linear((gain * MIX_GAIN).max(0.0)))
                .with_speed(request.pitch.clamp(0.5, 2.0)),
        ));
        if let Some(name) = request.hold {
            sound.insert(GuestHeld(name));
        }
    }
}
