//! Turning the sound bank's clips into playable PCM on background threads.
//!
//! A [`ClipRepository`] belongs to one installed bank. Callers ask for clips by
//! [`WaveformHandle`] and poll for the outcome; a resident cache lets the clips that
//! belong to the common zones survive from one match to the next.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::iter;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use asset_audio::{AudioRoster, ResidentRawAudio, SnapshotSoundAlias};
use asset_core::AssetCategoryScope;
use assets::CategorySoundArchive;
use bevy::prelude::*;

use crate::backend::SoundAudience;
use crate::sample_feed::{ClipPcm, decode_clip_bytes, adpcm_t5};
use crate::playback_gate::NoiseCategory;

/// Which clip: a PCM entry held by the bank, or a file streamed from an archive.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum WaveformHandle {
    Ready(usize),
    Streamed {
        ns: AssetCategoryScope,
        folder: String,
        name: String,
    },
}

impl WaveformHandle {
    /// The key for what the bank reports as a variant's streamed file.
    fn streamed((ns, dir, name): (AssetCategoryScope, String, String)) -> Self {
        Self::Streamed {
            ns,
            folder: dir,
            name,
        }
    }
}

/// A request and when it was made: the gap before a worker picks it up is the
/// queue's cost, not the clip's.
struct ClipJob {
    queued_t: Instant,
    key: WaveformHandle,
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
    NotFound,
}

impl ClipPath {
    pub const ALL_KINDS: [Self; 4] = [Self::Pcm, Self::Adpcm, Self::Streamed, Self::NotFound];

    pub const COUNT: usize = Self::ALL_KINDS.len();

    pub const fn name(self) -> &'static str {
        const NAMES: [&str; ClipPath::COUNT] = ["pcm", "t5 adpcm", "streamed", "unresolved"];
        NAMES[self as usize]
    }
}

#[derive(Clone, Debug)]
pub(crate) enum ClipLoadFault {
    Decode,
    Read,
    QueueClosed,
}

/// The bytes a finished clip keeps in memory.
fn sample_bytes(pcm: &ClipPcm) -> u64 {
    (pcm.data().len() * size_of::<f32>()) as u64
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// An encoded byte buffer compared and hashed by where it lives.
#[derive(Clone)]
struct SameBuffer(Arc<[u8]>);

impl SameBuffer {
    fn address(&self) -> usize {
        Arc::as_ptr(&self.0).cast::<u8>() as usize
    }
}

impl PartialEq for SameBuffer {
    fn eq(&self, rhs: &Self) -> bool {
        self.address() == rhs.address()
    }
}

impl Eq for SameBuffer {}

impl Hash for SameBuffer {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.address().hash(state);
    }
}

/// How a stored clip is laid out; two clips are the same only if these agree.
#[derive(Clone, PartialEq, Eq, Hash)]
struct StoredLayout {
    codec: i32,
    lanes: i32,
    depth: i32,
    hz: u32,
    frame_ct: u32,
    block: u32,
    seeks: Vec<u32>,
}

/// What identifies a clip held by the bank, for the cross-match cache.
#[derive(Clone, PartialEq, Eq, Hash)]
struct LoadedClipKey {
    layout: StoredLayout,
    encoded: SameBuffer,
}

impl LoadedClipKey {
    fn of(sound: &ResidentRawAudio) -> Self {
        Self {
            encoded: SameBuffer(sound.encoded_arc()),
            layout: StoredLayout {
                codec: sound.format(),
                lanes: sound.channel_total(),
                depth: sound.bits(),
                hz: sound.sample_rate,
                frame_ct: sound.frame_count,
                block: sound.block_bytes,
                seeks: sound.jump_index.clone(),
            },
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
enum ResidentClipKey {
    /// A bank clip, flagged when its zone is the same in every match.
    Loaded {
        common: bool,
        clip: LoadedClipKey,
    },
    Streamed(WaveformHandle),
}

impl ResidentClipKey {
    fn is_common(&self) -> bool {
        matches!(self, Self::Loaded { common: true, .. })
    }
}

/// Zones whose clips are the same in every match.
fn is_common_zone(zone: &str) -> bool {
    let bare = zone.strip_prefix("localized_").unwrap_or(zone);
    match bare {
        "common_mp" | "code_post_gfx_mp" => true,
        "patch_mp" => bare == zone,
        _ => false,
    }
}

fn resident_clip_key(bank: &AudioRoster, key: &WaveformHandle) -> Option<ResidentClipKey> {
    let WaveformHandle::Ready(index) = key else {
        return Some(ResidentClipKey::Streamed(key.clone()));
    };
    let sound = bank.resident_clip(*index)?;
    Some(ResidentClipKey::Loaded {
        common: is_common_zone(sound.zone.as_str()),
        clip: LoadedClipKey::of(sound),
    })
}

/// One cached clip: its decoded samples and the asset made from them.
#[derive(Default)]
struct ResidentSlot {
    pcm: Option<ClipPcm>,
    dry: Option<Handle<ClipPcm>>,
}

#[derive(Default)]
struct ResidentClipCacheInner {
    profile_id: u64,
    bank: Weak<AudioRoster>,
    slots: HashMap<ResidentClipKey, ResidentSlot>,
}

impl ResidentClipCacheInner {
    /// The slots, but only for the profile this cache holds. Profile 0 means
    /// "no profile", which never matches anything.
    fn slots_for(
        &mut self,
        profile_id: u64,
    ) -> Option<&mut HashMap<ResidentClipKey, ResidentSlot>> {
        let current = profile_id != 0 && profile_id == self.profile_id;
        current.then_some(&mut self.slots)
    }
}

/// Decoded clips (and their asset handles) kept between matches. Only entries
/// from the common zones outlive their bank; the whole cache is dropped when
/// the common profile changes.
#[derive(Resource, Clone, Default)]
pub(crate) struct ResidentClipCache(Arc<Mutex<ResidentClipCacheInner>>);

impl ResidentClipCache {
    fn use_profile(&self, profile_id: u64) {
        let mut inner = lock(&self.0);
        if inner.profile_id != profile_id {
            inner.profile_id = profile_id;
            inner.slots.clear();
        }
    }

    fn use_bank(&self, bank: &Arc<AudioRoster>) {
        let mut inner = lock(&self.0);
        let fresh = Arc::downgrade(bank);
        if !Weak::ptr_eq(&inner.bank, &fresh) {
            // A different bank: only the common clips stay valid.
            inner.slots.retain(|key, _| key.is_common());
            inner.bank = fresh;
        }
    }

    fn ready(&self, profile_id: u64, key: &ResidentClipKey) -> Option<ClipPcm> {
        let mut inner = lock(&self.0);
        inner.slots_for(profile_id)?.get(key)?.pcm.clone()
    }

    fn remember(&self, profile_id: u64, key: ResidentClipKey, pcm: ClipPcm) {
        let mut inner = lock(&self.0);
        if let Some(slots) = inner.slots_for(profile_id) {
            slots.entry(key).or_default().pcm.get_or_insert(pcm);
        }
    }

    /// The asset for a clip's dry sound, made once and then shared. The flag
    /// is true when the handle was already there.
    fn shared_dry(
        &self,
        profile_id: u64,
        key: ResidentClipKey,
        assets: &mut Assets<ClipPcm>,
        pcm: &ClipPcm,
    ) -> Option<(Handle<ClipPcm>, bool)> {
        let mut inner = lock(&self.0);
        let slot = inner.slots_for(profile_id)?.entry(key).or_default();
        let alive = slot
            .dry
            .iter()
            .find(|handle| assets.contains(handle.id()))
            .cloned();
        match alive {
            Some(handle) => Some((handle, true)),
            None => {
                let handle = assets.add(pcm.clone());
                slot.dry = Some(handle.clone());
                Some((handle, false))
            }
        }
    }
}

/// A cache together with the common profile a store was opened under.
#[derive(Clone)]
struct ResidentLink {
    cache: ResidentClipCache,
    profile_id: u64,
}

impl ResidentLink {
    fn fetch(&self, bank: &AudioRoster, key: &WaveformHandle) -> Option<ClipPcm> {
        let resident = resident_clip_key(bank, key)?;
        self.cache.ready(self.profile_id, &resident)
    }

    fn keep(&self, bank: &AudioRoster, key: &WaveformHandle, samples: &ClipPcm) {
        if let Some(resident) = resident_clip_key(bank, key) {
            self.cache.remember(self.profile_id, resident, samples.clone());
        }
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
        let total = self.handles.len();
        let panicked = self
            .handles
            .drain(..)
            .map(JoinHandle::join)
            .filter(Result::is_err)
            .count();
        if panicked > 0 {
            diag::warn!(
                Audio,
                "audio: {panicked} clip prep worker(s) panicked during retirement"
            );
        }
        if total > 0 {
            diag::info!(Audio, "sound: {total} clip prep threads finished");
        }
    }
}

/// Everything one prep thread owns.
#[derive(Clone)]
struct WorkerEnv {
    stop: Arc<AtomicBool>,
    catalog: Arc<AudioRoster>,
    archive: Option<Arc<CategorySoundArchive>>,
    jobs: Arc<Mutex<Receiver<ClipJob>>>,
    resident: Option<ResidentLink>,
    outcomes: Arc<Mutex<HashMap<WaveformHandle, Result<ClipPcm, ClipLoadFault>>>>,
}

impl WorkerEnv {
    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }

    /// Waits for a job, then takes whatever else is already queued, up to a
    /// batch. A match asks for all its clips at once and one of the decoders is
    /// an external process that costs more to start than to run, so batching
    /// pays; a lone late request still goes through alone.
    fn next_batch(&self) -> Option<Vec<ClipJob>> {
        let receiver = lock(&self.jobs);
        let first = receiver.recv().ok()?;
        let more = receiver.try_iter().take(PREP_BATCH - 1);
        Some(iter::once(first).chain(more).collect())
    }

    fn run(self) {
        while !self.stopped() {
            let Some(batch) = self.next_batch() else {
                break;
            };
            if self.stopped() || !self.process(batch) {
                break;
            }
        }
    }

    /// Prepares one batch and publishes it. False when the store was retired
    /// meanwhile and the results were dropped.
    fn process(&self, batch: Vec<ClipJob>) -> bool {
        let waited = batch
            .iter()
            .map(|job| job.queued_t.elapsed().as_nanos() as u64);
        tally::plus(Total::QueueWaitNs, waited.sum());

        let results = prepare_batch(&self.catalog, self.archive.as_deref(), &batch);
        if self.stopped() {
            return false;
        }
        if let Some(resident) = &self.resident {
            for (job, pcm) in batch.iter().zip(&results) {
                if let Ok(pcm) = pcm {
                    resident.keep(&self.catalog, &job.key, pcm);
                }
            }
        }
        let keys = batch.into_iter().map(|job| job.key);
        lock(&self.outcomes).extend(keys.zip(results));
        true
    }
}

/// How many clips (and bytes) were served from the resident cache.
#[derive(Clone, Copy, Default)]
struct Reuse {
    clips: usize,
    bytes: u64,
}

/// Where a requested clip stands before anything is queued for it.
enum Standing {
    /// An outcome is already published.
    Settled,
    /// The resident cache has the decoded clip.
    Resident(ClipPcm),
    /// Nothing yet.
    Fresh,
}

/// Decodes the bank's clips on a few worker threads and holds the results until
/// they are collected.
#[derive(Resource)]
pub struct ClipRepository {
    jobs_tx: Sender<ClipJob>,
    catalog: Arc<AudioRoster>,
    sent: HashSet<WaveformHandle>,
    outcomes: Arc<Mutex<HashMap<WaveformHandle, Result<ClipPcm, ClipLoadFault>>>>,
    resident: Option<ResidentLink>,
    reuse: Reuse,
    /// `None` until the match is live, then the count of clips prepared since.
    late: Option<u32>,
    /// Last, so the sender above is dropped first and closes the channel.
    workers: ClipWorkers,
}

/// Leave two cores to the game; one to four decode.
fn prep_thread_count() -> usize {
    let cores = thread::available_parallelism().map_or(3, |n| n.get());
    (cores.max(2) - 2).clamp(1, 4)
}

fn spawn_workers(env: &WorkerEnv, count: usize) -> Vec<JoinHandle<()>> {
    (0..count)
        .filter_map(|slot| {
            let mine = env.clone();
            let spawned = thread::Builder::new()
                .name(format!("clip-worker-{slot}"))
                .spawn(move || mine.run());
            match spawned {
                Ok(handle) => Some(handle),
                Err(error) => {
                    diag::warn!(
                        Audio,
                        "audio: prep thread #{slot} refused to start: {error}"
                    );
                    None
                }
            }
        })
        .collect()
}

impl ClipRepository {
    pub fn start(catalog: Arc<AudioRoster>, archive: Option<Arc<CategorySoundArchive>>) -> Self {
        Self::start_with_common(catalog, archive, 0, None)
    }

    pub(crate) fn start_with_common(
        catalog: Arc<AudioRoster>,
        archive: Option<Arc<CategorySoundArchive>>,
        common_profile_id: u64,
        clip_cache: Option<ResidentClipCache>,
    ) -> Self {
        let resident = clip_cache.map(|cache| {
            cache.use_profile(common_profile_id);
            cache.use_bank(&catalog);
            ResidentLink {
                cache,
                profile_id: common_profile_id,
            }
        });
        let (tx, rx) = channel();
        let outcomes = Arc::<Mutex<HashMap<WaveformHandle, Result<ClipPcm, ClipLoadFault>>>>::default();
        let stop = Arc::new(AtomicBool::new(false));
        let env = WorkerEnv {
            stop: Arc::clone(&stop),
            catalog: Arc::clone(&catalog),
            archive,
            jobs: Arc::new(Mutex::new(rx)),
            resident: resident.clone(),
            outcomes: Arc::clone(&outcomes),
        };
        let handles = spawn_workers(&env, prep_thread_count());
        tally::plus(Total::Workers, handles.len() as u64);

        Self {
            jobs_tx: tx,
            catalog,
            sent: HashSet::new(),
            outcomes,
            resident,
            reuse: Reuse::default(),
            late: None,
            workers: ClipWorkers { handles, stop },
        }
    }

    pub fn worker_n(&self) -> usize {
        self.workers.handles.len()
    }

    /// (clips, bytes) served from the resident cache instead of being decoded.
    pub fn reused_resident(&self) -> (usize, u64) {
        (self.reuse.clips, self.reuse.bytes)
    }

    pub(crate) fn resident_dry_handle(
        &self,
        wanted: &WaveformHandle,
        assets: &mut Assets<ClipPcm>,
        pcm: &ClipPcm,
    ) -> Option<(Handle<ClipPcm>, bool)> {
        let link = self.resident.as_ref()?;
        let key = resident_clip_key(&self.catalog, wanted)?;
        link.cache.shared_dry(link.profile_id, key, assets, pcm)
    }

    /// From now on a clip first prepared here is "late": the match is running.
    pub fn enable_round_playback(&mut self) {
        self.late.get_or_insert(0);
    }

    /// Late prepares of this store. A store lasts as long as its installed
    /// bank, so a map change starts the count over; [`clip_prep_cost`] has the
    /// process total for the exit hook.
    pub fn tardy_preloads(&self) -> u32 {
        self.late.unwrap_or(0)
    }

    /// Asks for every clip an alias can play; returns how many were newly queued.
    pub fn want_alias(&mut self, ns: AssetCategoryScope, cue_name: &str) -> usize {
        let keys = waveforms_for_cue(&self.catalog, ns, cue_name);
        keys.into_iter()
            .map(|key| usize::from(self.req_in(key)))
            .sum()
    }

    fn standing(&self, key: &WaveformHandle) -> Standing {
        if lock(&self.outcomes).contains_key(key) {
            return Standing::Settled;
        }
        let kept = self
            .resident
            .as_ref()
            .and_then(|link| link.fetch(&self.catalog, key));
        kept.map_or(Standing::Fresh, Standing::Resident)
    }

    /// Queues a clip unless it is already done, cached or queued. True only
    /// when a job was actually sent.
    pub(crate) fn req_in(&mut self, key: WaveformHandle) -> bool {
        tally::plus(Total::Requests, 1);
        match self.standing(&key) {
            Standing::Settled => false,
            Standing::Resident(pcm) => {
                self.reuse.clips += 1;
                self.reuse.bytes += sample_bytes(&pcm);
                lock(&self.outcomes).insert(key, Ok(pcm));
                false
            }
            Standing::Fresh => self.queue_key(key),
        }
    }

    fn queue_key(&mut self, key: WaveformHandle) -> bool {
        if !self.sent.insert(key.clone()) {
            return false;
        }
        if let Some(since_live) = self.late.as_mut() {
            *since_live = since_live.saturating_add(1);
            tally::plus(Total::Late, 1);
            diag::warn!(Audio, "sound: {key:?} only got ready mid-match");
        }
        tally::plus(Total::Queued, 1);
        let job = ClipJob {
            queued_t: Instant::now(),
            key,
        };
        let Err(refused) = self.jobs_tx.send(job) else {
            return true;
        };
        lock(&self.outcomes).insert(refused.0.key, Err(ClipLoadFault::QueueClosed));
        false
    }

    /// The outcome for a clip, if there is one yet.
    pub(crate) fn ready_pcm(&self, key: &WaveformHandle) -> Option<Result<ClipPcm, ClipLoadFault>> {
        lock(&self.outcomes).get(key).cloned()
    }
}

/// A one-shot waiting for its clip to finish preparing.
pub(crate) struct QueuedSingleCue {
    pub namespace: AssetCategoryScope,
    pub alias: String,
    pub mix_layer: Option<String>,
    pub reel: WaveformHandle,
    pub bound: Option<usize>,
    pub source_entity_no: Option<u32>,
    pub position_in_inches: Option<[f32; 3]>,
    pub class: NoiseCategory,
    pub due_at: Instant,
    pub gen_tag: u64,
    pub variant_ix: usize,
    pub gain: f32,
    pub pitch_mul: f32,
}

#[derive(Resource, Default)]
pub(crate) struct QueuedVoiceStarts {
    pub entries: Vec<QueuedSingleCue>,
}

impl QueuedVoiceStarts {
    pub fn queue_oneshot(&mut self, waiting: QueuedSingleCue) {
        self.entries.push(waiting);
    }

    /// Drops the waiting match sounds of one alias on one entity in one epoch.
    pub fn cancel_alias(
        &mut self,
        namespace: AssetCategoryScope,
        cue_id: &str,
        source_entity_no: Option<u32>,
        epoch: u64,
    ) {
        self.entries.retain(|entry| {
            let who = (entry.namespace, entry.source_entity_no, entry.gen_tag);
            let doomed = who == (namespace, source_entity_no, epoch)
                && entry.alias == cue_id
                && entry.class.scope() == SoundAudience::Match;
            !doomed
        });
    }

    pub fn clear(&mut self) {
        self.entries.truncate(0);
    }
}

pub(crate) fn due_time_for(kind: NoiseCategory) -> Instant {
    let wait = kind.oneshot_grace();
    Instant::now() + wait
}

/// Secondary-alias chains are followed this deep and no further.
const MAX_SECONDARY_DEPTH: u8 = 10;

/// The clip one alias row plays: its loaded entry, else its streamed file.
fn row_clip(
    catalog: &AudioRoster,
    id: (AssetCategoryScope, &str),
    bay: usize,
    entry: &SnapshotSoundAlias,
) -> Option<WaveformHandle> {
    let streamed = || {
        catalog
            .clip_for_variant(id.0, id.1, bay)
            .map(WaveformHandle::streamed)
    };
    entry
        .loaded
        .extent_position()
        .map(WaveformHandle::Ready)
        .or_else(streamed)
}

/// Adds the clips of `alias` to `found`, then those of its secondaries, row by
/// row. `visited` holds the aliases already walked.
fn gather_alias_clips(
    (visited, found): (&mut HashSet<String>, &mut Vec<WaveformHandle>),
    catalog: &AudioRoster,
    (ns, cue_key): (AssetCategoryScope, &str),
    depth: u8,
) {
    if depth > MAX_SECONDARY_DEPTH || !visited.insert(cue_key.to_owned()) {
        return;
    }
    let Some(sound) = catalog.entry_in(ns, cue_key) else {
        return;
    };
    for (slot, entry) in sound.aliases.iter().enumerate() {
        let clip = row_clip(catalog, (ns, cue_key), slot, entry);
        found.extend(clip.filter(|clip| !found.contains(clip)));
        match entry.layered_alias.as_deref() {
            Some(next) if !next.is_empty() => {
                let books = (&mut *visited, &mut *found);
                gather_alias_clips(books, catalog, (ns, next), depth + 1);
            }
            _ => {}
        }
    }
}

/// Every clip an alias can play, in order, including those reached through
/// secondary aliases. Each clip appears once.
pub(crate) fn waveforms_for_cue(
    catalog: &AudioRoster,
    ns: AssetCategoryScope,
    cue_id: &str,
) -> Vec<WaveformHandle> {
    let mut seen_keys = HashSet::new();
    let mut found = Vec::new();
    gather_alias_clips((&mut seen_keys, &mut found), catalog, (ns, cue_id), 0);
    found
}

/// The clip one variant of an alias plays: a named loaded clip if the caller
/// has one, else the variant's loaded row, else its streamed file.
pub(crate) fn waveform_for_variant(
    catalog: &AudioRoster,
    ns: AssetCategoryScope,
    cue_id: &str,
    bound: Option<usize>,
    pick: usize,
    named: Option<&str>,
    named_ns: Option<AssetCategoryScope>,
) -> Option<WaveformHandle> {
    if let Some((sound_name, from)) = named.zip(named_ns) {
        if let Some(index) = catalog.resident_slot_in(from, sound_name) {
            return Some(WaveformHandle::Ready(index));
        }
    }
    let sound = match bound {
        Some(index) => catalog.sound_at(index),
        None => catalog.entry_in(ns, cue_id),
    };
    let row_index = sound
        .and_then(|sound| sound.aliases.get(pick))
        .and_then(|row| row.loaded.extent_position());
    if let Some(index) = row_index {
        return Some(WaveformHandle::Ready(index));
    }
    let file = match bound {
        None => catalog.clip_for_variant(ns, cue_id, pick),
        Some(index) => catalog.streamed_for_variant_at(index, pick),
    };
    file.map(WaveformHandle::streamed)
}

/// Process-wide counters for the cost report; kept for the exit hook, which
/// has no store to ask.
mod tally {
    use super::{AtomicU64, ClipPath, Ordering, Total};

    /// One counter per [`Stat`] for each decoder path.
    #[derive(Clone, Copy)]
    pub(super) enum Stat {
        Prepared,
        Failed,
        WallNs,
        SampleBytes,
    }

    const STATS: usize = Stat::SampleBytes as usize + 1;
    const TOTALS: usize = Total::Late as usize + 1;

    static ALL: [AtomicU64; TOTALS] = [const { AtomicU64::new(0) }; TOTALS];
    static PATHS: [[AtomicU64; STATS]; ClipPath::COUNT] =
        [const { [const { AtomicU64::new(0) }; STATS] }; ClipPath::COUNT];

    pub(super) fn plus(total: Total, by: u64) {
        ALL[total as usize].fetch_add(by, Ordering::Relaxed);
    }

    pub(super) fn get(total: Total) -> u64 {
        ALL[total as usize].load(Ordering::Relaxed)
    }

    pub(super) fn add_path(path: ClipPath, stat: Stat, by: u64) {
        PATHS[path as usize][stat as usize].fetch_add(by, Ordering::Relaxed);
    }

    pub(super) fn get_path(path: ClipPath, stat: Stat) -> u64 {
        PATHS[path as usize][stat as usize].load(Ordering::Relaxed)
    }
}

use tally::Stat;

/// The process-wide counters that belong to no one decoder.
#[derive(Clone, Copy)]
enum Total {
    Workers,
    Requests,
    Queued,
    QueueWaitNs,
    Late,
}

/// What preparing clips has cost this process.
///
/// `requests` counts asks and `queued` counts jobs: the same clip is reached
/// from several aliases and weapons, and the difference is what the store's
/// dedupe saves. The durations add up over workers that run side by side, so
/// none of them is a stretch of wall-clock time.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClipPrepCost {
    pub paths: Vec<(ClipPath, ClipPathCost)>,
    pub queue_wait_ms: f64,
    pub requests: u64,
    pub queued: u64,
    pub late: u64,
    /// Prep threads that actually started, over every store this process has
    /// opened (a teardown closes one, a reload opens the next).
    pub workers: u64,
}

/// One decoder's share of the clip preparation.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ClipPathCost {
    /// Resident `f32` sample bytes this decoder produced. The store holds them
    /// for the whole match, so this measures memory, not speed.
    pub sample_bytes: u64,
    /// Worker time inside this decoder, summed over the prep threads.
    pub wall_ms: f64,
    pub failed: u64,
    pub prepared: u64,
}

fn nanos_to_ms(nanos: u64) -> f64 {
    nanos as f64 / 1.0e6
}

pub fn clip_prep_cost() -> ClipPrepCost {
    let share = |path: ClipPath| ClipPathCost {
        sample_bytes: tally::get_path(path, Stat::SampleBytes),
        wall_ms: nanos_to_ms(tally::get_path(path, Stat::WallNs)),
        failed: tally::get_path(path, Stat::Failed),
        prepared: tally::get_path(path, Stat::Prepared),
    };
    ClipPrepCost {
        paths: ClipPath::ALL_KINDS
            .iter()
            .map(|&path| (path, share(path)))
            .collect(),
        queue_wait_ms: nanos_to_ms(tally::get(Total::QueueWaitNs)),
        requests: tally::get(Total::Requests),
        queued: tally::get(Total::Queued),
        late: tally::get(Total::Late),
        workers: tally::get(Total::Workers),
    }
}

/// Prepares a worker's batch in order, charging each clip to its decoder.
fn prepare_batch(
    catalog: &AudioRoster,
    archive: Option<&CategorySoundArchive>,
    jobs: &[ClipJob],
) -> Vec<Result<ClipPcm, ClipLoadFault>> {
    let timed = |job: &ClipJob| {
        let t0 = Instant::now();
        let (path, outcome) = prepare_clip(catalog, archive, &job.key);
        tally::add_path(path, Stat::WallNs, t0.elapsed().as_nanos() as u64);
        match &outcome {
            Ok(pcm) => {
                tally::add_path(path, Stat::SampleBytes, sample_bytes(pcm));
                tally::add_path(path, Stat::Prepared, 1);
            }
            Err(_) => tally::add_path(path, Stat::Failed, 1),
        }
        outcome
    };
    jobs.iter().map(timed).collect()
}

fn prepare_clip(
    catalog: &AudioRoster,
    archive: Option<&CategorySoundArchive>,
    wanted: &WaveformHandle,
) -> (ClipPath, Result<ClipPcm, ClipLoadFault>) {
    let slot = match wanted {
        WaveformHandle::Ready(slot) => *slot,
        WaveformHandle::Streamed { ns, name, folder } => {
            let streamed = prepare_streamed_clip(archive, *ns, folder, name);
            return (ClipPath::Streamed, streamed);
        }
    };
    let Some(sound) = catalog.resident_clip(slot) else {
        return (ClipPath::NotFound, Err(ClipLoadFault::Decode));
    };
    match loaded_decoder(sound) {
        Some(path) => (path, prepare_loaded(sound, path)),
        None => (ClipPath::NotFound, Err(ClipLoadFault::Decode)),
    }
}

/// Which decoder reads a loaded clip: ADPCM bytes first, else plain PCM, else
/// none. Preparing a clip uses this same answer, so it is never counted under
/// one path and decoded by another.
fn loaded_decoder(sound: &ResidentRawAudio) -> Option<ClipPath> {
    match (sound.adpcm_payload(), sound.format()) {
        (Some(_), _) => Some(ClipPath::Adpcm),
        (None, asset_audio::RAW_WAVE_FORMAT) => Some(ClipPath::Pcm),
        (None, _) => None,
    }
}

fn prepare_loaded(clip_name: &ResidentRawAudio, route: ClipPath) -> Result<ClipPcm, ClipLoadFault> {
    match route {
        ClipPath::Adpcm => {
            let bytes = clip_name.adpcm_payload().ok_or(ClipLoadFault::Decode)?;
            let lanes =
                u16::try_from(clip_name.channel_total().max(1)).map_err(|_| ClipLoadFault::Decode)?;
            adpcm_t5::decode_xbadpcm(bytes, clip_name.frame_count, clip_name.sample_rate, lanes.into())
                .ok_or(ClipLoadFault::Decode)
        }
        ClipPath::Pcm => interleave_pcm(
            clip_name.encoded_bytes(),
            clip_name.bits(),
            clip_name.channel_total(),
            clip_name.sample_rate,
        ),
        ClipPath::Streamed | ClipPath::NotFound => Err(ClipLoadFault::Decode),
    }
}

/// Raw 8- or 16-bit samples as interleaved `f32`: a whole number of frames,
/// and never empty (no samples is a failed decode, not a silent clip).
fn interleave_pcm(
    raw: &[u8],
    bits: i32,
    lane_count: i32,
    hz: u32,
) -> Result<ClipPcm, ClipLoadFault> {
    let (width, to_float): (usize, fn(&[u8]) -> f32) = match bits {
        8 => (1, |one: &[u8]| (f32::from(one[0]) - 128.0) / 128.0),
        16 => (2, |two: &[u8]| {
            f32::from(i16::from_le_bytes([two[0], two[1]])) / 32768.0
        }),
        _ => return Err(ClipLoadFault::Decode),
    };
    let lanes = lane_count.max(1) as usize;
    let whole = raw.len() / width / lanes * lanes;
    let samples: Arc<[f32]> = raw.chunks_exact(width).take(whole).map(to_float).collect();
    let lanes = u16::try_from(lanes).map_err(|_| ClipLoadFault::Decode)?;
    ClipPcm::from_parts(samples, lanes, hz.max(1)).ok_or(ClipLoadFault::Decode)
}

/// A Black Ops II clip names its sound bank file and an entry in it.
fn prepare_bank_entry(dir: &str, name: &str) -> Result<ClipPcm, ClipLoadFault> {
    let clip = asset_audio::decode_t6_clip(dir, name).map_err(|error| {
        diag::warn!(Audio, "sound: clip `{name}` under `{dir}` failed: {error}");
        ClipLoadFault::Decode
    })?;
    let samples: Arc<[f32]> = clip
        .samples
        .iter()
        .map(|&s| f32::from(s) / 32768.0)
        .collect();
    ClipPcm::from_parts(samples, clip.channels.max(1), clip.rate).ok_or(ClipLoadFault::Decode)
}

fn prepare_streamed_clip(
    archive: Option<&CategorySoundArchive>,
    cue_scope: AssetCategoryScope,
    dir: &str,
    name: &str,
) -> Result<ClipPcm, ClipLoadFault> {
    if cue_scope == AssetCategoryScope::T6 {
        return prepare_bank_entry(dir, name);
    }
    let iwd = archive.ok_or(ClipLoadFault::Read)?;
    let path = format!("{dir}/{name}");
    let bytes = iwd
        .fetch_audio_clip(cue_scope, &path)
        .ok_or_else(|| {
            diag::warn!(
                Audio,
                "audio: streamed IWD miss `{}:{path}` (typed gap)",
                cue_scope.as_str()
            );
            ClipLoadFault::Read
        })?
        .map_err(|err| {
            diag::warn!(Audio, "sound: could not read `{path}` from the IWD ({err})");
            ClipLoadFault::Read
        })?;
    // T5 keeps its own container unless the file is a plain RIFF.
    let own_container = cue_scope == AssetCategoryScope::T5 && !bytes.starts_with(b"RIFF");
    let decoded = if own_container {
        adpcm_t5::decode(&bytes)
    } else {
        decode_clip_bytes(&bytes[..])
    };
    decoded.ok_or_else(|| {
        diag::warn!(
            Audio,
            "audio: failed to decode streamed `{}:{path}`",
            cue_scope.as_str()
        );
        ClipLoadFault::Decode
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use asset_audio::{SnapshotSoundAlias, SnapshotSoundEntry};
    use std::time::Duration;

    const T5: AssetCategoryScope = AssetCategoryScope::T5;
    const T6: AssetCategoryScope = AssetCategoryScope::T6;

    fn streamed_row(bank: &str, entry: &str, secondary: Option<&str>) -> SnapshotSoundAlias {
        let mut row = SnapshotSoundAlias::default();
        row.streamed = Some((bank.to_owned(), entry.to_owned()));
        row.layered_alias = secondary.map(str::to_owned);
        row
    }

    fn catalog_of(sounds: Vec<(&str, Vec<SnapshotSoundAlias>)>) -> AudioRoster {
        let mut catalog = AudioRoster::default();
        for (name, aliases) in sounds {
            catalog.add_sound(SnapshotSoundEntry {
                name: name.to_owned(),
                aliases,
            });
        }
        catalog
    }

    fn streamed_key(file: &str) -> WaveformHandle {
        WaveformHandle::Streamed {
            ns: T6,
            folder: "bank".to_owned(),
            name: file.to_owned(),
        }
    }

    fn t5_file(file: &str) -> WaveformHandle {
        WaveformHandle::Streamed {
            ns: T5,
            folder: "snd".to_owned(),
            name: file.to_owned(),
        }
    }

    fn held_key(tag: u8) -> LoadedClipKey {
        LoadedClipKey {
            encoded: SameBuffer(Arc::from(vec![tag])),
            layout: StoredLayout {
                codec: 1,
                lanes: 1,
                depth: 16,
                hz: 22_050,
                frame_ct: 1,
                block: 0,
                seeks: Vec::new(),
            },
        }
    }

    fn common(tag: u8) -> ResidentClipKey {
        ResidentClipKey::Loaded {
            common: true,
            clip: held_key(tag),
        }
    }

    fn map_only(tag: u8) -> ResidentClipKey {
        ResidentClipKey::Loaded {
            common: false,
            clip: held_key(tag),
        }
    }

    fn clip_of(data: &[f32]) -> ClipPcm {
        ClipPcm::from_parts(Arc::from(data), 1, 8_000).expect("pcm")
    }

    fn shape(reel: &ClipPcm) -> (Vec<f32>, u16, u32) {
        (reel.data().to_vec(), reel.lane_count(), reel.hz())
    }

    fn files_of(keys: Vec<WaveformHandle>) -> Vec<String> {
        keys.into_iter()
            .map(|key| match key {
                WaveformHandle::Streamed { name, .. } => name,
                WaveformHandle::Ready(_) => unreachable!("black ops II keeps nothing in memory"),
            })
            .collect()
    }

    #[test]
    fn common_zones_are_the_shared_ones_only() {
        for zone in [
            "localized_common_mp",
            "common_mp",
            "patch_mp",
            "localized_code_post_gfx_mp",
            "code_post_gfx_mp",
        ] {
            assert!(is_common_zone(zone), "{zone}");
        }
        for zone in ["localized_patch_mp", "mp_nuketown_2020", "", "common"] {
            assert!(!is_common_zone(zone), "{zone}");
        }
        assert!(common(1).is_common());
        assert!(!map_only(1).is_common());
        assert!(!ResidentClipKey::Streamed(streamed_key("a")).is_common());
    }

    #[test]
    fn streamed_keys_need_no_bank_entry_to_be_resident() {
        let bank = AudioRoster::default();
        let streamed = streamed_key("7");
        assert!(
            resident_clip_key(&bank, &streamed)
                == Some(ResidentClipKey::Streamed(streamed.clone()))
        );
        assert!(resident_clip_key(&bank, &WaveformHandle::Ready(0)).is_none());
    }

    #[test]
    fn eight_bit_samples_centre_on_silence() {
        let clip = interleave_pcm(&[128, 255, 0], 8, 1, 11_025).unwrap();
        assert_eq!(shape(&clip), (vec![0.0, 127.0 / 128.0, -1.0], 1, 11_025));
    }

    #[test]
    fn sixteen_bit_samples_scale_by_32768() {
        let clip = interleave_pcm(&[0x00, 0x40, 0x00, 0xC0], 16, 2, 22_050).unwrap();
        assert_eq!(shape(&clip), (vec![0.5, -0.5], 2, 22_050));
    }

    #[test]
    fn partial_frames_are_trimmed_and_empty_clips_fail() {
        let clip = interleave_pcm(&[128; 7], 8, 2, 8_000).unwrap();
        assert_eq!(clip.data().len(), 6);
        let odd = interleave_pcm(&[0, 0, 0, 0, 0, 0, 9], 16, 2, 8_000).unwrap();
        assert_eq!(odd.data().len(), 2);
        assert!(matches!(
            interleave_pcm(&[128], 8, 2, 8_000),
            Err(ClipLoadFault::Decode)
        ));
        assert!(matches!(
            interleave_pcm(&[], 8, 1, 8_000),
            Err(ClipLoadFault::Decode)
        ));
        assert!(matches!(
            interleave_pcm(&[0; 6], 24, 1, 8_000),
            Err(ClipLoadFault::Decode)
        ));
    }

    #[test]
    fn zero_rate_and_zero_channels_are_raised_to_one() {
        let clip = interleave_pcm(&[128, 128], 8, 0, 0).unwrap();
        assert_eq!((clip.lane_count(), clip.hz()), (1, 1));
        assert_eq!(clip.data().len(), 2);
    }

    #[test]
    fn finished_clips_report_their_float_bytes() {
        assert_eq!(sample_bytes(&clip_of(&[0.25, -0.25, 0.5])), 12);
    }

    #[test]
    fn same_buffer_means_same_allocation() {
        let one: Arc<[u8]> = Arc::from(vec![1_u8, 2]);
        let copy: Arc<[u8]> = Arc::from(vec![1_u8, 2]);
        assert!(SameBuffer(Arc::clone(&one)) == SameBuffer(Arc::clone(&one)));
        assert!(SameBuffer(one) != SameBuffer(copy));
    }

    #[test]
    fn resident_cache_ignores_profile_zero_and_other_profiles() {
        let cache = ResidentClipCache::default();
        let clip_key = common(1);
        cache.remember(0, clip_key.clone(), clip_of(&[1.0]));
        assert!(cache.ready(0, &clip_key).is_none());

        cache.use_profile(7);
        cache.remember(7, clip_key.clone(), clip_of(&[1.0]));
        assert!(cache.ready(7, &clip_key).is_some());
        assert!(cache.ready(8, &clip_key).is_none());

        cache.use_profile(7);
        assert!(
            cache.ready(7, &clip_key).is_some(),
            "same profile keeps the cache"
        );
        cache.use_profile(9);
        assert!(cache.ready(9, &clip_key).is_none(), "a new profile starts empty");
    }

    #[test]
    fn remembering_twice_keeps_the_first_clip() {
        let cache = ResidentClipCache::default();
        cache.use_profile(2);
        let clip_key = common(1);
        cache.remember(2, clip_key.clone(), clip_of(&[1.0]));
        cache.remember(2, clip_key.clone(), clip_of(&[2.0]));
        assert_eq!(shape(&cache.ready(2, &clip_key).unwrap()).0, vec![1.0]);
    }

    #[test]
    fn a_new_bank_keeps_only_the_common_clips() {
        let cache = ResidentClipCache::default();
        cache.use_profile(3);
        let first = Arc::new(AudioRoster::default());
        cache.use_bank(&first);
        let common = common(1);
        let map = map_only(2);
        let streamed = ResidentClipKey::Streamed(streamed_key("a"));
        for clip_key in [&common, &map, &streamed] {
            cache.remember(3, clip_key.clone(), clip_of(&[0.5]));
        }

        cache.use_bank(&first);
        assert!(cache.ready(3, &map).is_some(), "same bank changes nothing");

        let second = Arc::new(AudioRoster::default());
        cache.use_bank(&second);
        assert!(cache.ready(3, &common).is_some());
        assert!(cache.ready(3, &map).is_none());
        assert!(cache.ready(3, &streamed).is_none());
    }

    #[test]
    fn dry_handles_are_made_once_per_clip_and_dropped_with_their_asset() {
        let cache = ResidentClipCache::default();
        cache.use_profile(5);
        let mut assets = Assets::<ClipPcm>::default();
        let audio = ClipPcm::from_parts(Arc::from(vec![0.1_f32, 0.2]), 1, 8_000).unwrap();
        let clip_key = common(4);

        let unshared = cache.shared_dry(0, clip_key.clone(), &mut assets, &audio);
        assert!(unshared.is_none(), "profile 0 never shares");
        let (first, reused) = cache
            .shared_dry(5, clip_key.clone(), &mut assets, &audio)
            .unwrap();
        assert!(!reused);
        let (second, reused) = cache
            .shared_dry(5, clip_key.clone(), &mut assets, &audio)
            .unwrap();
        assert!(reused);
        assert_eq!(first.id(), second.id());

        assets.remove(second.id());
        let (third, reused) = cache.shared_dry(5, clip_key, &mut assets, &audio).unwrap();
        assert!(!reused, "a handle whose asset is gone is made again");
        assert_ne!(first.id(), third.id());
    }

    #[test]
    fn alias_clips_follow_secondaries_once_each_in_order() {
        let catalog = catalog_of(vec![
            (
                "main",
                vec![
                    streamed_row("bank", "01", Some("tail")),
                    streamed_row("bank", "02", None),
                ],
            ),
            (
                "tail",
                vec![
                    streamed_row("bank", "02", Some("main")),
                    streamed_row("bank", "03", None),
                ],
            ),
        ]);
        assert_eq!(
            files_of(waveforms_for_cue(&catalog, T6, "main")),
            ["01", "02", "03"]
        );
        assert!(waveforms_for_cue(&catalog, T6, "missing").is_empty());
    }

    #[test]
    fn empty_secondaries_and_clipless_rows_are_skipped() {
        let catalog = catalog_of(vec![(
            "main",
            vec![
                streamed_row("bank", "01", Some("")),
                SnapshotSoundAlias::default(),
                streamed_row("bank", "02", Some("ghost")),
            ],
        )]);
        assert_eq!(
            files_of(waveforms_for_cue(&catalog, T6, "main")),
            ["01", "02"]
        );
    }

    #[test]
    fn secondary_chains_stop_after_the_depth_limit() {
        let sounds: Vec<(String, Vec<SnapshotSoundAlias>)> = (0..15)
            .map(|n| {
                let next = format!("link{}", n + 1);
                let row = streamed_row("bank", &format!("{n:02}"), Some(&next));
                (format!("link{n}"), vec![row])
            })
            .collect();
        let catalog = catalog_of(
            sounds
                .iter()
                .map(|(n, a)| (n.as_str(), a.clone()))
                .collect(),
        );
        let reached = waveforms_for_cue(&catalog, T6, "link0").len();
        assert_eq!(reached, usize::from(MAX_SECONDARY_DEPTH) + 1);
    }

    #[test]
    fn variant_clips_come_from_the_archive_or_nowhere() {
        let catalog = catalog_of(vec![(
            "shot",
            vec![
                streamed_row("bank", "0a", None),
                SnapshotSoundAlias::default(),
            ],
        )]);
        let key = |alias, bound, variant| {
            waveform_for_variant(&catalog, T6, alias, bound, variant, None, None)
        };
        assert_eq!(key("shot", None, 0), Some(streamed_key("0a")));
        assert_eq!(key("shot", Some(0), 0), Some(streamed_key("0a")));
        assert_eq!(key("shot", None, 1), None);
        assert_eq!(key("shot", None, 5), None);
        assert_eq!(key("nothing", None, 0), None);
        assert_eq!(key("shot", Some(9), 0), None);
        let named = waveform_for_variant(&catalog, T6, "shot", None, 0, Some("x"), Some(T6));
        assert_eq!(
            named,
            Some(streamed_key("0a")),
            "no loaded clip by that name"
        );
    }

    fn waiting(
        cue_id: &str,
        class: NoiseCategory,
        epoch: u64,
        source_entity_no: Option<u32>,
    ) -> QueuedSingleCue {
        QueuedSingleCue {
            namespace: T6,
            alias: cue_id.to_owned(),
            bound: None,
            variant_ix: 0,
            gain: 1.0,
            pitch_mul: 1.0,
            position_in_inches: None,
            source_entity_no,
            reel: streamed_key("x"),
            mix_layer: None,
            class,
            gen_tag: epoch,
            due_at: Instant::now(),
        }
    }

    #[test]
    fn cancelling_drops_only_matching_match_sounds() {
        let mut starts = QueuedVoiceStarts::default();
        starts.queue_oneshot(waiting("a", NoiseCategory::World, 1, Some(4)));
        starts.queue_oneshot(waiting("a", NoiseCategory::World, 2, Some(4)));
        starts.queue_oneshot(waiting("a", NoiseCategory::World, 1, None));
        starts.queue_oneshot(waiting("b", NoiseCategory::World, 1, Some(4)));
        starts.queue_oneshot(waiting("a", NoiseCategory::Ui, 1, Some(4)));
        starts.queue_oneshot(waiting("a", NoiseCategory::Weapon, 1, Some(4)));

        starts.cancel_alias(T6, "a", Some(4), 1);
        let left: Vec<_> = starts
            .entries
            .iter()
            .map(|e| (e.alias.as_str(), e.gen_tag, e.source_entity_no))
            .collect();
        assert_eq!(
            left,
            [
                ("a", 2, Some(4)),
                ("a", 1, None),
                ("b", 1, Some(4)),
                ("a", 1, Some(4))
            ]
        );
        starts.clear();
        assert!(starts.entries.is_empty());
    }

    #[test]
    fn deadlines_follow_the_class_wait() {
        let before = Instant::now();
        let deadline = due_time_for(NoiseCategory::Weapon);
        assert!(deadline >= before + Duration::from_millis(200));
        assert!(deadline < before + Duration::from_secs(5));
    }

    #[test]
    fn clip_paths_count_and_name_themselves() {
        assert_eq!(ClipPath::COUNT, ClipPath::ALL_KINDS.len());
        for (slot, path) in ClipPath::ALL_KINDS.into_iter().enumerate() {
            assert_eq!(path as usize, slot);
        }
        let names: Vec<_> = ClipPath::ALL_KINDS.iter().map(|path| path.name()).collect();
        assert_eq!(names, ["pcm", "t5 adpcm", "streamed", "unresolved"]);
    }

    #[test]
    fn loaded_clips_without_a_known_format_have_no_decoder() {
        let sound = ResidentRawAudio::default();
        assert_eq!(loaded_decoder(&sound), None);
        assert!(matches!(
            prepare_loaded(&sound, ClipPath::NotFound),
            Err(ClipLoadFault::Decode)
        ));
        assert!(matches!(
            prepare_loaded(&sound, ClipPath::Adpcm),
            Err(ClipLoadFault::Decode)
        ));
    }

    #[test]
    fn preparing_streamed_clips_reports_why_it_failed() {
        let bank = AudioRoster::default();
        let (path, outcome) = prepare_clip(&bank, None, &t5_file("a.wav"));
        assert_eq!(path, ClipPath::Streamed);
        assert!(matches!(outcome, Err(ClipLoadFault::Read)));
        let (path, outcome) = prepare_clip(&bank, None, &streamed_key("zz"));
        assert_eq!(path, ClipPath::Streamed);
        assert!(matches!(outcome, Err(ClipLoadFault::Decode)));
        let (path, outcome) = prepare_clip(&bank, None, &WaveformHandle::Ready(4));
        assert_eq!(path, ClipPath::NotFound);
        assert!(matches!(outcome, Err(ClipLoadFault::Decode)));
    }

    #[test]
    fn a_batch_gives_one_result_per_job_in_order() {
        let jobs: Vec<ClipJob> = [t5_file("1"), WaveformHandle::Ready(0), t5_file("2")]
            .into_iter()
            .map(|key| ClipJob {
                key,
                queued_t: Instant::now(),
            })
            .collect();
        let results = prepare_batch(&AudioRoster::default(), None, &jobs);
        let kinds: Vec<_> = results
            .iter()
            .map(|result| matches!(result, Err(ClipLoadFault::Read)))
            .collect();
        assert_eq!(kinds, [true, false, true]);
    }

    fn wait_for(store: &ClipRepository, key: &WaveformHandle) -> Result<ClipPcm, ClipLoadFault> {
        let limit = Instant::now() + Duration::from_secs(20);
        let mut outcome = store.ready_pcm(key);
        while outcome.is_none() {
            assert!(Instant::now() < limit, "clip never finished");
            std::thread::sleep(Duration::from_millis(5));
            outcome = store.ready_pcm(key);
        }
        outcome.expect("loop ends only on an outcome")
    }

    #[test]
    fn store_reports_unreadable_archives_once_and_counts_late_asks() {
        let mut store = ClipRepository::start(Arc::new(AudioRoster::default()), None);
        assert!(store.worker_n() >= 1);
        let before = clip_prep_cost();

        let first = t5_file("one.wav");
        assert!(store.req_in(first.clone()));
        assert!(
            !store.req_in(first.clone()),
            "a queued clip is not sent twice"
        );
        assert!(matches!(wait_for(&store, &first), Err(ClipLoadFault::Read)));
        assert!(!store.req_in(first), "a finished clip is not sent again");
        assert_eq!(store.tardy_preloads(), 0);

        store.enable_round_playback();
        let second = t5_file("two.wav");
        assert!(store.req_in(second.clone()));
        assert!(matches!(
            wait_for(&store, &second),
            Err(ClipLoadFault::Read)
        ));
        assert_eq!(store.tardy_preloads(), 1);

        let after = clip_prep_cost();
        assert!(after.requests >= before.requests + 3);
        assert!(after.queued >= before.queued + 2);
        assert!(after.late > before.late);
        let failed_streams = |cost: &ClipPrepCost| {
            let streamed = cost
                .paths
                .iter()
                .find(|(path, _)| *path == ClipPath::Streamed);
            streamed.map(|(_, cost)| cost.failed).unwrap()
        };
        assert!(failed_streams(&after) >= failed_streams(&before) + 2);
    }

    #[test]
    fn store_marks_unknown_bank_clips_unresolved() {
        let mut store = ClipRepository::start(Arc::new(AudioRoster::default()), None);
        let key = WaveformHandle::Ready(3);
        assert!(store.req_in(key.clone()));
        assert!(matches!(wait_for(&store, &key), Err(ClipLoadFault::Decode)));
        assert_eq!(store.reused_resident(), (0, 0));
    }

    #[test]
    fn requesting_an_alias_queues_each_of_its_clips_once() {
        let catalog = catalog_of(vec![(
            "pair",
            vec![
                streamed_row("nobank", "0001", None),
                streamed_row("nobank", "0002", None),
            ],
        )]);
        let mut store = ClipRepository::start(Arc::new(catalog), None);
        assert_eq!(store.want_alias(T6, "pair"), 2);
        assert_eq!(store.want_alias(T6, "pair"), 0);
        assert_eq!(store.want_alias(T6, "absent"), 0);
    }

    #[test]
    fn a_dropped_store_joins_its_workers() {
        let store = ClipRepository::start(Arc::new(AudioRoster::default()), None);
        assert!((1..=4).contains(&store.worker_n()));
        drop(store);
    }
}
