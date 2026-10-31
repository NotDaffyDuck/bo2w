//! Shellshock sound: the ringing loop a stun leaves, and its end sounds.
//! Two paths exist: the plain tinnitus loop (non-Black Ops II banks) and
//! Black Ops II's own, which also holds a duck snapshot and fades its loop.

use asset_core::AssetCategoryScope;
use bevy::audio::{AudioSink, AudioSinkPlayback, Volume};
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use frame::{ShellScreen, GameSessionClosed, SpawnLifeOver};
use hud_t6::flashbang::{ShellshockSound, shellshock_left_ms};
use net::{FrameClock, HostViewClient, ShownWorldFrame};

use crate::backend::{self, SoundAudience};
use crate::clip_bank::{ClipRepository, waveforms_for_cue};
use crate::sample_feed::{self, ClipPcm, LoopedPcm};
use crate::play_core::{AudioCatalogRes, UnknownCueHoles};
use crate::{AliasCommand, SOUND_SOURCE_SELF, TriggerCueMsg};

/// Plays `alias` on the local player, with nothing to fall back to.
fn play_local(play: &mut MessageWriter<AliasCommand>, cue_name: &str) {
    play.write(AliasCommand::Play(TriggerCueMsg {
        source_entity_no: Some(SOUND_SOURCE_SELF),
        cue_ns: AssetCategoryScope::T6,
        position_in_inches: None,
        fallback: None,
        alias: cue_name.to_owned(),
    }));
}

/// The decoded first clip of a Black Ops II alias. Asks for it if it is not
/// resident yet (None for now); records a gap if the alias or its decode is bad.
fn clip_for_loop(
    bank: &AudioCatalogRes,
    clips: &mut ClipRepository,
    cue_gaps: &mut UnknownCueHoles,
    cue_id: &str,
) -> Option<ClipPcm> {
    let first_key = waveforms_for_cue(&bank.0, AssetCategoryScope::T6, cue_id)
        .into_iter()
        .next();
    let Some(wanted) = first_key else {
        cue_gaps.note_miss(cue_id);
        return None;
    };
    clips.req_in(wanted.clone());
    let outcome = clips.ready_pcm(&wanted)?;
    outcome.ok().or_else(|| {
        cue_gaps.note_miss(cue_id);
        None
    })
}

/// True while the local player is in a match, not a menu.
fn in_game(app_screen: Option<&ShellScreen>) -> bool {
    matches!(app_screen, Some(ShellScreen::InGame))
}

/// Whether the local player's life ended this frame.
fn local_died(died: &mut MessageReader<SpawnLifeOver>, local: Option<&HostViewClient>) -> bool {
    let id = local.map(|local| local.0.0);
    died.read()
        .any(|ended| id.is_some_and(|id| ended.client == id))
}

#[derive(Component)]
pub(crate) struct BlastEarRinging {
    ringing: String,
}

/// What the tinnitus loop needs to find, decode and start a clip.
#[derive(SystemParam)]
pub(crate) struct LoopSupplies<'w> {
    epoch: Res<'w, backend::GameRoundSerial>,
    gaps: ResMut<'w, UnknownCueHoles>,
    assets: ResMut<'w, Assets<LoopedPcm>>,
    clips: Option<ResMut<'w, ClipRepository>>,
    bank: Option<Res<'w, AudioCatalogRes>>,
}

/// Cuts the tinnitus loop and forgets that it was ringing.
fn hush(cmds: &mut Commands, rings: &Query<(Entity, &BlastEarRinging)>, previously_on: &mut bool) {
    stop_bed(cmds, rings);
    *previously_on = false;
}

pub(crate) fn update_tinnitus(
    mut supplies: LoopSupplies,
    mut torn: MessageReader<GameSessionClosed>,
    mut died: MessageReader<SpawnLifeOver>,
    ui_screen: Option<Res<ShellScreen>>,
    cg_clock: Option<Res<FrameClock>>,
    presented: Option<Res<ShownWorldFrame>>,
    viewer: Option<Res<HostViewClient>>,
    mut commands: Commands,
    mut play: MessageWriter<AliasCommand>,
    rings: Query<(Entity, &BlastEarRinging)>,
    mut previously_on: Local<bool>,
) {
    let match_over = torn.read().next().is_some();
    let local_dead = local_died(&mut died, viewer.as_deref());
    if match_over || !in_game(ui_screen.as_deref()) {
        return hush(&mut commands, &rings, &mut previously_on);
    }
    let (Some(clock), Some(frame_snap), Some(local_ix)) = (cg_clock, presented, viewer) else {
        return;
    };

    let alive = frame_snap.living_participant(local_ix.0).is_some();
    let ms_left = frame_snap
        .player(local_ix.0)
        .and_then(|state| {
            shellshock_left_ms(clock.time(), state.stun_start, state.stun_length)
        })
        .unwrap_or_default();
    let Some(parms) = frame_snap.shellshock(local_ix.0).map(|shock| shock.sound.clone()) else {
        return hush(&mut commands, &rings, &mut previously_on);
    };

    if ms_left > 0 && parms.touches && alive && !local_dead {
        supplies.ensure_loop(&mut commands, &rings, &parms.loop_alias);
        *previously_on = true;
        return;
    }

    // The ringing just ended: cut the loop, and say how it ended.
    let was_ringing = std::mem::replace(&mut *previously_on, false);
    if was_ringing {
        hush(&mut commands, &rings, &mut previously_on);
        let cut_short = local_dead || !alive;
        play_local(
            &mut play,
            if cut_short {
                &parms.end_abort_alias
            } else {
                &parms.end_alias
            },
        );
    }
}

/// Stops every voice entity in `entities`.
fn stop_each(commands: &mut Commands, entities: impl IntoIterator<Item = Entity>) {
    entities
        .into_iter()
        .for_each(|voice| backend::silence_cue(commands, voice));
}

fn stop_bed(cmds: &mut Commands, rings: &Query<(Entity, &BlastEarRinging)>) {
    stop_each(cmds, rings.iter().map(|(entity, _)| entity));
}

impl LoopSupplies<'_> {
    /// Starts `alias` as the tinnitus loop unless it already rings; any other
    /// tinnitus loop is cut first.
    fn ensure_loop(&mut self, cmds: &mut Commands, rings: &Query<(Entity, &BlastEarRinging)>, cue_key: &str) {
        let (same, others): (Vec<_>, Vec<_>) =
            rings.iter().partition(|(_, bed)| bed.ringing == cue_key);
        if !same.is_empty() {
            return;
        }
        stop_each(cmds, others.into_iter().map(|(entity, _)| entity));
        let (Some(snd_bank), Some(clips)) = (self.bank.as_deref(), self.clips.as_deref_mut()) else {
            return;
        };
        let Some(clip) = clip_for_loop(snd_bank, clips, &mut self.gaps, cue_key) else {
            return;
        };
        let entity = backend::begin_repeating_cue(
            cmds,
            self.assets.add(clip.looped()),
            Volume::Linear(1.0),
            self.epoch.0,
            SoundAudience::Match,
        );
        cmds.entity(entity).insert(BlastEarRinging {
            ringing: cue_key.to_owned(),
        });
    }
}

/// Whether the sound bank in use is Black Ops II's, whose shellshock sound is
/// `update_shellshock_sound_t6` instead of the plain loop above.
pub(crate) fn bank_is_t6(namespace: Option<Res<crate::map_sound_set::CatalogNameSpace>>) -> bool {
    namespace.is_some_and(|namespace| namespace.namespace == AssetCategoryScope::T6)
}

/// A Black Ops II shellshock loop: plays at its alias's volume, faded in or out.
#[derive(Component)]
pub(crate) struct ShockLoopT6 {
    cue_alias: String,
    base: f32,
    category: Option<u8>,
    from: f32,
    to: f32,
    fade_start_ms: i64,
    fade_ms: i64,
    /// Fading out: the loop stops when the fade ends.
    stop_at_end: bool,
}

impl ShockLoopT6 {
    fn faded(&self, now: i64) -> bool {
        self.fade_ms <= 0 || now >= self.fade_start_ms + self.fade_ms
    }

    /// Fade multiplier at `now`, moving linearly from `from` to `to`.
    fn gain(&self, now: i64) -> f32 {
        if self.faded(now) {
            return self.to;
        }
        let progress = (now - self.fade_start_ms) as f32 / self.fade_ms as f32;
        self.from + (self.to - self.from) * progress.clamp(0.0, 1.0)
    }

    /// Whether this loop is `alias` and not already on its way out.
    fn holds(&self, cue_id: &str) -> bool {
        self.cue_alias == cue_id && !self.stop_at_end
    }
}

#[derive(Default)]
pub(crate) struct ShockSoundT6 {
    /// Whether the shock's loop is playing (the original keeps this as a flag
    /// on the client game).
    playing: bool,
    /// When the loop started and how long it fades in; a clip that loads late
    /// joins the fade where it stands.
    fade_in_at: i64,
    fade_in_ms: i64,
    /// The last shock's sounds, kept for its abort sound once the shock is gone.
    last: Option<ShellshockSound>,
}

/// Black Ops II's shellshock sound. While the shock's sound is active (its
/// fade-in, then until `soundModEndDelay` past the shock less the fade-out) the
/// shock's snapshot is held on the SHELLSHOCK duck category and its loop
/// plays, faded in. Then the snapshot lets go, the loop fades out and the end
/// sound plays. A shock cut short (death, or the shock cleared) cuts the loop
/// and plays the abort sound. The shock's reverb is a typed gap (no reverb yet).
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn update_shellshock_sound_t6(
    time: Res<Time<Real>>,
    (mut torn, mut died): (
        MessageReader<GameSessionClosed>,
        MessageReader<SpawnLifeOver>,
    ),
    cur_screen: Option<Res<ShellScreen>>,
    cg_clock: Option<Res<FrameClock>>,
    (presented, viewer): (Option<Res<ShownWorldFrame>>, Option<Res<HostViewClient>>),
    bank: Option<Res<AudioCatalogRes>>,
    (mut clip_repo, mut loop_assets, mut gaps, epoch): (
        Option<ResMut<ClipRepository>>,
        ResMut<Assets<LoopedPcm>>,
        ResMut<UnknownCueHoles>,
        Res<backend::GameRoundSerial>,
    ),
    mut mixer: ResMut<crate::DuckMixer>,
    mut commands: Commands,
    mut play: MessageWriter<AliasCommand>,
    mut loops: Query<(Entity, &mut ShockLoopT6)>,
    mut state: Local<ShockSoundT6>,
) {
    use crate::duck::CATEGORY_SHELLSHOCK;

    let now = crate::duck::clock_millis(&time);
    let session_closed = torn.read().next().is_some();
    let died = local_died(&mut died, viewer.as_deref());
    if session_closed || !in_game(cur_screen.as_deref()) {
        stop_each(&mut commands, loops.iter().map(|(entity, _)| entity));
        *state = ShockSoundT6::default();
        return;
    }
    let (Some(cg_clock), Some(shown), Some(viewer), Some(bank)) =
        (cg_clock, presented, viewer, bank)
    else {
        return;
    };

    let alive = shown.living_participant(viewer.0).is_some();
    let shock_times = shown
        .player(viewer.0)
        .map(|state| (state.stun_start, state.stun_length));
    let shock_parms = shown
        .shellshock(viewer.0)
        .map(|shock| shock.sound.clone());

    // How far into the shock we are, if one is running for a live player.
    let running = shock_times.and_then(|(start, duration)| {
        let elapsed = cg_clock.time().wrapping_sub(start);
        (start != 0 && elapsed >= 0 && alive && !died).then_some((elapsed, duration))
    });
    let (Some((elapsed, duration)), Some(parms)) = (running, shock_parms) else {
        // Over or cut short: the snapshot lets go; a loop still playing is cut
        // and the abort sound plays.
        mixer.set_duck(&bank.0, CATEGORY_SHELLSHOCK, "default", now);
        if std::mem::take(&mut state.playing) {
            stop_each(&mut commands, loops.iter().map(|(entity, _)| entity));
            if let Some(last) = state.last.take()
                && !last.end_abort_alias.is_empty()
            {
                play_local(&mut play, &last.end_abort_alias);
            }
        }
        return;
    };
    if !parms.touches {
        return;
    }

    let remaining = parms.mod_end_delay_ms - elapsed + parms.fade_out_ms + duration;
    let active = elapsed < parms.fade_in_ms || remaining > parms.fade_out_ms;
    if active {
        mixer.set_duck(&bank.0, CATEGORY_SHELLSHOCK, &parms.snapshot, now);
    } else if (0..parms.fade_out_ms).contains(&remaining) {
        mixer.set_duck(&bank.0, CATEGORY_SHELLSHOCK, "default", now);
    }

    if active && !state.playing && !parms.loop_alias.is_empty() {
        state.playing = true;
        state.fade_in_at = now;
        state.fade_in_ms = i64::from(parms.fade_in_ms.max(0));
    }

    if active && state.playing {
        // Only the current loop may play; start it if it is not held yet.
        let mut held = false;
        for (ent, current) in &loops {
            if current.holds(&parms.loop_alias) {
                held = true;
            } else if !current.stop_at_end {
                backend::silence_cue(&mut commands, ent);
            }
        }
        if !held {
            start_loop_t6(
                &mut commands,
                &bank,
                clip_repo.as_deref_mut(),
                &mut loop_assets,
                &mut gaps,
                epoch.0,
                &parms.loop_alias,
                state.fade_in_at,
                state.fade_in_ms,
            );
        }
    } else if state.playing {
        // The loop fades out over the shock's fade-out, and so does a duck it holds.
        let fade_ms = i64::from(parms.fade_out_ms.max(0));
        for (ent_id, mut current) in &mut loops {
            if current.stop_at_end {
                continue;
            }
            current.from = current.gain(now);
            current.to = 0.0;
            current.fade_start_ms = now;
            current.fade_ms = fade_ms;
            current.stop_at_end = true;
            mixer.fade_stop_voice(ent_id, fade_ms, now);
        }
        if !parms.end_alias.is_empty() {
            play_local(&mut play, &parms.end_alias);
        }
        state.playing = false;
    }
    state.last = Some(parms);
}

#[allow(clippy::too_many_arguments)]
fn start_loop_t6(
    commands: &mut Commands,
    bank: &AudioCatalogRes,
    clips: Option<&mut ClipRepository>,
    looping_assets: &mut Assets<LoopedPcm>,
    gaps: &mut UnknownCueHoles,
    epoch: u64,
    cue_id: &str,
    fade_in_at: i64,
    fade_in_ms: i64,
) {
    let Some(decoded) = clips.and_then(|store| clip_for_loop(bank, store, gaps, cue_id)) else {
        return;
    };
    let entry = bank
        .0
        .entry_in(AssetCategoryScope::T6, cue_id)
        .and_then(|sound| sound.aliases.first());
    let decoded = sample_feed::with_stereo_pan(decoded, entry.and_then(|e| e.stereo_pan));
    let handle = looping_assets.add(decoded.looped());

    // Silent until the per-frame volume (base, master, menu, duck and fade) is set.
    let entity = backend::begin_repeating_cue(
        commands,
        handle,
        Volume::Linear(0.0),
        epoch,
        SoundAudience::Match,
    );
    let mut shock_loop = commands.entity(entity);
    shock_loop.insert(ShockLoopT6 {
        cue_alias: cue_id.to_owned(),
        base: entry.map_or(1.0, |e| e.volume_low.max(0.0)),
        category: entry.and_then(|e| e.menu_category),
        from: 0.0,
        to: 1.0,
        fade_start_ms: fade_in_at,
        fade_ms: fade_in_ms,
        stop_at_end: false,
    });
    crate::duck::tag(&mut shock_loop, entry, 0);
    diag::info!(
        Audio,
        "audio: shellshock loop `{cue_id}` (fade in {fade_in_ms} ms)"
    );
}

/// Sets each Black Ops II shellshock loop's volume for this frame; one that
/// has finished fading out stops.
#[allow(clippy::type_complexity)]
pub(crate) fn update_shock_loop_volume_t6(
    time: Res<Time<Real>>,
    settings: Res<frame::PlayerPrefsBundle>,
    ducks: Res<crate::DuckMixer>,
    mut commands: Commands,
    mut loops: Query<(
        Entity,
        &ShockLoopT6,
        Option<&mut AudioSink>,
        Option<&crate::DuckGroup>,
    )>,
) {
    let now = crate::duck::clock_millis(&time);
    for (ent, shock, sink, group) in &mut loops {
        if shock.stop_at_end && shock.faded(now) {
            backend::silence_cue(&mut commands, ent);
            continue;
        }
        let Some(mut sink) = sink else {
            continue;
        };
        let volume = shock.base
            * settings.master_gain()
            * settings.menu_volume(shock.category)
            * ducks.linear(group.map(|group| group.0))
            * shock.gain(now);
        sink.set_volume(Volume::Linear(volume.max(0.0)));
    }
}

#[cfg(test)]
mod t6_tests {
    use super::ShockLoopT6;

    fn loop_fading(from: f32, to: f32, start: i64, ms: i64, stop_at_end: bool) -> ShockLoopT6 {
        ShockLoopT6 {
            cue_alias: "loop".to_owned(),
            base: 1.0,
            category: None,
            from,
            to,
            fade_start_ms: start,
            fade_ms: ms,
            stop_at_end,
        }
    }

    #[test]
    fn shock_loop_fades_in_then_out() {
        let mut shock = loop_fading(0.0, 1.0, 1000, 400, false);
        assert_eq!(shock.gain(1000), 0.0);
        assert_eq!(shock.gain(1200), 0.5);
        assert_eq!(shock.gain(1400), 1.0);

        // Start fading out partway through (at 1300) over a second.
        shock.from = shock.gain(1300);
        shock.to = 0.0;
        shock.fade_start_ms = 1300;
        shock.fade_ms = 1000;
        shock.stop_at_end = true;
        assert!((shock.gain(1800) - 0.375).abs() < 1e-6);
        assert!(!shock.faded(2299));
        assert!(shock.faded(2300));
    }

    #[test]
    fn holds_only_a_loop_still_playing() {
        let mut shock = loop_fading(0.0, 1.0, 0, 100, false);
        assert!(shock.holds("loop"));
        assert!(!shock.holds("other"));
        shock.stop_at_end = true;
        assert!(!shock.holds("loop"));
    }
}
