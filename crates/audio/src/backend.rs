//! Voices: the bevy entities that carry a playing sound, how they are
//! started and stopped, and how a voice that never gets a device sink or
//! belongs to a finished match is cleaned up.

use std::time::{Duration, Instant};

use bevy::{
    audio::{AudioPlayer, AudioSink, AudioSinkPlayback, PlaybackSettings, Volume},
    prelude::*,
};
use frame::ClientSet;

use crate::pcm::{LoopingPcmAudio, LoopingPcmPlayback, PcmAudio};
use crate::voice::reclaim_finished_voices;

/// How long a new voice may wait for the device to hand it a sink.
const SINK_WAIT: Duration = Duration::from_millis(250);

/// Counts matches; match voices from an earlier count are cut.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MatchEpoch(pub u64);

impl MatchEpoch {
    pub fn bump(&mut self) {
        self.0 = self.0.wrapping_add(1);
    }
}

/// Whether a voice belongs to the menus or to a match.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioScope {
    Menu,
    Match,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VoiceKind {
    Oneshot,
    Loop,
}

/// Who owns the entity a voice lives on: a voice entity of its own is
/// despawned when done; a loop attached to some other entity is only taken off it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VoiceOwner {
    Exclusive,
    Attached,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VoicePhase {
    /// Waiting for the device to give the voice a sink.
    Starting,
    Playing,
}

#[derive(Component, Debug)]
pub struct Voice {
    pub epoch: u64,
    pub scope: AudioScope,
    kind: VoiceKind,
    owner: VoiceOwner,
    phase: VoicePhase,
    started_at: Instant,
}

impl Voice {
    fn new(kind: VoiceKind, owner: VoiceOwner, scope: AudioScope, epoch: u64) -> Self {
        Self {
            epoch,
            scope,
            kind,
            owner,
            phase: VoicePhase::Starting,
            started_at: Instant::now(),
        }
    }
}

pub(crate) fn register(app: &mut App) {
    app.init_resource::<MatchEpoch>().add_systems(
        Update,
        (cut_stale_match_voices, advance_voice_phases)
            .chain()
            .before(reclaim_finished_voices)
            .in_set(ClientSet::Effects),
    );
}

pub(crate) fn spawn_oneshot(
    commands: &mut Commands,
    handle: Handle<PcmAudio>,
    volume: Volume,
    speed: f32,
    epoch: u64,
    scope: AudioScope,
) -> Entity {
    let settings = PlaybackSettings::ONCE.with_volume(volume).with_speed(speed);
    let voice = Voice::new(VoiceKind::Oneshot, VoiceOwner::Exclusive, scope, epoch);
    commands.spawn((AudioPlayer(handle), settings, voice)).id()
}

pub(crate) fn spawn_loop(
    commands: &mut Commands,
    handle: Handle<LoopingPcmAudio>,
    volume: Volume,
    epoch: u64,
    scope: AudioScope,
) -> Entity {
    let voice = Voice::new(VoiceKind::Loop, VoiceOwner::Exclusive, scope, epoch);
    commands
        .spawn((LoopingPcmPlayback::new(handle, volume), voice))
        .id()
}

/// Starts a match loop on an entity that already exists.
pub(crate) fn attach_loop(
    commands: &mut Commands,
    entity: Entity,
    handle: Handle<LoopingPcmAudio>,
    volume: Volume,
    epoch: u64,
) {
    let voice = Voice::new(VoiceKind::Loop, VoiceOwner::Attached, AudioScope::Match, epoch);
    commands
        .entity(entity)
        .insert((LoopingPcmPlayback::new(handle, volume), voice));
}

/// Takes a loop's playback pieces off an entity without despawning it.
pub(crate) fn detach_loop(commands: &mut Commands, entity: Entity) {
    commands.entity(entity).remove::<(
        AudioPlayer<LoopingPcmAudio>,
        PlaybackSettings,
        AudioSink,
        Voice,
    )>();
}

pub(crate) fn stop(commands: &mut Commands, entity: Entity) {
    commands.entity(entity).try_despawn();
}

fn finish_voice(commands: &mut Commands, entity: Entity, owner: VoiceOwner) {
    match owner {
        VoiceOwner::Exclusive => stop(commands, entity),
        VoiceOwner::Attached => detach_loop(commands, entity),
    }
}

fn advance_voice_phases(
    mut voices: Query<(Entity, &mut Voice, Option<&AudioSink>)>,
    mut commands: Commands,
) {
    let now = Instant::now();
    for (entity, mut voice, sink) in &mut voices {
        if voice.phase == VoicePhase::Playing {
            // Only a one-shot ends by itself, when its sink runs dry.
            if voice.kind == VoiceKind::Oneshot && sink.is_some_and(|s| s.empty()) {
                finish_voice(&mut commands, entity, voice.owner);
            }
            continue;
        }

        if sink.is_some() {
            voice.phase = VoicePhase::Playing;
        } else if crate::AudioSilent::no_device() {
            // Headless run: no sink will ever come. A one-shot is over at
            // once; a loop counts as playing (silently) until stopped.
            match voice.kind {
                VoiceKind::Oneshot => finish_voice(&mut commands, entity, voice.owner),
                VoiceKind::Loop => voice.phase = VoicePhase::Playing,
            }
        } else if now.duration_since(voice.started_at) >= SINK_WAIT {
            diag::warn!(
                Audio,
                "audio: voice start timed out waiting for sink (typed gap)"
            );
            finish_voice(&mut commands, entity, voice.owner);
        }
    }
}

fn cut_stale_match_voices(
    epoch: Res<MatchEpoch>,
    voices: Query<(Entity, &Voice)>,
    mut commands: Commands,
) {
    for (entity, voice) in &voices {
        if voice.scope == AudioScope::Match && voice.epoch != epoch.0 {
            finish_voice(&mut commands, entity, voice.owner);
        }
    }
}
