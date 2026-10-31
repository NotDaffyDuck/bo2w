//! What the map itself sounds like: the room tone, the CreateFX loop emitters,
//! the sound bank that is put together in the background while a match loads,
//! and the level each positional emitter has from frame to frame.

use std::{
    collections::HashMap,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use asset_audio::{
    SnapshotSoundAlias, InMemoryAudioBank, RandomPointSound, AudioRoster, StereoPan, compose_sound_bank,
    gather_sound_sources, zone_sound_namespace,
};
use asset_core::AssetCategoryScope;
use asset_transport::{GameInstallDir, locate_zone_file};
use assets::{
    SessionLoadCancel, SessionLoadApproved, CategorySoundArchive, CategoryTreeSet, PreparedMatchSound,
};
use bevy::{
    audio::{AudioSink, AudioSinkPlayback as _, Volume},
    ecs::system::SystemParam,
    prelude::*,
    tasks::{
        AsyncComputeTaskPool, Task, TaskPool,
        futures_lite::future::{block_on, poll_once},
    },
};
use frame::{GameSessionClosed, ReturnedToMenu};

use crate::{
    attenuation::attenuate,
    backend::{SoundAudience, GameRoundSerial, PlayingCue, bind_repeating_cue, unbind_repeating_cue, begin_repeating_cue},
    clip_bank::waveforms_for_cue,
    sample_feed::{PanHandle, LoopedPcm, ClipPcm, with_stereo_pan},
    play_core::{
        BackgroundHearer, UnknownCueHoles, CommonPlaybackStore, AudioCatalogRes,
        oneshot_lr_gains,
    },
    distance::{gap_in_inches, inches_of},
};

/// How many emitters may hold a voice at once.
const VOICE_BUDGET: usize = 8;

/// The level under which an emitter is not worth a voice.
const GAIN_FLOOR: f32 = 0.002;

/// Silence of this long from the compose job gets one warning.
const STALL_AFTER: Duration = Duration::from_secs(20);

/// Marks everything that belongs to the map's ambience and goes with the match.
#[derive(Component)]
pub struct LevelBackdropSound;

/// Only this many emitters hold a voice at once; the loudest win.
pub const MAP_EMITTER_CAP: usize = VOICE_BUDGET;

/// An emitter quieter than this is not worth a voice.
pub const EMITTER_GAIN_FLOOR: f32 = GAIN_FLOOR;

#[derive(Component)]
pub struct LevelSoundSource {
    pub speed: f32,
    pub pcm: Handle<ClipPcm>,
    pub knots: Arc<[[f32; 2]]>,
    pub rest_gain: f32,
    pub reach_upper: f32,
    pub reach_lower: f32,
    pub position_in_inches: [f32; 3],
    pub active_stereo_handle: Option<PanHandle>,
    /// A sound with a pan of its own falls off with distance but does not
    /// turn with the listener.
    pub fixed_pan: Option<StereoPan>,
    /// The menu volume this sound answers to.
    pub category: Option<u8>,
    /// bo2zm: a line emitter's other end (heard from the nearest point).
    pub line_end_inches: Option<[f32; 3]>,
}

impl LevelSoundSource {
    /// Where the listener hears it from: its place, or a line emitter's
    /// point nearest the listener (`closest_point_on_line_to_point`).
    pub fn heard_from(&self, ear_pos: [f32; 3]) -> [f32; 3] {
        let Some(b) = self.line_end_inches else {
            return self.position_in_inches;
        };
        let a = self.position_in_inches;
        let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let len2 = ab[0] * ab[0] + ab[1] * ab[1] + ab[2] * ab[2];
        if len2 <= f32::EPSILON {
            return a;
        }
        let t = (((ear_pos[0] - a[0]) * ab[0]
            + (ear_pos[1] - a[1]) * ab[1]
            + (ear_pos[2] - a[2]) * ab[2])
            / len2)
            .clamp(0.0, 1.0);
        [a[0] + ab[0] * t, a[1] + ab[1] * t, a[2] + ab[2] * t]
    }
}

impl Default for LevelSoundSource {
    /// An emitter at the origin that hears nothing yet: no curve, no range,
    /// played at its recorded speed.
    fn default() -> Self {
        Self {
            speed: 1.0,
            pcm: Handle::<ClipPcm>::default(),
            knots: Arc::<[[f32; 2]]>::default(),
            rest_gain: 0.0,
            reach_upper: 0.0,
            reach_lower: 0.0,
            position_in_inches: [0.0; 3],
            active_stereo_handle: Option::default(),
            fixed_pan: Option::default(),
            category: Option::default(),
            line_end_inches: None,
        }
    }
}

#[derive(Resource, Clone)]
pub struct SoundArchive(pub Arc<CategorySoundArchive>);

#[derive(Resource, Default)]
pub struct AmbientBedStarted(pub bool);

/// Set once the sound bank for the current load has been started (or skipped).
#[derive(Resource, Default)]
pub(crate) struct BankLoadTried(pub bool);

/// Which zone the live sound bank was composed for, and its game.
#[derive(Resource)]
pub(crate) struct CatalogNameSpace {
    pub(crate) zone: String,
    pub(crate) namespace: AssetCategoryScope,
}

fn compute_pool() -> &'static AsyncComputeTaskPool {
    AsyncComputeTaskPool::get_or_init(TaskPool::default)
}

/// One look at a background task: its result if it has finished.
fn poll_now<T>(task: &mut Task<T>) -> Option<T> {
    block_on(poll_once(task))
}

// ---------------------------------------------------------------------------
// Sounds that come and go on a timer
// ---------------------------------------------------------------------------

/// One step of a xorshift generator; the numbers only need to look scattered.
fn scatter_step(seed: &mut u32) -> u32 {
    let stirred = *seed ^ (*seed << 13);
    let folded = stirred ^ (stirred >> 17);
    *seed = folded ^ (folded << 5);
    *seed
}

/// Whole seconds until a sound plays again: somewhere inside its range.
fn wait_between(sound: &RandomPointSound, seed: &mut u32) -> f64 {
    let [low, high] = sound.wait_secs.map(|secs| secs.max(0.0) as u32);
    let span = high.max(low + 1) - low;
    f64::from(low + scatter_step(seed) % span)
}

/// One of the places, chosen by the generator (none when there are none).
fn pick_place(places: &[[f32; 3]], seed: &mut u32) -> Option<[f32; 3]> {
    if places.is_empty() {
        return None;
    }
    places.get(scatter_step(seed) as usize % places.len()).copied()
}

/// When each scattered sound is next due, for one match epoch.
#[derive(Default)]
pub(crate) struct ScatterClock {
    epoch: u64,
    due: Vec<f64>,
    seed: u32,
}

impl ScatterClock {
    fn covers(&self, epoch: u64, sounds: usize) -> bool {
        self.epoch == epoch && self.due.len() == sounds
    }

    /// Starts every sound's first wait from `now`.
    fn restart(&mut self, round_no: u64, now: f64, sounds: &[RandomPointSound]) {
        let mut seed = 0x9e37_79b9 ^ (now.to_bits() as u32);
        let due = sounds
            .iter()
            .map(|sound| now + wait_between(sound, &mut seed))
            .collect();
        self.epoch = round_no;
        self.due = due;
        self.seed = seed;
    }
}

fn scattered_play(sound: &RandomPointSound, at: [f32; 3]) -> crate::AliasCommand {
    crate::AliasCommand::Play(crate::TriggerCueMsg {
        alias: sound.cue_alias.clone(),
        position_in_inches: Some(at),
        cue_ns: AssetCategoryScope::T6,
        source_entity_no: None,
        fallback: Option::default(),
    })
}

/// A Black Ops II map's now-and-then sounds (Nuketown's thunder): each waits a
/// whole number of seconds in its range, then plays at one of its places.
pub(crate) fn play_random_point_sounds(
    mut play: MessageWriter<crate::AliasCommand>,
    mut clock: Local<ScatterClock>,
    epoch: Res<GameRoundSerial>,
    booted: Res<AmbientBedStarted>,
    bank: Option<Res<AudioCatalogRes>>,
    time: Res<Time>,
) {
    let Some(bank) = bank.filter(|_| booted.0) else {
        return;
    };
    let sounds = bank.0.random_point_sounds();
    if sounds.is_empty() {
        return;
    }
    let now = time.elapsed_secs_f64();
    if !clock.covers(epoch.0, sounds.len()) {
        clock.restart(epoch.0, now, sounds);
    }

    let ScatterClock { due, seed, .. } = &mut *clock;
    for (sound, next) in sounds.iter().zip(due.iter_mut()) {
        if now < *next {
            continue;
        }
        let Some(at) = pick_place(&sound.places_inches, seed) else {
            continue;
        };
        play.write(scattered_play(sound, at));
        *next = now + wait_between(sound, seed);
    }
}

// ---------------------------------------------------------------------------
// The sound bank, composed while the match loads
// ---------------------------------------------------------------------------

/// The loose-file sound index: still opening, or open.
enum Archives {
    Opening(Task<(CategorySoundArchive, Vec<String>)>),
    Ready(Arc<CategorySoundArchive>),
}

/// A composed catalog with what was missing from it and the game it is for.
#[derive(Clone)]
struct Product {
    catalog: Arc<AudioRoster>,
    namespace: AssetCategoryScope,
    gaps: usize,
}

struct BankOutcome {
    product: Result<Product, String>,
    reused: bool,
}

/// A sound bank being composed off-thread for the accepted load.
#[derive(Resource)]
pub(crate) struct SoundBankCompose {
    zone: String,
    since: Instant,
    load: frame::OwnLevelLoadTicket,
    archives: Archives,
    composing: Option<Task<BankOutcome>>,
    product_stamp: u64,
    common_stamp: u64,
    stall_logged: bool,
}

/// The last composed bank, kept so loading the same map again skips the work.
#[derive(Resource, Default)]
pub(crate) struct ResidentSoundBank(Option<ResidentBank>);

struct ResidentBank {
    archives: Arc<CategorySoundArchive>,
    zone: String,
    composed: Option<ResidentComposed>,
}

struct ResidentComposed {
    product_stamp: u64,
    common_stamp: u64,
    product: Product,
}

impl ResidentSoundBank {
    /// The kept product, when it was composed for this zone from the same
    /// map products and common profile.
    fn reusable(&self, zone: &str, product_stamp: u64, common_stamp: u64) -> Option<Product> {
        let kept = self.0.as_ref().filter(|kept| kept.zone == zone)?;
        let made = kept.composed.as_ref()?;
        let same = made.product_stamp == product_stamp && made.common_stamp == common_stamp;
        same.then(|| made.product.clone())
    }
}

/// The accepted load, the abort that may have followed it, and who we are.
#[derive(SystemParam)]
pub(crate) struct LoadSignals<'w> {
    launch: Option<Res<'w, frame::StartupPersona>>,
    stopped: Option<Res<'w, SessionLoadCancel>>,
    accepted: Option<Res<'w, SessionLoadApproved>>,
}

/// What a compose is for: the load that asked, its zone and where the games are.
struct LoadTarget {
    load: frame::OwnLevelLoadTicket,
    zone: String,
    titles_install_dir: std::path::PathBuf,
}

impl LoadSignals<'_> {
    /// The load worth composing for: named, with games to read, not aborted.
    fn target(&self) -> Option<LoadTarget> {
        let (accepted, identity) = self.accepted.as_deref().zip(self.launch.as_deref())?;
        let aborted = self
            .stopped
            .as_deref()
            .is_some_and(|abort| abort.0 == accepted.ticket);
        let unnamed = accepted.zone.is_empty() || identity.titles_install_dir.as_os_str().is_empty();
        if aborted || unnamed {
            return None;
        }
        Some(LoadTarget {
            load: accepted.decrypt_secret,
            zone: accepted.zone.clone(),
            titles_install_dir: identity.titles_install_dir.clone(),
        })
    }
}

impl SoundBankCompose {
    fn open(load: frame::OwnLevelLoadTicket, map_zone: String, archives: Archives) -> Self {
        Self {
            zone: map_zone,
            since: Instant::now(),
            load,
            archives,
            composing: None,
            product_stamp: 0,
            common_stamp: 0,
            stall_logged: false,
        }
    }

    /// True once the session has aborted this load or accepted another one.
    fn session_moved_on(&self, signals: &LoadSignals) -> bool {
        let aborted = signals
            .stopped
            .as_deref()
            .is_some_and(|abort| abort.0 == self.load.own_level_ticket_no);
        let replaced = signals
            .accepted
            .as_deref()
            .is_some_and(|next| next.decrypt_secret != self.load);
        aborted || replaced
    }

    /// The bank installs on the load key. Waiting on the launch identity's
    /// spelling of the zone could hold SoundSystemLive and the world spawn forever,
    /// so a different spelling only gets a note.
    fn note_display_zone(&self, shown: Option<&frame::StartupPersona>) {
        let Some(shown) = shown.map(|identity| identity.zone.as_str()) else {
            return;
        };
        if shown != self.zone {
            diag::warn!(
                Audio,
                "audio: the launch identity names `{shown}` but the accepted load walked `{}`; installing on the load key",
                self.zone
            );
        }
    }

    /// Takes the opened archive index off its task once it is done.
    fn absorb_archives(&mut self) {
        let Archives::Opening(task) = &mut self.archives else {
            return;
        };
        let Some((index, notes)) = poll_now(task) else {
            return;
        };
        for note in &notes {
            diag::info!(Audio, "{}", note);
        }
        self.archives = Archives::Ready(Arc::new(index));
    }

    /// The map's sound has arrived from the loader: compose from it, or reuse
    /// the resident product, on a background task.
    fn begin_composing(
        &mut self,
        arrival: &mut PreparedMatchSound,
        resident: &ResidentSoundBank,
        games: Option<GameInstallDir>,
    ) {
        self.common_stamp = arrival.common_profile_id;
        self.product_stamp = arrival.products_id;
        let map = std::mem::replace(&mut arrival.sound, Err(String::new()));
        let reusable = resident.reusable(&self.zone, self.product_stamp, self.common_stamp);
        let zone = self.zone.clone();
        self.composing = Some(compute_pool().spawn(async move {
            match reusable {
                Some(product) => BankOutcome {
                    product: Ok(product),
                    reused: true,
                },
                None => BankOutcome {
                    product: compose_zone(games, &zone, map),
                    reused: false,
                },
            }
        }));
    }

    /// The archive index and the outcome, when both are done.
    fn finished(&mut self) -> Option<(Arc<CategorySoundArchive>, BankOutcome)> {
        let Archives::Ready(pick_no) = &self.archives else {
            return None;
        };
        let pick_no = Arc::clone(pick_no);
        let outcome = poll_now(self.composing.as_mut()?)?;
        Some((pick_no, outcome))
    }

    /// One warning if composing drags on.
    fn report_stall(&mut self) {
        let waited = self.since.elapsed();
        if self.stall_logged || waited < STALL_AFTER {
            return;
        }
        self.stall_logged = true;
        let archives = match self.archives {
            Archives::Ready(_) => "open",
            Archives::Opening(_) => "opening",
        };
        let map_sound = if self.composing.is_some() { "arrived" } else { "pending" };
        diag::warn!(
            Audio,
            "audio: `{}` has no sound bank after {:.0}s (archives {archives}, map sound {map_sound}); AudioReady, the world spawn and host admission all wait on it",
            self.zone,
            waited.as_secs_f32(),
        );
    }

    /// What the resident cache keeps from this compose.
    fn residence(&self, archives: &Arc<CategorySoundArchive>, product: &Product) -> ResidentBank {
        ResidentBank {
            archives: Arc::clone(archives),
            zone: self.zone.clone(),
            composed: Some(ResidentComposed {
                product_stamp: self.product_stamp,
                common_stamp: self.common_stamp,
                product: product.clone(),
            }),
        }
    }
}

/// The loose-file sound index for `zone`: the resident one if it is for the same zone,
/// else one opened on a background task.
fn open_archives(kept: &ResidentSoundBank, zone: &str, titles_install_dir: &Path) -> Archives {
    if let Some(resident) = kept.0.as_ref().filter(|resident| resident.zone == zone) {
        diag::info!(Audio, "sound: `{zone}` archives already open, kept");
        return Archives::Ready(Arc::clone(&resident.archives));
    }
    let games = GameInstallDir(titles_install_dir.to_path_buf());
    let zone = zone.to_owned();
    Archives::Opening(compute_pool().spawn(async move { index_sound_archives(&games, &zone) }))
}

fn index_sound_archives(games: &GameInstallDir, zone: &str) -> (CategorySoundArchive, Vec<String>) {
    let mut trees = CategoryTreeSet::discover(games);
    if let Ok(found) = locate_zone_file(games, zone) {
        trees.take_zone_root(&found.path);
    }
    CategorySoundArchive::open(&trees)
}

/// Composes the bank for `zone` from the map's sound sources.
fn compose_zone(
    games: Option<GameInstallDir>,
    zone: &str,
    map: Result<AudioRoster, String>,
) -> Result<Product, String> {
    let games = games.ok_or("no launch identity to find the zones by")?;
    let namespace = zone_sound_namespace(&games, zone);
    let zone_path = locate_zone_file(&games, zone)?;
    let sources = gather_sound_sources(&games, &zone_path.path);
    let InMemoryAudioBank { catalog, gaps, .. } = compose_sound_bank(sources, zone, namespace, map);
    Ok(Product {
        catalog: Arc::new(catalog),
        namespace,
        gaps: gaps.len(),
    })
}

pub(crate) fn start_sound_bank_compose(
    mut tried: ResMut<BankLoadTried>,
    mut commands: Commands,
    signals: LoadSignals,
    kept: Res<ResidentSoundBank>,
    silent: Option<Res<crate::AudioSilent>>,
) {
    if tried.0 {
        return;
    }
    if silent.is_some() && !crate::AudioSilent::composes() {
        tried.0 = true;
        return;
    }
    let Some(LoadTarget {
        load,
        zone,
        titles_install_dir,
    }) = signals.target()
    else {
        return;
    };
    tried.0 = true;
    let archives = open_archives(&kept, &zone, &titles_install_dir);
    commands.insert_resource(SoundBankCompose::open(load, zone, archives));
}

/// bo2zm: with the sound off (`BO2MP_SOUND=off`, every lane's test game) no
/// sound bank is built, but a Black Ops II map's placed effects (fires,
/// smoke, sparks: its createfx list, read with the sound data) still start,
/// straight from the loaded map's sound data. Nothing plays.
pub(crate) fn boot_silent_map_effects(
    silent: Option<Res<crate::AudioSilent>>,
    map_sound: Option<Res<assets::PreparedMatchSound>>,
    mut done: Local<Option<frame::OwnLevelLoadTicket>>,
    mut commands: Commands,
) {
    let (Some(_), Some(map_sound)) = (silent, map_sound) else {
        return;
    };
    if *done == Some(map_sound.decrypt_secret) {
        return;
    }
    *done = Some(map_sound.decrypt_secret);
    let placed = map_sound.sound.as_ref().ok().and_then(|bank| bank.scripted_map_fx.as_ref());
    let Some(fx) = placed else {
        return;
    };
    diag::info!(
        Audio,
        "audio: Silent; createfx oneshots {} for {} (placed effects only)",
        fx.oneshots.len(),
        map_sound.zone
    );
    commands.insert_resource(asset_audio::CreateFxOneshotEmitters(fx.oneshots.clone()));
}

/// Everything installing a composed bank touches.
#[derive(SystemParam)]
pub(crate) struct InstallRun<'w, 's> {
    job: Option<ResMut<'w, SoundBankCompose>>,
    signals: LoadSignals<'w>,
    arrival: Option<ResMut<'w, PreparedMatchSound>>,
    era: ResMut<'w, GameRoundSerial>,
    clip_cache: Res<'w, crate::clip_bank::ResidentClipCache>,
    resident: ResMut<'w, ResidentSoundBank>,
    commands: Commands<'w, 's>,
}

/// Puts a fresh clip store in place of the one in the world.
fn swap_clip_store(commands: &mut Commands, fresh: crate::ClipRepository) {
    commands.queue(move |world: &mut World| {
        frame::retire::retire_resources(world, |batch| {
            batch.resource::<crate::ClipRepository>();
        });
        world.insert_resource(fresh);
    });
}

pub(crate) fn install_sound_bank(run: InstallRun) {
    let InstallRun {
        job,
        signals,
        mut arrival,
        mut era,
        clip_cache,
        mut resident,
        commands: mut cmds,
    } = run;
    let Some(mut job) = job else {
        return;
    };

    if job.session_moved_on(&signals) {
        diag::info!(
            Audio,
            "audio: the session moved on, so the sound bank for `{}` is dropped",
            job.zone
        );
        cmds.remove_resource::<SoundBankCompose>();
        return;
    }
    job.note_display_zone(signals.launch.as_deref());
    job.absorb_archives();

    let arrived = arrival
        .as_mut()
        .filter(|sound| job.composing.is_none() && sound.decrypt_secret == job.load);
    if let Some(sound) = arrived {
        let games = signals
            .launch
            .as_deref()
            .map(|launch| GameInstallDir(launch.titles_install_dir.clone()));
        job.begin_composing(sound, &resident, games);
        cmds.remove_resource::<PreparedMatchSound>();
    }

    let Some((archive, outcome)) = job.finished() else {
        job.report_stall();
        return;
    };
    cmds.remove_resource::<SoundBankCompose>();
    let BankOutcome { product, reused } = outcome;
    let product = match product {
        Ok(product) => product,
        Err(error) => {
            diag::warn!(Audio, "sound: {} has no bank ({error})", job.zone);
            return;
        }
    };

    era.advance();
    if archive.is_empty() {
        diag::warn!(
            Audio,
            "audio: `{}` has no IWD sound archives, so streamed aliases will gap",
            job.zone
        );
    }
    resident.0 = Some(job.residence(&archive, &product));

    let clips = crate::ClipRepository::start_with_common(
        Arc::clone(&product.catalog),
        Some(Arc::clone(&archive)),
        job.common_stamp,
        Some(clip_cache.clone()),
    );
    swap_clip_store(&mut cmds, clips);
    cmds.insert_resource(SoundArchive(archive));
    cmds.insert_resource(crate::clip_bank::QueuedVoiceStarts::default());
    cmds.insert_resource(CommonPlaybackStore::default());
    cmds.insert_resource(AudioCatalogRes(Arc::clone(&product.catalog)));
    diag::info!(
        Audio,
        "audio: {} sound bank for {} in {:.0}ms, {} zone gaps",
        if reused { "reused" } else { "composed" },
        job.zone,
        job.since.elapsed().as_secs_f32() * 1000.0,
        product.gaps
    );
    let installed_for = CatalogNameSpace {
        zone: job.zone.clone(),
        namespace: product.namespace,
    };
    cmds.insert_resource(installed_for);
}

// ---------------------------------------------------------------------------
// Match end
// ---------------------------------------------------------------------------

pub fn end_map_ambience(commands: &mut Commands, marked: &Query<Entity, With<LevelBackdropSound>>) {
    marked
        .iter()
        .for_each(|entity| commands.entity(entity).despawn());
}

/// The two ways a match can end, read from their message queues.
#[derive(SystemParam)]
pub(crate) struct MatchEnds<'w, 's> {
    torn: MessageReader<'w, 's, GameSessionClosed>,
    back: MessageReader<'w, 's, ReturnedToMenu>,
}

/// Which worlds went down this frame and whether the player left for the menu.
struct Ended {
    retired: Vec<frame::LevelGeneration>,
    to_menu: bool,
}

impl MatchEnds<'_, '_> {
    fn collect(&mut self) -> Option<Ended> {
        let retired: Vec<_> = self.torn.read().map(|fact| fact.level_generation).collect();
        let to_menu = self.back.read().count() > 0;
        (to_menu || !retired.is_empty()).then_some(Ended { retired, to_menu })
    }
}

impl Ended {
    /// A bank already composing for the map that loads next stays.
    fn spares(&self, job: &SoundBankCompose, next: &SessionLoadApproved) -> bool {
        let world = frame::LevelGeneration::out_of_setup_dir(job.load.own_level_ticket_no);
        !self.to_menu && job.load == next.decrypt_secret && !self.retired.contains(&world)
    }
}

/// The bookkeeping that follows a map from load to load.
#[derive(SystemParam)]
pub(crate) struct AmbienceState<'w> {
    epoch: ResMut<'w, GameRoundSerial>,
    booted: ResMut<'w, AmbientBedStarted>,
    attempted: ResMut<'w, BankLoadTried>,
}

#[derive(SystemParam)]
pub(crate) struct PendingCompose<'w> {
    job: Option<Res<'w, SoundBankCompose>>,
    next: Option<Res<'w, SessionLoadApproved>>,
}

/// Hands the map's sound resources to the retire queue.
fn retire_map_sound(world: &mut World) {
    frame::retire::retire_resources(world, |batch| {
        batch
            .resource::<CatalogNameSpace>()
            .resource::<AudioCatalogRes>()
            .resource::<SoundArchive>()
            .resource::<crate::ClipRepository>();
    });
}

pub(crate) fn stop_map_ambient_on_match_end(
    mut ends: MatchEnds,
    mut state: AmbienceState,
    pending: PendingCompose,
    marked: Query<Entity, With<LevelBackdropSound>>,
    mut commands: Commands,
) {
    let Some(ended) = ends.collect() else {
        return;
    };
    state.epoch.advance();
    end_map_ambience(&mut commands, &marked);
    state.booted.0 = false;
    commands.remove_resource::<asset_audio::CreateFxOneshotEmitters>();

    let spared = pending
        .job
        .as_deref()
        .zip(pending.next.as_deref())
        .is_some_and(|(job, next)| ended.spares(job, next));
    if !spared {
        commands.remove_resource::<SoundBankCompose>();
        commands.remove_resource::<PreparedMatchSound>();
        state.attempted.0 = false;
    }
    commands.queue(retire_map_sound);
    perf::ambient_hold(i64::from(state.booted.0));
    diag::info!(Audio, "sound: map ambience off");
}

// ---------------------------------------------------------------------------
// Starting the map's ambience
// ---------------------------------------------------------------------------

/// Who is asking: the map, its game, the catalog and the decoded clips.
struct MapContext<'a> {
    bank: &'a AudioRoster,
    cue_ns: AssetCategoryScope,
    zone: &'a str,
    clips: Option<&'a crate::ClipRepository>,
    epoch: u64,
}

impl<'a> MapContext<'a> {
    /// The alias row a map sound plays by: the first variant of the sound the
    /// map's game has, else of the one sound that name belongs to anywhere.
    fn lead_alias(&self, name: &str) -> Option<&'a SnapshotSoundAlias> {
        let sound = match self.bank.entry_in(self.cue_ns, name) {
            Some(own) => own,
            None => self.bank.sounds.get(self.bank.slot_unique(name)?)?,
        };
        sound.aliases.first()
    }

    /// The first decoded clip any of the alias's keys has, if one is ready.
    fn clip_for(&self, name: &str) -> Option<ClipPcm> {
        let store = self.clips?;
        waveforms_for_cue(self.bank, self.cue_ns, name)
            .into_iter()
            .find_map(|key| store.ready_pcm(&key)?.ok())
    }
}

/// How a loop emitter fades with distance.
struct Falloff {
    near: f32,
    far: f32,
    curve: Arc<[[f32; 2]]>,
    gain: f32,
}

impl Falloff {
    fn for_row(
        row: Option<&SnapshotSoundAlias>,
        name: &str,
        shared: &mut CommonPlaybackStore,
    ) -> Self {
        let curve = row
            .and_then(|row| row.volume_falloff.as_ref())
            .map_or_else(Arc::<[[f32; 2]]>::default, |named| {
                shared.curve_knots(&named.name, &named.knots)
            });
        if row.is_some() && curve.is_empty() {
            diag::warn!(
                Audio,
                "audio: CreateFX loop `{name}` has no falloff curve (typed gap)"
            );
        }
        Self {
            near: row.map_or(0.0, |row| row.reach_lower),
            far: row.map_or(0.0, |row| row.reach_upper),
            curve,
            gain: row.map_or(0.0, |row| row.volume_low.max(0.0)),
        }
    }

    /// The emitter carrying this falloff; the caller fills in the rest.
    fn emitter(self) -> LevelSoundSource {
        let Self {
            near,
            far,
            curve,
            gain,
        } = self;
        LevelSoundSource {
            knots: curve,
            rest_gain: gain,
            reach_lower: near,
            reach_upper: far,
            ..LevelSoundSource::default()
        }
    }
}

/// What starting the CreateFX loop emitters came to.
#[derive(Default)]
struct LoopTally {
    wanted: usize,
    spawned: usize,
    unresolved: usize,
    distinct: usize,
}

impl LoopTally {
    fn report(&self, zone_name: &str) {
        if self.spawned > 0 {
            diag::info!(
                Audio,
                "audio: {zone_name} runs {}/{} createfx loops ({} clips, at most {VOICE_BUDGET} voiced, {} unresolved)",
                self.spawned,
                self.wanted,
                self.distinct,
                self.unresolved
            );
        } else if self.wanted > 0 {
            diag::info!(
                Audio,
                "audio: {zone_name} lists {} createfx loops and none has decoded PCM",
                self.wanted
            );
        }
    }
}

/// Where the map's ambience is created.
#[derive(SystemParam)]
pub(crate) struct BootOutputs<'w, 's> {
    commands: Commands<'w, 's>,
    pcm: ResMut<'w, Assets<ClipPcm>>,
    looping: ResMut<'w, Assets<LoopedPcm>>,
    shared: ResMut<'w, CommonPlaybackStore>,
    gaps: ResMut<'w, UnknownCueHoles>,
}

impl BootOutputs<'_, '_> {
    /// The room-tone alias row, for a Black Ops II map only.
    fn room_row<'a>(ctx: &MapContext<'a>, cue_id: &str) -> Option<&'a SnapshotSoundAlias> {
        if ctx.cue_ns != AssetCategoryScope::T6 {
            return None;
        }
        ctx.bank
            .entry_in(ctx.cue_ns, cue_id)
            .and_then(|sound| sound.aliases.first())
    }

    /// Starts the room-tone loop the map script named (if any).
    fn room_tone(&mut self, ctx: &MapContext, cue_name: Option<&str>) {
        let Some(alias) = cue_name else {
            diag::warn!(
                Audio,
                "audio: the map script of `{}` has no ambientPlay (typed gap)",
                ctx.zone
            );
            return;
        };
        let Some(clip) = ctx.clip_for(alias) else {
            self.gaps.note_miss(alias);
            diag::warn!(
                Audio,
                "audio: no clip for the ambient alias `{alias}` of {}",
                ctx.zone
            );
            return;
        };

        // The room tone plays at its alias's volume, through its pan, in its
        // menu category.
        let row = Self::room_row(ctx, alias);
        let volume = row.map_or(0.55, |row| row.volume_low.max(0.0));
        let clip = with_stereo_pan(clip, row.and_then(|row| row.stereo_pan));
        let handle = self.looping.add(clip.looped());
        let scope = SoundAudience::Match;
        let entity = begin_repeating_cue(&mut self.commands, handle, Volume::Linear(volume), ctx.epoch, scope);
        let mut room = self.commands.entity(entity);
        room.insert(LevelBackdropSound);
        crate::duck::tag(&mut room, row, 0);
        if let Some(category) = row.and_then(|row| row.menu_category) {
            room.insert(crate::MenuVolume {
                base: volume,
                cat_ix: category,
            });
        }
        diag::info!(Audio, "sound: looping room tone `{alias}`");
    }

    /// One entity per CreateFX loop emitter, sharing a decoded clip per alias.
    fn loop_emitters(&mut self, ctx: &MapContext) -> LoopTally {
        let spots = ctx.bank.createfx_loop_sounds(ctx.cue_ns, ctx.zone);
        let mut tally = LoopTally {
            wanted: spots.len(),
            ..LoopTally::default()
        };
        // Every alias is decoded once; a miss is remembered as well.
        let mut decoded: HashMap<&str, Option<Handle<ClipPcm>>> = HashMap::new();
        for spot in &spots {
            let name = spot.cue_alias.as_str();
            let handle = match decoded.get(name) {
                Some(known) => known.clone(),
                None => {
                    let made = ctx.clip_for(name).map(|clip| self.pcm.add(clip));
                    if made.is_none() {
                        self.gaps.note_miss(name);
                    }
                    decoded.insert(name, made.clone());
                    made
                }
            };
            let Some(pcm) = handle else {
                tally.unresolved += 1;
                continue;
            };

            let row = ctx.lead_alias(name);
            let emitter = LevelSoundSource {
                position_in_inches: spot.position_in_inches,
                pcm,
                fixed_pan: row.and_then(|row| row.stereo_pan),
                category: row.and_then(|row| row.menu_category),
                line_end_inches: spot.line_end_inches,
                ..Falloff::for_row(row, name, &mut self.shared).emitter()
            };
            let at = Transform::from_translation(Vec3::from_array(spot.position_in_inches));
            let mut spawned = self.commands.spawn((LevelBackdropSound, emitter, at));
            crate::duck::tag(&mut spawned, row, 0);
            tally.spawned += 1;
        }
        tally.distinct = decoded.values().flatten().count();
        tally
    }

    /// Room tone, loop emitters and the one-shot emitters the host marks.
    fn start(&mut self, ctx: &MapContext, background_sound_entry: Option<&str>) {
        self.room_tone(ctx, background_sound_entry);
        self.loop_emitters(ctx).report(ctx.zone);

        let oneshots = ctx.bank.createfx_oneshots(ctx.cue_ns, ctx.zone);
        let count = oneshots.len();
        let marked = asset_audio::CreateFxOneshotEmitters(oneshots);
        self.commands.insert_resource(marked);
        if count > 0 {
            diag::info!(
                Audio,
                "audio: {} createfx oneshots are marked for the host in {}",
                count,
                ctx.zone
            );
        }
    }
}

/// What has to hold before the ambience may start.
#[derive(SystemParam)]
pub(crate) struct BootGate<'w> {
    booted: ResMut<'w, AmbientBedStarted>,
    ready: Res<'w, crate::SoundSystemLive>,
    loading: Option<Res<'w, assets::LoadBackdrop>>,
}

/// What the ambience is made from.
#[derive(SystemParam)]
pub(crate) struct MapSources<'w> {
    bank: Option<Res<'w, AudioCatalogRes>>,
    launch: Option<Res<'w, frame::StartupPersona>>,
    namespace: Option<Res<'w, CatalogNameSpace>>,
    script: Option<Res<'w, asset_audio::SessionMapScriptSound>>,
    clips: Option<Res<'w, crate::ClipRepository>>,
    epoch: Res<'w, GameRoundSerial>,
}

pub(crate) fn boot_ambient(mut gate: BootGate, from: MapSources, mut out: BootOutputs) {
    let loading = gate.loading.as_deref().is_some_and(|screen| !screen.is_done());
    if gate.booted.0 || !gate.ready.0 || loading {
        return;
    }
    let (Some(snd_bank), Some(launch)) = (from.bank.as_deref(), from.launch.as_deref()) else {
        return;
    };
    let Some(namespace) = from
        .namespace
        .as_deref()
        .filter(|named| !launch.zone.is_empty() && named.zone == launch.zone)
    else {
        return;
    };

    let background_sound_entry = from
        .script
        .as_deref()
        .and_then(|script| script.0.background_sound_entry.as_deref());
    let ctx = MapContext {
        bank: snd_bank.0.as_ref(),
        cue_ns: namespace.namespace,
        zone: &launch.zone,
        clips: from.clips.as_deref(),
        epoch: from.epoch.0,
    };
    out.start(&ctx, background_sound_entry);
    gate.booted.0 = true;
    perf::ambient_boot(&launch.zone, background_sound_entry);
    diag::info!(Audio, "sound: ambience running for {}", launch.zone);
}

// ---------------------------------------------------------------------------
// Emitter levels, every frame
// ---------------------------------------------------------------------------

/// What an emitter contributes at the ear: its base gain scaled by the
/// falloff curve at that distance (nothing without a curve, or past it).
pub(crate) fn emitter_gain(emitter: &LevelSoundSource, ear: [f32; 3]) -> f32 {
    let curve = &emitter.knots;
    if curve.is_empty() {
        return 0.0;
    }
    let apart = gap_in_inches(ear, emitter.heard_from(ear));
    let fade = attenuate(curve, apart, emitter.reach_lower, emitter.reach_upper);
    if fade < 0.0 {
        0.0
    } else {
        emitter.rest_gain * fade
    }
}

/// One emitter's standing in this frame's ranking.
pub struct Ranked {
    gain: f32,
    owner_ent: Entity,
    voiced: bool,
    spot: [f32; 3],
}

/// What this frame does with a ranked emitter's voice.
#[derive(Debug, PartialEq)]
enum ChairSlot {
    Idle,
    Grant,
    Retune,
    Release,
}

/// Only the loudest few that are loud enough hold voices.
fn seat_for(rank: usize, gain: f32, voiced: bool) -> ChairSlot {
    let earned = rank < VOICE_BUDGET && gain >= GAIN_FLOOR;
    match (earned, voiced) {
        (true, true) => ChairSlot::Retune,
        (true, false) => ChairSlot::Grant,
        (false, true) => ChairSlot::Release,
        (false, false) => ChairSlot::Idle,
    }
}

fn loudest_first(ranking: &mut [Ranked]) {
    ranking.sort_by(|a, b| {
        b.gain
            .partial_cmp(&a.gain)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

/// A copy of `dry` that can be panned while it plays, set to the given gains.
fn live_panned(dry: ClipPcm, left: f32, right: f32) -> (ClipPcm, PanHandle) {
    let copy = dry.with_pan_handle();
    let Some(live) = copy.pan_handle().cloned() else {
        unreachable!("with_pan_handle always attaches a pan");
    };
    live.store(left, right);
    (copy, live)
}

/// The listener's pose.
struct Ear {
    at: Vec3,
    right: Vec3,
}

impl Ear {
    fn of(pose: &Transform) -> Self {
        Self {
            at: pose.translation,
            right: pose.rotation * Vec3::X,
        }
    }

    /// Left and right gains for a sound at `origin`.
    fn pans(&self, spot: [f32; 3]) -> (f32, f32) {
        oneshot_lr_gains(self.at, self.right, Vec3::from_array(spot), 1.0)
    }
}

/// The settings and ducking that scale every emitter.
#[derive(SystemParam)]
struct Levels<'w, 's> {
    prefs: Res<'w, frame::PlayerPrefsBundle>,
    ducks: Res<'w, crate::DuckMixer>,
    groups: Query<'w, 's, &'static crate::DuckGroup>,
}

impl Levels<'_, '_> {
    /// The menu slider of the emitter's category, times its current duck.
    fn frontend_loudness(&self, ent: Entity, category: Option<u8>) -> f32 {
        let duck = self.ducks.linear(self.groups.get(ent).ok().map(|group| group.0));
        self.prefs.menu_volume(category) * duck
    }
}

/// Everything the emitter levels system reads and changes.
#[derive(SystemParam)]
pub struct EmitterWorld<'w, 's> {
    listeners: Query<'w, 's, &'static Transform, With<BackgroundHearer>>,
    emitters: Query<'w, 's, (Entity, &'static mut LevelSoundSource, Has<PlayingCue>)>,
    sinks: Query<'w, 's, &'static mut AudioSink, With<LevelSoundSource>>,
    pcm: Res<'w, Assets<ClipPcm>>,
    looping: ResMut<'w, Assets<LoopedPcm>>,
    levels: Levels<'w, 's>,
    epoch: Res<'w, GameRoundSerial>,
    commands: Commands<'w, 's>,
}

impl EmitterWorld<'_, '_> {
    /// The one listener there is; more than one is a bug.
    fn ear(&self) -> Option<Ear> {
        let mut poses = self.listeners.iter();
        match (poses.next(), poses.next()) {
            (None, _) => None,
            (Some(only), None) => Some(Ear::of(only)),
            (Some(_), Some(_)) => panic!("more than one ambient listener"),
        }
    }

    /// Takes an emitter's voice away and forgets its live pan.
    fn release(&mut self, ent_id: Entity) {
        if let Ok((_, mut owner, _)) = self.emitters.get_mut(ent_id) {
            owner.active_stereo_handle = None;
        }
        unbind_repeating_cue(&mut self.commands, ent_id);
    }

    /// A voiced emitter follows the ear: pan, level and speed.
    fn retune(&mut self, bay: &Ranked, ear: &Ear) {
        let Ok((_, emitter, _)) = self.emitters.get(bay.owner_ent) else {
            return;
        };
        if let (None, Some(pan)) = (emitter.fixed_pan, &emitter.active_stereo_handle) {
            let (left, right) = ear.pans(bay.spot);
            pan.store(left, right);
        }
        let speed = emitter.speed;
        let menu = self.levels.frontend_loudness(bay.owner_ent, emitter.category);
        let Ok(mut sink) = self.sinks.get_mut(bay.owner_ent) else {
            return;
        };
        let level = bay.gain * self.levels.prefs.master_gain() * menu * crate::bus_gain::sfx();
        sink.set_volume(Volume::Linear(level));
        if (sink.speed() - speed).abs() > 0.001 {
            sink.set_speed(speed);
        }
        sink.is_paused().then(|| sink.play());
    }

    /// Gives an emitter a voice: a copy of its PCM that can be panned live.
    fn grant(&mut self, bay: &Ranked, ear: &Ear) {
        let Ok((_, emitter, _)) = self.emitters.get(bay.owner_ent) else {
            return;
        };
        let (fixed, category) = (emitter.fixed_pan, emitter.category);
        let Some(dry) = self.pcm.get(&emitter.pcm).cloned() else {
            return;
        };
        let menu = self.levels.frontend_loudness(bay.owner_ent, category);
        let (left, right) = match fixed {
            Some(pan) => pan.gains(dry.lane_count()),
            None => ear.pans(bay.spot),
        };
        let (copy, live) = live_panned(dry, left, right);
        let handle = self.looping.add(copy.looped());
        if let Ok((_, mut owner, _)) = self.emitters.get_mut(bay.owner_ent) {
            owner.active_stereo_handle = Some(live);
        }
        bind_repeating_cue(
            &mut self.commands,
            bay.owner_ent,
            handle,
            Volume::Linear(bay.gain * menu),
            self.epoch.0,
        );
    }
}

pub fn map_emitter_gain_tick(mut world: EmitterWorld, mut rank_in: Local<Vec<Ranked>>) {
    let Some(ear) = world.ear() else {
        return;
    };
    let ear_xyz = inches_of(ear.at);

    rank_in.clear();
    rank_in.extend(world.emitters.iter().map(|(entity, emitter, voiced)| Ranked {
        gain: emitter_gain(emitter, ear_xyz),
        owner_ent: entity,
        voiced,
        spot: emitter.heard_from(ear_xyz),
    }));
    loudest_first(&mut rank_in);

    for (rank, slot) in rank_in.iter().enumerate() {
        match seat_for(rank, slot.gain, slot.voiced) {
            ChairSlot::Idle => {}
            ChairSlot::Release => world.release(slot.owner_ent),
            ChairSlot::Retune => world.retune(slot, &ear),
            ChairSlot::Grant => world.grant(slot, &ear),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asset_audio::{
        SnapshotSoundAlias, SnapshotAudioCurve, SnapshotSoundEntry, SpawnEffectAudioLoop, ScriptedMapFx,
    };
    use bevy::ecs::system::RunSystemOnce;

    const RAMP: [[f32; 2]; 2] = [[0.0, 1.0], [1.0, 0.0]];

    /// An emitter at the origin with a 100..300 inch range.
    fn ranged(curve: &[[f32; 2]], rest_gain: f32) -> LevelSoundSource {
        LevelSoundSource {
            reach_lower: 100.0,
            reach_upper: 300.0,
            knots: curve.into(),
            rest_gain,
            ..LevelSoundSource::default()
        }
    }

    fn gain_at(emitter: &LevelSoundSource, x: f32) -> f32 {
        emitter_gain(emitter, [x, 0.0, 0.0])
    }

    #[test]
    fn a_default_emitter_is_silent_and_plays_at_its_recorded_speed() {
        let blank = LevelSoundSource::default();
        assert_eq!(blank.speed, 1.0);
        assert!(blank.knots.is_empty() && blank.active_stereo_handle.is_none() && blank.category.is_none());
        assert_eq!(gain_at(&blank, 5.0), 0.0);
        assert_eq!(ranged(&[], 0.9).position_in_inches, [0.0; 3]);
    }

    #[test]
    fn an_emitter_without_a_curve_is_silent() {
        assert_eq!(gain_at(&ranged(&[], 0.9), 10.0), 0.0);
    }

    #[test]
    fn an_emitter_is_full_inside_its_near_distance_and_silent_past_its_far_one() {
        let ramp = ranged(&RAMP, 0.5);
        let heard: Vec<f32> = [100.0, 300.0, 900.0].map(|x| gain_at(&ramp, x)).to_vec();
        assert_eq!(heard, [0.5, 0.0, 0.0]);
    }

    #[test]
    fn an_emitter_fades_between_near_and_far() {
        let ramp = ranged(&RAMP, 0.8);
        let [near, mid, far] = [120.0, 200.0, 280.0].map(|x| gain_at(&ramp, x));
        assert!(near > mid && mid > far && far > 0.0, "{near} {mid} {far}");
        assert!((mid - 0.4).abs() < 1e-3, "{mid}");
    }

    #[test]
    fn the_base_gain_scales_the_whole_curve() {
        let soft = gain_at(&ranged(&RAMP, 0.25), 180.0);
        let full = gain_at(&ranged(&RAMP, 1.0), 180.0);
        assert!((full - 4.0 * soft).abs() < 1e-5);
    }

    #[test]
    fn a_loop_with_no_row_has_no_falloff() {
        let mut shared = CommonPlaybackStore::default();
        let bare = Falloff::for_row(None, "amb_x", &mut shared);
        assert_eq!([bare.near, bare.far, bare.gain], [0.0; 3]);
        assert!(bare.curve.is_empty());
    }

    #[test]
    fn a_falloff_becomes_an_emitter_with_the_rest_left_blank() {
        let falloff = Falloff {
            near: 3.0,
            far: 9.0,
            curve: Arc::<[[f32; 2]]>::from(vec![[0.0, 1.0]]),
            gain: 0.6,
        };
        let made = falloff.emitter();
        assert_eq!((made.reach_lower, made.reach_upper, made.rest_gain), (3.0, 9.0, 0.6));
        assert_eq!(&*made.knots, &[[0.0, 1.0]]);
        assert_eq!(made.speed, 1.0);
        assert!(made.fixed_pan.is_none() && made.category.is_none());
    }

    #[test]
    fn the_tally_reports_every_outcome_without_panicking() {
        let started = LoopTally {
            wanted: 3,
            spawned: 2,
            unresolved: 1,
            distinct: 2,
        };
        started.report("mp_test");
        LoopTally {
            wanted: 4,
            ..LoopTally::default()
        }
        .report("mp_test");
        LoopTally::default().report("mp_test");
    }

    #[test]
    fn a_live_panned_copy_starts_at_the_requested_gains() {
        let dry = ClipPcm::from_parts(Arc::from(vec![0.1, 0.2, 0.3, 0.4]), 2, 44_100)
            .expect("stereo clip");
        let (copy, live) = live_panned(dry, 0.25, 0.75);
        assert_eq!(live.load(), (0.25, 0.75));
        let seen_by_copy = |copy: &ClipPcm| copy.pan_handle().map(|pan| pan.load());
        assert_eq!(seen_by_copy(&copy), Some((0.25, 0.75)));
        live.store(1.0, 0.0);
        assert_eq!(seen_by_copy(&copy), Some((1.0, 0.0)));
    }

    #[test]
    fn the_scatter_sequence_is_stable_and_never_zero() {
        let run = |start: u32| {
            let mut seed = start;
            (0..4).map(|_| scatter_step(&mut seed)).collect::<Vec<_>>()
        };
        let (first, second) = (run(0x1234_5678), run(0x1234_5678));
        assert_eq!(first, second);
        assert!(first.iter().all(|&n| n != 0));
        assert_ne!(first, run(0x1234_5679));
    }

    #[test]
    fn the_scatter_generator_keeps_the_exact_xorshift_steps() {
        let mut seed = 1;
        assert_eq!(scatter_step(&mut seed), 270_369);
        assert_eq!(seed, 270_369);
        assert_eq!(scatter_step(&mut seed), 67_634_689);
    }

    fn thunder(wait_secs: [f32; 2], places_inches: Vec<[f32; 3]>) -> RandomPointSound {
        RandomPointSound {
            cue_alias: "thunder".into(),
            wait_secs,
            places_inches,
        }
    }

    #[test]
    fn a_wait_stays_inside_its_range() {
        let ranged_sound = thunder([10.0, 14.0], vec![]);
        let mut seed = 7;
        for _ in 0..50 {
            let waited = wait_between(&ranged_sound, &mut seed);
            assert!((10.0..14.0).contains(&waited), "{waited}");
        }
        assert_eq!(wait_between(&thunder([5.0, 5.0], vec![]), &mut seed), 5.0);
    }

    #[test]
    fn a_wait_range_below_zero_counts_from_zero() {
        let mut seed = 9;
        let clipped = thunder([-4.0, 3.0], vec![]);
        let all_waits: Vec<f64> = (0..20).map(|_| wait_between(&clipped, &mut seed)).collect();
        assert!(all_waits.iter().all(|w| (0.0..3.0).contains(w)), "{all_waits:?}");
    }

    #[test]
    fn a_place_is_picked_by_the_generator_or_not_at_all() {
        let mut seed = 5;
        assert_eq!(pick_place(&[], &mut seed), None);
        assert_eq!(seed, 5, "nothing to pick must not spend a step");
        let spots = [[1.0; 3], [2.0; 3], [3.0; 3]];
        let mut twin = 5;
        let expected = scatter_step(&mut twin) as usize % spots.len();
        assert_eq!(pick_place(&spots, &mut seed), Some(spots[expected]));
        assert_eq!(seed, twin);
    }

    #[test]
    fn a_scatter_clock_covers_only_its_own_epoch_and_sound_count() {
        let mut clock = ScatterClock::default();
        assert!(clock.covers(0, 0));
        let sounds = [thunder([1.0, 1.0], vec![])];
        clock.restart(4, 10.0, &sounds);
        assert!(clock.covers(4, 1));
        assert!(!clock.covers(5, 1) && !clock.covers(4, 2));
        assert_eq!(clock.due, [11.0]);
    }

    // ---- the systems ----

    fn mono_clip() -> ClipPcm {
        ClipPcm::from_parts(Arc::from(vec![0.0f32, 0.5]), 1, 8_000).expect("mono clip")
    }

    fn bank_with_fx(fx: ScriptedMapFx) -> AudioCatalogRes {
        let mut catalog = AudioRoster::default();
        catalog.scripted_map_fx = Some(fx);
        AudioCatalogRes(Arc::new(catalog))
    }

    fn scatter_app(bank: Option<AudioCatalogRes>, booted: bool) -> App {
        let mut app = App::default();
        app.add_message::<crate::AliasCommand>()
            .insert_resource(Time::<()>::default())
            .insert_resource(AmbientBedStarted(booted))
            .insert_resource(GameRoundSerial(1))
            .add_systems(Update, play_random_point_sounds);
        bank.into_iter().for_each(|sounds| {
            app.insert_resource(sounds);
        });
        app
    }

    fn tick_and_drain(app: &mut App, secs: f64) -> Vec<crate::AliasCommand> {
        let mut clock = app.world_mut().resource_mut::<Time>();
        clock.advance_by(Duration::from_secs_f64(secs));
        app.update();
        let mut queue = app.world_mut().resource_mut::<Messages<crate::AliasCommand>>();
        queue.drain().collect()
    }

    fn hourly_thunder(places: Vec<[f32; 3]>) -> AudioCatalogRes {
        bank_with_fx(ScriptedMapFx {
            random_sounds: vec![thunder([1.0, 1.0], places)],
            ..ScriptedMapFx::default()
        })
    }

    #[test]
    fn a_random_point_sound_plays_when_its_wait_is_up_then_waits_again() {
        let spots = vec![[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]];
        let mut app = scatter_app(Some(hourly_thunder(spots.clone())), true);
        assert!(tick_and_drain(&mut app, 0.0).is_empty());
        assert!(tick_and_drain(&mut app, 0.5).is_empty());
        let fired = tick_and_drain(&mut app, 0.5);
        assert_eq!(fired.len(), 1);
        let crate::AliasCommand::Play(play) = &fired[0] else {
            panic!("expected a plain play, got {fired:?}");
        };
        assert_eq!(play.alias, "thunder");
        assert_eq!(play.cue_ns, AssetCategoryScope::T6);
        assert!(play.fallback.is_none() && play.source_entity_no.is_none());
        assert!(spots.contains(&play.position_in_inches.expect("placed")));
        assert!(tick_and_drain(&mut app, 0.5).is_empty());
        assert_eq!(tick_and_drain(&mut app, 0.5).len(), 1);
    }

    #[test]
    fn a_new_match_epoch_restarts_the_random_point_waits() {
        let mut app = scatter_app(Some(hourly_thunder(vec![[0.0; 3]])), true);
        for secs in [0.0, 1.0, 1.0] {
            tick_and_drain(&mut app, secs);
        }
        app.world_mut().resource_mut::<GameRoundSerial>().advance();
        assert!(tick_and_drain(&mut app, 0.5).is_empty());
        assert!(tick_and_drain(&mut app, 0.5).is_empty());
        assert_eq!(tick_and_drain(&mut app, 0.5).len(), 1);
    }

    #[test]
    fn random_point_sounds_stay_quiet_without_places_a_boot_or_a_bank() {
        let mut apps = [
            scatter_app(Some(hourly_thunder(vec![])), true),
            scatter_app(Some(hourly_thunder(vec![[0.0; 3]])), false),
            scatter_app(None, true),
        ];
        for app in &mut apps {
            let heard: usize = (0..4).map(|_| tick_and_drain(app, 1.0).len()).sum();
            assert_eq!(heard, 0);
        }
    }

    fn ranking_app() -> App {
        let mut app = App::default();
        app.insert_resource(Assets::<ClipPcm>::default())
            .insert_resource(Assets::<LoopedPcm>::default())
            .insert_resource(frame::PlayerPrefsBundle::default())
            .insert_resource(GameRoundSerial(3))
            .insert_resource(crate::DuckMixer::default())
            .add_systems(Update, map_emitter_gain_tick);
        app
    }

    fn spawn_ear(app: &mut App) -> Entity {
        let pose = Transform::default();
        app.world_mut().spawn((BackgroundHearer, pose)).id()
    }

    fn who_has_a_voice(app: &App, among: &[Entity]) -> Vec<bool> {
        let world = app.world();
        among
            .iter()
            .map(|&entity| world.entity(entity).contains::<PlayingCue>())
            .collect()
    }

    #[test]
    fn only_the_loudest_audible_emitters_hold_voices_and_lose_them_when_quiet() {
        let mut app = ranking_app();
        let ear = spawn_ear(&mut app);
        let clip = app
            .world_mut()
            .resource_mut::<Assets<ClipPcm>>()
            .add(mono_clip());
        let spawn_at = |app: &mut App, x: f32, curve: &[[f32; 2]]| {
            let one = LevelSoundSource {
                position_in_inches: [x, 0.0, 0.0],
                pcm: clip.clone(),
                ..ranged(curve, 1.0)
            };
            app.world_mut().spawn((LevelBackdropSound, one)).id()
        };
        let mut everyone: Vec<Entity> = (0..10)
            .map(|i| spawn_at(&mut app, 110.0 + 20.0 * i as f32, &RAMP))
            .collect();
        everyone.push(spawn_at(&mut app, 100.0, &[]));
        everyone.push(spawn_at(&mut app, 500.0, &RAMP));

        app.update();
        let held = who_has_a_voice(&app, &everyone);
        assert_eq!(&held[..8], &[true; 8], "the eight nearest are voiced");
        assert_eq!(&held[8..], &[false; 4], "the rest are not");
        let pan_of = |app: &App, which: Entity| {
            let world = app.world();
            world.entity(which).get::<LevelSoundSource>().unwrap().active_stereo_handle.is_some()
        };
        assert!(pan_of(&app, everyone[0]));
        assert!(!pan_of(&app, everyone[9]));

        // The ear walks off: every gain falls to nothing and the voices go.
        let far_away = Transform::from_xyz(5_000.0, 0.0, 0.0);
        app.world_mut().entity_mut(ear).insert(far_away);
        app.update();
        assert_eq!(who_has_a_voice(&app, &everyone), vec![false; 12]);
        assert!(!pan_of(&app, everyone[0]));

        // And it comes back: the nearest get theirs again.
        app.world_mut().entity_mut(ear).insert(Transform::default());
        app.update();
        assert_eq!(&who_has_a_voice(&app, &everyone)[..8], &[true; 8]);
    }

    #[test]
    fn a_voiced_emitter_is_retuned_without_being_marked_changed() {
        let mut app = ranking_app();
        spawn_ear(&mut app);
        let clip = app
            .world_mut()
            .resource_mut::<Assets<ClipPcm>>()
            .add(mono_clip());
        let one = LevelSoundSource {
            position_in_inches: [150.0, 0.0, 0.0],
            pcm: clip,
            ..ranged(&RAMP, 1.0)
        };
        let id = app.world_mut().spawn((LevelBackdropSound, one)).id();
        app.update();
        app.update();
        app.world_mut().clear_trackers();
        app.update();
        let changed = app.world().entity(id).get_ref::<LevelSoundSource>().unwrap().is_changed();
        assert!(!changed, "retuning reads the emitter, it does not write it");
    }

    #[test]
    fn a_second_ear_is_refused() {
        let mut app = ranking_app();
        spawn_ear(&mut app);
        app.update();
        spawn_ear(&mut app);
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| app.update()));
        assert!(outcome.is_err());
    }

    #[test]
    fn a_voice_is_earned_by_rank_and_by_level_and_changes_by_whether_one_is_held() {
        let table = [
            (0, 0.5, false, ChairSlot::Grant),
            (0, 0.5, true, ChairSlot::Retune),
            (7, GAIN_FLOOR, false, ChairSlot::Grant),
            (7, GAIN_FLOOR, true, ChairSlot::Retune),
            (8, 0.9, false, ChairSlot::Idle),
            (8, 0.9, true, ChairSlot::Release),
            (0, GAIN_FLOOR * 0.5, false, ChairSlot::Idle),
            (0, 0.0, true, ChairSlot::Release),
            (0, f32::NAN, true, ChairSlot::Release),
            (0, f32::NAN, false, ChairSlot::Idle),
        ];
        for (rank, gain, voiced, expected) in table {
            assert_eq!(seat_for(rank, gain, voiced), expected, "{rank} {gain} {voiced}");
        }
    }

    fn compose_waiting(request: u64, zone: &str) -> SoundBankCompose {
        let load = frame::OwnLevelLoadTicket {
            own_level_ticket_no: request,
            ..frame::OwnLevelLoadTicket::default()
        };
        SoundBankCompose::open(load, zone.to_owned(), Archives::Ready(Arc::default()))
    }

    fn match_end_app() -> App {
        let mut app = App::default();
        app.add_message::<GameSessionClosed>()
            .add_message::<ReturnedToMenu>()
            .insert_resource(AmbientBedStarted(true))
            .insert_resource(BankLoadTried(true))
            .insert_resource(GameRoundSerial(10))
            .insert_resource(AudioCatalogRes(Arc::default()))
            .insert_resource(SoundArchive(Arc::default()))
            .add_systems(Update, stop_map_ambient_on_match_end);
        app
    }

    fn world_down(request: u64) -> GameSessionClosed {
        GameSessionClosed {
            reason: frame::ShutdownCause::RoundOver,
            level_generation: frame::LevelGeneration::out_of_setup_dir(request),
            game_session_key: frame::GameSessionStamp::default(),
            game_generation: 1,
        }
    }

    fn accepted_load(request: u64, zone: &str) -> SessionLoadApproved {
        SessionLoadApproved {
            ticket: request,
            decrypt_secret: frame::OwnLevelLoadTicket {
                own_level_ticket_no: request,
                ..frame::OwnLevelLoadTicket::default()
            },
            zone: zone.into(),
        }
    }

    #[test]
    fn nothing_ends_without_a_teardown_or_a_return_to_the_menu() {
        let mut app = match_end_app();
        let room = app.world_mut().spawn(LevelBackdropSound).id();
        app.update();
        let world = app.world();
        assert!(world.get_entity(room).is_ok());
        assert!(world.resource::<AmbientBedStarted>().0);
        assert_eq!(world.resource::<GameRoundSerial>().0, 10);
        assert!(world.contains_resource::<AudioCatalogRes>());
    }

    #[test]
    fn leaving_for_the_menu_clears_the_ambience_and_every_pending_bank() {
        let mut app = match_end_app();
        let room = app.world_mut().spawn(LevelBackdropSound).id();
        app.insert_resource(compose_waiting(7, "mp_a"));
        app.insert_resource(accepted_load(7, "mp_a"));
        app.world_mut().write_message(ReturnedToMenu {
            swap_id: 1,
            had_world: true,
        });
        app.update();
        let world = app.world();
        assert!(world.get_entity(room).is_err());
        assert!(!world.resource::<AmbientBedStarted>().0);
        assert!(!world.resource::<BankLoadTried>().0);
        assert_eq!(world.resource::<GameRoundSerial>().0, 11);
        assert!(!world.contains_resource::<SoundBankCompose>());
        assert!(!world.contains_resource::<AudioCatalogRes>());
        assert!(!world.contains_resource::<SoundArchive>());
        assert!(!world.contains_resource::<asset_audio::CreateFxOneshotEmitters>());
    }

    #[test]
    fn a_bank_composing_for_the_next_load_survives_a_teardown_of_another_world() {
        let mut app = match_end_app();
        app.insert_resource(compose_waiting(8, "mp_b"));
        app.insert_resource(accepted_load(8, "mp_b"));
        app.world_mut().write_message(world_down(5));
        app.update();
        let world = app.world();
        assert!(world.contains_resource::<SoundBankCompose>());
        assert!(world.resource::<BankLoadTried>().0);
        assert!(!world.resource::<AmbientBedStarted>().0);
        assert!(!world.contains_resource::<AudioCatalogRes>());
    }

    #[test]
    fn a_bank_composing_for_the_world_that_went_down_is_dropped() {
        let cases = [
            ("same world", world_down(8), Some(accepted_load(8, "mp_b"))),
            ("other load key", world_down(5), Some(accepted_load(9, "mp_b"))),
            ("nothing accepted", world_down(5), None),
        ];
        for (label, down, next) in cases {
            let mut app = match_end_app();
            app.insert_resource(compose_waiting(8, "mp_b"));
            if let Some(next) = next {
                app.insert_resource(next);
            }
            app.world_mut().write_message(down);
            app.update();
            let world = app.world();
            assert!(!world.contains_resource::<SoundBankCompose>(), "{label}");
            assert!(!world.resource::<BankLoadTried>().0, "{label}");
        }
    }

    #[test]
    fn a_pending_bank_is_spared_only_when_the_menu_was_not_the_reason() {
        let job = compose_waiting(8, "mp_b");
        let next = accepted_load(8, "mp_b");
        let spare = |retired: Vec<frame::LevelGeneration>, to_menu: bool| {
            Ended { retired, to_menu }.spares(&job, &next)
        };
        let elsewhere = frame::LevelGeneration::out_of_setup_dir(5);
        let ours = frame::LevelGeneration::out_of_setup_dir(8);
        assert!(spare(vec![elsewhere], false));
        assert!(spare(vec![], false));
        assert!(!spare(vec![elsewhere], true));
        assert!(!spare(vec![elsewhere, ours], false));
        assert!(!Ended { retired: vec![], to_menu: false }
            .spares(&job, &accepted_load(9, "mp_b")));
    }

    fn start_app(resident: ResidentSoundBank) -> App {
        let mut app = App::default();
        app.insert_resource(BankLoadTried(false))
            .insert_resource(resident)
            .add_systems(Update, start_sound_bank_compose);
        app
    }

    fn identity_in(zone: &str, root: &str) -> frame::StartupPersona {
        frame::StartupPersona {
            zone: zone.into(),
            titles_install_dir: root.into(),
            ..frame::StartupPersona::default()
        }
    }

    #[test]
    fn a_compose_starts_only_for_a_named_zone_with_a_games_root_and_no_abort() {
        let cases: [(&'static str, Option<SessionLoadApproved>, Option<frame::StartupPersona>, Option<SessionLoadCancel>); 5] = [
            ("no load", None, Some(identity_in("mp_a", "games")), None),
            ("no identity", Some(accepted_load(4, "mp_a")), None, None),
            ("no zone", Some(accepted_load(4, "")), Some(identity_in("", "games")), None),
            ("no root", Some(accepted_load(4, "mp_a")), Some(identity_in("mp_a", "")), None),
            (
                "aborted",
                Some(accepted_load(4, "mp_a")),
                Some(identity_in("mp_a", "games")),
                Some(SessionLoadCancel(4)),
            ),
        ];
        for (label, load, who, abort) in cases {
            let mut app = start_app(ResidentSoundBank::default());
            load.into_iter().for_each(|load| {
                app.insert_resource(load);
            });
            who.into_iter().for_each(|who| {
                app.insert_resource(who);
            });
            abort.into_iter().for_each(|abort| {
                app.insert_resource(abort);
            });
            app.update();
            let world = app.world();
            assert!(!world.contains_resource::<SoundBankCompose>(), "{label}");
            assert!(!world.resource::<BankLoadTried>().0, "{label}");
        }
    }

    #[test]
    fn a_compose_for_the_same_zone_reuses_the_resident_archives() {
        let kept = Arc::new(CategorySoundArchive::default());
        let mut app = start_app(ResidentSoundBank(Some(ResidentBank {
            archives: Arc::clone(&kept),
            zone: "mp_a".into(),
            composed: None,
        })));
        app.insert_resource(accepted_load(4, "mp_a"));
        app.insert_resource(identity_in("mp_a", "games"));
        app.insert_resource(SessionLoadCancel(99));
        app.update();
        assert!(app.world().resource::<BankLoadTried>().0);
        let job = app.world().resource::<SoundBankCompose>();
        assert_eq!(job.zone, "mp_a");
        assert_eq!(job.load.own_level_ticket_no, 4);
        assert!(job.composing.is_none() && !job.stall_logged);
        let Archives::Ready(shared) = &job.archives else {
            panic!("the resident archives are open already");
        };
        assert!(Arc::ptr_eq(shared, &kept));
        // A second pass does nothing: the load has been tried.
        app.world_mut().remove_resource::<SoundBankCompose>();
        app.update();
        assert!(!app.world().contains_resource::<SoundBankCompose>());
    }

    #[test]
    fn a_compose_for_another_zone_opens_its_archives_in_the_background() {
        let mut app = start_app(ResidentSoundBank(Some(ResidentBank {
            archives: Arc::default(),
            zone: "mp_other".into(),
            composed: None,
        })));
        app.insert_resource(accepted_load(4, "mp_a"));
        app.insert_resource(identity_in("mp_a", "no_such_games_root"));
        app.update();
        let job = app.world().resource::<SoundBankCompose>();
        assert!(matches!(job.archives, Archives::Opening(_)));
    }

    fn install_app() -> App {
        let mut app = App::default();
        app.insert_resource(GameRoundSerial(20))
            .insert_resource(crate::clip_bank::ResidentClipCache::default())
            .insert_resource(ResidentSoundBank::default())
            .add_systems(Update, install_sound_bank);
        app
    }

    #[test]
    fn a_compose_the_session_moved_past_is_dropped() {
        let outcomes = [
            (SessionLoadCancel(6), None, false),
            (SessionLoadCancel(0), Some(accepted_load(7, "mp_a")), false),
            (SessionLoadCancel(5), None, true),
        ];
        for (abort, accepted, survives) in outcomes {
            let mut app = install_app();
            app.insert_resource(compose_waiting(6, "mp_a"));
            if abort.0 != 0 {
                app.insert_resource(abort);
            }
            if let Some(accepted) = accepted {
                app.insert_resource(accepted);
            }
            app.update();
            let world = app.world();
            assert_eq!(world.contains_resource::<SoundBankCompose>(), survives);
            if survives {
                assert_eq!(world.resource::<GameRoundSerial>().0, 20);
            }
        }
    }

    fn arrived_sound(request: u64, zone: &str, products: u64, common: u64) -> PreparedMatchSound {
        PreparedMatchSound {
            decrypt_secret: accepted_load(request, zone).decrypt_secret,
            zone: zone.into(),
            common_profile_id: common,
            products_id: products,
            sound: Ok(AudioRoster::default()),
        }
    }

    fn run_until_installed(app: &mut App) {
        for _ in 0..400 {
            app.update();
            if !app.world().contains_resource::<SoundBankCompose>() {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("the compose never finished");
    }

    #[test]
    fn a_waiting_compose_stays_until_the_maps_sound_arrives() {
        let mut app = install_app();
        app.insert_resource(compose_waiting(6, "mp_a"));
        (0..3).for_each(|_| app.update());
        let job = app.world().resource::<SoundBankCompose>();
        assert!(job.composing.is_none() && !job.stall_logged);
        assert!(!app.world().contains_resource::<AudioCatalogRes>());
    }

    #[test]
    fn a_resident_bank_with_the_same_products_is_installed_as_reused() {
        let mut app = install_app();
        let cat_ref = Arc::new(AudioRoster::default());
        let archives = Arc::new(CategorySoundArchive::default());
        app.insert_resource(ResidentSoundBank(Some(ResidentBank {
            archives: Arc::clone(&archives),
            zone: "mp_a".into(),
            composed: Some(ResidentComposed {
                product_stamp: 31,
                common_stamp: 17,
                product: Product {
                    catalog: Arc::clone(&cat_ref),
                    namespace: AssetCategoryScope::T6,
                    gaps: 2,
                },
            }),
        })));
        let mut waiting = compose_waiting(6, "mp_a");
        waiting.archives = Archives::Ready(Arc::clone(&archives));
        app.insert_resource(waiting);
        app.insert_resource(arrived_sound(6, "mp_a", 31, 17));
        run_until_installed(&mut app);
        app.update();

        let world = app.world();
        assert!(Arc::ptr_eq(&world.resource::<AudioCatalogRes>().0, &cat_ref));
        assert!(Arc::ptr_eq(&world.resource::<SoundArchive>().0, &archives));
        let named = world.resource::<CatalogNameSpace>();
        assert_eq!((named.zone.as_str(), named.namespace), ("mp_a", AssetCategoryScope::T6));
        assert_eq!(world.resource::<GameRoundSerial>().0, 21);
        assert!(world.contains_resource::<crate::ClipRepository>());
        assert!(world.contains_resource::<crate::clip_bank::QueuedVoiceStarts>());
        assert!(world.contains_resource::<CommonPlaybackStore>());
        assert!(!world.contains_resource::<PreparedMatchSound>());
        let kept = world.resource::<ResidentSoundBank>().0.as_ref().unwrap();
        assert_eq!(kept.zone, "mp_a");
        let stamps = kept
            .composed
            .as_ref()
            .map(|made| (made.product.gaps, made.product_stamp, made.common_stamp));
        assert_eq!(stamps, Some((2, 31, 17)));
    }

    #[test]
    fn a_bank_that_cannot_find_its_zone_is_not_installed() {
        let mut app = install_app();
        app.insert_resource(compose_waiting(6, "mp_a"));
        app.insert_resource(arrived_sound(6, "mp_a", 31, 17));
        run_until_installed(&mut app);
        let world = app.world();
        assert!(!world.contains_resource::<AudioCatalogRes>());
        assert!(!world.contains_resource::<CatalogNameSpace>());
        assert_eq!(world.resource::<GameRoundSerial>().0, 20);
        assert!(world.resource::<ResidentSoundBank>().0.is_none());
    }

    #[test]
    fn a_kept_product_is_reusable_only_for_its_zone_and_both_stamps() {
        let product = Product {
            catalog: Arc::default(),
            namespace: AssetCategoryScope::T6,
            gaps: 1,
        };
        let kept = ResidentSoundBank(Some(ResidentBank {
            archives: Arc::default(),
            zone: "mp_a".into(),
            composed: Some(ResidentComposed {
                product_stamp: 2,
                common_stamp: 3,
                product: product.clone(),
            }),
        }));
        let reused = kept.reusable("mp_a", 2, 3).expect("same everything");
        assert!(Arc::ptr_eq(&reused.catalog, &product.catalog));
        assert!(kept.reusable("mp_b", 2, 3).is_none());
        assert!(kept.reusable("mp_a", 9, 3).is_none());
        assert!(kept.reusable("mp_a", 2, 9).is_none());
        assert!(ResidentSoundBank::default().reusable("mp_a", 2, 3).is_none());
    }

    fn boot_app(ready: bool) -> App {
        let mut app = App::default();
        app.insert_resource(AmbientBedStarted(false))
            .insert_resource(GameRoundSerial(2))
            .insert_resource(crate::SoundSystemLive(ready))
            .insert_resource(Assets::<ClipPcm>::default())
            .insert_resource(Assets::<LoopedPcm>::default())
            .insert_resource(CommonPlaybackStore::default())
            .insert_resource(UnknownCueHoles { aliases: vec![] })
            .insert_resource(identity_in("mp_a", "games"))
            .insert_resource(CatalogNameSpace {
                zone: "mp_a".into(),
                namespace: AssetCategoryScope::T6,
            })
            .add_systems(Update, boot_ambient);
        app
    }

    fn loops_named(names: &[&str]) -> AudioCatalogRes {
        let loop_sounds = names
            .iter()
            .map(|name| SpawnEffectAudioLoop {
                cue_alias: name.to_string(),
                position_in_inches: [1.0, 2.0, 3.0],
                line_end_inches: None,
            })
            .collect();
        bank_with_fx(ScriptedMapFx {
            loop_sounds,
            ..ScriptedMapFx::default()
        })
    }

    #[test]
    fn the_map_ambient_boots_once_and_notes_each_unresolved_loop_alias_once() {
        let mut app = boot_app(true);
        app.insert_resource(loops_named(&["amb_a", "amb_b", "amb_a"]));
        app.update();
        let world = app.world();
        assert!(world.resource::<AmbientBedStarted>().0);
        assert_eq!(world.resource::<UnknownCueHoles>().aliases, ["amb_a", "amb_b"]);
        assert!(world.contains_resource::<asset_audio::CreateFxOneshotEmitters>());
        let mut marked = app.world_mut().query::<&LevelBackdropSound>();
        assert_eq!(marked.iter(app.world()).count(), 0);
    }

    #[test]
    fn the_map_ambient_waits_for_audio_ready_a_bank_and_the_matching_zone() {
        let mut not_ready = boot_app(false);
        not_ready.insert_resource(loops_named(&[]));
        let mut no_bank = boot_app(true);
        let mut other_zone = boot_app(true);
        other_zone.insert_resource(loops_named(&[]));
        other_zone.insert_resource(identity_in("mp_z", "games"));
        for app in [&mut not_ready, &mut no_bank, &mut other_zone] {
            app.update();
            assert!(!app.world().resource::<AmbientBedStarted>().0);
        }

        let mut booted_once = boot_app(true);
        booted_once.insert_resource(loops_named(&["amb_a"]));
        booted_once.update();
        booted_once
            .world_mut()
            .resource_mut::<UnknownCueHoles>()
            .aliases
            .clear();
        booted_once.update();
        assert!(booted_once.world().resource::<UnknownCueHoles>().aliases.is_empty());
    }

    fn row_of(near: f32, far: f32, volume: f32) -> SnapshotSoundAlias {
        SnapshotSoundAlias {
            reach_lower: near,
            reach_upper: far,
            volume_low: volume,
            volume_falloff: Some(SnapshotAudioCurve {
                name: "bend".into(),
                knots: vec![(0.0, 1.0), (1.0, 0.0)],
            }),
            ..SnapshotSoundAlias::default()
        }
    }

    #[test]
    fn a_loop_row_gives_its_range_curve_and_a_gain_never_below_zero() {
        let mut shared = CommonPlaybackStore::default();
        let made = Falloff::for_row(Some(&row_of(30.0, 90.0, 0.7)), "amb_x", &mut shared);
        assert_eq!([made.near, made.far, made.gain], [30.0, 90.0, 0.7]);
        assert_eq!(&*made.curve, &RAMP);
        let hushed = Falloff::for_row(Some(&row_of(1.0, 2.0, -3.0)), "amb_x", &mut shared);
        assert_eq!(hushed.gain, 0.0);
        let curveless = SnapshotSoundAlias {
            volume_falloff: Option::default(),
            ..row_of(5.0, 6.0, 1.0)
        };
        let flat = Falloff::for_row(Some(&curveless), "amb_x", &mut shared);
        assert!(flat.curve.is_empty());
        assert_eq!((flat.near, flat.far), (5.0, 6.0));
    }

    #[test]
    fn a_map_sound_plays_by_its_first_variant_in_any_game_when_the_name_is_unique() {
        let mut catalog = AudioRoster::default();
        catalog.add_sound(SnapshotSoundEntry {
            name: "amb_wind".into(),
            aliases: vec![row_of(1.0, 2.0, 0.1), row_of(3.0, 4.0, 0.2)],
        });
        let near_in = |namespace, name: &str| {
            let ctx = MapContext {
                bank: &catalog,
                cue_ns: namespace,
                zone: "mp_a",
                clips: None,
                epoch: 0,
            };
            ctx.lead_alias(name).map(|row| row.reach_lower)
        };
        assert_eq!(near_in(AssetCategoryScope::T6, "amb_wind"), Some(1.0));
        assert_eq!(near_in(AssetCategoryScope::T5, "amb_wind"), Some(1.0));
        assert_eq!(near_in(AssetCategoryScope::T6, "amb_calm"), None);
    }

    #[test]
    fn the_ranking_orders_loudest_first_and_keeps_ties_in_place() {
        let entries = [0.1, 0.9, 0.5, 0.9].map(|gain| Ranked {
            gain,
            owner_ent: Entity::PLACEHOLDER,
            voiced: false,
            spot: [gain; 3],
        });
        let mut ranking = entries.into_iter().collect::<Vec<_>>();
        loudest_first(&mut ranking);
        let by_origin: Vec<f32> = ranking.iter().map(|slot| slot.spot[0]).collect();
        assert_eq!(by_origin, [0.9, 0.9, 0.5, 0.1]);
    }

    #[test]
    fn a_stopped_ambient_removes_every_entity_it_marked() {
        let mut world = World::new();
        let marked: Vec<Entity> = (0..3).map(|_| world.spawn(LevelBackdropSound).id()).collect();
        let bystander = world.spawn_empty().id();
        let _ = world.run_system_once(
            |mut commands: Commands, found: Query<Entity, With<LevelBackdropSound>>| {
                end_map_ambience(&mut commands, &found);
            },
        );
        assert!(marked.iter().all(|&entity| world.get_entity(entity).is_err()));
        assert!(world.get_entity(bystander).is_ok());
    }
}
