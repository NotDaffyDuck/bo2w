//! Shellshock sound: the ringing loop a stun leaves, and its end sounds.
//! Two paths exist: the plain tinnitus loop (non-Black Ops II banks) and
//! Black Ops II's own, which also holds a duck snapshot and fades its loop.

use asset_core::AssetNamespace;
use bevy::{
    audio::{AudioSink, AudioSinkPlayback, Volume},
    prelude::*,
};
use frame::{AppScreen, LifeEnded, MatchTornDown};
use hud_t6::flashbang::{ShellshockSound, shellshock_remaining_ms};
use net::{FrameClock, LocalPresentClient, PresentedSnapshot};

use crate::{
    AliasCommand, PlayAlias, SND_ENT_LOCAL,
    backend::{self, AudioScope},
    clip_store::{ClipStore, clip_keys_for_alias},
    pcm::{self, LoopingPcmAudio, PcmAudio},
    playback::{MissingAliasGaps, SoundBank},
};

/// Plays `alias` on the local player, with nothing to fall back to.
fn play_local(play: &mut MessageWriter<AliasCommand>, alias: &str) {
    play.write(AliasCommand::Play(PlayAlias {
        namespace: AssetNamespace::T6,
        alias: alias.to_owned(),
        fallback: None,
        origin_inches: None,
        snd_ent: Some(SND_ENT_LOCAL),
    }));
}

/// The decoded first clip of a Black Ops II alias. Asks for it if it is not
/// resident yet (None for now); records a gap if the alias or its decode is bad.
fn clip_for_loop(
    bank: &SoundBank,
    clips: &mut ClipStore,
    gaps: &mut MissingAliasGaps,
    alias: &str,
) -> Option<PcmAudio> {
    let Some(key) = clip_keys_for_alias(&bank.0, AssetNamespace::T6, alias)
        .into_iter()
        .next()
    else {
        gaps.record(alias);
        return None;
    };
    clips.request(key.clone());
    match clips.ready(&key)? {
        Ok(decoded) => Some(decoded),
        Err(_) => {
            gaps.record(alias);
            None
        }
    }
}

/// True while the local player is in a match, not a menu.
fn in_game(screen: Option<&AppScreen>) -> bool {
    matches!(screen, Some(AppScreen::InGame))
}

/// Whether the local player's life ended this frame.
fn local_died(died: &mut MessageReader<LifeEnded>, local: Option<&LocalPresentClient>) -> bool {
    let id = local.map(|local| local.0.0);
    died.read().any(|ended| id.is_some_and(|id| ended.client == id))
}

#[derive(Component)]
pub(crate) struct ShellshockTinnitus {
    alias: String,
}

pub(crate) fn update_shellshock_tinnitus(
    mut torn: MessageReader<MatchTornDown>,
    mut died: MessageReader<LifeEnded>,
    screen: Option<Res<AppScreen>>,
    cg_clock: Option<Res<FrameClock>>,
    presented: Option<Res<PresentedSnapshot>>,
    local: Option<Res<LocalPresentClient>>,
    bank: Option<Res<SoundBank>>,
    mut clips: Option<ResMut<ClipStore>>,
    mut looping_assets: ResMut<Assets<LoopingPcmAudio>>,
    mut commands: Commands,
    mut play: MessageWriter<AliasCommand>,
    mut gaps: ResMut<MissingAliasGaps>,
    epoch: Res<backend::MatchEpoch>,
    playing: Query<(Entity, &ShellshockTinnitus)>,
    mut was_active: Local<bool>,
) {
    let torn = torn.read().next().is_some();
    let died = local_died(&mut died, local.as_deref());
    if torn || !in_game(screen.as_deref()) {
        stop_loop(&mut commands, &playing);
        *was_active = false;
        return;
    }
    let (Some(cg_clock), Some(presented), Some(local)) = (cg_clock, presented, local) else {
        return;
    };

    let alive = presented.alive_player(local.0).is_some();
    let remaining = presented
        .player(local.0)
        .and_then(|state| {
            shellshock_remaining_ms(
                cg_clock.time(),
                state.shellshock_time,
                state.shellshock_duration,
            )
        })
        .unwrap_or(0);
    let Some(parms) = presented.shellshock(local.0).map(|shock| shock.sound.clone()) else {
        stop_loop(&mut commands, &playing);
        *was_active = false;
        return;
    };

    if remaining > 0 && parms.affect && alive && !died {
        ensure_loop(
            &mut commands,
            &playing,
            bank.as_deref(),
            clips.as_deref_mut(),
            &mut looping_assets,
            &mut gaps,
            epoch.0,
            &parms.loop_alias,
        );
        *was_active = true;
        return;
    }

    // The ringing just ended: cut the loop, and say how it ended.
    if std::mem::take(&mut *was_active) {
        stop_loop(&mut commands, &playing);
        let cut_short = died || !alive;
        let alias = if cut_short {
            &parms.end_abort_alias
        } else {
            &parms.end_alias
        };
        play_local(&mut play, alias);
    }
}

fn stop_loop(commands: &mut Commands, playing: &Query<(Entity, &ShellshockTinnitus)>) {
    for (entity, _) in playing {
        backend::stop(commands, entity);
    }
}

fn ensure_loop(
    commands: &mut Commands,
    playing: &Query<(Entity, &ShellshockTinnitus)>,
    bank: Option<&SoundBank>,
    clips: Option<&mut ClipStore>,
    looping_assets: &mut Assets<LoopingPcmAudio>,
    gaps: &mut MissingAliasGaps,
    epoch: u64,
    alias: &str,
) {
    if playing.iter().any(|(_, bed)| bed.alias == alias) {
        return;
    }
    stop_loop(commands, playing);
    let (Some(bank), Some(clips)) = (bank, clips) else {
        return;
    };
    let Some(decoded) = clip_for_loop(bank, clips, gaps, alias) else {
        return;
    };
    let handle = looping_assets.add(decoded.into_looping());
    let entity = backend::spawn_loop(
        commands,
        handle,
        Volume::Linear(1.0),
        epoch,
        AudioScope::Match,
    );
    commands.entity(entity).insert(ShellshockTinnitus {
        alias: alias.to_owned(),
    });
}

/// Whether the sound bank in use is Black Ops II's, whose shellshock sound is
/// `update_shellshock_sound_t6` instead of the plain loop above.
pub(crate) fn bank_is_t6(namespace: Option<Res<crate::ambient::SoundBankNamespace>>) -> bool {
    namespace.is_some_and(|namespace| namespace.namespace == AssetNamespace::T6)
}

/// A Black Ops II shellshock loop: plays at its alias's volume, faded in or out.
#[derive(Component)]
pub(crate) struct ShockLoopT6 {
    alias: String,
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
    fn holds(&self, alias: &str) -> bool {
        self.alias == alias && !self.stop_at_end
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
    (mut torn, mut died): (MessageReader<MatchTornDown>, MessageReader<LifeEnded>),
    screen: Option<Res<AppScreen>>,
    cg_clock: Option<Res<FrameClock>>,
    (presented, local): (Option<Res<PresentedSnapshot>>, Option<Res<LocalPresentClient>>),
    bank: Option<Res<SoundBank>>,
    (mut clips, mut looping_assets, mut gaps, epoch): (
        Option<ResMut<ClipStore>>,
        ResMut<Assets<LoopingPcmAudio>>,
        ResMut<MissingAliasGaps>,
        Res<backend::MatchEpoch>,
    ),
    mut mixer: ResMut<crate::DuckMixer>,
    mut commands: Commands,
    mut play: MessageWriter<AliasCommand>,
    mut loops: Query<(Entity, &mut ShockLoopT6)>,
    mut state: Local<ShockSoundT6>,
) {
    use crate::duck::CATEGORY_SHELLSHOCK;

    let now = crate::duck::now_ms(&time);
    let torn = torn.read().next().is_some();
    let died = local_died(&mut died, local.as_deref());
    if torn || !in_game(screen.as_deref()) {
        for (entity, _) in &loops {
            backend::stop(&mut commands, entity);
        }
        *state = ShockSoundT6::default();
        return;
    }
    let (Some(cg_clock), Some(presented), Some(local), Some(bank)) =
        (cg_clock, presented, local, bank)
    else {
        return;
    };

    let alive = presented.alive_player(local.0).is_some();
    let shock_times = presented
        .player(local.0)
        .map(|state| (state.shellshock_time, state.shellshock_duration));
    let parms = presented.shellshock(local.0).map(|shock| shock.sound.clone());

    // How far into the shock we are, if one is running for a live player.
    let running = shock_times.and_then(|(start, duration)| {
        let elapsed = cg_clock.time().wrapping_sub(start);
        (start != 0 && elapsed >= 0 && alive && !died).then_some((elapsed, duration))
    });
    let (Some((elapsed, duration)), Some(parms)) = (running, parms) else {
        // Over or cut short: the snapshot lets go; a loop still playing is cut
        // and the abort sound plays.
        mixer.set_duck(&bank.0, CATEGORY_SHELLSHOCK, "default", now);
        if std::mem::take(&mut state.playing) {
            for (entity, _) in &loops {
                backend::stop(&mut commands, entity);
            }
            if let Some(last) = state.last.take()
                && !last.end_abort_alias.is_empty()
            {
                play_local(&mut play, &last.end_abort_alias);
            }
        }
        return;
    };
    if !parms.affect {
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
        for (entity, current) in &loops {
            if current.holds(&parms.loop_alias) {
                held = true;
            } else if !current.stop_at_end {
                backend::stop(&mut commands, entity);
            }
        }
        if !held {
            start_loop_t6(
                &mut commands,
                &bank,
                clips.as_deref_mut(),
                &mut looping_assets,
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
        for (entity, mut current) in &mut loops {
            if current.stop_at_end {
                continue;
            }
            current.from = current.gain(now);
            current.to = 0.0;
            current.fade_start_ms = now;
            current.fade_ms = fade_ms;
            current.stop_at_end = true;
            mixer.fade_stop_voice(entity, fade_ms, now);
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
    bank: &SoundBank,
    clips: Option<&mut ClipStore>,
    looping_assets: &mut Assets<LoopingPcmAudio>,
    gaps: &mut MissingAliasGaps,
    epoch: u64,
    alias: &str,
    fade_in_at: i64,
    fade_in_ms: i64,
) {
    let Some(clips) = clips else {
        return;
    };
    let Some(decoded) = clip_for_loop(bank, clips, gaps, alias) else {
        return;
    };
    let row = bank
        .0
        .sound_in(AssetNamespace::T6, alias)
        .and_then(|sound| sound.aliases.first());
    let decoded = pcm::with_stereo_pan(decoded, row.and_then(|row| row.stereo_pan));
    let handle = looping_assets.add(decoded.into_looping());

    // Silent until the per-frame volume (base, master, menu, duck and fade) is set.
    let entity = backend::spawn_loop(
        commands,
        handle,
        Volume::Linear(0.0),
        epoch,
        AudioScope::Match,
    );
    let mut spawned = commands.entity(entity);
    spawned.insert(ShockLoopT6 {
        alias: alias.to_owned(),
        base: row.map_or(1.0, |row| row.vol_min.max(0.0)),
        category: row.and_then(|row| row.menu_category),
        from: 0.0,
        to: 1.0,
        fade_start_ms: fade_in_at,
        fade_ms: fade_in_ms,
        stop_at_end: false,
    });
    crate::duck::tag(&mut spawned, row, 0);
    diag::info!(Audio, "audio: shellshock loop `{alias}` (fade in {fade_in_ms} ms)");
}

/// Sets each Black Ops II shellshock loop's volume for this frame; one that
/// has finished fading out stops.
#[allow(clippy::type_complexity)]
pub(crate) fn update_shock_loop_volume_t6(
    time: Res<Time<Real>>,
    settings: Res<frame::GameSettings>,
    ducks: Res<crate::DuckMixer>,
    mut commands: Commands,
    mut loops: Query<(
        Entity,
        &ShockLoopT6,
        Option<&mut AudioSink>,
        Option<&crate::DuckGroup>,
    )>,
) {
    let now = crate::duck::now_ms(&time);
    for (entity, shock, sink, group) in &mut loops {
        if shock.stop_at_end && shock.faded(now) {
            backend::stop(&mut commands, entity);
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
            alias: "loop".to_owned(),
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
