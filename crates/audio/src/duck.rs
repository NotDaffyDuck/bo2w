//! bo2mp: Black Ops II's sound ducks (`SndDuck`). While a duck is on, each of the 32 duck
//! groups is turned down toward the duck's attenuation for it; every sound is scaled by
//! its alias's duck group's volume. A sound whose alias names a duck starts one when it
//! starts and ends it when it stops; the game also holds one duck per category: the
//! shellshock's snapshot (a frag's `cmn_pain_plr`) and the scripts' snapshot (the
//! Reaper's cockpit, the end of the match). Black Ops II also low-passes each group
//! toward the duck's filter; that is not played (no filter yet, a typed gap).

use std::collections::HashSet;
use std::sync::Arc;

use asset_audio::{AudioRoster, SnapshotSoundAlias, SndDuck};
use asset_core::AssetCategoryScope;
use bevy::{audio::Volume, prelude::*};
use frame::{ClientStep, GameSessionClosed};
use net::{HostViewClient, ShownWorldFrame};

use crate::map_sound_set::{LevelBackdropSound, LevelSoundSource};
use crate::backend::{GameRoundSerial, PlayingCue, SoundAudience};
use crate::clip_bank::{ClipRepository, waveforms_for_cue};
use crate::alerts::{AliasCommand, SOUND_SOURCE_SELF, TriggerCueMsg};
use crate::sample_feed::{ClipPcm, LoopedPcm};
use crate::play_core::{AudioCatalogRes, BackgroundHearer, SpatialVoice, UnknownCueHoles};
use crate::distance::inches_of;

/// Black Ops II's "zero" for duck times, amounts and distances.
pub(crate) const EPS: f32 = 1.52879e-05;
const SLOTS: usize = 128;
const CATEGORIES: usize = 12;
const GROUPS: usize = 32;

/// `SND_DUCK_CATEGORY_*`: a sound's own duck, and the categories the game holds one in.
pub(crate) const CATEGORY_ALIAS: usize = 0;
pub(crate) const CATEGORY_SCRIPT: usize = 2;
pub(crate) const CATEGORY_SHELLSHOCK: usize = 3;

/// The client dvar the server sets for the client scripts' sound notifies (`krms`, `pm`,
/// ...): "<n> <state> ... <state>", the newest last and numbered n, the one before n - 1.
const SND_NOTIFY_DVAR: &str = "bo2mp_sndnotify";
const REAPER_SNAPSHOT: &str = "mpl_ks_reaper";
const REAPER_LOOP: &str = "mpl_ks_reaper_interior_loop";
const POST_MATCH_SNAPSHOT: &str = "mpl_post_match";
const POST_MATCH_FULL_SNAPSHOT: &str = "mpl_post_match_full";
const POST_MATCH_FULL_DELAY_MS: i64 = 2500;
const FINAL_KILLCAM_STING: &str = "mpl_final_kill_cam_sting";

/// A sound's duck group (`SndAlias::duckGroup`): its volume is scaled by that group's.
#[derive(Component, Clone, Copy, Debug)]
pub struct DuckGroup(pub u8);

/// A sound whose alias names a duck: the duck starts once when its voice starts and ends
/// when the voice stops.
#[derive(Component, Debug)]
pub(crate) struct DuckSource {
    duck: u32,
    begun: bool,
    /// The alias is 3D (flags bit 1): the duck follows the sound and may weaken with distance.
    positional: bool,
    /// The alias loops (flags bit 0): its duck lasts until the sound is stopped.
    repeats: bool,
    /// The clip's length (not scaled by pitch, as Black Ops II).
    total_ms: i64,
}

/// Tags a new sound with its alias's duck group and duck.
pub(crate) fn tag(e: &mut EntityCommands, row: Option<&SnapshotSoundAlias>, total_ms: i64) {
    if let Some(group) = row.and_then(|r| r.duck_group) {
        e.insert(DuckGroup(group));
    }
    let Some((duck, flags)) = row.and_then(|r| Some((r.duck?, r.alias_bits.unwrap_or(0)))) else {
        return;
    };
    e.insert(DuckSource {
        begun: false,
        total_ms,
        positional: flags & 0b10 != 0,
        repeats: flags & 0b01 != 0,
        duck,
    });
}

/// A clip's length in milliseconds.
pub(crate) fn clip_ms(pcm: &ClipPcm) -> i64 {
    let frames = pcm.data().len() as u64 / u64::from(pcm.lane_count().max(1));
    let millis = frames * 1000 / u64::from(pcm.hz().max(1));
    millis as i64
}

#[derive(Clone, Debug)]
struct Active {
    duck: SndDuck,
    began_ms: i64,
    /// 0 = no end yet.
    end_ms: i64,
    effect: f32,
    voice: Option<Entity>,
    heard_at: [f32; 3],
    strength: f32,
}

impl Active {
    /// A duck that has just begun, not tied to a voice and not yet in effect.
    fn begin(duck: &SndDuck, now: i64, end_ms: i64) -> Self {
        Self {
            duck: duck.clone(),
            began_ms: now,
            end_ms,
            effect: 0.0,
            voice: None,
            heard_at: [0.0; 3],
            strength: 1.0,
        }
    }
}

/// The duck slots and the duck groups' volumes.
#[derive(Resource)]
pub struct DuckMixer {
    slot_list: Vec<Option<Active>>,
    category_index: [Option<usize>; CATEGORIES],
    value: [f32; GROUPS],
    linear: [f32; GROUPS],
    /// A group's volume moved this frame: sounds that are not rescaled every frame are.
    pub(crate) dirty: bool,
    warned: HashSet<String>,
}

impl Default for DuckMixer {
    fn default() -> Self {
        Self {
            slot_list: vec![None; SLOTS],
            category_index: [None; CATEGORIES],
            value: [1.0; GROUPS],
            linear: [1.0; GROUPS],
            dirty: false,
            warned: HashSet::new(),
        }
    }
}

fn is_default(duck: &SndDuck) -> bool {
    duck.name.eq_ignore_ascii_case("default")
}

impl DuckMixer {
    /// The volume scale for a sound in this duck group (1 for none).
    pub fn linear(&self, band: Option<u8>) -> f32 {
        band
            .and_then(|g| self.linear.get(usize::from(g)))
            .map_or(1.0, |&scale| scale)
    }

    /// Back to no ducks, keeping the warnings already given; every group's
    /// volume counts as moved so sounds pick up the reset.
    pub(crate) fn reset(&mut self) {
        let already_warned = std::mem::take(&mut self.warned);
        *self = Self::default();
        self.warned = already_warned;
        self.dirty = true;
    }

    fn warn_once(&mut self, line: String) {
        if self.warned.insert(line.clone()) {
            diag::warn!(Audio, "{line}");
        }
    }

    /// `SND_SetDuck(category, name, 0, 1)`: holds `name` on the category ("default", or
    /// an empty name, lets the category's duck go).
    pub(crate) fn set_duck(&mut self, bank: &AudioRoster, category: usize, name: &str, now: i64) {
        let name = if name.is_empty() { "default" } else { name };
        let Some(duck) = bank
            .ducks
            .values()
            .find(|d| d.name.eq_ignore_ascii_case(name))
            .cloned()
        else {
            self.warn_once(format!("SOUND: Could not find duck {name}"));
            return;
        };
        self.set_duck_by_category(category, &duck, 1.0, now);
    }

    fn set_duck_by_category(&mut self, category: usize, duck: &SndDuck, duck_amt: f32, now: i64) {
        if category == CATEGORY_ALIAS || category >= CATEGORIES || duck.fade_range > EPS {
            return;
        }
        match self.category_index[category] {
            None if is_default(duck) => return,
            Some(slot_no) => {
                if let Some(active) = self.slot_list[slot_no].as_mut() {
                    if active.duck.id == duck.id && duck_amt > EPS {
                        active.strength = duck_amt;
                        return;
                    }
                    if active.duck.length < EPS {
                        active.end_ms = now;
                    }
                }
                self.category_index[category] = None;
            }
            None => {}
        }
        if is_default(duck) || duck_amt < EPS {
            return;
        }
        let Some(free) = self.slot_list.iter().position(Option::is_none) else {
            self.warn_once("SOUND: no free sound ducks.".to_owned());
            return;
        };
        let timed = duck.length >= EPS;
        let end_ms = timed.then(|| now + (duck.length * 1000.0) as i64);
        self.slot_list[free] = Some(Active {
            strength: duck_amt,
            ..Active::begin(duck, now, end_ms.unwrap_or(0))
        });
        if end_ms.is_none() {
            self.category_index[category] = Some(free);
        }
    }

    /// A sound's own duck, when its voice starts.
    fn start_voice(
        &mut self,
        duck: &SndDuck,
        voice: Entity,
        source: &DuckSource,
        cue_id: &str,
        position: [f32; 3],
        now: i64,
    ) {
        if duck.fade_range > EPS && !source.positional {
            self.warn_once(format!(
                "SOUND: cannot use a duck with distance ({}) on a 2d sound ({cue_id})",
                duck.name
            ));
            return;
        }
        let Some(free) = self.slot_list.iter().position(Option::is_none) else {
            self.warn_once("SOUND: no free sound ducks.".to_owned());
            return;
        };
        let end_ms = if duck.length > EPS {
            now + (duck.length * 1000.0) as i64
        } else if !source.repeats {
            now + source.total_ms - (duck.ease_off * 1000.0) as i64
        } else {
            0
        };
        self.slot_list[free] = Some(Active {
            voice: Some(voice),
            heard_at: position,
            ..Active::begin(duck, now, end_ms)
        });
    }

    /// A sound's voice stopped: its duck stops at once if it has no effect yet, else
    /// it fades out from now.
    fn end_voice(&mut self, voice: Entity, now: i64) {
        for i in 0..SLOTS {
            let Some(active) = self.slot_list[i].as_mut().filter(|a| a.voice == Some(voice)) else {
                continue;
            };
            if active.effect < EPS {
                self.stop(i);
            } else {
                active.end_ms = now;
                active.voice = None;
            }
        }
    }

    /// A looping sound is faded out over `fade_ms`: its duck (with no end of its own)
    /// ends so that it is gone when the sound is.
    pub(crate) fn fade_stop_voice(&mut self, voice: Entity, fade_ms: i64, now: i64) {
        for active in self.slot_list.iter_mut().flatten() {
            if active.voice == Some(voice) && active.end_ms == 0 && active.duck.length < EPS {
                active.end_ms = now - (active.duck.ease_off * 1000.0) as i64 + fade_ms;
            }
        }
    }

    fn stop(&mut self, lane_no: usize) {
        for index in &mut self.category_index {
            if *index == Some(lane_no) {
                *index = None;
            }
        }
        self.slot_list[lane_no] = None;
    }

    /// One frame: each duck's effect, then each group's volume moves toward the lowest
    /// any duck asks for, by at most a tenth (`dt` x 10) a frame.
    fn refresh(&mut self, now: i64, dt: f32, listener: Option<[f32; 3]>) {
        let mut target = [1.0f32; GROUPS];
        for i in 0..SLOTS {
            let Some(active) = self.slot_list[i].as_mut() else {
                continue;
            };
            let Some(time) = time_effect(&active.duck, active.began_ms, active.end_ms, now) else {
                self.stop(i);
                continue;
            };
            let distance = distance_effect(&active.duck, active.heard_at, listener);
            active.effect = active.strength * time.min(distance);
            for (t, att) in target.iter_mut().zip(&active.duck.attenuation) {
                *t = t.min(1.0 - active.effect * (1.0 - att));
            }
        }
        let rate = (dt * 10.0).min(0.1);
        let mut any_moved = false;
        for (j, t) in target.iter().enumerate() {
            let v = self.value[j];
            let next = if *t > v {
                (v + rate).min(*t)
            } else {
                (v - rate).max(*t)
            };
            self.value[j] = next;
            let linear = next * next * next;
            any_moved |= linear != self.linear[j];
            self.linear[j] = linear;
        }
        self.dirty = any_moved;
    }
}

/// A duck's fade: None once it has faded out (its slot is freed).
fn time_effect(duck: &SndDuck, start_ms: i64, end_ms: i64, now: i64) -> Option<f32> {
    let t = (now - start_ms) as f32 / 1000.0 - duck.start_delay;
    if end_ms == 0 || now <= end_ms {
        return Some(if t < 0.0 {
            0.0
        } else if t >= duck.ease_on {
            1.0
        } else {
            t / duck.ease_on
        });
    }
    let e = (now - end_ms) as f32 / 1000.0;
    if t < 0.0 || e > duck.ease_off {
        None
    } else if t >= duck.ease_on {
        Some(1.0 - e / duck.ease_off)
    } else {
        Some((1.0 - ((1.0 - t / duck.ease_on) * duck.ease_off + e) / duck.ease_off).max(0.0))
    }
}

/// A duck with a distance weakens from full at its sound to nothing at that distance.
fn distance_effect(duck: &SndDuck, at: [f32; 3], listener: Option<[f32; 3]>) -> f32 {
    if duck.fade_range <= EPS {
        return 1.0;
    }
    let Some(ear) = listener else {
        return 0.0;
    };
    let d2: f32 = (0..3).map(|k| (at[k] - ear[k]).powi(2)).sum();
    if d2 < duck.fade_range * duck.fade_range {
        1.0 - d2.sqrt() / duck.fade_range
    } else {
        0.0
    }
}

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct DuckSet;

pub(crate) fn wire_up(app: &mut App) {
    app.init_resource::<DuckMixer>().add_systems(
        Update,
        (
            (
                crate::shock_sound::update_shellshock_sound_t6.run_if(crate::shock_sound::bank_is_t6),
                script_sound_notifies,
                update_mixer,
            )
                .chain()
                .in_set(DuckSet)
                .after(crate::shock_sound::update_tinnitus),
            crate::shock_sound::update_shock_loop_volume_t6.after(DuckSet),
        )
            .in_set(ClientStep::Effects),
    );
}

pub(crate) fn clock_millis(time: &Time<Real>) -> i64 {
    time.elapsed().as_millis() as i64
}

#[allow(clippy::type_complexity)]
fn update_mixer(
    clock: Res<Time<Real>>,
    mut torn: MessageReader<GameSessionClosed>,
    mut mixer: ResMut<DuckMixer>,
    bank: Option<Res<AudioCatalogRes>>,
    ears_q: Query<&Transform, With<BackgroundHearer>>,
    mut sources: Query<(
        Entity,
        &mut DuckSource,
        Has<PlayingCue>,
        Option<&SpatialVoice>,
        Option<&LevelSoundSource>,
        Option<&crate::play_core::AliasPlayback>,
    )>,
) {
    let session_closed = torn.read().count() > 0;
    if session_closed {
        mixer.reset();
    }
    let now = clock_millis(&clock);
    // A voice that stopped (or was despawned) ends its duck.
    let ended: Vec<Entity> = mixer
        .slot_list
        .iter()
        .flatten()
        .filter_map(|a| a.voice)
        .filter(|&e| {
            sources
                .get(e)
                .map_or(true, |(_, _, has_voice, ..)| !has_voice)
        })
        .collect();
    for voice in ended {
        mixer.end_voice(voice, now);
    }
    for (ent, mut source, has_voice, ch, emitter, playback) in &mut sources {
        let position = ch
            .map(|c| c.position_in_inches)
            .or_else(|| emitter.map(|m| m.position_in_inches))
            .unwrap_or_default();
        if !has_voice {
            source.begun = false;
            continue;
        }
        if !source.begun {
            source.begun = true;
            let Some(duck) = bank
                .as_ref()
                .and_then(|b| b.0.ducks.get(&source.duck))
                .cloned()
            else {
                continue;
            };
            let alias = playback.map_or("", |p| p.alias.as_str());
            mixer.start_voice(&duck, ent, &source, alias, position, now);
            continue;
        }
        if source.positional {
            for active in mixer.slot_list.iter_mut().flatten() {
                if active.voice == Some(ent) {
                    active.heard_at = position;
                }
            }
        }
    }
    let listener = ears_q
        .iter()
        .next()
        .map(|t| inches_of(t.translation));
    mixer.refresh(now, clock.delta_secs(), listener);
}

/// The Reaper's cockpit loop, while its snapshot is on.
#[derive(Component)]
struct ReaperBed;

#[derive(Default)]
struct ScriptSound {
    seen: u64,
    active: String,
    want_bed: bool,
    full_at: Option<i64>,
}

/// The client scripts' sound notifies (`clientscripts/mp/_audio.csc`), sent by the
/// server as `bo2mp_sndnotify`: the Reaper's snapshot and cockpit loop (`krms`/`krme`),
/// the end of the match (`pm`, then `pmf` 2.5 s on), and the final killcam's sting
/// (`fkcb`; its room's reverb is a typed gap, as is `fkce`).
#[allow(clippy::too_many_arguments)]
fn script_sound_notifies(
    time: Res<Time<Real>>,
    mut torn: MessageReader<GameSessionClosed>,
    presented: Option<Res<ShownWorldFrame>>,
    local: Option<Res<HostViewClient>>,
    bank: Option<Res<AudioCatalogRes>>,
    mut mixer: ResMut<DuckMixer>,
    mut play: MessageWriter<AliasCommand>,
    mut commands: Commands,
    mut clip_repo: Option<ResMut<ClipRepository>>,
    mut loop_assets: ResMut<Assets<LoopedPcm>>,
    mut clip_assets: ResMut<Assets<ClipPcm>>,
    mut gaps: ResMut<UnknownCueHoles>,
    epoch: Res<GameRoundSerial>,
    beds: Query<Entity, With<ReaperBed>>,
    mut state: Local<ScriptSound>,
) {
    let session_closed = torn.read().count() > 0;
    if session_closed {
        *state = ScriptSound::default();
        for bed in &beds {
            crate::backend::silence_cue(&mut commands, bed);
        }
    }
    let (Some(presented), Some(local), Some(bank)) = (presented, local, bank) else {
        return;
    };
    let now = clock_millis(&time);
    let value = presented
        .snapshot()
        .map(|frame| {
            frame
                .meta
                .script_dvars(local.0)
                .string(SND_NOTIFY_DVAR)
                .unwrap_or_default()
                .to_owned()
        })
        .unwrap_or_default();
    let mut words = value.split_whitespace();
    if let Some(newest) = words.next().and_then(|n| n.parse::<u64>().ok()) {
        let states: Vec<&str> = words.collect();
        if newest < state.seen {
            // A new map numbers from 1 again.
            state.seen = 0;
        }
        let first = (newest + 1).saturating_sub(states.len() as u64);
        for (k, notify) in states.iter().enumerate() {
            if first + k as u64 <= state.seen {
                continue;
            }
            diag::info!(Audio, "sound: script notify `{notify}` fired");
            match *notify {
                "krms" => {
                    set_snapshot(&mut state, &mut mixer, &bank.0, REAPER_SNAPSHOT, now);
                    state.want_bed = true;
                }
                "krme" => {
                    set_snapshot(&mut state, &mut mixer, &bank.0, "default", now);
                    state.want_bed = false;
                }
                "pm" => set_snapshot(&mut state, &mut mixer, &bank.0, POST_MATCH_SNAPSHOT, now),
                "pmf" => state.full_at = Some(now + POST_MATCH_FULL_DELAY_MS),
                "fkcb" => {
                    play.write(AliasCommand::Play(TriggerCueMsg {
                        cue_ns: AssetCategoryScope::T6,
                        alias: FINAL_KILLCAM_STING.to_owned(),
                        fallback: None,
                        position_in_inches: None,
                        source_entity_no: Some(SOUND_SOURCE_SELF),
                    }));
                }
                _ => {}
            }
        }
        state.seen = newest;
    }
    if state.full_at.is_some_and(|at| now >= at) {
        state.full_at = None;
        set_snapshot(
            &mut state,
            &mut mixer,
            &bank.0,
            POST_MATCH_FULL_SNAPSHOT,
            now,
        );
    }
    if !state.want_bed {
        for bed in &beds {
            crate::backend::silence_cue(&mut commands, bed);
        }
    } else if beds.is_empty() {
        start_reaper_bed(
            &mut commands,
            &bank,
            clip_repo.as_deref_mut(),
            &mut loop_assets,
            &mut clip_assets,
            &mut gaps,
            epoch.0,
        );
    }
}

/// `snd_set_snapshot`: a script snapshot is set only when it changes.
fn set_snapshot(
    state: &mut ScriptSound,
    mixer: &mut DuckMixer,
    bank: &AudioRoster,
    next: &str,
    now: i64,
) {
    let active = if state.active.is_empty() {
        "default"
    } else {
        state.active.as_str()
    };
    if active == next {
        return;
    }
    mixer.set_duck(bank, CATEGORY_SCRIPT, next, now);
    state.active = next.to_owned();
}

/// The Reaper's cockpit loop, a loop emitter at the world's origin: a 2D alias plays
/// flat, a 3D one from that point.
fn start_reaper_bed(
    commands: &mut Commands,
    bank: &AudioCatalogRes,
    store: Option<&mut ClipRepository>,
    loop_assets: &mut Assets<LoopedPcm>,
    clip_assets: &mut Assets<ClipPcm>,
    gaps: &mut UnknownCueHoles,
    epoch: u64,
) {
    let Some(store) = store else {
        return;
    };
    let Some(key) = waveforms_for_cue(&bank.0, AssetCategoryScope::T6, REAPER_LOOP)
        .into_iter()
        .next()
    else {
        gaps.note_miss(REAPER_LOOP);
        return;
    };
    store.req_in(key.clone());
    let Some(Ok(pcm)) = store.ready_pcm(&key) else {
        return;
    };
    let entry = bank
        .0
        .entry_in(AssetCategoryScope::T6, REAPER_LOOP)
        .and_then(|sound| sound.aliases.first());
    let volume = entry.map_or(1.0, |a| a.volume_low.max(0.0));
    let positional = entry.and_then(|a| a.alias_bits).is_some_and(|f| f & 2 != 0);
    let mut e = if positional {
        let entry = entry.expect("positional row");
        let knots: Arc<[[f32; 2]]> = entry
            .volume_falloff
            .as_ref()
            .map(|c| c.knots.iter().map(|&(x, y)| [x, y]).collect::<Vec<_>>())
            .unwrap_or_default()
            .into();
        commands.spawn((
            LevelBackdropSound,
            LevelSoundSource {
                position_in_inches: [0.0; 3],
                reach_lower: entry.reach_lower,
                reach_upper: entry.reach_upper,
                knots,
                rest_gain: volume,
                pcm: clip_assets.add(pcm),
                speed: 1.0,
                active_stereo_handle: None,
                fixed_pan: entry.stereo_pan,
                category: entry.menu_category,
                line_end_inches: None,
            },
            Transform::default(),
        ))
    } else {
        let pcm = crate::sample_feed::with_stereo_pan(pcm, entry.and_then(|a| a.stereo_pan));
        let handle = loop_assets.add(pcm.looped());
        let entity = crate::backend::begin_repeating_cue(
            commands,
            handle,
            Volume::Linear(volume),
            epoch,
            SoundAudience::Match,
        );
        let mut e = commands.entity(entity);
        if let Some(category) = entry.and_then(|a| a.menu_category) {
            e.insert(crate::MenuVolume {
                cat_ix: category,
                base: volume,
            });
        }
        e
    };
    e.insert(ReaperBed);
    tag(&mut e, entry, 0);
    diag::info!(
        Audio,
        "audio: Reaper cockpit loop at {volume:.2} ({})",
        if positional { "3D at the origin" } else { "2D" }
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn duck(name: &str, ease_on: f32, ease_off: f32, length: f32, distance: f32) -> SndDuck {
        let mut attenuation = [1.0; 32];
        attenuation[3] = 0.0;
        attenuation[5] = 0.5;
        SndDuck {
            name: name.to_owned(),
            id: name
                .bytes()
                .fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b))),
            ease_on,
            ease_off,
            start_delay: 0.0,
            fade_range: distance,
            length,
            update_while_paused: false,
            attenuation,
            filter: [1.0; 32],
        }
    }

    fn catalog(ducks: &[SndDuck]) -> AudioRoster {
        let mut c = AudioRoster::default();
        for d in ducks {
            c.ducks.insert(d.id, d.clone());
        }
        c
    }

    fn settle(m: &mut DuckMixer, from: i64, to: i64) {
        let mut now = from;
        while now <= to {
            m.refresh(now, 0.016, Some([0.0; 3]));
            now += 16;
        }
    }

    #[test]
    fn fade_in_then_out() {
        let d = duck("a", 1.0, 2.0, 0.0, 0.0);
        assert_eq!(time_effect(&d, 0, 0, 500), Some(0.5));
        assert_eq!(time_effect(&d, 0, 0, 1500), Some(1.0));
        assert_eq!(time_effect(&d, 0, 2000, 3000), Some(0.5));
        assert_eq!(time_effect(&d, 0, 2000, 4001), None);
        // Ended before it had faded in: while its fade-in time is still running it fades
        // out from where it was; once that time has passed (BO2 times the fade-in up to
        // now) it fades out from full.
        let e = time_effect(&d, 0, 500, 1000).unwrap();
        assert!((e - 0.75).abs() < 1e-6, "{e}");
        let e = time_effect(&d, 0, 800, 900).unwrap();
        assert!((e - 0.85).abs() < 1e-5, "{e}");
    }

    #[test]
    fn category_duck_holds_then_default_lets_go() {
        let snap = duck("snap", 0.0, 0.5, 0.0, 0.0);
        let bank = catalog(&[snap.clone(), duck("default", 0.5, 0.5, 0.0, 0.0)]);
        let mut m = DuckMixer::default();
        m.set_duck(&bank, CATEGORY_SHELLSHOCK, "snap", 0);
        settle(&mut m, 0, 1000);
        assert!(m.linear(Some(3)) < 1e-6);
        assert!((m.linear(Some(5)) - 0.125).abs() < 1e-4);
        assert_eq!(m.linear(Some(4)), 1.0);
        // Setting the same duck again keeps the one slot.
        m.set_duck(&bank, CATEGORY_SHELLSHOCK, "snap", 1000);
        assert_eq!(m.slot_list.iter().flatten().count(), 1);
        m.set_duck(&bank, CATEGORY_SHELLSHOCK, "default", 1000);
        settle(&mut m, 1016, 3000);
        assert_eq!(m.slot_list.iter().flatten().count(), 0);
        assert_eq!(m.linear(Some(3)), 1.0);
    }

    #[test]
    fn timed_duck_ends_by_itself() {
        let bank = catalog(&[duck("pain", 0.0, 0.1, 0.2, 0.0)]);
        let mut m = DuckMixer::default();
        m.set_duck(&bank, CATEGORY_SHELLSHOCK, "pain", 0);
        assert_eq!(m.category_index[CATEGORY_SHELLSHOCK], None);
        settle(&mut m, 0, 100);
        assert!(m.linear(Some(3)) < 1.0);
        settle(&mut m, 116, 1000);
        assert_eq!(m.slot_list.iter().flatten().count(), 0);
        assert_eq!(m.linear(Some(3)), 1.0);
    }

    #[test]
    fn volume_moves_a_tenth_a_frame_at_most() {
        let bank = catalog(&[duck("snap", 0.0, 0.5, 0.0, 0.0)]);
        let mut m = DuckMixer::default();
        m.set_duck(&bank, CATEGORY_SCRIPT, "snap", 0);
        m.refresh(0, 0.5, None);
        assert!((m.value[3] - 0.9).abs() < 1e-6);
        assert!((m.linear(Some(3)) - 0.729).abs() < 1e-5);
        m.refresh(16, 0.005, None);
        assert!((m.value[3] - 0.85).abs() < 1e-5);
    }

    #[test]
    fn distance_weakens_a_duck() {
        let d = duck("near", 0.0, 0.5, 0.0, 1000.0);
        assert_eq!(distance_effect(&d, [0.0; 3], Some([250.0, 0.0, 0.0])), 0.75);
        assert_eq!(distance_effect(&d, [0.0; 3], Some([1000.0, 0.0, 0.0])), 0.0);
        assert_eq!(distance_effect(&d, [0.0; 3], None), 0.0);
        // A category duck never has a distance.
        let bank = catalog(&[d]);
        let mut m = DuckMixer::default();
        m.set_duck(&bank, CATEGORY_SCRIPT, "near", 0);
        assert_eq!(m.slot_list.iter().flatten().count(), 0);
    }

    #[test]
    fn voice_duck_lasts_its_clip_then_fades() {
        let d = duck("shot", 0.0, 0.25, 0.0, 0.0);
        let mut m = DuckMixer::default();
        let voice = Entity::from_raw_u32(7).unwrap();
        let source = DuckSource {
            duck: d.id,
            positional: false,
            repeats: false,
            total_ms: 1000,
            begun: true,
        };
        m.start_voice(&d, voice, &source, "shot", [0.0; 3], 0);
        let a = m.slot_list.iter().flatten().next().unwrap();
        assert_eq!(a.end_ms, 750);
        // A 2D sound may not carry a duck with a distance.
        let far = duck("far", 0.0, 0.25, 0.0, 500.0);
        m.start_voice(&far, voice, &source, "shot", [0.0; 3], 0);
        assert_eq!(m.slot_list.iter().flatten().count(), 1);
        // The voice stops before its duck took effect: the duck goes at once.
        m.end_voice(voice, 0);
        assert_eq!(m.slot_list.iter().flatten().count(), 0);
        // Stopped once it had: it fades out from then.
        m.start_voice(&d, voice, &source, "shot", [0.0; 3], 0);
        m.refresh(16, 0.016, None);
        m.end_voice(voice, 100);
        let a = m.slot_list.iter().flatten().next().unwrap();
        assert_eq!((a.end_ms, a.voice), (100, None));
    }

    #[test]
    fn looping_voice_duck_fades_with_its_sound() {
        let d = duck("loop", 0.5, 1.0, 0.0, 0.0);
        let mut m = DuckMixer::default();
        let voice = Entity::from_raw_u32(9).unwrap();
        let source = DuckSource {
            duck: d.id,
            positional: true,
            repeats: true,
            total_ms: 300,
            begun: true,
        };
        m.start_voice(&d, voice, &source, "loop", [0.0; 3], 0);
        assert_eq!(m.slot_list.iter().flatten().next().unwrap().end_ms, 0);
        m.fade_stop_voice(voice, 2000, 5000);
        assert_eq!(m.slot_list.iter().flatten().next().unwrap().end_ms, 6000);
    }

    #[test]
    fn clip_ms_counts_frames_not_samples() {
        let mono = ClipPcm::from_parts(Arc::from(vec![0.0f32; 8_000]), 1, 8_000).expect("mono");
        let stereo = ClipPcm::from_parts(Arc::from(vec![0.0f32; 8_000]), 2, 8_000).expect("stereo");
        assert_eq!(clip_ms(&mono), 1000);
        assert_eq!(clip_ms(&stereo), 500);
    }

    #[test]
    fn clip_ms_truncates_a_partial_millisecond() {
        let short = ClipPcm::from_parts(Arc::from(vec![0.0f32; 7]), 1, 8_000).expect("short");
        assert_eq!(clip_ms(&short), 0);
    }
}
