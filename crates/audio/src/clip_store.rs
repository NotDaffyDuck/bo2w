//! Turning the sound bank's clips into playable PCM on background threads.
//!
//! A [`ClipStore`] belongs to one installed bank. Callers ask for clips by
//! [`ClipKey`] and poll for the outcome; a resident cache lets the clips that
//! belong to the common zones survive from one match to the next.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::thread::JoinHandle;
use std::time::Instant;

use asset_audio::{LoadedSoundPcm, SoundCatalog};
use asset_core::AssetNamespace;
use assets::NamespaceSoundIwd;
use bevy::prelude::*;

use crate::pcm::{PcmAudio, decode_audio_bytes};
use crate::start::SoundClass;

/// Which clip: a PCM entry held by the bank, or a file streamed from an archive.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ClipKey {
    Loaded(usize),
    Streamed {
        ns: AssetNamespace,
        dir: String,
        name: String,
    },
}

/// A request and when it was made: the gap before a worker picks it up is the
/// queue's cost, not the clip's.
struct ClipJob {
    key: ClipKey,
    queued_at: Instant,
}

/// The decoder a clip went through. Each clip takes exactly one, and the cost
/// report keeps them apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipPath {
    /// Linear PCM from the zone, only converted to float.
    Pcm,
    /// T5 ADPCM, decoded here.
    Adpcm,
    /// Read from an archive instead of the zone.
    Streamed,
    /// No decoder takes the key (nothing at that index, or a format none of
    /// the others read); counted on its own so it hides in no other path.
    Unresolved,
}

impl ClipPath {
    pub const COUNT: usize = Self::Unresolved as usize + 1;

    pub const ALL: [Self; Self::COUNT] = [Self::Pcm, Self::Adpcm, Self::Streamed, Self::Unresolved];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Pcm => "pcm",
            Self::Adpcm => "t5 adpcm",
            Self::Streamed => "streamed",
            Self::Unresolved => "unresolved",
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum ClipError {
    Decode,
    Read,
    QueueClosed,
}

/// A decoded clip as the workers hand it over, before it becomes an asset.
#[derive(Clone, Debug)]
pub(crate) struct PreparedPcm {
    samples: Arc<[f32]>,
    channels: u16,
    sample_rate: u32,
}

impl PreparedPcm {
    pub(crate) fn into_audio(self) -> Option<PcmAudio> {
        PcmAudio::from_prepared(self.samples, self.channels, self.sample_rate)
    }

    fn from_audio(pcm: &PcmAudio) -> Self {
        Self {
            samples: Arc::clone(pcm.samples()),
            channels: pcm.channel_count(),
            sample_rate: pcm.rate(),
        }
    }

    fn byte_len(&self) -> u64 {
        (self.samples.len() * size_of::<f32>()) as u64
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// An encoded byte buffer compared and hashed by where it lives.
#[derive(Clone)]
struct SameBuffer(Arc<[u8]>);

impl PartialEq for SameBuffer {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for SameBuffer {}

impl Hash for SameBuffer {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.0).cast::<u8>().hash(state);
    }
}

/// What identifies a clip held by the bank, for the cross-match cache.
#[derive(Clone, PartialEq, Eq, Hash)]
struct LoadedClipKey {
    encoded: SameBuffer,
    format: i32,
    rate: u32,
    bits: i32,
    channels: i32,
    samples: u32,
    block_size: u32,
    seek_table: Vec<u32>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
enum ResidentClipKey {
    Common(LoadedClipKey),
    Map(LoadedClipKey),
    Streamed(ClipKey),
}

impl ResidentClipKey {
    fn is_common(&self) -> bool {
        matches!(self, Self::Common(_))
    }
}

/// Zones whose clips are the same in every match.
const COMMON_ZONES: [&str; 5] = [
    "code_post_gfx_mp",
    "localized_code_post_gfx_mp",
    "patch_mp",
    "common_mp",
    "localized_common_mp",
];

fn resident_clip_key(bank: &SoundCatalog, key: &ClipKey) -> Option<ResidentClipKey> {
    let index = match key {
        ClipKey::Loaded(index) => *index,
        streamed => return Some(ResidentClipKey::Streamed(streamed.clone())),
    };
    let sound = bank.pcm_at(index)?;
    let loaded = LoadedClipKey {
        encoded: SameBuffer(sound.encoded_arc()),
        format: sound.format(),
        rate: sound.rate,
        bits: sound.bits(),
        channels: sound.channels(),
        samples: sound.samples,
        block_size: sound.block_size,
        seek_table: sound.seek_table.clone(),
    };
    Some(if COMMON_ZONES.contains(&sound.zone.as_str()) {
        ResidentClipKey::Common(loaded)
    } else {
        ResidentClipKey::Map(loaded)
    })
}

#[derive(Default)]
struct ResidentClipCacheInner {
    profile_id: u64,
    bank: Weak<SoundCatalog>,
    prepared: HashMap<ResidentClipKey, PreparedPcm>,
    dry_handles: HashMap<ResidentClipKey, Handle<PcmAudio>>,
}

/// Decoded clips (and their asset handles) kept between matches. Only entries
/// from the common zones outlive their bank; the whole cache is dropped when
/// the common profile changes.
#[derive(Resource, Clone, Default)]
pub(crate) struct ResidentClipCache(Arc<Mutex<ResidentClipCacheInner>>);

impl ResidentClipCache {
    fn use_profile(&self, profile_id: u64) {
        let mut inner = lock(&self.0);
        if inner.profile_id == profile_id {
            return;
        }
        inner.prepared.clear();
        inner.dry_handles.clear();
        inner.profile_id = profile_id;
    }

    fn use_bank(&self, bank: &Arc<SoundCatalog>) {
        let mut inner = lock(&self.0);
        if Weak::ptr_eq(&inner.bank, &Arc::downgrade(bank)) {
            return;
        }
        // A different bank: only the common clips stay valid.
        inner.prepared.retain(|key, _| key.is_common());
        inner.dry_handles.retain(|key, _| key.is_common());
        inner.bank = Arc::downgrade(bank);
    }

    /// Profile 0 means "no profile", which never matches anything.
    fn matches(inner: &ResidentClipCacheInner, profile_id: u64) -> bool {
        profile_id != 0 && inner.profile_id == profile_id
    }

    fn ready(&self, profile_id: u64, key: &ResidentClipKey) -> Option<PreparedPcm> {
        let inner = lock(&self.0);
        if !Self::matches(&inner, profile_id) {
            return None;
        }
        inner.prepared.get(key).cloned()
    }

    fn remember(&self, profile_id: u64, key: ResidentClipKey, pcm: PreparedPcm) {
        let mut inner = lock(&self.0);
        if Self::matches(&inner, profile_id) {
            inner.prepared.entry(key).or_insert(pcm);
        }
    }

    /// The asset for a clip's dry sound, made once and then shared. The flag
    /// is true when the handle was already there.
    fn dry_handle(
        &self,
        profile_id: u64,
        key: ResidentClipKey,
        assets: &mut Assets<PcmAudio>,
        pcm: &PcmAudio,
    ) -> Option<(Handle<PcmAudio>, bool)> {
        let mut inner = lock(&self.0);
        if !Self::matches(&inner, profile_id) {
            return None;
        }
        let existing = inner
            .dry_handles
            .get(&key)
            .filter(|handle| assets.get(handle.id()).is_some())
            .cloned();
        if let Some(handle) = existing {
            return Some((handle, true));
        }
        let handle = assets.add(pcm.clone());
        inner.dry_handles.insert(key, handle.clone());
        Some((handle, false))
    }
}

/// The most jobs a worker takes in one go. Everything in a batch is published
/// once its last clip is done, so this also caps how long an outcome waits.
pub const PREP_BATCH: usize = 64;

/// Declared after the sender in the store so the channel closes first and the
/// workers can be joined.
struct ClipWorkers {
    handles: Vec<JoinHandle<()>>,
    stop: Arc<AtomicBool>,
}

impl Drop for ClipWorkers {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let count = self.handles.len();
        for handle in self.handles.drain(..) {
            if handle.join().is_err() {
                diag::warn!(Audio, "audio: clip prep worker panicked during retirement");
            }
        }
        if count != 0 {
            diag::info!(Audio, "audio: clip prep workers joined={count}");
        }
    }
}

type Outcomes = Arc<Mutex<HashMap<ClipKey, Result<PreparedPcm, ClipError>>>>;

/// Everything one prep thread owns.
struct WorkerEnv {
    jobs: Arc<Mutex<Receiver<ClipJob>>>,
    bank: Arc<SoundCatalog>,
    iwd: Option<Arc<NamespaceSoundIwd>>,
    outcomes: Outcomes,
    cache: Option<ResidentClipCache>,
    profile_id: u64,
    stop: Arc<AtomicBool>,
}

impl WorkerEnv {
    /// Waits for a job, then takes whatever else is already queued, up to a
    /// batch. A match asks for all its clips at once and one of the decoders is
    /// an external process that costs more to start than to run, so batching
    /// pays; a lone late request still goes through alone.
    fn next_batch(&self) -> Option<Vec<ClipJob>> {
        let receiver = lock(&self.jobs);
        let mut batch = vec![receiver.recv().ok()?];
        while batch.len() < PREP_BATCH {
            match receiver.try_recv() {
                Ok(job) => batch.push(job),
                Err(_) => break,
            }
        }
        Some(batch)
    }

    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }

    fn run(self) {
        while !self.stopped() {
            let Some(batch) = self.next_batch() else {
                return;
            };
            if self.stopped() {
                return;
            }
            let waited: u64 = batch
                .iter()
                .map(|job| job.queued_at.elapsed().as_nanos() as u64)
                .sum();
            TALLY.queue_wait_ns.fetch_add(waited, Ordering::Relaxed);

            let results = prepare_batch(&self.bank, self.iwd.as_deref(), &batch);
            if self.stopped() {
                return;
            }
            if let Some(cache) = &self.cache {
                for (job, result) in batch.iter().zip(&results) {
                    if let (Ok(pcm), Some(key)) = (result, resident_clip_key(&self.bank, &job.key))
                    {
                        cache.remember(self.profile_id, key, pcm.clone());
                    }
                }
            }
            let mut outcomes = lock(&self.outcomes);
            outcomes.extend(batch.into_iter().map(|job| job.key).zip(results));
        }
    }
}

/// Decodes the bank's clips on a few worker threads and holds the results until
/// they are collected.
#[derive(Resource)]
pub struct ClipStore {
    bank: Arc<SoundCatalog>,
    tx: Sender<ClipJob>,
    queued: HashSet<ClipKey>,
    outcomes: Outcomes,
    workers: ClipWorkers,
    clip_cache: Option<ResidentClipCache>,
    common_profile_id: u64,
    reused_clips: usize,
    reused_bytes: u64,

    match_live: bool,
    late_prepares: u32,
}

impl ClipStore {
    pub fn start(bank: Arc<SoundCatalog>, iwd: Option<Arc<NamespaceSoundIwd>>) -> Self {
        Self::start_with_common(bank, iwd, 0, None)
    }

    pub(crate) fn start_with_common(
        bank: Arc<SoundCatalog>,
        iwd: Option<Arc<NamespaceSoundIwd>>,
        common_profile_id: u64,
        clip_cache: Option<ResidentClipCache>,
    ) -> Self {
        if let Some(cache) = &clip_cache {
            cache.use_profile(common_profile_id);
            cache.use_bank(&bank);
        }
        // Leave two cores to the game; one to four decode.
        let thread_count = std::thread::available_parallelism()
            .map_or(1, |cores| cores.get().saturating_sub(2).clamp(1, 4));
        let (tx, rx) = channel::<ClipJob>();
        let jobs = Arc::new(Mutex::new(rx));
        let outcomes: Outcomes = Arc::default();
        let stop = Arc::new(AtomicBool::new(false));

        let mut handles = Vec::with_capacity(thread_count);
        for slot in 0..thread_count {
            let env = WorkerEnv {
                jobs: Arc::clone(&jobs),
                bank: Arc::clone(&bank),
                iwd: iwd.clone(),
                outcomes: Arc::clone(&outcomes),
                cache: clip_cache.clone(),
                profile_id: common_profile_id,
                stop: Arc::clone(&stop),
            };
            let spawned = std::thread::Builder::new()
                .name(format!("clip-prep-{slot}"))
                .spawn(move || env.run());
            match spawned {
                Ok(handle) => handles.push(handle),
                Err(error) => {
                    diag::warn!(Audio, "audio: clip prep worker {slot} not started ({error})");
                }
            }
        }
        TALLY
            .workers
            .fetch_add(handles.len() as u64, Ordering::Relaxed);

        Self {
            bank,
            tx,
            queued: HashSet::new(),
            outcomes,
            workers: ClipWorkers { handles, stop },
            clip_cache,
            common_profile_id,
            reused_clips: 0,
            reused_bytes: 0,
            match_live: false,
            late_prepares: 0,
        }
    }

    pub fn workers(&self) -> usize {
        self.workers.handles.len()
    }

    /// (clips, bytes) served from the resident cache instead of being decoded.
    pub fn reused_resident(&self) -> (usize, u64) {
        (self.reused_clips, self.reused_bytes)
    }

    pub(crate) fn resident_dry_handle(
        &self,
        clip: &ClipKey,
        assets: &mut Assets<PcmAudio>,
        pcm: &PcmAudio,
    ) -> Option<(Handle<PcmAudio>, bool)> {
        let cache = self.clip_cache.as_ref()?;
        let key = resident_clip_key(&self.bank, clip)?;
        cache.dry_handle(self.common_profile_id, key, assets, pcm)
    }

    /// From now on a clip first prepared here is "late": the match is running.
    pub fn arm_match_live(&mut self) {
        self.match_live = true;
    }

    /// Late prepares of this store. A store lasts as long as its installed
    /// bank, so a map change starts the count over; [`clip_prep_cost`] has the
    /// process total for the exit hook.
    pub fn late_prepares(&self) -> u32 {
        self.late_prepares
    }

    fn note_late(&mut self, key: &ClipKey) {
        if !self.match_live {
            return;
        }
        self.late_prepares = self.late_prepares.saturating_add(1);
        TALLY.late.fetch_add(1, Ordering::Relaxed);
        diag::warn!(Audio, "audio: clip prepare after AudioReady ({key:?})");
    }

    /// Asks for every clip an alias can play; returns how many were newly queued.
    pub fn request_alias(&mut self, ns: AssetNamespace, alias: &str) -> usize {
        clip_keys_for_alias(&self.bank, ns, alias)
            .into_iter()
            .filter(|key| self.request(key.clone()))
            .count()
    }

    fn publish(&self, key: ClipKey, outcome: Result<PreparedPcm, ClipError>) {
        lock(&self.outcomes).insert(key, outcome);
    }

    /// Queues a clip unless it is already done, cached or queued. True only
    /// when a job was actually sent.
    pub(crate) fn request(&mut self, key: ClipKey) -> bool {
        TALLY.requests.fetch_add(1, Ordering::Relaxed);
        if self.ready(&key).is_some() {
            return false;
        }
        let cached = self.clip_cache.as_ref().and_then(|cache| {
            let resident = resident_clip_key(&self.bank, &key)?;
            cache.ready(self.common_profile_id, &resident)
        });
        if let Some(pcm) = cached {
            self.reused_clips += 1;
            self.reused_bytes += pcm.byte_len();
            self.publish(key, Ok(pcm));
            return false;
        }
        if !self.queued.insert(key.clone()) {
            return false;
        }
        self.note_late(&key);
        TALLY.queued.fetch_add(1, Ordering::Relaxed);
        let job = ClipJob {
            key: key.clone(),
            queued_at: Instant::now(),
        };
        if self.tx.send(job).is_err() {
            self.publish(key, Err(ClipError::QueueClosed));
            return false;
        }
        true
    }

    /// The outcome for a clip, if there is one yet.
    pub(crate) fn ready(&self, key: &ClipKey) -> Option<Result<PcmAudio, ClipError>> {
        let outcomes = lock(&self.outcomes);
        let outcome = outcomes.get(key)?;
        Some(match outcome {
            Ok(prepared) => prepared.clone().into_audio().ok_or(ClipError::Decode),
            Err(error) => Err(error.clone()),
        })
    }
}

/// A one-shot waiting for its clip to finish preparing.
pub(crate) struct PendingOneshot {
    pub namespace: AssetNamespace,
    pub alias: String,
    pub bound: Option<usize>,
    pub variant: usize,
    pub volume: f32,
    pub pitch: f32,
    pub origin_inches: Option<[f32; 3]>,
    pub snd_ent: Option<u32>,
    pub clip: ClipKey,
    pub layer: Option<String>,
    pub class: SoundClass,
    pub epoch: u64,
    pub deadline: Instant,
}

#[derive(Resource, Default)]
pub(crate) struct PendingStarts {
    pub entries: Vec<PendingOneshot>,
}

impl PendingStarts {
    pub fn push_oneshot(&mut self, pending: PendingOneshot) {
        self.entries.push(pending);
    }

    /// Drops the waiting match sounds of one alias on one entity in one epoch.
    pub fn cancel_alias(
        &mut self,
        namespace: AssetNamespace,
        alias: &str,
        snd_ent: Option<u32>,
        epoch: u64,
    ) {
        self.entries.retain(|entry| {
            let same = entry.namespace == namespace
                && entry.alias == alias
                && entry.snd_ent == snd_ent
                && entry.epoch == epoch;
            !(same && entry.class.scope() == crate::backend::AudioScope::Match)
        });
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

pub(crate) fn deadline_for(class: SoundClass) -> Instant {
    Instant::now() + class.oneshot_wait()
}

/// Secondary-alias chains are followed this deep and no further.
const MAX_SECONDARY_DEPTH: u8 = 10;

/// Every clip an alias can play, in order, including those reached through
/// secondary aliases. Each clip appears once.
pub(crate) fn clip_keys_for_alias(
    bank: &SoundCatalog,
    ns: AssetNamespace,
    alias: &str,
) -> Vec<ClipKey> {
    let mut walk = AliasWalk::default();
    walk.visit(bank, ns, alias, 0);
    walk.found
}

#[derive(Default)]
struct AliasWalk {
    aliases: HashSet<String>,
    keys: HashSet<ClipKey>,
    found: Vec<ClipKey>,
}

impl AliasWalk {
    fn add(&mut self, key: ClipKey) {
        if self.keys.insert(key.clone()) {
            self.found.push(key);
        }
    }

    fn visit(&mut self, bank: &SoundCatalog, ns: AssetNamespace, alias: &str, depth: u8) {
        if depth > MAX_SECONDARY_DEPTH || !self.aliases.insert(alias.to_owned()) {
            return;
        }
        let Some(sound) = bank.sound_in(ns, alias) else {
            return;
        };
        for (variant, row) in sound.aliases.iter().enumerate() {
            match row.loaded.bound_index() {
                Some(index) => self.add(ClipKey::Loaded(index)),
                None => {
                    if let Some((sns, dir, name)) = bank.streamed_for_variant(ns, alias, variant) {
                        self.add(ClipKey::Streamed { ns: sns, dir, name });
                    }
                }
            }
            if let Some(next) = row.secondary.as_deref().filter(|next| !next.is_empty()) {
                self.visit(bank, ns, next, depth + 1);
            }
        }
    }
}

/// The clip one variant of an alias plays: a named loaded clip if the caller
/// has one, else the variant's loaded row, else its streamed file.
pub(crate) fn clip_key_for_variant(
    bank: &SoundCatalog,
    ns: AssetNamespace,
    alias: &str,
    bound: Option<usize>,
    variant: usize,
    loaded_name: Option<&str>,
    loaded_ns: Option<AssetNamespace>,
) -> Option<ClipKey> {
    let named = loaded_name
        .zip(loaded_ns)
        .and_then(|(name, loaded_ns)| bank.loaded_index_in(loaded_ns, name));
    if let Some(index) = named {
        return Some(ClipKey::Loaded(index));
    }

    let sound = match bound {
        Some(index) => bank.sound_at(index),
        None => bank.sound_in(ns, alias),
    };
    let row_index = sound
        .and_then(|sound| sound.aliases.get(variant))
        .and_then(|row| row.loaded.bound_index());
    if let Some(index) = row_index {
        return Some(ClipKey::Loaded(index));
    }

    let streamed = match bound {
        Some(index) => bank.streamed_for_variant_at(index, variant),
        None => bank.streamed_for_variant(ns, alias, variant),
    };
    streamed.map(|(sns, dir, name)| ClipKey::Streamed { ns: sns, dir, name })
}

/// Process-wide counters for the cost report; kept for the exit hook, which
/// has no store to ask.
struct Tally {
    workers: AtomicU64,
    requests: AtomicU64,
    queued: AtomicU64,
    queue_wait_ns: AtomicU64,
    late: AtomicU64,
    paths: [PathTally; ClipPath::COUNT],
}

struct PathTally {
    prepared: AtomicU64,
    failed: AtomicU64,
    wall_ns: AtomicU64,
    sample_bytes: AtomicU64,
}

impl PathTally {
    const fn new() -> Self {
        Self {
            prepared: AtomicU64::new(0),
            failed: AtomicU64::new(0),
            wall_ns: AtomicU64::new(0),
            sample_bytes: AtomicU64::new(0),
        }
    }
}

static TALLY: Tally = Tally {
    workers: AtomicU64::new(0),
    requests: AtomicU64::new(0),
    queued: AtomicU64::new(0),
    queue_wait_ns: AtomicU64::new(0),
    late: AtomicU64::new(0),
    paths: [const { PathTally::new() }; ClipPath::COUNT],
};

/// One decoder's share of the clip preparation.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ClipPathCost {
    pub prepared: u64,
    pub failed: u64,
    /// Worker time inside this decoder, summed over the prep threads.
    pub wall_ms: f64,
    /// Resident `f32` sample bytes this decoder produced. The store holds them
    /// for the whole match, so this measures memory, not speed.
    pub sample_bytes: u64,
}

/// What preparing clips has cost this process.
///
/// `requests` counts asks and `queued` counts jobs: the same clip is reached
/// from several aliases and weapons, and the difference is what the store's
/// dedupe saves. The durations add up over workers that run side by side, so
/// none of them is a stretch of wall-clock time.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClipPrepCost {
    /// Prep threads that actually started, over every store this process has
    /// opened (a teardown closes one, a reload opens the next).
    pub workers: u64,
    pub requests: u64,
    pub queued: u64,
    pub queue_wait_ms: f64,
    pub late: u64,
    pub paths: Vec<(ClipPath, ClipPathCost)>,
}

fn ns_to_ms(ns: &AtomicU64) -> f64 {
    ns.load(Ordering::Relaxed) as f64 / 1.0e6
}

pub fn clip_prep_cost() -> ClipPrepCost {
    let paths = ClipPath::ALL
        .into_iter()
        .map(|path| {
            let tally = &TALLY.paths[path as usize];
            let cost = ClipPathCost {
                prepared: tally.prepared.load(Ordering::Relaxed),
                failed: tally.failed.load(Ordering::Relaxed),
                wall_ms: ns_to_ms(&tally.wall_ns),
                sample_bytes: tally.sample_bytes.load(Ordering::Relaxed),
            };
            (path, cost)
        })
        .collect();
    ClipPrepCost {
        workers: TALLY.workers.load(Ordering::Relaxed),
        requests: TALLY.requests.load(Ordering::Relaxed),
        queued: TALLY.queued.load(Ordering::Relaxed),
        queue_wait_ms: ns_to_ms(&TALLY.queue_wait_ns),
        late: TALLY.late.load(Ordering::Relaxed),
        paths,
    }
}

fn record_prepare(
    path: ClipPath,
    started: Instant,
    outcome: Result<&PreparedPcm, &ClipError>,
) {
    let tally = &TALLY.paths[path as usize];
    tally
        .wall_ns
        .fetch_add(started.elapsed().as_nanos() as u64, Ordering::Relaxed);
    match outcome {
        Ok(prepared) => {
            tally.prepared.fetch_add(1, Ordering::Relaxed);
            tally
                .sample_bytes
                .fetch_add(prepared.byte_len(), Ordering::Relaxed);
        }
        Err(_) => {
            tally.failed.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Prepares a worker's batch in order.
fn prepare_batch(
    bank: &SoundCatalog,
    iwd: Option<&NamespaceSoundIwd>,
    jobs: &[ClipJob],
) -> Vec<Result<PreparedPcm, ClipError>> {
    let mut results = Vec::with_capacity(jobs.len());
    for job in jobs {
        let started = Instant::now();
        let (path, result) = prepare_clip(bank, iwd, &job.key);
        record_prepare(path, started, result.as_ref());
        results.push(result);
    }
    results
}

fn prepare_clip(
    bank: &SoundCatalog,
    iwd: Option<&NamespaceSoundIwd>,
    key: &ClipKey,
) -> (ClipPath, Result<PreparedPcm, ClipError>) {
    match key {
        ClipKey::Loaded(index) => {
            let sound = bank.pcm_at(*index);
            match sound.and_then(|sound| Some((sound, loaded_decoder(sound)?))) {
                Some((sound, path)) => (path, prepare_loaded(sound, path)),
                None => (ClipPath::Unresolved, Err(ClipError::Decode)),
            }
        }
        ClipKey::Streamed { ns, dir, name } => (
            ClipPath::Streamed,
            prepare_streamed(iwd, *ns, dir, name),
        ),
    }
}

/// Which decoder reads a loaded clip: ADPCM bytes first, else plain PCM, else
/// none. Preparing a clip uses this same answer, so it is never counted under
/// one path and decoded by another.
fn loaded_decoder(sound: &LoadedSoundPcm) -> Option<ClipPath> {
    if sound.t5_adpcm_bytes().is_some() {
        Some(ClipPath::Adpcm)
    } else if sound.format() == asset_audio::MSS_PCM {
        Some(ClipPath::Pcm)
    } else {
        None
    }
}

fn prepare_loaded(sound: &LoadedSoundPcm, path: ClipPath) -> Result<PreparedPcm, ClipError> {
    match (path, sound.t5_adpcm_bytes()) {
        (ClipPath::Adpcm, Some(bytes)) => {
            let channels =
                u16::try_from(sound.channels().max(1)).map_err(|_| ClipError::Decode)?;
            crate::pcm::t5_stream::decode_adpcm(
                bytes,
                sound.samples,
                sound.rate,
                u32::from(channels),
            )
            .map(|pcm| PreparedPcm::from_audio(&pcm))
            .ok_or(ClipError::Decode)
        }
        (ClipPath::Pcm, _) => interleave_pcm(
            sound.bits(),
            sound.encoded_bytes(),
            sound.channels(),
            sound.rate,
        ),
        _ => Err(ClipError::Decode),
    }
}

/// Raw 8- or 16-bit samples as interleaved `f32`: a whole number of frames,
/// and never empty (no samples is a failed decode, not a silent clip).
fn interleave_pcm(
    bits: i32,
    bytes: &[u8],
    channels: i32,
    rate: u32,
) -> Result<PreparedPcm, ClipError> {
    let mut samples: Vec<f32> = match bits {
        8 => bytes
            .iter()
            .map(|&byte| (f32::from(byte) - 128.0) / 128.0)
            .collect(),
        16 => bytes
            .chunks_exact(2)
            .map(|pair| f32::from(i16::from_le_bytes([pair[0], pair[1]])) / 32768.0)
            .collect(),
        _ => return Err(ClipError::Decode),
    };
    let lanes = channels.max(1) as usize;
    samples.truncate(samples.len() - samples.len() % lanes);
    if samples.is_empty() {
        return Err(ClipError::Decode);
    }
    Ok(PreparedPcm {
        samples: samples.into(),
        channels: u16::try_from(lanes).map_err(|_| ClipError::Decode)?,
        sample_rate: rate.max(1),
    })
}

fn prepare_streamed(
    iwd: Option<&NamespaceSoundIwd>,
    ns: AssetNamespace,
    dir: &str,
    name: &str,
) -> Result<PreparedPcm, ClipError> {
    // A Black Ops II clip names its sound bank file and an entry in it.
    if ns == AssetNamespace::T6 {
        let clip = asset_audio::decode_t6_clip(dir, name).map_err(|error| {
            diag::warn!(Audio, "audio: T6 clip `{name}` in `{dir}`: {error}");
            ClipError::Decode
        })?;
        return Ok(PreparedPcm {
            samples: clip.samples.iter().map(|&s| f32::from(s) / 32768.0).collect(),
            channels: clip.channels.max(1),
            sample_rate: clip.rate,
        });
    }

    let iwd = iwd.ok_or(ClipError::Read)?;
    let path = format!("{dir}/{name}");
    let bytes = match iwd.read_sound(ns, &path) {
        Some(Ok(bytes)) => bytes,
        Some(Err(error)) => {
            diag::warn!(Audio, "audio: IWD read `{path}` failed: {error}");
            return Err(ClipError::Read);
        }
        None => {
            diag::warn!(
                Audio,
                "audio: streamed IWD miss `{}:{path}` (typed gap)",
                ns.as_str()
            );
            return Err(ClipError::Read);
        }
    };
    // T5 keeps its own container unless the file is a plain RIFF.
    let decoded = if ns == AssetNamespace::T5 && !bytes.starts_with(b"RIFF") {
        crate::pcm::t5_stream::decode(&bytes)
    } else {
        decode_audio_bytes(&bytes)
    };
    match decoded {
        Some(pcm) => Ok(PreparedPcm::from_audio(&pcm)),
        None => {
            diag::warn!(
                Audio,
                "audio: failed to decode streamed `{}:{path}`",
                ns.as_str()
            );
            Err(ClipError::Decode)
        }
    }
}
