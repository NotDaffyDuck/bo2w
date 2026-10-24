//! The menu's sound: a frontend bank built off-thread at startup, restored
//! whenever the player returns from a match, plus the UI sound and music
//! messages the menu sends.

use std::sync::Arc;

use asset_audio::{SoundCatalog, load_mp_sound_bank};
use asset_core::AssetNamespace;
use asset_transport::GamesRoot;
use assets::{NamespaceSoundIwd, NamespaceTrees};
use bevy::{
    audio::Volume,
    prelude::*,
    tasks::{AsyncComputeTaskPool, Task, TaskPool, futures_lite::future},
};
use frame::{
    ClientSet, LaunchIdentity, ReturnedToMenu, UiPlayMusic, UiPlaySound, UiStopMusic,
    register_ui_contracts,
};

use crate::{
    ClipStore, SoundClass,
    ambient::SoundIwd,
    clip_store::{PendingStarts, clip_keys_for_alias},
    pcm::{LoopingPcmAudio, PcmAudio},
    playback::{MissingAliasGaps, SharedPlayAssets, SoundBank, SoundPickState, play_alias_oneshot},
    start::StartDecisions,
};

/// The resident menu sound bank and the loose-file index its clips come from.
#[derive(Resource, Clone)]
pub struct FrontendAudio {
    pub bank: Arc<SoundCatalog>,
    pub iwd: Arc<NamespaceSoundIwd>,
}

const FRONTEND_SOUND_ZONE: &str = "t6:code_post_gfx_mp";

/// The background build of the frontend bank.
#[derive(Resource)]
struct FrontendAudioPrepare(Task<FrontendAudioWalked>);

/// Marks a frontend bank that failed to build, so it is not walked again every
/// frame (an install without the zone can never produce one).
#[derive(Resource)]
struct FrontendAudioGap;

struct FrontendAudioWalked {
    bank: Option<Arc<SoundCatalog>>,
    iwd: Arc<NamespaceSoundIwd>,
    lines: Vec<String>,
}

/// The menu music loop that is playing.
#[derive(Component)]
struct MenuMusicBed {
    alias: String,
}

/// The menu music the UI last asked for, whether or not it has started yet.
#[derive(Resource, Default)]
struct PendingMenuBed {
    alias: Option<String>,
}

pub(crate) fn register_frontend_audio(app: &mut App) {
    register_ui_contracts(app);
    app.init_resource::<PendingMenuBed>()
        .add_message::<ReturnedToMenu>()
        .add_systems(
            Update,
            (
                start_frontend_audio_prepare,
                install_frontend_audio_prepare.after(start_frontend_audio_prepare),
                restore_frontend_audio_on_menu
                    .after(install_frontend_audio_prepare)
                    .after(crate::ambient::stop_map_ambient_on_match_end),
            )
                .in_set(ClientSet::Load),
        )
        .add_systems(
            Update,
            (
                play_ui_sound_messages.after(crate::voice::reclaim_finished_voices),
                play_ui_music_messages,
            )
                .in_set(ClientSet::Effects),
        );
}

/// Builds the bank and loose-file index on a worker thread.
async fn walk_frontend(games: GamesRoot) -> FrontendAudioWalked {
    let mut lines = Vec::new();
    let bank = match load_mp_sound_bank(&games, FRONTEND_SOUND_ZONE) {
        Ok(loaded) => {
            lines.extend(loaded.gap_lines());
            Some(Arc::new(loaded.catalog))
        }
        Err(error) => {
            lines.push(format!("frontend sound bank: {error}"));
            None
        }
    };
    let (iwd, open_lines) = NamespaceSoundIwd::open(&NamespaceTrees::discover(&games));
    lines.extend(open_lines);
    FrontendAudioWalked {
        bank,
        iwd: Arc::new(iwd),
        lines,
    }
}

fn start_frontend_audio_prepare(
    frontend: Option<Res<FrontendAudio>>,
    running: Option<Res<FrontendAudioPrepare>>,
    identity: Option<Res<LaunchIdentity>>,
    silent: Option<Res<crate::AudioSilent>>,
    gap: Option<Res<FrontendAudioGap>>,
    loading: (
        Option<Res<assets::MatchLoadBusy>>,
        Option<Res<assets::MatchLoadRequest>>,
        Option<Res<assets::MatchLoadAccepted>>,
        Option<Res<assets::PreparedMatchReady>>,
        Option<Res<crate::ambient::SoundBankCompose>>,
    ),
    mut commands: Commands,
) {
    if frontend.is_some() || running.is_some() || silent.is_some() || gap.is_some() {
        return;
    }
    // Leave the disk to a match that is loading or about to.
    let (busy, request, accepted, prepared, composing) = loading;
    let match_loading = busy.is_some_and(|busy| busy.0)
        || request.is_some()
        || accepted.is_some()
        || prepared.is_some()
        || composing.is_some();
    if match_loading {
        return;
    }
    let Some(identity) = identity else {
        return;
    };
    if identity.games_root.as_os_str().is_empty() {
        return;
    }
    let games = GamesRoot(identity.games_root.clone());
    let pool = AsyncComputeTaskPool::get_or_init(TaskPool::default);
    commands.insert_resource(FrontendAudioPrepare(pool.spawn(walk_frontend(games))));
}

fn install_frontend_audio_prepare(
    mut prepare: Option<ResMut<FrontendAudioPrepare>>,
    mut commands: Commands,
) {
    let Some(prepare) = prepare.as_deref_mut() else {
        return;
    };
    let Some(walked) = future::block_on(future::poll_once(&mut prepare.0)) else {
        return;
    };
    commands.remove_resource::<FrontendAudioPrepare>();
    for line in &walked.lines {
        diag::info!(Audio, "audio: frontend {line}");
    }
    match walked.bank {
        Some(bank) => {
            commands.insert_resource(FrontendAudio {
                bank,
                iwd: walked.iwd,
            });
            diag::info!(Audio, "audio: frontend sound resident");
        }
        None => {
            diag::warn!(
                Audio,
                "audio: frontend sound bank did not build — the menu stays silent (typed gap)"
            );
            commands.insert_resource(FrontendAudioGap);
        }
    }
}

pub(crate) fn restore_frontend_audio_on_menu(
    mut returned: MessageReader<ReturnedToMenu>,
    frontend: Option<Res<FrontendAudio>>,
    live: Option<Res<SoundBank>>,
    silent: Option<Res<crate::AudioSilent>>,
    mut commands: Commands,
) {
    let returns = returned.read().count();
    if returns == 0 || silent.is_some() {
        return;
    }
    let Some(frontend) = frontend else {
        diag::warn!(
            Audio,
            "audio: no resident frontend bank — the menu runs without sound (typed gap)"
        );
        return;
    };
    let already_live = live.is_some_and(|live| Arc::ptr_eq(&live.0, &frontend.bank));
    if already_live {
        return;
    }
    commands.insert_resource(PendingStarts::default());
    commands.insert_resource(SharedPlayAssets::default());
    commands.insert_resource(SoundBank(Arc::clone(&frontend.bank)));
    commands.insert_resource(SoundIwd(Arc::clone(&frontend.iwd)));
    commands.insert_resource(ClipStore::start(
        Arc::clone(&frontend.bank),
        Some(Arc::clone(&frontend.iwd)),
    ));
    diag::info!(Audio, "audio: frontend sound restored");
}

/// The game a menu alias belongs to. Black Ops II's front end and match banks
/// are T6 (looked up as legacy, every menu sound would be missing); T6 is also
/// the answer when no game has it.
fn alias_namespace(bank: &SoundCatalog, alias: &str) -> AssetNamespace {
    [AssetNamespace::T6, AssetNamespace::T5]
        .into_iter()
        .find(|&namespace| bank.index_in(namespace, alias).is_some())
        .unwrap_or(AssetNamespace::T6)
}

fn play_ui_sound_messages(
    mut events: MessageReader<UiPlaySound>,
    mut commands: Commands,
    mut pcm_assets: ResMut<Assets<PcmAudio>>,
    mut shared: ResMut<SharedPlayAssets>,
    mut pick: ResMut<SoundPickState>,
    mut gaps: ResMut<MissingAliasGaps>,
    mut clips: Option<ResMut<ClipStore>>,
    mut pending: ResMut<PendingStarts>,
    mut occupancy: ResMut<crate::VoiceOccupancy>,
    mut decisions: ResMut<StartDecisions>,
    bank: Option<Res<SoundBank>>,
    epoch: Res<crate::backend::MatchEpoch>,
) {
    let Some(bank) = bank else {
        if !crate::AudioSilent::active() {
            for event in events.read() {
                diag::warn!(
                    Audio,
                    "audio: UiPlaySound `{}` dropped — no SoundBank (typed gap)",
                    event.alias
                );
            }
        } else {
            events.clear();
        }
        return;
    };
    for event in events.read().filter(|event| !event.alias.is_empty()) {
        let outcome = play_alias_oneshot(
            &mut commands,
            &mut pcm_assets,
            &mut shared,
            &bank.0,
            alias_namespace(&bank.0, &event.alias),
            &event.alias,
            None,
            None,
            &mut pick,
            clips.as_deref_mut(),
            &mut pending,
            &mut occupancy,
            &mut decisions,
            Some(crate::SND_ENT_LOCAL),
            SoundClass::Ui,
            epoch.0,
        );
        if outcome.allows_binding_fallback() {
            gaps.record(&event.alias);
        }
    }
}

fn play_ui_music_messages(
    mut play_events: MessageReader<UiPlayMusic>,
    mut stop_events: MessageReader<UiStopMusic>,
    mut commands: Commands,
    mut looping_assets: ResMut<Assets<LoopingPcmAudio>>,
    mut gaps: ResMut<MissingAliasGaps>,
    mut desired: ResMut<PendingMenuBed>,
    mut clips: Option<ResMut<ClipStore>>,
    bank: Option<Res<SoundBank>>,
    playing: Query<(Entity, &MenuMusicBed)>,
) {
    if stop_events.read().count() > 0 {
        desired.alias = None;
        for (entity, _) in &playing {
            crate::backend::stop(&mut commands, entity);
        }
        diag::info!(Audio, "audio: menu bed stopped");
    }
    // The last request this frame wins.
    if let Some(event) = play_events.read().filter(|event| !event.alias.is_empty()).last() {
        desired.alias = Some(event.alias.clone());
    }

    let Some(alias) = desired.alias.clone() else {
        return;
    };
    let (Some(bank), Some(clips)) = (bank, clips.as_mut()) else {
        return;
    };
    if playing.iter().any(|(_, bed)| bed.alias == alias) {
        return;
    }
    for (entity, _) in &playing {
        crate::backend::stop(&mut commands, entity);
    }

    let namespace = alias_namespace(&bank.0, &alias);
    let Some(key) = clip_keys_for_alias(&bank.0, namespace, &alias)
        .into_iter()
        .next()
    else {
        gaps.record(&alias);
        desired.alias = None;
        return;
    };
    clips.request(key.clone());
    let Some(decoded) = clips.ready(&key) else {
        return;
    };
    let Ok(pcm) = decoded else {
        gaps.record(&alias);
        desired.alias = None;
        return;
    };

    // A Black Ops II bed plays at its alias's volume, through its pan, in its
    // menu category (music). Anything else gets a fixed default level.
    let row = (namespace == AssetNamespace::T6)
        .then(|| bank.0.sound_in(namespace, &alias))
        .flatten()
        .and_then(|sound| sound.aliases.first());
    let volume = row.map_or(0.55, |row| row.vol_min.max(0.0));
    let pcm = crate::pcm::with_stereo_pan(pcm, row.and_then(|row| row.stereo_pan));
    let handle = looping_assets.add(pcm.into_looping());
    let entity = crate::backend::spawn_loop(
        &mut commands,
        handle,
        Volume::Linear(volume),
        0,
        crate::backend::AudioScope::Menu,
    );
    commands.entity(entity).insert(MenuMusicBed {
        alias: alias.clone(),
    });
    if let Some(category) = row.and_then(|row| row.menu_category) {
        commands.entity(entity).insert(crate::MenuVolume {
            category,
            base: volume,
        });
    }
    diag::info!(Audio, "audio: menu bed `{alias}`");
}
