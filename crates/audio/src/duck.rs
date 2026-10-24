//! bo2mp: Black Ops II's sound ducks (`SndDuck`). While a duck is on, each of the 32 duck
//! groups is turned down toward the duck's attenuation for it; every sound is scaled by
//! its alias's duck group's volume. A sound whose alias names a duck starts one when it
//! starts and ends it when it stops; the game also holds one duck per category: the
//! shellshock's snapshot (a frag's `cmn_pain_plr`) and the scripts' snapshot (the
//! Reaper's cockpit, the end of the match). Black Ops II also low-passes each group
//! toward the duck's filter; that is not played (no filter yet, a typed gap).

use std::collections::HashSet;
use std::sync::Arc;

use asset_audio::{CapturedAlias, SndDuck, SoundCatalog};
use asset_core::AssetNamespace;
use bevy::{audio::Volume, prelude::*};
use frame::{ClientSet, MatchTornDown};
use net::{LocalPresentClient, PresentedSnapshot};

use crate::ambient::{MapAmbient, MapEmitter};
use crate::backend::{AudioScope, MatchEpoch, Voice};
use crate::clip_store::{ClipStore, clip_keys_for_alias};
use crate::messages::{AliasCommand, PlayAlias, SND_ENT_LOCAL};
use crate::pcm::{LoopingPcmAudio, PcmAudio};
use crate::playback::{AmbientListener, Channel3d, MissingAliasGaps, SoundBank};
use crate::space::transform_inches;

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
    /// The alias is 3D (flags bit 1): the duck follows the sound and may weaken with distance.
    positional: bool,
    /// The alias loops (flags bit 0): its duck lasts until the sound is stopped.
    looping: bool,
    /// The clip's length (not scaled by pitch, as Black Ops II).
    total_ms: i64,
    started: bool,
}

/// Tags a new sound with its alias's duck group and duck.
pub(crate) fn tag(e: &mut EntityCommands, row: Option<&CapturedAlias>, total_ms: i64) {
    let Some(row) = row else {
        return;
    };
    if let Some(group) = row.duck_group {
        e.insert(DuckGroup(group));
    }
    if let Some(duck) = row.duck {
        let flags = row.flags.unwrap_or(0);
        e.insert(DuckSource {
            duck,
            positional: flags & 2 != 0,
            looping: flags & 1 != 0,
            total_ms,
            started: false,
        });
    }
}

/// A clip's length in milliseconds.
pub(crate) fn clip_ms(pcm: &PcmAudio) -> i64 {
    let channels = u64::from(pcm.channel_count().max(1));
    let rate = u64::from(pcm.rate().max(1));
    let frames = pcm.samples().len() as u64 / channels;
    (frames * 1000 / rate) as i64
}

#[derive(Clone, Debug)]
struct Active {
    duck: SndDuck,
    start_ms: i64,
    /// 0 = no end yet.
    end_ms: i64,
    amount: f32,
    effect: f32,
    voice: Option<Entity>,
    position: [f32; 3],
}

/// The duck slots and the duck groups' volumes.
#[derive(Resource)]
pub struct DuckMixer {
    slots: Vec<Option<Active>>,
    category_index: [Option<usize>; CATEGORIES],
    value: [f32; GROUPS],
    linear: [f32; GROUPS],
    /// A group's volume moved this frame: sounds that are not rescaled every frame are.
    pub(crate) changed: bool,
    warned: HashSet<String>,
}

impl Default for DuckMixer {
    fn default() -> Self {
        Self {
            slots: vec![None; SLOTS],
            category_index: [None; CATEGORIES],
            value: [1.0; GROUPS],
            linear: [1.0; GROUPS],
            changed: false,
            warned: HashSet::new(),
        }
    }
}

fn is_default(duck: &SndDuck) -> bool {
    duck.name.eq_ignore_ascii_case("default")
}

impl DuckMixer {
    /// The volume scale for a sound in this duck group (1 for none).
    pub fn linear(&self, group: Option<u8>) -> f32 {
        group
            .and_then(|g| self.linear.get(usize::from(g)))
            .copied()
            .unwrap_or(1.0)
    }

    pub(crate) fn reset(&mut self) {
        let warned = std::mem::take(&mut self.warned);
        *self = Self {
            warned,
            changed: true,
            ..Self::default()
        };
    }

    fn warn_once(&mut self, line: String) {
        if self.warned.insert(line.clone()) {
            diag::warn!(Audio, "{line}");
        }
    }

    /// `SND_SetDuck(category, name, 0, 1)`: holds `name` on the category ("default", or
    /// an empty name, lets the category's duck go).
    pub(crate) fn set_duck(&mut self, bank: &SoundCatalog, category: usize, name: &str, now: i64) {
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

    fn set_duck_by_category(&mut self, category: usize, duck: &SndDuck, amount: f32, now: i64) {
        if category == CATEGORY_ALIAS || category >= CATEGORIES || duck.distance > EPS {
            return;
        }
        match self.category_index[category] {
            None if is_default(duck) => return,
            Some(i) => {
                if let Some(active) = self.slots[i].as_mut() {
                    if active.duck.id == duck.id && amount > EPS {
                        active.amount = amount;
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
        if is_default(duck) || amount < EPS {
            return;
        }
        let Some(free) = self.slots.iter().position(Option::is_none) else {
            self.warn_once("SOUND: no free sound ducks.".to_owned());
            return;
        };
        let timed = duck.length >= EPS;
        self.slots[free] = Some(Active {
            duck: duck.clone(),
            start_ms: now,
            end_ms: if timed {
                now + (duck.length * 1000.0) as i64
            } else {
                0
            },
            amount,
            effect: 0.0,
            voice: None,
            position: [0.0; 3],
        });
        if !timed {
            self.category_index[category] = Some(free);
        }
    }

    /// A sound's own duck, when its voice starts.
    fn start_voice(
        &mut self,
        duck: &SndDuck,
        voice: Entity,
        source: &DuckSource,
        alias: &str,
        position: [f32; 3],
        now: i64,
    ) {
        if duck.distance > EPS && !source.positional {
            self.warn_once(format!(
                "SOUND: cannot use a duck with distance ({}) on a 2d sound ({alias})",
                duck.name
            ));
            return;
        }
        let Some(free) = self.slots.iter().position(Option::is_none) else {
            self.warn_once("SOUND: no free sound ducks.".to_owned());
            return;
        };
        let end_ms = if duck.length > EPS {
            now + (duck.length * 1000.0) as i64
        } else if !source.looping {
            now + source.total_ms - (duck.fade_out * 1000.0) as i64
        } else {
            0
        };
        self.slots[free] = Some(Active {
            duck: duck.clone(),
            start_ms: now,
            end_ms,
            amount: 1.0,
            effect: 0.0,
            voice: Some(voice),
            position,
        });
    }

    /// A sound's voice stopped: its duck stops at once if it has no effect yet, else
    /// it fades out from now.
    fn end_voice(&mut self, voice: Entity, now: i64) {
        for i in 0..SLOTS {
            let Some(active) = self.slots[i].as_mut().filter(|a| a.voice == Some(voice)) else {
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
        for active in self.slots.iter_mut().flatten() {
            if active.voice == Some(voice) && active.end_ms == 0 && active.duck.length < EPS {
                active.end_ms = now - (active.duck.fade_out * 1000.0) as i64 + fade_ms;
            }
        }
    }

    fn stop(&mut self, slot: usize) {
        for index in &mut self.category_index {
            if *index == Some(slot) {
                *index = None;
            }
        }
        self.slots[slot] = None;
    }

    /// One frame: each duck's effect, then each group's volume moves toward the lowest
    /// any duck asks for, by at most a tenth (`dt` x 10) a frame.
    fn update(&mut self, now: i64, dt: f32, listener: Option<[f32; 3]>) {
        let mut target = [1.0f32; GROUPS];
        for i in 0..SLOTS {
            let Some(active) = self.slots[i].as_mut() else {
                continue;
            };
            let Some(time) = time_effect(&active.duck, active.start_ms, active.end_ms, now) else {
                self.stop(i);
                continue;
            };
            let distance = distance_effect(&active.duck, active.position, listener);
            active.effect = active.amount * time.min(distance);
            for (t, att) in target.iter_mut().zip(&active.duck.attenuation) {
                *t = t.min(1.0 - active.effect * (1.0 - att));
            }
        }
        let rate = (dt * 10.0).min(0.1);
        let mut changed = false;
        for (j, t) in target.iter().enumerate() {
            let v = self.value[j];
            let next = if *t > v {
                (v + rate).min(*t)
            } else {
                (v - rate).max(*t)
            };
            self.value[j] = next;
            let linear = next * next * next;
            changed |= linear != self.linear[j];
            self.linear[j] = linear;
        }
        self.changed = changed;
    }
}

/// A duck's fade: None once it has faded out (its slot is freed).
fn time_effect(duck: &SndDuck, start_ms: i64, end_ms: i64, now: i64) -> Option<f32> {
    let t = (now - start_ms) as f32 / 1000.0 - duck.start_delay;
    if end_ms == 0 || now <= end_ms {
        return Some(if t < 0.0 {
            0.0
        } else if t >= duck.fade_in {
            1.0
        } else {
            t / duck.fade_in
        });
    }
    let e = (now - end_ms) as f32 / 1000.0;
    if t < 0.0 || e > duck.fade_out {
        None
    } else if t >= duck.fade_in {
        Some(1.0 - e / duck.fade_out)
    } else {
        Some((1.0 - ((1.0 - t / duck.fade_in) * duck.fade_out + e) / duck.fade_out).max(0.0))
    }
}

/// A duck with a distance weakens from full at its sound to nothing at that distance.
fn distance_effect(duck: &SndDuck, position: [f32; 3], listener: Option<[f32; 3]>) -> f32 {
    if duck.distance <= EPS {
        return 1.0;
    }
    let Some(ear) = listener else {
        return 0.0;
    };
    let d2: f32 = (0..3).map(|k| (position[k] - ear[k]).powi(2)).sum();
    if d2 < duck.distance * duck.distance {
        1.0 - d2.sqrt() / duck.distance
    } else {
        0.0
    }
}

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct DuckSet;

pub(crate) fn register(app: &mut App) {
    app.init_resource::<DuckMixer>().add_systems(
        Update,
        (
            (
                crate::shellshock::update_shellshock_sound_t6
                    .run_if(crate::shellshock::bank_is_t6),
                script_sound_notifies,
                update_mixer,
            )
                .chain()
                .in_set(DuckSet)
                .after(crate::shellshock::update_shellshock_tinnitus),
            crate::shellshock::update_shock_loop_volume_t6.after(DuckSet),
        )
            .in_set(ClientSet::Effects),
    );
}

pub(crate) fn now_ms(time: &Time<Real>) -> i64 {
    time.elapsed().as_millis() as i64
}

#[allow(clippy::type_complexity)]
fn update_mixer(
    time: Res<Time<Real>>,
    mut torn: MessageReader<MatchTornDown>,
    mut mixer: ResMut<DuckMixer>,
    bank: Option<Res<SoundBank>>,
    listeners: Query<&Transform, With<AmbientListener>>,
    mut sources: Query<(
        Entity,
        &mut DuckSource,
        Has<Voice>,
        Option<&Channel3d>,
        Option<&MapEmitter>,
        Option<&crate::playback::AliasPlayback>,
    )>,
) {
    if torn.read().count() > 0 {
        mixer.reset();
    }
    let now = now_ms(&time);
    // A voice that stopped (or was despawned) ends its duck.
    let ended: Vec<Entity> = mixer
        .slots
        .iter()
        .flatten()
        .filter_map(|a| a.voice)
        .filter(|&e| sources.get(e).map_or(true, |(_, _, has_voice, ..)| !has_voice))
        .collect();
    for voice in ended {
        mixer.end_voice(voice, now);
    }
    for (entity, mut source, has_voice, ch, emitter, playback) in &mut sources {
        let position = ch
            .map(|c| c.origin_inches)
            .or_else(|| emitter.map(|m| m.origin_inches))
            .unwrap_or([0.0; 3]);
        if !has_voice {
            source.started = false;
            continue;
        }
        if !source.started {
            source.started = true;
            let Some(duck) = bank.as_ref().and_then(|b| b.0.ducks.get(&source.duck)).cloned()
            else {
                continue;
            };
            let alias = playback.map_or("", |p| p.alias.as_str());
            mixer.start_voice(&duck, entity, &source, alias, position, now);
            continue;
        }
        if source.positional {
            for active in mixer.slots.iter_mut().flatten() {
                if active.voice == Some(entity) {
                    active.position = position;
                }
            }
        }
    }
    let listener = listeners.iter().next().map(|t| transform_inches(t.translation));
    mixer.update(now, time.delta_secs(), listener);
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
    mut torn: MessageReader<MatchTornDown>,
    presented: Option<Res<PresentedSnapshot>>,
    local: Option<Res<LocalPresentClient>>,
    bank: Option<Res<SoundBank>>,
    mut mixer: ResMut<DuckMixer>,
    mut play: MessageWriter<AliasCommand>,
    mut commands: Commands,
    mut clips: Option<ResMut<ClipStore>>,
    mut looping_assets: ResMut<Assets<LoopingPcmAudio>>,
    mut pcm_assets: ResMut<Assets<PcmAudio>>,
    mut gaps: ResMut<MissingAliasGaps>,
    epoch: Res<MatchEpoch>,
    beds: Query<Entity, With<ReaperBed>>,
    mut state: Local<ScriptSound>,
) {
    if torn.read().count() > 0 {
        *state = ScriptSound::default();
        for entity in &beds {
            crate::backend::stop(&mut commands, entity);
        }
    }
    let (Some(presented), Some(local), Some(bank)) = (presented, local, bank) else {
        return;
    };
    let now = now_ms(&time);
    let value = presented
        .snapshot()
        .map(|snap| {
            snap.meta
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
            diag::info!(Audio, "audio: script sound notify `{notify}`");
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
                    play.write(AliasCommand::Play(PlayAlias {
                        namespace: AssetNamespace::T6,
                        alias: FINAL_KILLCAM_STING.to_owned(),
                        fallback: None,
                        origin_inches: None,
                        snd_ent: Some(SND_ENT_LOCAL),
                    }));
                }
                _ => {}
            }
        }
        state.seen = newest;
    }
    if state.full_at.is_some_and(|at| now >= at) {
        state.full_at = None;
        set_snapshot(&mut state, &mut mixer, &bank.0, POST_MATCH_FULL_SNAPSHOT, now);
    }
    if !state.want_bed {
        for entity in &beds {
            crate::backend::stop(&mut commands, entity);
        }
    } else if beds.is_empty() {
        start_reaper_bed(
            &mut commands,
            &bank,
            clips.as_deref_mut(),
            &mut looping_assets,
            &mut pcm_assets,
            &mut gaps,
            epoch.0,
        );
    }
}

/// `snd_set_snapshot`: a script snapshot is set only when it changes.
fn set_snapshot(
    state: &mut ScriptSound,
    mixer: &mut DuckMixer,
    bank: &SoundCatalog,
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
    bank: &SoundBank,
    clips: Option<&mut ClipStore>,
    looping_assets: &mut Assets<LoopingPcmAudio>,
    pcm_assets: &mut Assets<PcmAudio>,
    gaps: &mut MissingAliasGaps,
    epoch: u64,
) {
    let Some(clips) = clips else {
        return;
    };
    let Some(key) = clip_keys_for_alias(&bank.0, AssetNamespace::T6, REAPER_LOOP)
        .into_iter()
        .next()
    else {
        gaps.record(REAPER_LOOP);
        return;
    };
    clips.request(key.clone());
    let Some(Ok(pcm)) = clips.ready(&key) else {
        return;
    };
    let row = bank
        .0
        .sound_in(AssetNamespace::T6, REAPER_LOOP)
        .and_then(|s| s.aliases.first());
    let volume = row.map_or(1.0, |a| a.vol_min.max(0.0));
    let positional = row.and_then(|a| a.flags).is_some_and(|f| f & 2 != 0);
    let mut e = if positional {
        let row = row.expect("positional row");
        let knots: Arc<[[f32; 2]]> = row
            .volume_falloff
            .as_ref()
            .map(|c| c.knots.iter().map(|&(x, y)| [x, y]).collect::<Vec<_>>())
            .unwrap_or_default()
            .into();
        commands.spawn((
            MapAmbient,
            MapEmitter {
                origin_inches: [0.0; 3],
                dist_min: row.dist_min,
                dist_max: row.dist_max,
                knots,
                base_gain: volume,
                pcm: pcm_assets.add(pcm),
                speed: 1.0,
                live_pan: None,
                fixed_pan: row.stereo_pan,
                category: row.menu_category,
            },
            Transform::default(),
        ))
    } else {
        let pcm = crate::pcm::with_stereo_pan(pcm, row.and_then(|a| a.stereo_pan));
        let handle = looping_assets.add(pcm.into_looping());
        let entity = crate::backend::spawn_loop(
            commands,
            handle,
            Volume::Linear(volume),
            epoch,
            AudioScope::Match,
        );
        let mut e = commands.entity(entity);
        if let Some(category) = row.and_then(|a| a.menu_category) {
            e.insert(crate::MenuVolume {
                category,
                base: volume,
            });
        }
        e
    };
    e.insert(ReaperBed);
    tag(&mut e, row, 0);
    diag::info!(
        Audio,
        "audio: Reaper cockpit loop at {volume:.2} ({})",
        if positional { "3D at the origin" } else { "2D" }
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn duck(name: &str, fade_in: f32, fade_out: f32, length: f32, distance: f32) -> SndDuck {
        let mut attenuation = [1.0; 32];
        attenuation[3] = 0.0;
        attenuation[5] = 0.5;
        SndDuck {
            name: name.to_owned(),
            id: name.bytes().fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b))),
            fade_in,
            fade_out,
            start_delay: 0.0,
            distance,
            length,
            update_while_paused: false,
            attenuation,
            filter: [1.0; 32],
        }
    }

    fn catalog(ducks: &[SndDuck]) -> SoundCatalog {
        let mut c = SoundCatalog::default();
        for d in ducks {
            c.ducks.insert(d.id, d.clone());
        }
        c
    }

    fn settle(m: &mut DuckMixer, from: i64, to: i64) {
        let mut now = from;
        while now <= to {
            m.update(now, 0.016, Some([0.0; 3]));
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
        assert_eq!(m.slots.iter().flatten().count(), 1);
        m.set_duck(&bank, CATEGORY_SHELLSHOCK, "default", 1000);
        settle(&mut m, 1016, 3000);
        assert_eq!(m.slots.iter().flatten().count(), 0);
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
        assert_eq!(m.slots.iter().flatten().count(), 0);
        assert_eq!(m.linear(Some(3)), 1.0);
    }

    #[test]
    fn volume_moves_a_tenth_a_frame_at_most() {
        let bank = catalog(&[duck("snap", 0.0, 0.5, 0.0, 0.0)]);
        let mut m = DuckMixer::default();
        m.set_duck(&bank, CATEGORY_SCRIPT, "snap", 0);
        m.update(0, 0.5, None);
        assert!((m.value[3] - 0.9).abs() < 1e-6);
        assert!((m.linear(Some(3)) - 0.729).abs() < 1e-5);
        m.update(16, 0.005, None);
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
        assert_eq!(m.slots.iter().flatten().count(), 0);
    }

    #[test]
    fn voice_duck_lasts_its_clip_then_fades() {
        let d = duck("shot", 0.0, 0.25, 0.0, 0.0);
        let mut m = DuckMixer::default();
        let voice = Entity::from_raw_u32(7).unwrap();
        let source = DuckSource {
            duck: d.id,
            positional: false,
            looping: false,
            total_ms: 1000,
            started: true,
        };
        m.start_voice(&d, voice, &source, "shot", [0.0; 3], 0);
        let a = m.slots.iter().flatten().next().unwrap();
        assert_eq!(a.end_ms, 750);
        // A 2D sound may not carry a duck with a distance.
        let far = duck("far", 0.0, 0.25, 0.0, 500.0);
        m.start_voice(&far, voice, &source, "shot", [0.0; 3], 0);
        assert_eq!(m.slots.iter().flatten().count(), 1);
        // The voice stops before its duck took effect: the duck goes at once.
        m.end_voice(voice, 0);
        assert_eq!(m.slots.iter().flatten().count(), 0);
        // Stopped once it had: it fades out from then.
        m.start_voice(&d, voice, &source, "shot", [0.0; 3], 0);
        m.update(16, 0.016, None);
        m.end_voice(voice, 100);
        let a = m.slots.iter().flatten().next().unwrap();
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
            looping: true,
            total_ms: 300,
            started: true,
        };
        m.start_voice(&d, voice, &source, "loop", [0.0; 3], 0);
        assert_eq!(m.slots.iter().flatten().next().unwrap().end_ms, 0);
        m.fade_stop_voice(voice, 2000, 5000);
        assert_eq!(m.slots.iter().flatten().next().unwrap().end_ms, 6000);
    }
}
