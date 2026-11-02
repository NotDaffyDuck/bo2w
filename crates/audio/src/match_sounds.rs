//! Readies the sounds a match will need before play starts: queues every
//! alias the loaded weapons, map, teams and HUD can ask for, then reports the
//! audio stage done once their clips are decoded.

use std::collections::{BTreeSet, HashSet};

use asset_audio::AudioRoster;
use asset_core::AssetCategoryScope;
use asset_game::WeaponLedger;
use assets::{MapLoadProcess, RoundTenthKindAudioClues, StagedWeapons};
use asset_transport::{StageHandle, StageId};
use bevy::prelude::*;
use frame::{ClientStep, StartupPersona, GameSessionClosed, ReturnedToMenu};

use crate::cue_names::motion_preload_cues;
use crate::map_sound_set::{SoundBankCompose, BankLoadTried, CatalogNameSpace};
use crate::clip_bank::{WaveformHandle, ClipRepository, waveforms_for_cue};
use crate::match_voices::{AnnouncerRoutes, t6_announcer_aliases, t6_combat_aliases, team_voice_aliases};
use crate::play_core::AudioCatalogRes;

/// Set once every clip the match needs is decoded.
#[derive(Resource, Default)]
pub struct SoundSystemLive(pub bool);

/// Present when nothing should reach the speakers (no device, or sound off).
#[derive(Resource)]
pub struct AudioSilent;

impl AudioSilent {
    pub fn active() -> bool {
        static SILENT: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *SILENT.get_or_init(|| {
            let sound_off = std::env::var("BO2MP_SOUND")
                .is_ok_and(|value| value == "off" || value == "0");
            frame::Headless::requested() || sound_off
        })
    }

    /// Headless runs open no sound device, so no voice ever gets a sink;
    /// their starts, loops and stops still run and reach the log.
    pub fn no_device() -> bool {
        frame::Headless::requested()
    }

    /// A silent run that also logs sound (BO2MP_SOUND_LOG) still composes the
    /// map's sound bank and prepares its clips, so real lookups, starts, loops
    /// and stops happen and are logged; the PCM decoder hands the device zeros,
    /// so nothing is audible. Skipping the bank would send every lookup to the
    /// legacy namespace, where no Black Ops II alias resolves.
    pub fn composes() -> bool {
        Self::active() && std::env::var_os("BO2MP_SOUND_LOG").is_some()
    }
}

const HUD_PULSE: [&str; 2] = ["ui_pulse_text_type", "ui_pulse_text_delete"];
const MOTION_TRACKER: [&str; 2] = ["motiontracker_ping", "motiontracker_pong"];
const MENU_CODE: [&str; 2] = ["mouse_over", "mouse_click"];
const MATCH_CLOCK: [&str; 1] = ["ui_mp_timer_countdown"];

/// Black Ops II's hit feedback (`_damagefeedback.gsc`'s hit alerts and the
/// engine's headshot crack): ready before the first hit, since a late clip
/// leaves the opening hits of a match silent.
const T6_HIT_SOUNDS: [&str; 4] = [
    "mpl_hit_alert",
    "mpl_hit_alert_low",
    "prj_bullet_impact_headshot",
    "prj_bullet_impact_headshot_2d",
];

/// What queuing the match's aliases produced.
#[derive(Default)]
struct MatchRequests {
    /// Aliases that name no loaded sound.
    missing: BTreeSet<String>,
    resolved_aliases: usize,
    /// Clips that were still being decoded when asked for.
    unready: HashSet<WaveformHandle>,
}

impl MatchRequests {
    /// Asks for every clip behind `alias`, remembering the ones still pending.
    fn ask(
        &mut self,
        namespace: AssetCategoryScope,
        catalog: &AudioRoster,
        cue_id: &str,
        store: &mut ClipRepository,
    ) {
        let keys = waveforms_for_cue(catalog, namespace, cue_id);
        if let [] = keys.as_slice() {
            self.missing.insert(cue_id.to_owned());
            return;
        }
        self.resolved_aliases += 1;
        for key in keys {
            store.req_in(key.clone());
            if store.ready_pcm(&key).is_none() {
                self.unready.insert(key);
            }
        }
    }

    fn warn_about_gaps(&self) {
        if self.missing.is_empty() {
            return;
        }
        let listed = self
            .missing
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(", ");
        diag::warn!(
            Audio,
            "audio: match-set gap, {} alias(es) name no loaded sound: {}",
            self.missing.len(),
            listed
        );
    }
}

/// Where the clip preparation for the current match stands.
#[derive(Resource, Default)]
struct ClipPrep {
    queued: bool,
    awaiting: HashSet<WaveformHandle>,
    /// Clips this match still had to convert at queue time; neither the alias
    /// count nor the whole cache.
    converting: usize,
    stage: Option<StageHandle>,
    /// Decoded sample bytes the process held when the set was queued. The
    /// decoders count for the life of the process, so this match's cost is
    /// whatever has been added since.
    bytes_at_queue: u64,
}

impl ClipPrep {
    fn bytes_since_queue(&self) -> u64 {
        prepared_sample_bytes().saturating_sub(self.bytes_at_queue)
    }

    /// Shows the load screen the audio stage, or notes it was served from memory.
    fn open_stage(&mut self, loading: &MapLoadProcess, resolved_aliases: usize) {
        self.bytes_at_queue = prepared_sample_bytes();
        let from_memory = self.converting == 0 && resolved_aliases > 0;
        if from_memory {
            loading
                .progress
                .record_reused_scoped(StageId::Audio, "clips");
            return;
        }
        let total = Some(self.converting as u64);
        self.stage = Some(loading.progress.begin(StageId::Audio, total));
    }

    fn report_progress(&self) {
        let Some(stage) = &self.stage else {
            return;
        };
        let outstanding = self.awaiting.len().min(self.converting);
        stage.set_completed((self.converting - outstanding) as u64);
        stage.set_bytes(self.bytes_since_queue());
    }

    fn finish_stage(&mut self) {
        let Some(stage) = self.stage.take() else {
            return;
        };
        stage.set_completed(self.converting as u64);
        stage.set_bytes(self.bytes_since_queue());
        stage.done();
    }
}

/// All `f32` sample bytes the decoders have produced so far.
fn prepared_sample_bytes() -> u64 {
    let cost = crate::clip_prep_cost();
    cost.paths.iter().map(|(_, entry)| entry.sample_bytes).sum()
}

pub(crate) fn wire_up(host: &mut App) {
    if AudioSilent::no_device() {
        diag::info!(Audio, "sound: muted, no device (BO2MP_HEADLESS=1)");
        host.insert_resource(AudioSilent);
    } else if AudioSilent::active() {
        // Muted at the very end of the master bus, so its meter reads the real mix.
        bevy::audio::master_bus::mute();
        diag::info!(Audio, "sound: muted by BO2MP_SOUND=off");
        host.insert_resource(AudioSilent);
    }
    host.init_resource::<ClipPrep>()
        .init_resource::<AnnouncerRoutes>()
        .init_resource::<SoundSystemLive>();
    let match_audio = (
        queue_match_set.after(crate::map_sound_set::install_sound_bank),
        poll_match_set.after(queue_match_set),
        forget_match_audio,
    );
    host.add_systems(Update, match_audio.in_set(ClientStep::Load));
}

fn forget_match_audio(
    mut announcer: ResMut<AnnouncerRoutes>,
    mut prep: ResMut<ClipPrep>,
    mut ready: ResMut<SoundSystemLive>,
    mut returned: MessageReader<ReturnedToMenu>,
    mut torn: MessageReader<GameSessionClosed>,
) {
    // Both readers are drained, whichever message came.
    let ended = torn.read().count() + returned.read().count();
    if ended == 0 {
        return;
    }
    ready.0 = false;
    *announcer = AnnouncerRoutes::default();
    let gone = std::mem::take(&mut *prep);
    if let Some(stage) = gone.stage {
        stage.cancel();
    }
}

/// Reports the audio stage as skipped for a match that needs no clips.
fn skip_audio_stage(loading: Option<&MapLoadProcess>) {
    for process in loading {
        process.progress.record_skipped(StageId::Audio);
    }
}

/// Marks audio ready without any clips, saying why in the log.
fn settle_without_clips(pcm_ready: &mut SoundSystemLive, loading: Option<&MapLoadProcess>, why: &str) {
    pcm_ready.0 = true;
    skip_audio_stage(loading);
    diag::info!(Audio, "sound: ready ({})", why);
}

/// Everything the match could ask the sound bank for, and the game each alias is
/// looked up in.
struct Wants<'a> {
    catalog: &'a AudioRoster,
    list: Vec<(AssetCategoryScope, String)>,
}

impl<'a> Wants<'a> {
    fn new(catalog: &'a AudioRoster) -> Self {
        Self {
            catalog,
            list: Vec::new(),
        }
    }

    fn push_all<S: AsRef<str>>(
        &mut self,
        namespace: AssetCategoryScope,
        aliases: impl IntoIterator<Item = S>,
    ) {
        let owned = aliases
            .into_iter()
            .map(|alias| (namespace, alias.as_ref().to_owned()));
        self.list.extend(owned);
    }

    /// What one weapon can play: its reachable aliases, then each distinct alias its
    /// notetracks name.
    fn weapon_ix(&mut self, registry: &WeaponLedger, wpn_slot: u32) {
        let namespace = registry.scope_for(wpn_slot).unwrap_or(AssetCategoryScope::T6);
        let reachable = registry
            .audio_for(wpn_slot)
            .into_iter()
            .flat_map(|sounds| sounds.attainable_entries());
        let mut seen_keys = HashSet::new();
        let noted = registry
            .notetrack_sound_aliases_of(wpn_slot)
            .into_iter()
            .filter(|alias| seen_keys.insert(*alias));
        self.push_all(namespace, reachable.chain(noted));
    }

    /// An effect's type-10 sound lives in whichever game the catalog finds it in.
    fn type10(&mut self, cue_name: &str) {
        let found = self
            .catalog
            .slot_in(AssetCategoryScope::T6, cue_name)
            .or_else(|| self.catalog.slot_unique(cue_name));
        let home = found.map_or(AssetCategoryScope::T6, |index| {
            self.catalog.space_of_slot(index)
        });
        self.push_all(home, [cue_name]);
    }
}

/// What the match's own state contributes to the wanted aliases.
struct MatchSources<'a> {
    type10: &'a RoundTenthKindAudioClues,
    ambient: Option<&'a str>,
    teams: Option<&'a asset_game::MapTeamSettings>,
    launch: Option<&'a StartupPersona>,
    bank: &'a CatalogNameSpace,
    weapons: &'a WeaponLedger,
}

/// The interface and feedback sounds every match plays, whatever it loaded.
fn fixed_aliases() -> impl Iterator<Item = &'static str> {
    let groups: [&[&'static str]; 5] = [
        &MENU_CODE,
        &HUD_PULSE,
        &MOTION_TRACKER,
        &crate::weapon_lock::ALIASES,
        &MATCH_CLOCK,
    ];
    groups.into_iter().flatten().copied()
}

/// Lists the aliases this match wants ready. `None` while the bank on hand was
/// composed for a different zone than the one being launched.
fn gather<'a>(
    catalog: &'a AudioRoster,
    src: &MatchSources<'_>,
    announcer: &mut AnnouncerRoutes,
) -> Option<Wants<'a>> {
    let home = src.bank.namespace;
    let mut wants = Wants::new(catalog);

    (1..=src.weapons.len() as u32).for_each(|weapon| wants.weapon_ix(src.weapons, weapon));
    wants.push_all(AssetCategoryScope::T6, motion_preload_cues());

    if let Some(launch) = src.launch {
        if src.bank.zone != launch.zone {
            return None;
        }
        let loops = catalog.createfx_loop_sounds(home, &src.bank.zone);
        wants.push_all(home, loops.into_iter().map(|emitter| emitter.cue_alias.clone()));

        // The old faction table went with its menu catalog; Black Ops II's
        // own team settings route the announcer.
        let voices = team_voice_aliases(catalog, None, None);
        *announcer = AnnouncerRoutes::build(catalog, home, [None, None], src.teams, &voices);
        diag::info!(
            Audio,
            "audio: announcer: {} of {} team voice aliases play as {:?}",
            announcer.len(),
            voices.len(),
            home
        );
        let routes = &*announcer;
        wants.list.extend(voices.iter().map(|alias| {
            let (routed_ns, routed) = routes.route(alias);
            (routed_ns, routed.to_owned())
        }));
    }

    wants.push_all(home, src.ambient);
    wants.push_all(AssetCategoryScope::T6, fixed_aliases());

    if home == AssetCategoryScope::T6 {
        wants.push_all(AssetCategoryScope::T6, T6_HIT_SOUNDS);
        // The team announcer (`vox_<faction>_<line>`: match ending soon, lead
        // taken or lost, ...) must be ready before the script plays it.
        let lines = t6_announcer_aliases(catalog);
        diag::info!(
            Audio,
            "audio: announcer: {} Black Ops II team voice lines queued",
            lines.len()
        );
        wants.push_all(AssetCategoryScope::T6, lines);
        let combat = t6_combat_aliases(catalog);
        diag::info!(Audio, "audio: {} combat aliases queued (impacts, steps, voices)", combat.len());
        wants.push_all(AssetCategoryScope::T6, combat);
    }

    for alias in &src.type10.0 {
        wants.type10(alias);
    }
    Some(wants)
}

fn queue_match_set(
    mut ready: ResMut<SoundSystemLive>,
    mut prep: ResMut<ClipPrep>,
    mut announcer: ResMut<AnnouncerRoutes>,
    mut store: Option<ResMut<ClipRepository>>,
    tried: Res<BankLoadTried>,
    compose: Option<Res<SoundBankCompose>>,
    silent: Option<Res<AudioSilent>>,
    loading: Option<Res<MapLoadProcess>>,
    weapons: Option<Res<StagedWeapons>>,
    hints: Option<Res<RoundTenthKindAudioClues>>,
    bank: Option<Res<AudioCatalogRes>>,
    bank_ns: Option<Res<CatalogNameSpace>>,
    identity: Option<Res<StartupPersona>>,
    script_sound: Option<Res<asset_audio::SessionMapScriptSound>>,
    teams: Option<Res<asset_game::SessionTeamSettings>>,
) {
    if ready.0 || prep.queued {
        return;
    }
    // bo2zm: a silent run prepares no clips, even when it logs sound.
    // BO2MP_SOUND_DECODE=1 (debug only, with BO2MP_SOUND=off and BO2MP_SOUND_LOG=1):
    // still decode them, so a muted run carries the real decode load.
    if silent.is_some() && std::env::var_os("BO2MP_SOUND_DECODE").is_none() {
        settle_without_clips(
            &mut ready,
            loading.as_deref(),
            "\u{2014} Silent, no clips prepared",
        );
        return;
    }
    if !tried.0 || compose.is_some() {
        return;
    }
    let Some(store) = store.as_deref_mut() else {
        settle_without_clips(
            &mut ready,
            loading.as_deref(),
            "skipped \u{2014} sound bank did not install",
        );
        return;
    };
    let (Some(weapons), Some(hints), Some(snd_bank), Some(bank_ns), Some(script_sound)) =
        (weapons, hints, bank, bank_ns, script_sound)
    else {
        return;
    };
    let catalog: &AudioRoster = &snd_bank.0;
    let sources = MatchSources {
        type10: &hints,
        ambient: script_sound.0.background_sound_entry.as_deref(),
        teams: teams.as_deref().map(|teams| &teams.0),
        launch: identity.as_deref(),
        bank: &bank_ns,
        weapons: &weapons.0,
    };
    let Some(wants) = gather(catalog, &sources, &mut announcer) else {
        return;
    };

    let mut requests = MatchRequests::default();
    for (cue_ns, alias) in &wants.list {
        requests.ask(*cue_ns, catalog, alias, store);
    }
    // bo2zm: the sounds the map's scripts name.
    for alias in &script_sound.0.prepare {
        requests.ask(bank_ns.namespace, catalog, alias, store);
    }
    requests.warn_about_gaps();

    let converting = requests.unready.len();
    *prep = ClipPrep {
        queued: true,
        converting,
        awaiting: requests.unready,
        ..ClipPrep::default()
    };
    for process in loading.as_deref() {
        prep.open_stage(process, requests.resolved_aliases);
    }
    let (reused_clips, reused_bytes) = store.reused_resident();
    diag::info!(
        Audio,
        "audio: match-set queued: {} aliases ({} of them type-10); {} clips to convert on {} workers; {} clips ({} bytes) already resident",
        wants.list.len(),
        hints.0.len(),
        converting,
        store.worker_n(),
        reused_clips,
        reused_bytes,
    );
    if converting == 0 {
        complete(&mut prep, &mut ready, store);
    }
}

fn poll_match_set(
    mut ready: ResMut<SoundSystemLive>,
    mut prep: ResMut<ClipPrep>,
    mut store: Option<ResMut<ClipRepository>>,
) {
    let Some(store) = store.as_deref_mut() else {
        return;
    };
    if ready.0 || !prep.queued {
        return;
    }
    // A clip whose result is in, good or failed, is no longer awaited.
    prep.awaiting.retain(|key| store.ready_pcm(key).is_none());
    prep.report_progress();
    if prep.awaiting.is_empty() {
        complete(&mut prep, &mut ready, store);
    }
}

fn complete(prep: &mut ClipPrep, ready: &mut SoundSystemLive, store: &mut ClipRepository) {
    prep.finish_stage();
    let was_ready = std::mem::replace(&mut ready.0, true);
    if was_ready {
        return;
    }
    diag::info!(Audio, "sound: ready with {} clips", prep.converting);
    store.enable_round_playback();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_aliases_are_the_ui_groups_in_order() {
        let all: Vec<&str> = fixed_aliases().collect();
        assert_eq!(all.len(), 2 + 2 + 2 + crate::weapon_lock::ALIASES.len() + 1);
        assert_eq!(&all[..2], &MENU_CODE);
        assert_eq!(&all[2..4], &HUD_PULSE);
        assert_eq!(&all[4..6], &MOTION_TRACKER);
        assert_eq!(all.last().copied(), Some("ui_mp_timer_countdown"));
    }

    #[test]
    fn unknown_type10_sound_falls_back_to_the_black_ops_ii_names() {
        let catalog = AudioRoster::default();
        let mut wants = Wants::new(&catalog);
        wants.type10("anything");
        assert_eq!(wants.list, vec![(AssetCategoryScope::T6, "anything".to_owned())]);
    }

    #[test]
    fn push_all_tags_every_alias_with_its_game() {
        let catalog = AudioRoster::default();
        let mut wants = Wants::new(&catalog);
        wants.push_all(AssetCategoryScope::T6, ["a", "b"]);
        wants.push_all(AssetCategoryScope::T6, vec!["c".to_owned()]);
        let names: Vec<&str> = wants.list.iter().map(|(_, name)| name.as_str()).collect();
        assert_eq!(names, ["a", "b", "c"]);
    }

    #[test]
    fn a_match_with_nothing_missing_warns_about_nothing() {
        let set = MatchRequests::default();
        assert!(set.missing.is_empty() && set.unready.is_empty());
        set.warn_about_gaps();
    }
}
