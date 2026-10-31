//! Voices: the bevy entities that carry a playing sound, how they are
//! started and stopped, and how a voice that never gets a device sink or
//! belongs to a finished match is cleaned up.

use std::time::{Duration, Instant};

use bevy::audio::{AudioPlayer, AudioSink, AudioSinkPlayback, PlaybackSettings, Volume};
use bevy::prelude::*;
use frame::ClientStep;

use crate::sample_feed::{ClipPcm, LoopedPcm, LoopedPcmPlayer};
use crate::voice_slots::free_spent_channels;

/// How long a new voice may wait for the device to hand it a sink.
const SINK_WAIT: Duration = Duration::from_millis(250);

/// Counts matches; match voices from an earlier count are cut.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GameRoundSerial(pub u64);

impl GameRoundSerial {
    pub fn advance(&mut self) {
        self.0 = u64::wrapping_add(self.0, 1);
    }
}

/// Whether a voice belongs to the menus or to a match.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoundAudience {
    Menu,
    Match,
}

/// What the voice plays: a sound that ends on its own, or one that goes on until stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    Loop,
    Oneshot,
}

/// Whose entity a voice lives on: its own entity is despawned when done; a
/// loop borrowed from some other entity is only taken off it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Home {
    Borrowed,
    Own,
}

#[derive(Component, Debug)]
pub struct PlayingCue {
    pub cue_epoch: u64,
    pub scope: SoundAudience,
    shape: Shape,
    home: Home,
    /// When the voice began waiting for the device to give it a sink; `None`
    /// once it is playing.
    waiting_since: Option<Instant>,
}

impl PlayingCue {
    fn new(shape: Shape, home: Home, scope: SoundAudience, cue_epoch: u64) -> Self {
        Self {
            cue_epoch,
            scope,
            shape,
            home,
            waiting_since: Some(Instant::now()),
        }
    }
}

pub(crate) fn install(app: &mut App) {
    // Stale match voices go first, then the rest step on; both before the
    // finished-voice sweep.
    let ordered = (cut_stale_match_voices, step_channel_lifecycles).chain();
    app.init_resource::<GameRoundSerial>();
    app.add_systems(
        Update,
        ordered
            .in_set(ClientStep::Effects)
            .before(free_spent_channels),
    );
    app.add_systems(Update, count_voices.in_set(ClientStep::Effects));
}

/// Every 10 s: how many voices are alive, by kind, and how much the mixer
/// pulls. A count that only climbs during a match is a voice that never frees.
fn count_voices(
    cues: Query<(&PlayingCue, Option<&AudioSink>)>,
    sinks: Query<(&AudioSink, Has<PlayingCue>)>,
    mut last: Local<Option<(Instant, u64)>>,
) {
    use std::sync::atomic::Ordering;
    let now = Instant::now();
    let mixed = crate::sample_feed::SAMPLES_MIXED.load(Ordering::Relaxed);
    let Some((then, mixed_then)) = *last else {
        *last = Some((now, mixed));
        return;
    };
    let secs = now.duration_since(then).as_secs_f64();
    if secs < 10.0 {
        return;
    }
    *last = Some((now, mixed));
    let mut kinds = [0usize; 4];
    let mut waiting = 0;
    let mut drained = 0;
    for (cue, sink) in &cues {
        let at = usize::from(cue.shape == Shape::Loop) * 2 + usize::from(cue.home == Home::Borrowed);
        kinds[at] += 1;
        waiting += usize::from(cue.waiting_since.is_some());
        drained += usize::from(sink.is_some_and(|s| s.empty()));
    }
    let (mut sink_n, mut bare, mut paused) = (0, 0, 0);
    for (sink, cued) in &sinks {
        sink_n += 1;
        bare += usize::from(!cued);
        paused += usize::from(sink.is_paused());
    }
    diag::info!(
        Audio,
        "sound voices: {} cues (oneshot {}, loop own {}, loop borrowed {}), {waiting} waiting, {drained} drained; {sink_n} sinks ({bare} without a cue, {paused} paused); mixer sources {} ({} echo), {:.0} samples/s",
        kinds.iter().sum::<usize>(),
        kinds[0] + kinds[1],
        kinds[2],
        kinds[3],
        crate::sample_feed::LIVE_SOURCES.load(Ordering::Relaxed),
        crate::sample_feed::LIVE_ECHOES.load(Ordering::Relaxed),
        (mixed - mixed_then) as f64 / secs,
    );
    let (calls, mean, longest, wait, late, over) = crate::mix_probe::take_report();
    diag::info!(
        Audio,
        "sound device: {calls} callbacks, mix {mean:.2} ms mean {longest:.2} ms max, longest wait {wait:.1} ms, {late} late, {over} overrun"
    );
    let (samples, over_in, over_out, peak_in, peak_out) = bevy::audio::master_bus::take_meter();
    diag::info!(
        Audio,
        "sound master: {samples} samples, {over_in} over full scale before the limiter (peak {peak_in:.2}), {over_out} after (peak {peak_out:.2}); device thread Pro Audio: {}",
        bevy::audio::master_bus::pro_audio()
    );
}

pub(crate) fn begin_single_cue(
    commands: &mut Commands,
    handle: Handle<ClipPcm>,
    gain: Volume,
    speed: f32,
    epoch: u64,
    scope: SoundAudience,
) -> Entity {
    let settings = PlaybackSettings::ONCE.with_volume(gain).with_speed(speed);
    let voice = PlayingCue::new(Shape::Oneshot, Home::Own, scope, epoch);
    commands.spawn((AudioPlayer(handle), settings, voice)).id()
}

pub(crate) fn begin_repeating_cue(
    commands: &mut Commands,
    handle: Handle<LoopedPcm>,
    gain: Volume,
    epoch: u64,
    scope: SoundAudience,
) -> Entity {
    let voice = PlayingCue::new(Shape::Loop, Home::Own, scope, epoch);
    commands
        .spawn((LoopedPcmPlayer::new(handle, gain), voice))
        .id()
}

/// Starts a match loop on an entity that already exists.
pub(crate) fn bind_repeating_cue(
    commands: &mut Commands,
    ent: Entity,
    handle: Handle<LoopedPcm>,
    gain: Volume,
    epoch: u64,
) {
    let voice = PlayingCue::new(Shape::Loop, Home::Borrowed, SoundAudience::Match, epoch);
    commands
        .entity(ent)
        .insert((LoopedPcmPlayer::new(handle, gain), voice));
}

/// Takes a loop's playback pieces off an entity without despawning it.
pub(crate) fn unbind_repeating_cue(commands: &mut Commands, ent: Entity) {
    commands.entity(ent).remove::<(AudioPlayer<LoopedPcm>, PlaybackSettings, AudioSink, PlayingCue)>();
}

pub(crate) fn silence_cue(commands: &mut Commands, cue_ent: Entity) {
    commands.entity(cue_ent).try_despawn();
}

fn finish_voice(commands: &mut Commands, ent_id: Entity, home: Home) {
    match home {
        Home::Own => silence_cue(commands, ent_id),
        Home::Borrowed => unbind_repeating_cue(commands, ent_id),
    }
}

/// What one voice does this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    /// Nothing changes.
    Stay,
    /// It has a sink now (or never will, and plays on silently).
    Started,
    /// It is over.
    Done,
    /// It waited too long for a sink.
    GaveUp,
}

/// Steps a voice on. `waited` is how long it has waited for a sink (`None`
/// once playing); `sink` is `Some(drained)` when it has one.
fn next_phase(shape: Shape, waited: Option<Duration>, sink: Option<bool>, headless: bool) -> Step {
    let oneshot = shape == Shape::Oneshot;
    match (waited, sink) {
        // Playing: only a one-shot ends by itself, when its sink runs dry.
        (None, Some(true)) if oneshot => Step::Done,
        (None, _) => Step::Stay,
        (Some(_), Some(_)) => Step::Started,
        // Headless run: no sink will ever come. A one-shot is over at once; a
        // loop counts as playing (silently) until stopped.
        (Some(_), None) if headless && oneshot => Step::Done,
        (Some(_), None) if headless => Step::Started,
        (Some(waited), None) if waited >= SINK_WAIT => Step::GaveUp,
        (Some(_), None) => Step::Stay,
    }
}

fn step_channel_lifecycles(
    mut cue_voices: Query<(Entity, &mut PlayingCue, Option<&AudioSink>)>,
    mut commands: Commands,
) {
    let now = Instant::now();
    for (voice_id, mut cue, sink) in &mut cue_voices {
        let waited = cue.waiting_since.map(|began| now.duration_since(began));
        let drained = sink.map(|s| s.empty());
        // Only asked while waiting without a sink, as before.
        let headless = waited.is_some() && drained.is_none() && crate::AudioSilent::no_device();
        match next_phase(cue.shape, waited, drained, headless) {
            Step::Stay => {}
            Step::Started => cue.waiting_since = None,
            Step::GaveUp => {
                diag::warn!(
                    Audio,
                    "sound: gave up on a cue whose sink never came up (typed gap)"
                );
                finish_voice(&mut commands, voice_id, cue.home);
            }
            Step::Done => finish_voice(&mut commands, voice_id, cue.home),
        }
    }
}

fn cut_stale_match_voices(
    epoch: Res<GameRoundSerial>,
    live_cues: Query<(Entity, &PlayingCue)>,
    mut commands: Commands,
) {
    live_cues
        .iter()
        .filter(|(_, voice)| voice.scope == SoundAudience::Match && voice.cue_epoch != epoch.0)
        .for_each(|(entity, voice)| finish_voice(&mut commands, entity, voice.home));
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;

    fn epoch_world(at: u64) -> World {
        let mut world = World::new();
        world.insert_resource(GameRoundSerial(at));
        world
    }

    /// Runs the stale-voice cut and applies its commands.
    fn cut_stale(ecs: &mut World) {
        ecs.run_system_once(cut_stale_match_voices).unwrap();
        ecs.flush();
    }

    #[test]
    fn a_match_voice_from_an_older_match_is_despawned() {
        let mut world = epoch_world(2);
        let [old, menu, current] = [
            (SoundAudience::Match, 1),
            (SoundAudience::Menu, 1),
            (SoundAudience::Match, 2),
        ]
        .map(|(scope, epoch)| {
            world
                .spawn(PlayingCue::new(Shape::Loop, Home::Own, scope, epoch))
                .id()
        });
        cut_stale(&mut world);
        assert!(world.get_entity(old).is_err());
        assert!(world.get_entity(menu).is_ok());
        assert!(world.get_entity(current).is_ok());
    }

    #[test]
    fn a_borrowed_stale_loop_keeps_its_entity() {
        let mut world = epoch_world(5);
        let host = world
            .spawn((
                Name::new("host"),
                PlayingCue::new(Shape::Loop, Home::Borrowed, SoundAudience::Match, 4),
            ))
            .id();
        cut_stale(&mut world);
        let host = world.get_entity(host).expect("the host entity stays");
        assert!(host.get::<PlayingCue>().is_none());
        assert!(host.get::<Name>().is_some());
    }

    #[test]
    fn the_epoch_bump_wraps() {
        let mut last = GameRoundSerial(u64::MAX);
        last.advance();
        assert_eq!(last.0, 0);
    }

    #[test]
    fn phases_follow_the_sink() {
        let long = Some(SINK_WAIT);
        let short = Some(Duration::ZERO);
        assert_eq!(
            next_phase(Shape::Oneshot, None, Some(true), false),
            Step::Done
        );
        assert_eq!(next_phase(Shape::Loop, None, Some(true), false), Step::Stay);
        assert_eq!(
            next_phase(Shape::Loop, short, Some(false), false),
            Step::Started
        );
        assert_eq!(next_phase(Shape::Oneshot, short, None, true), Step::Done);
        assert_eq!(next_phase(Shape::Loop, short, None, true), Step::Started);
        assert_eq!(next_phase(Shape::Loop, long, None, false), Step::GaveUp);
        assert_eq!(next_phase(Shape::Loop, short, None, false), Step::Stay);
    }
}
