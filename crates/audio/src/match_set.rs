//! Readies the sounds a match will need before play starts: queues every
//! alias the loaded weapons, map, teams and HUD can ask for, then reports the
//! audio stage done once their clips are decoded.

use std::collections::{BTreeSet, HashSet};

use asset_audio::SoundCatalog;
use asset_core::AssetNamespace;
use asset_game::WeaponRegistry;
use assets::{MapLoadProcess, MatchType10SoundHints, PreparedWeapons};
use asset_transport::{StageHandle, StageId};
use bevy::prelude::*;
use frame::{ClientSet, LaunchIdentity, MatchTornDown, ReturnedToMenu};

use crate::aliases::movement_prepare_names;
use crate::ambient::{SoundBankCompose, SoundBankLoadAttempted, SoundBankNamespace};
use crate::clip_store::{ClipKey, ClipStore, clip_keys_for_alias};
use crate::match_voices::{AnnouncerRoutes, t6_announcer_aliases, team_voice_aliases};
use crate::playback::SoundBank;

/// Set once every clip the match needs is decoded.
#[derive(Resource, Default)]
pub struct AudioReady(pub bool);

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
    /// Clips not yet decoded when asked for.
    required: HashSet<ClipKey>,
    /// Aliases that name no loaded sound.
    missing: BTreeSet<String>,
    resolved_aliases: usize,
}

#[derive(Resource, Default)]
struct MatchClipPrep {
    submitted: bool,
    required: HashSet<ClipKey>,
    /// Clips this match still had to convert at queue time; neither the alias
    /// count nor the whole cache.
    total: usize,
    stage: Option<StageHandle>,
    /// Decoded sample bytes the process held when the set was queued. The
    /// decoders count for the life of the process, so this match's cost is
    /// whatever has been added since.
    sample_bytes_at_queue: u64,
}

impl MatchClipPrep {
    fn bytes_decoded_since_queue(&self) -> u64 {
        prepared_sample_bytes().saturating_sub(self.sample_bytes_at_queue)
    }
}

/// All `f32` sample bytes the decoders have produced so far.
fn prepared_sample_bytes() -> u64 {
    let cost = crate::clip_prep_cost();
    cost.paths.iter().map(|(_, entry)| entry.sample_bytes).sum()
}

pub(crate) fn register(app: &mut App) {
    if AudioSilent::no_device() {
        diag::info!(Audio, "audio: Silent (BO2MP_HEADLESS=1, no sound device)");
        app.insert_resource(AudioSilent);
    } else if AudioSilent::active() {
        diag::info!(Audio, "audio: Silent (BO2MP_SOUND=off)");
        app.insert_resource(AudioSilent);
    }
    app.init_resource::<AudioReady>()
        .init_resource::<MatchClipPrep>()
        .init_resource::<AnnouncerRoutes>()
        .add_systems(
            Update,
            (
                queue_match_clips.after(crate::ambient::install_sound_bank),
                poll_match_audio_ready.after(queue_match_clips),
                reset_match_audio_on_match_end,
            )
                .in_set(ClientSet::Load),
        );
}

fn reset_match_audio_on_match_end(
    mut torn: MessageReader<MatchTornDown>,
    mut returned: MessageReader<ReturnedToMenu>,
    mut ready: ResMut<AudioReady>,
    mut prep: ResMut<MatchClipPrep>,
    mut announcer: ResMut<AnnouncerRoutes>,
) {
    let torn_down = torn.read().count();
    let back_to_menu = returned.read().count();
    if torn_down + back_to_menu == 0 {
        return;
    }
    ready.0 = false;
    if let Some(stage) = prep.stage.take() {
        stage.cancel();
    }
    *prep = MatchClipPrep::default();
    *announcer = AnnouncerRoutes::default();
}

/// Reports the audio stage as skipped for a match that needs no clips.
fn skip_audio_stage(loading: Option<&MapLoadProcess>) {
    if let Some(loading) = loading {
        loading.progress.record_skipped(StageId::Audio);
    }
}

fn queue_match_clips(
    attempted: Res<SoundBankLoadAttempted>,
    walk: Option<Res<SoundBankCompose>>,
    mut clips: Option<ResMut<ClipStore>>,
    weapons: Option<Res<PreparedWeapons>>,
    type10: Option<Res<MatchType10SoundHints>>,
    bank: Option<Res<SoundBank>>,
    identity: Option<Res<LaunchIdentity>>,
    script_sound: Option<Res<asset_audio::SessionMapScriptSound>>,
    teams: Option<Res<asset_game::SessionTeamSettings>>,
    namespace: Option<Res<SoundBankNamespace>>,
    loading: Option<Res<MapLoadProcess>>,
    mut prep: ResMut<MatchClipPrep>,
    mut announcer: ResMut<AnnouncerRoutes>,
    mut ready: ResMut<AudioReady>,
    silent: Option<Res<AudioSilent>>,
) {
    if ready.0 || prep.submitted {
        return;
    }
    if silent.is_some() && !AudioSilent::composes() {
        ready.0 = true;
        skip_audio_stage(loading.as_deref());
        diag::info!(Audio, "audio: AudioReady — Silent, no clips prepared");
        return;
    }
    if !attempted.0 || walk.is_some() {
        return;
    }
    let Some(clips) = clips.as_mut() else {
        ready.0 = true;
        skip_audio_stage(loading.as_deref());
        diag::info!(
            Audio,
            "audio: AudioReady skipped — sound bank did not install"
        );
        return;
    };
    let (Some(weapons), Some(type10), Some(bank), Some(namespace), Some(script_sound)) =
        (weapons, type10, bank, namespace, script_sound)
    else {
        return;
    };
    let catalog: &SoundCatalog = &bank.0;
    let mut set = MatchRequests::default();
    let mut aliases = 0usize;

    for weapon in 1..=weapons.0.len() as u32 {
        aliases += request_weapon_aliases(clips, catalog, &weapons.0, weapon, &mut set);
    }
    for alias in movement_prepare_names() {
        aliases += 1;
        request_named(clips, catalog, AssetNamespace::T6, alias, &mut set);
    }

    if let Some(identity) = identity.as_deref() {
        // The bank must be the one composed for this launch's zone.
        if namespace.zone != identity.zone {
            return;
        }
        for emitter in catalog.createfx_loop_sounds(namespace.namespace, &namespace.zone) {
            aliases += 1;
            request_named(
                clips,
                catalog,
                namespace.namespace,
                &emitter.soundalias,
                &mut set,
            );
        }

        // The old faction table went with its menu catalog; Black Ops II's
        // own team settings route the announcer.
        let voices = team_voice_aliases(catalog, None, None);
        let native = teams.as_deref().map(|teams| &teams.0);
        *announcer =
            AnnouncerRoutes::build(catalog, namespace.namespace, [None, None], native, &voices);
        diag::info!(
            Audio,
            "audio: announcer: {} of {} team voice aliases play as {:?}",
            announcer.len(),
            voices.len(),
            namespace.namespace
        );
        for alias in &voices {
            aliases += 1;
            let (routed_ns, routed) = announcer.route(alias);
            request_named(clips, catalog, routed_ns, routed, &mut set);
        }
    }

    if let Some(alias) = script_sound.0.ambient_alias.as_deref() {
        aliases += 1;
        request_named(clips, catalog, namespace.namespace, alias, &mut set);
    }

    let hud = MENU_CODE
        .iter()
        .chain(&HUD_PULSE)
        .chain(&MOTION_TRACKER)
        .chain(&crate::weapon_lock::ALIASES)
        .chain(&MATCH_CLOCK);
    for alias in hud {
        aliases += 1;
        request_named(clips, catalog, AssetNamespace::T6, alias, &mut set);
    }

    if namespace.namespace == AssetNamespace::T6 {
        for alias in T6_HIT_SOUNDS {
            aliases += 1;
            request_named(clips, catalog, AssetNamespace::T6, alias, &mut set);
        }
        // The team announcer (`vox_<faction>_<line>`: match ending soon, lead
        // taken or lost, ...) must be ready before the script plays it.
        let lines = t6_announcer_aliases(catalog);
        diag::info!(
            Audio,
            "audio: announcer: {} Black Ops II team voice lines queued",
            lines.len()
        );
        for alias in &lines {
            aliases += 1;
            request_named(clips, catalog, AssetNamespace::T6, alias, &mut set);
        }
    }

    for alias in &type10.0 {
        aliases += 1;
        request_fx_type10(clips, catalog, alias, &mut set);
    }

    if !set.missing.is_empty() {
        let names = set
            .missing
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        diag::warn!(
            Audio,
            "audio: match-set gap: {} aliases name no loaded sound: {}",
            names.len(),
            names.join(" ")
        );
    }

    prep.total = set.required.len();
    prep.required = set.required;
    prep.submitted = true;
    if let Some(loading) = loading {
        prep.sample_bytes_at_queue = prepared_sample_bytes();
        if prep.total == 0 && set.resolved_aliases > 0 {
            loading
                .progress
                .record_reused_scoped(StageId::Audio, "clips");
        } else {
            prep.stage = Some(
                loading
                    .progress
                    .begin(StageId::Audio, Some(prep.total as u64)),
            );
        }
    }
    let (reused_clips, reused_bytes) = clips.reused_resident();
    diag::info!(
        Audio,
        "audio: match-set queued {} aliases ({} type-10), {} clips still converting ({} workers); resident reused={} clips/{}B",
        aliases,
        type10.0.len(),
        prep.total,
        clips.workers(),
        reused_clips,
        reused_bytes,
    );
    if prep.total == 0 {
        mark_ready(&mut ready, &mut prep, Some(&mut **clips));
    }
}

fn poll_match_audio_ready(
    mut clips: Option<ResMut<ClipStore>>,
    mut prep: ResMut<MatchClipPrep>,
    mut ready: ResMut<AudioReady>,
) {
    if ready.0 || !prep.submitted {
        return;
    }
    let Some(clips) = clips.as_mut() else {
        return;
    };
    // A clip whose result is in, good or failed, is no longer awaited.
    prep.required.retain(|key| clips.ready(key).is_none());
    if let Some(stage) = prep.stage.as_ref() {
        let finished = prep.total.saturating_sub(prep.required.len());
        stage.set_completed(finished as u64);
        stage.set_bytes(prep.bytes_decoded_since_queue());
    }
    if prep.required.is_empty() {
        mark_ready(&mut ready, &mut prep, Some(&mut **clips));
    }
}

fn mark_ready(ready: &mut AudioReady, prep: &mut MatchClipPrep, clips: Option<&mut ClipStore>) {
    if let Some(stage) = prep.stage.take() {
        stage.set_completed(prep.total as u64);
        stage.set_bytes(prep.bytes_decoded_since_queue());
        stage.done();
    }
    if ready.0 {
        return;
    }
    ready.0 = true;
    diag::info!(Audio, "audio: AudioReady ({} clips prepared)", prep.total);
    if let Some(clips) = clips {
        clips.arm_match_live();
    }
}

/// Asks for every clip behind `alias` and notes which are still pending.
fn request_named(
    clips: &mut ClipStore,
    bank: &SoundCatalog,
    namespace: AssetNamespace,
    alias: &str,
    set: &mut MatchRequests,
) {
    let keys = clip_keys_for_alias(bank, namespace, alias);
    if keys.is_empty() {
        set.missing.insert(alias.to_owned());
        return;
    }
    set.resolved_aliases += 1;
    for key in keys {
        clips.request(key.clone());
        if clips.ready(&key).is_none() {
            set.required.insert(key);
        }
    }
}

/// An effect's type-10 sound lives in whichever game the catalog finds it in.
fn request_fx_type10(
    clips: &mut ClipStore,
    bank: &SoundCatalog,
    alias: &str,
    set: &mut MatchRequests,
) {
    let home = bank
        .index_in(AssetNamespace::T6, alias)
        .or_else(|| bank.index_unique(alias))
        .map_or(AssetNamespace::T6, |index| bank.namespace_of_alias(index));
    request_named(clips, bank, home, alias, set);
}

/// Queues what one weapon can play: its reachable aliases, then each distinct
/// alias its notetracks name. Returns how many aliases that was.
fn request_weapon_aliases(
    clips: &mut ClipStore,
    bank: &SoundCatalog,
    registry: &WeaponRegistry,
    weapon: u32,
    set: &mut MatchRequests,
) -> usize {
    let namespace = registry
        .namespace_of(weapon)
        .unwrap_or(AssetNamespace::T6);
    let mut queued = 0usize;

    if let Some(sounds) = registry.sounds_of(weapon) {
        for alias in sounds.reachable_aliases() {
            queued += 1;
            request_named(clips, bank, namespace, alias, set);
        }
    }

    let mut once = HashSet::new();
    for alias in registry.notetrack_sound_aliases_of(weapon) {
        if once.insert(alias) {
            queued += 1;
            request_named(clips, bank, namespace, alias, set);
        }
    }
    queued
}
