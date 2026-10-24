//! The map's own sound: the ambient room tone, CreateFX loop emitters, the
//! sound bank (composed in the background while a match loads) and the
//! per-frame gain of each positional emitter.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use asset_audio::{
    LoadedSoundBank, RandomPointSound, SoundCatalog, compose_sound_bank, gather_sound_sources,
    namespace_for_zone,
};
use asset_core::AssetNamespace;
use asset_transport::GamesRoot;
use assets::{NamespaceSoundIwd, NamespaceTrees};
use bevy::{
    audio::{AudioSink, AudioSinkPlayback, Volume},
    prelude::*,
    tasks::{AsyncComputeTaskPool, Task, TaskPool, futures_lite::future},
};
use frame::{MatchTornDown, ReturnedToMenu};

use crate::attenuation::attenuate;
use crate::backend::{AudioScope, MatchEpoch, Voice};
use crate::pcm::{LoopingPcmAudio, PcmAudio};
use crate::playback::{
    AmbientListener, MissingAliasGaps, SharedPlayAssets, SoundBank, world_oneshot_channel_gains,
};
use crate::space::{distance_inches, transform_inches};

/// Marks everything that belongs to the map's ambience and goes with the match.
#[derive(Component)]
pub struct MapAmbient;

/// Only this many emitters hold a voice at once; the loudest win.
pub const MAX_ACTIVE_MAP_EMITTERS: usize = 8;

/// An emitter quieter than this is not worth a voice.
pub const MIN_AUDIBLE_EMITTER_GAIN: f32 = 0.002;

#[derive(Component)]
pub struct MapEmitter {
    pub origin_inches: [f32; 3],
    pub dist_min: f32,
    pub dist_max: f32,
    pub knots: Arc<[[f32; 2]]>,
    pub base_gain: f32,
    pub pcm: Handle<PcmAudio>,
    /// Playback speed (pitch scale; 1 = as recorded): a vehicle engine's loop
    /// follows its speed.
    pub speed: f32,
    pub live_pan: Option<crate::pcm::LivePan>,
    /// A sound with a pan of its own falls off with distance but does not
    /// turn with the listener.
    pub fixed_pan: Option<asset_audio::StereoPan>,
    /// The menu volume this sound answers to.
    pub category: Option<u8>,
}

#[derive(Resource, Clone)]
pub struct SoundIwd(pub Arc<NamespaceSoundIwd>);

#[derive(Resource, Default)]
pub struct MapAmbientBooted(pub bool);

/// A xorshift step; the sequence only needs to look scattered.
fn xorshift(state: &mut u32) -> u32 {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    *state
}

/// Whole seconds to wait before a sound's next play: somewhere in its range.
fn wait_seconds(sound: &RandomPointSound, state: &mut u32) -> f64 {
    let low = sound.wait_secs[0].max(0.0) as u32;
    let high = (sound.wait_secs[1].max(0.0) as u32).max(low + 1);
    f64::from(low + xorshift(state) % (high - low))
}

/// A Black Ops II map's now-and-then sounds (Nuketown's thunder): each waits a
/// whole number of seconds in its range, then plays at one of its places.
pub(crate) fn play_random_point_sounds(
    time: Res<Time>,
    bank: Option<Res<SoundBank>>,
    booted: Res<MapAmbientBooted>,
    epoch: Res<MatchEpoch>,
    // (epoch the timers are for, each sound's next play time, rng state)
    mut timers: Local<(u64, Vec<f64>, u32)>,
    mut play: MessageWriter<crate::AliasCommand>,
) {
    let Some(bank) = bank.filter(|_| booted.0) else {
        return;
    };
    let sounds = bank.0.random_point_sounds();
    if sounds.is_empty() {
        return;
    }

    let now = time.elapsed_secs_f64();
    if timers.0 != epoch.0 || timers.1.len() != sounds.len() {
        timers.0 = epoch.0;
        let mut state = 0x9e37_79b9 ^ (now.to_bits() as u32);
        timers.1 = sounds
            .iter()
            .map(|sound| now + wait_seconds(sound, &mut state))
            .collect();
        timers.2 = state;
    }

    let mut state = timers.2;
    for (index, sound) in sounds.iter().enumerate() {
        if now < timers.1[index] || sound.places_inches.is_empty() {
            continue;
        }
        let pick = xorshift(&mut state) as usize % sound.places_inches.len();
        play.write(crate::AliasCommand::Play(crate::PlayAlias {
            namespace: AssetNamespace::T6,
            alias: sound.soundalias.clone(),
            fallback: None,
            origin_inches: Some(sound.places_inches[pick]),
            snd_ent: None,
        }));
        timers.1[index] = now + wait_seconds(sound, &mut state);
    }
    timers.2 = state;
}

/// Set once the sound bank for the current load has been started (or skipped).
#[derive(Resource, Default)]
pub(crate) struct SoundBankLoadAttempted(pub bool);

/// How long composing may take before it is reported as stuck.
const COMPOSE_STALL: Duration = Duration::from_secs(20);

/// A sound bank being composed off-thread for the accepted load.
#[derive(Resource)]
pub(crate) struct SoundBankCompose {
    load_key: frame::LocalLoadKey,
    zone: String,
    iwd: IwdOpen,
    bank: Option<Task<ComposedBank>>,
    common_profile_id: u64,
    products_id: u64,
    started: Instant,
    stall_reported: bool,
}

/// The loose-file sound index: opening, or open.
enum IwdOpen {
    Opening(Task<(NamespaceSoundIwd, Vec<String>)>),
    Open(Arc<NamespaceSoundIwd>),
}

impl IwdOpen {
    fn is_open(&self) -> bool {
        matches!(self, Self::Open(_))
    }
}

struct ComposedBank {
    loaded: Result<(Arc<SoundCatalog>, usize), String>,
    namespace: AssetNamespace,
    reused: bool,
}

/// The last composed bank, kept so loading the same map again skips the work.
#[derive(Resource, Default)]
pub(crate) struct ResidentSoundBank(Option<ResidentBank>);

struct ResidentBank {
    zone: String,
    iwd: Arc<NamespaceSoundIwd>,
    bank: Option<ResidentComposed>,
}

struct ResidentComposed {
    products_id: u64,
    common_profile_id: u64,
    catalog: Arc<SoundCatalog>,
    namespace: AssetNamespace,
    gaps: usize,
}

/// Which zone the live sound bank was composed for, and its game.
#[derive(Resource)]
pub(crate) struct SoundBankNamespace {
    pub(crate) zone: String,
    pub(crate) namespace: AssetNamespace,
}

pub fn stop_map_ambient(commands: &mut Commands, ambient: &Query<Entity, With<MapAmbient>>) {
    for entity in ambient {
        commands.entity(entity).despawn();
    }
}

pub(crate) fn stop_map_ambient_on_match_end(
    mut torn: MessageReader<MatchTornDown>,
    mut returned: MessageReader<ReturnedToMenu>,
    ambient: Query<Entity, With<MapAmbient>>,
    mut booted: ResMut<MapAmbientBooted>,
    mut attempted: ResMut<SoundBankLoadAttempted>,
    mut epoch: ResMut<MatchEpoch>,
    compose: Option<Res<SoundBankCompose>>,
    accepted: Option<Res<assets::MatchLoadAccepted>>,
    mut commands: Commands,
) {
    let retired: Vec<_> = torn.read().map(|fact| fact.world_generation).collect();
    let left_session = returned.read().count() > 0;
    if retired.is_empty() && !left_session {
        return;
    }
    epoch.bump();
    stop_map_ambient(&mut commands, &ambient);
    booted.0 = false;
    commands.remove_resource::<asset_audio::CreateFxOneshotEmitters>();

    // A bank already composing for the map that is loading next stays.
    let keeps_compose = !left_session
        && compose
            .as_deref()
            .zip(accepted.as_deref())
            .is_some_and(|(compose, accepted)| {
                let generation =
                    frame::WorldGeneration::from_install(compose.load_key.local_load_request_id);
                compose.load_key == accepted.load_key && !retired.contains(&generation)
            });
    if !keeps_compose {
        commands.remove_resource::<SoundBankCompose>();
        commands.remove_resource::<assets::PreparedMatchSound>();
        attempted.0 = false;
    }
    commands.queue(|world: &mut World| {
        frame::retire::retire_resources(world, |batch| {
            batch
                .resource::<SoundBankNamespace>()
                .resource::<SoundBank>()
                .resource::<SoundIwd>()
                .resource::<crate::ClipStore>();
        });
    });
    perf::ambient_hold(i64::from(booted.0));
    diag::info!(Audio, "audio: map ambient stopped");
}

pub(crate) fn start_sound_bank_compose(
    mut attempted: ResMut<SoundBankLoadAttempted>,
    accepted: Option<Res<assets::MatchLoadAccepted>>,
    abort: Option<Res<assets::MatchLoadAbort>>,
    identity: Option<Res<frame::LaunchIdentity>>,
    silent: Option<Res<crate::AudioSilent>>,
    resident: Res<ResidentSoundBank>,
    mut commands: Commands,
) {
    if attempted.0 {
        return;
    }
    if silent.is_some() && !crate::AudioSilent::composes() {
        attempted.0 = true;
        return;
    }
    let Some(accepted) = accepted else {
        return;
    };
    if abort.is_some_and(|abort| abort.0 == accepted.request_id) {
        return;
    }
    let Some(identity) = identity else {
        return;
    };
    if accepted.zone.is_empty() || identity.games_root.as_os_str().is_empty() {
        return;
    }
    attempted.0 = true;

    let zone = accepted.zone.clone();
    let iwd = match resident.0.as_ref().filter(|kept| kept.zone == zone) {
        Some(kept) => {
            diag::info!(Audio, "audio: sound archives for `{zone}` reused");
            IwdOpen::Open(Arc::clone(&kept.iwd))
        }
        None => {
            let games = GamesRoot(identity.games_root.clone());
            let zone = zone.clone();
            let pool = AsyncComputeTaskPool::get_or_init(TaskPool::default);
            IwdOpen::Opening(pool.spawn(async move {
                let mut trees = NamespaceTrees::discover(&games);
                if let Ok(file) = asset_transport::find_zone_file(&games, &zone) {
                    trees.adopt_zone(&file.path);
                }
                NamespaceSoundIwd::open(&trees)
            }))
        }
    };
    commands.insert_resource(SoundBankCompose {
        load_key: accepted.load_key,
        zone,
        iwd,
        bank: None,
        common_profile_id: 0,
        products_id: 0,
        started: Instant::now(),
        stall_reported: false,
    });
}

/// Composes the bank for `zone` from the map's sound sources.
async fn compose_zone_bank(
    games: Option<GamesRoot>,
    zone: String,
    map: Result<SoundCatalog, String>,
) -> ComposedBank {
    let Some(games) = games else {
        return ComposedBank {
            loaded: Err("no launch identity to find the zones by".to_owned()),
            namespace: AssetNamespace::T6,
            reused: false,
        };
    };
    let namespace = namespace_for_zone(&games, &zone);
    let loaded = asset_transport::find_zone_file(&games, &zone).map(|found| {
        let sources = gather_sound_sources(&games, &found.path);
        let LoadedSoundBank { catalog, gaps, .. } =
            compose_sound_bank(sources, &zone, namespace, map);
        (Arc::new(catalog), gaps.len())
    });
    ComposedBank {
        loaded,
        namespace,
        reused: false,
    }
}

pub(crate) fn install_sound_bank(
    mut compose: Option<ResMut<SoundBankCompose>>,
    identity: Option<Res<frame::LaunchIdentity>>,
    accepted: Option<Res<assets::MatchLoadAccepted>>,
    abort: Option<Res<assets::MatchLoadAbort>>,
    mut map_sound: Option<ResMut<assets::PreparedMatchSound>>,
    mut epoch: ResMut<MatchEpoch>,
    resident_clips: Res<crate::clip_store::ResidentClipCache>,
    mut resident: ResMut<ResidentSoundBank>,
    mut commands: Commands,
) {
    let Some(compose) = compose.as_deref_mut() else {
        return;
    };

    let superseded = accepted
        .as_ref()
        .is_some_and(|accepted| accepted.load_key != compose.load_key);
    let aborted = abort.is_some_and(|abort| abort.0 == compose.load_key.local_load_request_id);
    if aborted || superseded {
        diag::info!(
            Audio,
            "audio: sound bank for `{}` dropped — the session moved on",
            compose.zone
        );
        commands.remove_resource::<SoundBankCompose>();
        return;
    }
    // Install on the load key, not on `identity.zone`: gating on a display
    // spelling that never converges would hold AudioReady and the world spawn
    // forever.
    if let Some(zone) = identity
        .as_ref()
        .map(|identity| identity.zone.as_str())
        .filter(|zone| *zone != compose.zone)
    {
        diag::warn!(
            Audio,
            "audio: launch identity says `{zone}` while the accepted load walked `{}` — installing on the load key",
            compose.zone
        );
    }

    if let IwdOpen::Opening(task) = &mut compose.iwd
        && let Some((index, lines)) = future::block_on(future::poll_once(task))
    {
        for line in lines {
            diag::info!(Audio, "{line}");
        }
        compose.iwd = IwdOpen::Open(Arc::new(index));
    }

    // The map's sound arrives from the loader; start composing once it does.
    if compose.bank.is_none()
        && let Some(arrived) = map_sound.as_deref_mut()
        && arrived.load_key == compose.load_key
    {
        compose.common_profile_id = arrived.common_profile_id;
        compose.products_id = arrived.products_id;
        let map = std::mem::replace(&mut arrived.sound, Err(String::new()));
        commands.remove_resource::<assets::PreparedMatchSound>();

        let kept = resident
            .0
            .as_ref()
            .filter(|kept| kept.zone == compose.zone)
            .and_then(|kept| kept.bank.as_ref())
            .filter(|bank| {
                bank.products_id == compose.products_id
                    && bank.common_profile_id == compose.common_profile_id
            })
            .map(|bank| ComposedBank {
                loaded: Ok((Arc::clone(&bank.catalog), bank.gaps)),
                namespace: bank.namespace,
                reused: true,
            });
        let games = identity
            .as_ref()
            .map(|identity| GamesRoot(identity.games_root.clone()));
        let zone = compose.zone.clone();
        let pool = AsyncComputeTaskPool::get_or_init(TaskPool::default);
        compose.bank = Some(pool.spawn(async move {
            match kept {
                Some(kept) => kept,
                None => compose_zone_bank(games, zone, map).await,
            }
        }));
    }

    let composed = match (&compose.iwd, compose.bank.as_mut()) {
        (IwdOpen::Open(_), Some(task)) => future::block_on(future::poll_once(task)),
        _ => None,
    };
    let Some(composed) = composed else {
        if !compose.stall_reported && compose.started.elapsed() >= COMPOSE_STALL {
            compose.stall_reported = true;
            diag::warn!(
                Audio,
                "audio: sound bank for `{}` still not composed after {:.0}s (archives {}, map sound {}) — AudioReady, the world spawn and host admission all wait on it",
                compose.zone,
                compose.started.elapsed().as_secs_f32(),
                if compose.iwd.is_open() { "open" } else { "opening" },
                if compose.bank.is_some() { "arrived" } else { "pending" },
            );
        }
        return;
    };
    commands.remove_resource::<SoundBankCompose>();
    let IwdOpen::Open(iwd) = std::mem::replace(&mut compose.iwd, IwdOpen::Open(Arc::default()))
    else {
        unreachable!("the bank is polled only once the archives are open");
    };

    let ComposedBank {
        loaded,
        namespace,
        reused,
    } = composed;
    let (bank, gaps) = match loaded {
        Ok(composed) => composed,
        Err(error) => {
            diag::warn!(
                Audio,
                "audio: sound bank load failed for {}: {error}",
                compose.zone
            );
            return;
        }
    };

    epoch.bump();
    if iwd.is_empty() {
        diag::warn!(
            Audio,
            "audio: no IWD sound archives for `{}` — streamed aliases will gap",
            compose.zone
        );
    }
    commands.insert_resource(SoundIwd(Arc::clone(&iwd)));
    resident.0 = Some(ResidentBank {
        zone: compose.zone.clone(),
        iwd: Arc::clone(&iwd),
        bank: Some(ResidentComposed {
            products_id: compose.products_id,
            common_profile_id: compose.common_profile_id,
            catalog: Arc::clone(&bank),
            namespace,
            gaps,
        }),
    });

    let new_clips = crate::ClipStore::start_with_common(
        Arc::clone(&bank),
        Some(iwd),
        compose.common_profile_id,
        Some(resident_clips.clone()),
    );
    commands.queue(move |world: &mut World| {
        frame::retire::retire_resources(world, |batch| {
            batch.resource::<crate::ClipStore>();
        });
        world.insert_resource(new_clips);
    });
    commands.insert_resource(crate::clip_store::PendingStarts::default());
    commands.insert_resource(crate::playback::SharedPlayAssets::default());
    commands.insert_resource(SoundBank(bank));
    diag::info!(
        Audio,
        "audio: sound bank {} for {} ({} zone gaps) in {:.0}ms",
        if reused { "reused" } else { "ready" },
        compose.zone,
        gaps,
        compose.started.elapsed().as_secs_f32() * 1000.0
    );
    commands.insert_resource(SoundBankNamespace {
        zone: compose.zone.clone(),
        namespace,
    });
}

pub(crate) fn boot_map_ambient_once(
    mut booted: ResMut<MapAmbientBooted>,
    loading: Option<Res<assets::LoadingScreen>>,
    bank: Option<Res<SoundBank>>,
    identity: Option<Res<frame::LaunchIdentity>>,
    script_sound: Option<Res<asset_audio::SessionMapScriptSound>>,
    namespace: Option<Res<SoundBankNamespace>>,
    epoch: Res<MatchEpoch>,
    clips: Option<Res<crate::ClipStore>>,
    ready: Res<crate::AudioReady>,
    mut commands: Commands,
    mut pcm_assets: ResMut<Assets<PcmAudio>>,
    mut looping_assets: ResMut<Assets<LoopingPcmAudio>>,
    mut shared: ResMut<SharedPlayAssets>,
    mut gaps: ResMut<MissingAliasGaps>,
) {
    if booted.0 || !ready.0 || loading.is_some_and(|screen| !screen.is_complete()) {
        return;
    }
    let (Some(bank), Some(identity)) = (bank, identity) else {
        return;
    };
    if identity.zone.is_empty() {
        return;
    }
    let Some(namespace) = namespace.filter(|namespace| namespace.zone == identity.zone) else {
        return;
    };

    let ambient_alias = script_sound
        .as_deref()
        .and_then(|facts| facts.0.ambient_alias.as_deref());
    start_map_ambient_prepared(
        &mut commands,
        &mut pcm_assets,
        &mut looping_assets,
        &mut shared,
        bank.0.as_ref(),
        namespace.namespace,
        &identity.zone,
        ambient_alias,
        clips.as_deref(),
        epoch.0,
        &mut gaps,
    );
    booted.0 = true;
    perf::ambient_boot(&identity.zone, ambient_alias);
    diag::info!(
        Audio,
        "audio: map ambient boot complete for {}",
        identity.zone
    );
}

/// Starts the room-tone loop the map script named (if any).
fn start_room_tone(
    commands: &mut Commands,
    looping_assets: &mut Assets<LoopingPcmAudio>,
    gaps: &mut MissingAliasGaps,
    bank: &SoundCatalog,
    namespace: AssetNamespace,
    map_name: &str,
    alias: Option<&str>,
    clips: Option<&crate::ClipStore>,
    epoch: u64,
) {
    let Some(alias) = alias else {
        diag::warn!(
            Audio,
            "audio: no ambientPlay in map script for `{map_name}` (typed gap)"
        );
        return;
    };
    let Some(pcm) = pcm_for_map_alias(clips, bank, namespace, alias) else {
        gaps.record(alias);
        diag::warn!(
            Audio,
            "audio: ambient alias `{alias}` unresolved for {map_name}"
        );
        return;
    };

    // A Black Ops II room tone plays at its alias's volume, through its pan,
    // in its menu category.
    let row = (namespace == AssetNamespace::T6)
        .then(|| bank.sound_in(namespace, alias))
        .flatten()
        .and_then(|sound| sound.aliases.first());
    let volume = row.map_or(0.55, |row| row.vol_min.max(0.0));
    let pcm = crate::pcm::with_stereo_pan(pcm, row.and_then(|row| row.stereo_pan));
    let handle = looping_assets.add(pcm.into_looping());
    let entity =
        crate::backend::spawn_loop(commands, handle, Volume::Linear(volume), epoch, AudioScope::Match);
    commands.entity(entity).insert(MapAmbient);
    crate::duck::tag(&mut commands.entity(entity), row, 0);
    if let Some(category) = row.and_then(|row| row.menu_category) {
        commands.entity(entity).insert(crate::MenuVolume {
            category,
            base: volume,
        });
    }
    diag::info!(Audio, "audio: ambient loop `{alias}`");
}

fn start_map_ambient_prepared(
    commands: &mut Commands,
    pcm_assets: &mut Assets<PcmAudio>,
    looping_assets: &mut Assets<LoopingPcmAudio>,
    shared: &mut SharedPlayAssets,
    bank: &SoundCatalog,
    map_ns: AssetNamespace,
    map_name: &str,
    ambient_alias: Option<&str>,
    clips: Option<&crate::ClipStore>,
    epoch: u64,
    gaps: &mut MissingAliasGaps,
) {
    start_room_tone(
        commands,
        looping_assets,
        gaps,
        bank,
        map_ns,
        map_name,
        ambient_alias,
        clips,
        epoch,
    );

    // CreateFX loop emitters: one PCM asset per alias, one entity per emitter.
    let loops = bank.createfx_loop_sounds(map_ns, map_name);
    let mut handles: HashMap<&str, Handle<PcmAudio>> = HashMap::new();
    let (mut started, mut missed) = (0usize, 0usize);
    for emitter in &loops {
        let alias = emitter.soundalias.as_str();
        let handle = match handles.get(alias) {
            Some(handle) => handle.clone(),
            None => {
                let Some(pcm) = pcm_for_map_alias(clips, bank, map_ns, alias) else {
                    gaps.record(alias);
                    missed += 1;
                    continue;
                };
                let handle = pcm_assets.add(pcm);
                handles.insert(alias, handle.clone());
                handle
            }
        };

        let row = bank
            .sound_in(map_ns, alias)
            .or_else(|| {
                let index = bank.index_unique(alias)?;
                bank.sounds.get(index)
            })
            .and_then(|sound| sound.aliases.first());
        let (dist_min, dist_max, knots, base_gain) = match row {
            Some(row) => {
                let knots = match &row.volume_falloff {
                    Some(curve) => shared.intern_curve(&curve.name, &curve.knots),
                    None => Arc::from(Vec::<[f32; 2]>::new()),
                };
                if knots.is_empty() {
                    diag::warn!(
                        Audio,
                        "audio: CreateFX loop `{alias}` has no falloff curve (typed gap)"
                    );
                }
                (row.dist_min, row.dist_max, knots, row.vol_min.max(0.0))
            }
            None => (0.0, 0.0, Arc::from(Vec::<[f32; 2]>::new()), 0.0),
        };

        let origin_inches = emitter.origin_inches;
        let mut spawned = commands.spawn((
            MapAmbient,
            MapEmitter {
                origin_inches,
                dist_min,
                dist_max,
                knots,
                base_gain,
                pcm: handle,
                speed: 1.0,
                live_pan: None,
                fixed_pan: row.and_then(|row| row.stereo_pan),
                category: row.and_then(|row| row.menu_category),
            },
            Transform::from_translation(Vec3::from_array(origin_inches)),
        ));
        crate::duck::tag(&mut spawned, row, 0);
        started += 1;
    }
    if started > 0 {
        diag::info!(
            Audio,
            "audio: createfx emitters {started}/{} ({} unique, max {} active; {missed} miss) for {map_name}",
            loops.len(),
            handles.len(),
            MAX_ACTIVE_MAP_EMITTERS,
        );
    } else if !loops.is_empty() {
        diag::info!(
            Audio,
            "audio: createfx listed {} emitters but none resolved PCM",
            loops.len()
        );
    }

    let oneshots = bank.createfx_oneshots(map_ns, map_name);
    let oneshot_count = oneshots.len();
    commands.insert_resource(asset_audio::CreateFxOneshotEmitters(oneshots));
    if oneshot_count > 0 {
        diag::info!(
            Audio,
            "audio: createfx oneshots {oneshot_count} parsed for {map_name} (host markers)"
        );
    }
}

/// What an emitter contributes at the ear: its base gain scaled by the
/// falloff curve at that distance (nothing without a curve, or past it).
pub(crate) fn emitter_gain(emitter: &MapEmitter, ear_inches: [f32; 3]) -> f32 {
    if emitter.knots.is_empty() {
        return 0.0;
    }
    let distance = distance_inches(ear_inches, emitter.origin_inches);
    let falloff = attenuate(&emitter.knots, distance, emitter.dist_min, emitter.dist_max);
    if falloff < 0.0 {
        0.0
    } else {
        emitter.base_gain * falloff
    }
}

/// One emitter's standing in this frame's ranking.
pub struct Ranked {
    entity: Entity,
    gain: f32,
    pcm: Handle<PcmAudio>,
    origin: [f32; 3],
    has_voice: bool,
}

pub fn update_map_emitter_gain(
    listeners: Query<&Transform, With<AmbientListener>>,
    mut emitters: Query<(Entity, &mut MapEmitter, Has<Voice>)>,
    mut sinks: Query<&mut AudioSink, With<MapEmitter>>,
    pcm_assets: Res<Assets<PcmAudio>>,
    mut looping_assets: ResMut<Assets<LoopingPcmAudio>>,
    mut commands: Commands,
    settings: Res<frame::GameSettings>,
    epoch: Res<MatchEpoch>,
    mut ranked: Local<Vec<Ranked>>,
    ducks: Res<crate::DuckMixer>,
    groups: Query<&crate::DuckGroup>,
) {
    let mut ears = listeners.iter();
    let Some(listener) = ears.next() else {
        return;
    };
    assert!(ears.next().is_none(), "more than one ambient listener");

    let ear = listener.translation;
    let right = listener.rotation * Vec3::X;
    let ear_inches = transform_inches(ear);

    // Loudest first; only the top few get voices.
    ranked.clear();
    ranked.extend(emitters.iter().map(|(entity, emitter, has_voice)| Ranked {
        entity,
        gain: emitter_gain(&emitter, ear_inches),
        pcm: emitter.pcm.clone(),
        origin: emitter.origin_inches,
        has_voice,
    }));
    ranked.sort_by(|a, b| b.gain.partial_cmp(&a.gain).unwrap_or(std::cmp::Ordering::Equal));

    for (rank, slot) in ranked.iter().enumerate() {
        let Ranked {
            entity,
            gain,
            ref pcm,
            origin,
            has_voice,
        } = *slot;
        let audible = rank < MAX_ACTIVE_MAP_EMITTERS && gain >= MIN_AUDIBLE_EMITTER_GAIN;

        if !audible {
            if has_voice {
                if let Ok((_, mut emitter, _)) = emitters.get_mut(entity) {
                    emitter.live_pan = None;
                }
                crate::backend::detach_loop(&mut commands, entity);
            }
            continue;
        }

        let (pan_left, pan_right) =
            world_oneshot_channel_gains(ear, right, Vec3::from_array(origin), 1.0);
        let (fixed_pan, menu) = match emitters.get(entity) {
            Ok((_, emitter, _)) => {
                let duck = ducks.linear(groups.get(entity).ok().map(|group| group.0));
                (emitter.fixed_pan, settings.menu_volume(emitter.category) * duck)
            }
            Err(_) => (None, 1.0),
        };

        if !has_voice {
            // Give it a voice: a copy of its PCM that can be panned live.
            let Some(dry) = pcm_assets.get(pcm).cloned() else {
                continue;
            };
            let (pan_left, pan_right) =
                fixed_pan.map_or((pan_left, pan_right), |pan| pan.gains(dry.channel_count()));
            let panned = dry.with_live_pan();
            let live_pan = panned
                .live_pan()
                .expect("with_live_pan always attaches a pan")
                .clone();
            live_pan.set(pan_left, pan_right);
            let handle = looping_assets.add(panned.into_looping());
            if let Ok((_, mut emitter, _)) = emitters.get_mut(entity) {
                emitter.live_pan = Some(live_pan);
            }
            crate::backend::attach_loop(
                &mut commands,
                entity,
                handle,
                Volume::Linear(gain * menu),
                epoch.0,
            );
            continue;
        }

        // Already voiced: retune its pan, level and speed.
        let mut speed = 1.0;
        if let Ok((_, emitter, _)) = emitters.get(entity) {
            speed = emitter.speed;
            if fixed_pan.is_none()
                && let Some(pan) = &emitter.live_pan
            {
                pan.set(pan_left, pan_right);
            }
        }
        if let Ok(mut sink) = sinks.get_mut(entity) {
            sink.set_volume(Volume::Linear(gain * settings.master_gain() * menu));
            if (sink.speed() - speed).abs() > 0.001 {
                sink.set_speed(speed);
            }
            if sink.is_paused() {
                sink.play();
            }
        }
    }
}

/// The first decoded clip any of the alias's keys has, if one is ready.
fn pcm_for_map_alias(
    clips: Option<&crate::ClipStore>,
    bank: &SoundCatalog,
    namespace: AssetNamespace,
    alias: &str,
) -> Option<PcmAudio> {
    let clips = clips?;
    crate::clip_store::clip_keys_for_alias(bank, namespace, alias)
        .into_iter()
        .find_map(|key| clips.ready(&key)?.ok())
}
