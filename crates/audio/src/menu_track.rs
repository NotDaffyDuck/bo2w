//! The menu's sound: a frontend bank built off-thread at startup, restored
//! whenever the player returns from a match, plus the UI sound and music
//! messages the menu sends.

use std::sync::Arc;

use asset_audio::{AudioRoster, read_mp_sound_bank};
use asset_core::AssetCategoryScope;
use asset_transport::GameInstallDir;
use assets::{CategorySoundArchive, CategoryTreeSet};
use bevy::audio::Volume;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::tasks::futures_lite::future;
use bevy::tasks::{AsyncComputeTaskPool, Task, TaskPool};
use frame::{ClientStep, StartupPersona, ReturnedToMenu};
use frame::{MenuSongStart, MenuCueStart, MenuSongHalt, register_ui_contracts};

use crate::map_sound_set::SoundArchive;
use crate::clip_bank::{QueuedVoiceStarts, waveforms_for_cue};
use crate::sample_feed::{LoopedPcm, ClipPcm, with_stereo_pan};
use crate::play_core::{UnknownCueHoles, CommonPlaybackStore, AudioCatalogRes, CuePickState};
use crate::play_core::fire_single_cue;
use crate::playback_gate::PlaybackVerdictLog;
use crate::{ClipRepository, NoiseCategory};

/// The resident menu sound bank and the loose-file index its clips come from.
#[derive(Resource, Clone)]
pub struct FrontendAudio {
    pub bank: Arc<AudioRoster>,
    pub iwd: Arc<CategorySoundArchive>,
}

const MENU_AUDIO_ZONE: &str = "t6:code_post_gfx_mp";

/// The background build of the frontend bank.
#[derive(Resource)]
struct FrontendAudioPrepare(Task<FrontendAudioWalked>);

/// Marks a frontend bank that failed to build, so it is not walked again every
/// frame (an install without the zone can never produce one).
#[derive(Resource)]
struct FrontendAudioGap;

/// What the worker thread hands back: the bank (if it built), the loose-file
/// index, and a note per problem it met on the way.
struct FrontendAudioWalked {
    iwd: Arc<CategorySoundArchive>,
    notes: Vec<String>,
    bank: Option<Arc<AudioRoster>>,
}

/// The menu music loop that is playing.
#[derive(Component)]
struct MenuBedVoice {
    track: String,
}

/// The menu music the UI last asked for, whether or not it has started yet.
#[derive(Resource, Default)]
struct WantedMenuBed {
    wanted: Option<String>,
}

/// Whether the frontend bank is already resolved one way or the other.
#[derive(SystemParam)]
struct MenuBankProgress<'w> {
    resident: Option<Res<'w, FrontendAudio>>,
    walking: Option<Res<'w, FrontendAudioPrepare>>,
    gave_up: Option<Res<'w, FrontendAudioGap>>,
    silent: Option<Res<'w, crate::AudioSilent>>,
}

impl MenuBankProgress<'_> {
    fn nothing_to_start(&self) -> bool {
        let flags = [
            self.resident.is_some(),
            self.walking.is_some(),
            self.gave_up.is_some(),
            self.silent.is_some(),
        ];
        flags.contains(&true)
    }
}

/// Everything that means a match owns the disk right now.
#[derive(SystemParam)]
struct MatchLoadPressure<'w> {
    busy: Option<Res<'w, assets::SessionLoadOccupied>>,
    asked: Option<Res<'w, assets::SessionLoadOrder>>,
    accepted: Option<Res<'w, assets::SessionLoadApproved>>,
    prepared: Option<Res<'w, assets::ReadiedSessionSignal>>,
    composing: Option<Res<'w, crate::map_sound_set::SoundBankCompose>>,
}

impl MatchLoadPressure<'_> {
    fn is_loading(&self) -> bool {
        let working = self.busy.as_ref().map(|flag| flag.0).unwrap_or(false);
        working
            || [
                self.asked.is_some(),
                self.accepted.is_some(),
                self.prepared.is_some(),
                self.composing.is_some(),
            ]
            .into_iter()
            .any(|present| present)
    }
}

/// The state a one-shot UI sound is played through.
#[derive(SystemParam)]
struct OneShotState<'w> {
    pcm: ResMut<'w, Assets<ClipPcm>>,
    shared: ResMut<'w, CommonPlaybackStore>,
    pick: ResMut<'w, CuePickState>,
    gaps: ResMut<'w, UnknownCueHoles>,
    store: Option<ResMut<'w, ClipRepository>>,
    queued: ResMut<'w, QueuedVoiceStarts>,
    voices: ResMut<'w, crate::ChannelUsageTally>,
    decisions: ResMut<'w, PlaybackVerdictLog>,
}

pub(crate) fn install_menu_sound(host: &mut App) {
    register_ui_contracts(host);
    host.add_message::<ReturnedToMenu>();
    host.init_resource::<WantedMenuBed>();
    let loading = (
        begin_menu_bank_walk,
        adopt_menu_bank_walk,
        reinstate_menu_bank.after(crate::map_sound_set::stop_map_ambient_on_match_end),
    )
        .chain();
    host.add_systems(Update, loading.in_set(ClientStep::Load));
    host.add_systems(
        Update,
        menu_click_sounds
            .after(crate::voice_slots::free_spent_channels)
            .in_set(ClientStep::Effects),
    );
    host.add_systems(Update, menu_bed_driver.in_set(ClientStep::Effects));
}

/// Builds the bank and loose-file index on a worker thread.
async fn walk_menu_install(games: GameInstallDir) -> FrontendAudioWalked {
    let (bank, mut notes) = match read_mp_sound_bank(&games, MENU_AUDIO_ZONE) {
        Ok(zone_path) => {
            let gap_notes = zone_path.hole_rows();
            (Some(Arc::new(zone_path.catalog)), gap_notes)
        }
        Err(why) => (None, vec![format!("frontend sound bank: {why}")]),
    };
    let trees = CategoryTreeSet::discover(&games);
    let (iwd, opened) = CategorySoundArchive::open(&trees);
    notes.extend(opened);
    FrontendAudioWalked {
        iwd: Arc::new(iwd),
        notes,
        bank,
    }
}

fn begin_menu_bank_walk(
    progress: MenuBankProgress,
    pressure: MatchLoadPressure,
    identity: Option<Res<StartupPersona>>,
    mut commands: Commands,
) {
    // Leave the disk to a match that is loading or about to.
    if progress.nothing_to_start() || pressure.is_loading() {
        return;
    }
    let root = identity
        .map(|launch| launch.titles_install_dir.clone())
        .filter(|root| !root.as_os_str().is_empty());
    let Some(root) = root else {
        return;
    };
    let workers = AsyncComputeTaskPool::get_or_init(TaskPool::default);
    let walk = workers.spawn(walk_menu_install(GameInstallDir(root)));
    commands.insert_resource(FrontendAudioPrepare(walk));
}

fn adopt_menu_bank_walk(walk: Option<ResMut<FrontendAudioPrepare>>, mut cmds: Commands) {
    let Some(mut walk) = walk else {
        return;
    };
    let Some(finished) = future::block_on(future::poll_once(&mut walk.0)) else {
        return;
    };
    cmds.remove_resource::<FrontendAudioPrepare>();
    for note in &finished.notes {
        diag::info!(Audio, "sound: menu set scan: {note}");
    }
    let FrontendAudioWalked { iwd, bank, .. } = finished;
    let Some(snd_bank) = bank else {
        diag::warn!(
            Audio,
            "audio: the menu bank never built, so the menu stays silent (typed gap)"
        );
        cmds.insert_resource(FrontendAudioGap);
        return;
    };
    cmds.insert_resource(FrontendAudio { bank: snd_bank, iwd });
    diag::info!(Audio, "sound: menu set loaded");
}

fn reinstate_menu_bank(
    mut back: MessageReader<ReturnedToMenu>,
    resident: Option<Res<FrontendAudio>>,
    current: Option<Res<AudioCatalogRes>>,
    silent: Option<Res<crate::AudioSilent>>,
    mut commands: Commands,
) {
    let came_back = !back.is_empty();
    back.clear();
    if !came_back || silent.is_some() {
        return;
    }
    let Some(resident) = resident else {
        diag::warn!(
            Audio,
            "audio: the menu has no resident bank and runs without sound (typed gap)"
        );
        return;
    };
    if current.is_some_and(|live| Arc::ptr_eq(&live.0, &resident.bank)) {
        return;
    }
    let (bank, iwd) = (resident.bank.clone(), resident.iwd.clone());
    commands.insert_resource(ClipRepository::start(bank.clone(), Some(iwd.clone())));
    commands.insert_resource(SoundArchive(iwd));
    commands.insert_resource(AudioCatalogRes(Arc::clone(&bank)));
    commands.insert_resource(CommonPlaybackStore::default());
    commands.insert_resource(QueuedVoiceStarts::default());
    diag::info!(Audio, "sound: menu set reloaded post-match");
}

/// The game a menu alias belongs to. Black Ops II's front end and match banks
/// are T6 (looked up as legacy, every menu sound would be missing); T6 is also
/// the answer when no game has it.
fn menu_alias_game(bank: &AudioRoster, cue_id: &str) -> AssetCategoryScope {
    let in_t6 = bank.slot_in(AssetCategoryScope::T6, cue_id).is_some();
    let in_t5 = bank.slot_in(AssetCategoryScope::T5, cue_id).is_some();
    if in_t5 && !in_t6 {
        AssetCategoryScope::T5
    } else {
        AssetCategoryScope::T6
    }
}

fn menu_click_sounds(
    mut queued_reqs: MessageReader<MenuCueStart>,
    bank: Option<Res<AudioCatalogRes>>,
    game_generation: Res<crate::backend::GameRoundSerial>,
    mut commands: Commands,
    mut state: OneShotState,
) {
    let Some(snd_set) = bank else {
        if crate::AudioSilent::active() {
            queued_reqs.clear();
            return;
        }
        for dropped in queued_reqs.read() {
            diag::warn!(
                Audio,
                "audio: UiPlaySound `{}` has no SoundBank, dropped (typed gap)",
                dropped.alias
            );
        }
        return;
    };
    let wanted = queued_reqs
        .read()
        .map(|request| request.alias.as_str())
        .filter(|cue_name| !cue_name.is_empty());
    for alias in wanted {
        let result = fire_single_cue(
            &mut commands,
            &mut state.pcm,
            &mut state.shared,
            &snd_set.0,
            menu_alias_game(&snd_set.0, alias),
            alias,
            None,
            None,
            &mut state.pick,
            state.store.as_deref_mut(),
            &mut state.queued,
            &mut state.voices,
            &mut state.decisions,
            Some(crate::SOUND_SOURCE_SELF),
            NoiseCategory::Ui,
            game_generation.0,
        );
        if result.allows_binding_fallback() {
            state.gaps.note_miss(alias);
        }
    }
}

fn cut_beds(commands: &mut Commands, beds: &Query<(Entity, &MenuBedVoice)>) {
    for (bed, _) in beds {
        crate::backend::silence_cue(commands, bed);
    }
}

fn abandon_bed(gaps: &mut UnknownCueHoles, want: &mut WantedMenuBed, track: &str) {
    gaps.note_miss(track);
    want.wanted = None;
}

fn menu_bed_driver(
    mut stops: MessageReader<MenuSongHalt>,
    mut starts: MessageReader<MenuSongStart>,
    mut commands: Commands,
    mut gaps: ResMut<UnknownCueHoles>,
    mut want: ResMut<WantedMenuBed>,
    mut loops: ResMut<Assets<LoopedPcm>>,
    mut store: Option<ResMut<ClipRepository>>,
    bank: Option<Res<AudioCatalogRes>>,
    beds: Query<(Entity, &MenuBedVoice)>,
) {
    if !stops.is_empty() {
        stops.clear();
        want.wanted = None;
        cut_beds(&mut commands, &beds);
        diag::info!(Audio, "sound: menu music off");
    }
    // The last request this frame wins.
    let newest = starts
        .read()
        .map(|request| request.alias.as_str())
        .filter(|cue_key| !cue_key.is_empty())
        .last();
    if let Some(alias) = newest {
        want.wanted = Some(alias.to_owned());
    }

    let (Some(track), Some(menu_set), Some(store)) = (want.wanted.clone(), bank, store.as_deref_mut())
    else {
        return;
    };
    if beds.iter().any(|(_, bed)| bed.track == track) {
        return;
    }
    cut_beds(&mut commands, &beds);

    let game = menu_alias_game(&menu_set.0, &track);
    let Some(key) = waveforms_for_cue(&menu_set.0, game, &track).into_iter().next() else {
        abandon_bed(&mut gaps, &mut want, &track);
        return;
    };
    store.req_in(key.clone());
    let pcm = match store.ready_pcm(&key) {
        None => return,
        Some(Err(_undecodable)) => {
            abandon_bed(&mut gaps, &mut want, &track);
            return;
        }
        Some(Ok(pcm)) => pcm,
    };

    // A Black Ops II bed plays at its alias's volume, through its pan, in its
    // menu category (music). Anything else gets a fixed default level.
    let bank_row = menu_set
        .0
        .entry_in(game, &track)
        .filter(|_| game == AssetCategoryScope::T6)
        .and_then(|sound| sound.aliases.first());
    let level = bank_row.map_or(0.55, |alias_row| alias_row.volume_low.max(0.0));
    let panned = with_stereo_pan(pcm, bank_row.and_then(|alias_row| alias_row.stereo_pan));
    let source = loops.add(panned.looped());
    let voice = crate::backend::begin_repeating_cue(
        &mut commands,
        source,
        Volume::Linear(level),
        0,
        crate::backend::SoundAudience::Menu,
    );
    let mut bed = commands.entity(voice);
    bed.insert(MenuBedVoice {
        track: track.clone(),
    });
    if let Some(category) = bank_row.and_then(|alias_row| alias_row.menu_category) {
        bed.insert(crate::MenuVolume {
            cat_ix: category,
            base: level,
        });
    }
    diag::info!(Audio, "sound: menu music `{track}` on");
}

#[cfg(test)]
mod tests {
    use asset_audio::{SnapshotSoundAlias, SnapshotSoundEntry};

    use super::*;

    fn t6_catalog(names: &[&str]) -> Arc<AudioRoster> {
        let mut catalog = AudioRoster::default();
        for &name in names {
            catalog.add_sound(SnapshotSoundEntry {
                name: name.to_owned(),
                aliases: vec![SnapshotSoundAlias::default()],
            });
        }
        Arc::new(catalog)
    }

    fn resident(bank: &Arc<AudioRoster>) -> FrontendAudio {
        FrontendAudio {
            bank: Arc::clone(bank),
            iwd: Arc::new(CategorySoundArchive::default()),
        }
    }

    fn menu_app() -> App {
        let mut app = App::new();
        app.add_message::<ReturnedToMenu>()
            .add_message::<MenuCueStart>()
            .add_message::<MenuSongStart>()
            .add_message::<MenuSongHalt>()
            .init_resource::<WantedMenuBed>()
            .init_resource::<UnknownCueHoles>()
            .init_resource::<Assets<LoopedPcm>>();
        app
    }

    fn with_bank(app: &mut App, bank: &Arc<AudioRoster>) {
        app.insert_resource(AudioCatalogRes(Arc::clone(bank)))
            .insert_resource(ClipRepository::start(Arc::clone(bank), None));
    }

    fn with_play_resources(app: &mut App) {
        app.init_resource::<Assets<ClipPcm>>()
            .init_resource::<CommonPlaybackStore>()
            .init_resource::<CuePickState>()
            .init_resource::<QueuedVoiceStarts>()
            .init_resource::<crate::ChannelUsageTally>()
            .init_resource::<PlaybackVerdictLog>()
            .init_resource::<crate::backend::GameRoundSerial>()
            .add_systems(Update, menu_click_sounds);
    }

    fn beds(app: &mut App) -> Vec<String> {
        let mut query = app.world_mut().query::<&MenuBedVoice>();
        let mut found: Vec<String> = query
            .iter(app.world())
            .map(|bed| bed.track.clone())
            .collect();
        found.sort();
        found
    }

    fn back_to_menu(app: &mut App, swap_id: u64) {
        app.world_mut().write_message(ReturnedToMenu {
            swap_id,
            had_world: true,
        });
    }

    #[test]
    fn menu_aliases_resolve_in_t6() {
        let bank = t6_catalog(&["mouse_over"]);
        assert_eq!(menu_alias_game(&bank, "mouse_over"), AssetCategoryScope::T6);
        assert_eq!(menu_alias_game(&bank, "MOUSE_OVER"), AssetCategoryScope::T6);
        assert_eq!(menu_alias_game(&bank, "nothing"), AssetCategoryScope::T6);
    }

    #[test]
    fn returning_to_the_menu_puts_the_resident_bank_back() {
        let bank = t6_catalog(&["mouse_over"]);
        let mut app = menu_app();
        app.insert_resource(resident(&bank))
            .add_systems(Update, reinstate_menu_bank);
        app.update();
        assert!(!app.world().contains_resource::<AudioCatalogRes>());
        back_to_menu(&mut app, 1);
        app.update();
        let world = app.world();
        assert!(Arc::ptr_eq(&world.resource::<AudioCatalogRes>().0, &bank));
        assert!(Arc::ptr_eq(
            &world.resource::<SoundArchive>().0,
            &world.resource::<FrontendAudio>().iwd
        ));
        assert!(world.contains_resource::<ClipRepository>());
        assert!(world.contains_resource::<QueuedVoiceStarts>());
        assert!(world.contains_resource::<CommonPlaybackStore>());
    }

    #[test]
    fn a_bank_that_is_already_live_is_left_alone() {
        let bank = t6_catalog(&["mouse_over"]);
        let mut app = menu_app();
        app.insert_resource(resident(&bank))
            .insert_resource(AudioCatalogRes(Arc::clone(&bank)))
            .add_systems(Update, reinstate_menu_bank);
        back_to_menu(&mut app, 2);
        app.update();
        assert!(!app.world().contains_resource::<ClipRepository>());
        assert!(!app.world().contains_resource::<QueuedVoiceStarts>());
    }

    #[test]
    fn a_different_live_bank_is_replaced_and_no_frontend_means_no_change() {
        let bank = t6_catalog(&["mouse_over"]);
        let other = t6_catalog(&["other"]);
        let mut app = menu_app();
        app.insert_resource(AudioCatalogRes(Arc::clone(&other)))
            .add_systems(Update, reinstate_menu_bank);
        back_to_menu(&mut app, 3);
        app.update();
        assert!(Arc::ptr_eq(&app.world().resource::<AudioCatalogRes>().0, &other));
        app.insert_resource(resident(&bank));
        back_to_menu(&mut app, 4);
        app.update();
        assert!(Arc::ptr_eq(&app.world().resource::<AudioCatalogRes>().0, &bank));
    }

    #[test]
    fn a_silent_run_restores_nothing() {
        let bank = t6_catalog(&["mouse_over"]);
        let mut app = menu_app();
        app.insert_resource(resident(&bank))
            .insert_resource(crate::AudioSilent)
            .add_systems(Update, reinstate_menu_bank);
        back_to_menu(&mut app, 5);
        app.update();
        assert!(!app.world().contains_resource::<AudioCatalogRes>());
    }

    fn walked(bank: Option<Arc<AudioRoster>>, lines: &[&str]) -> FrontendAudioWalked {
        FrontendAudioWalked {
            bank,
            iwd: Arc::new(CategorySoundArchive::default()),
            notes: lines.iter().map(|line| (*line).to_owned()).collect(),
        }
    }

    fn finished(walk: FrontendAudioWalked) -> FrontendAudioPrepare {
        let workers = AsyncComputeTaskPool::get_or_init(TaskPool::default);
        FrontendAudioPrepare(workers.spawn(async move { walk }))
    }

    fn settle(app: &mut App) {
        for _ in 0..400 {
            app.update();
            if !app.world().contains_resource::<FrontendAudioPrepare>() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("the background walk never finished");
    }

    #[test]
    fn a_finished_walk_with_a_bank_makes_the_frontend_resident() {
        let bank = t6_catalog(&["mouse_over"]);
        let mut host = menu_app();
        host.add_systems(Update, adopt_menu_bank_walk)
            .insert_resource(finished(walked(Some(Arc::clone(&bank)), &["one", "two"])));
        settle(&mut host);
        let world = host.world();
        assert!(Arc::ptr_eq(&world.resource::<FrontendAudio>().bank, &bank));
        assert!(!world.contains_resource::<FrontendAudioGap>());
    }

    #[test]
    fn a_finished_walk_without_a_bank_marks_a_gap() {
        let mut host = menu_app();
        host.add_systems(Update, adopt_menu_bank_walk)
            .insert_resource(finished(walked(None, &["frontend sound bank: nope"])));
        settle(&mut host);
        let world = host.world();
        assert!(world.contains_resource::<FrontendAudioGap>());
        assert!(!world.contains_resource::<FrontendAudio>());
    }

    fn launched(root: &str) -> StartupPersona {
        StartupPersona {
            titles_install_dir: std::path::PathBuf::from(root),
            ..StartupPersona::default()
        }
    }

    fn prepare_app() -> App {
        let mut app = menu_app();
        app.insert_resource(launched("Z:/no/such/games"))
            .add_systems(Update, begin_menu_bank_walk);
        app
    }

    #[test]
    fn the_walk_does_not_start_while_something_blocks_it() {
        let mut blocked: Vec<App> = Vec::new();
        let mut silent = prepare_app();
        silent.insert_resource(crate::AudioSilent);
        blocked.push(silent);
        let mut gapped = prepare_app();
        gapped.insert_resource(FrontendAudioGap);
        blocked.push(gapped);
        let mut loading = prepare_app();
        loading.insert_resource(assets::SessionLoadOccupied(true));
        blocked.push(loading);
        let mut rootless = prepare_app();
        rootless.insert_resource(launched(""));
        blocked.push(rootless);
        let mut nameless = menu_app();
        nameless.add_systems(Update, begin_menu_bank_walk);
        blocked.push(nameless);
        let mut resident_already = prepare_app();
        resident_already.insert_resource(resident(&t6_catalog(&[])));
        blocked.push(resident_already);
        for (index, app) in blocked.iter_mut().enumerate() {
            app.update();
            assert!(
                !app.world().contains_resource::<FrontendAudioPrepare>(),
                "blocker {index} did not hold"
            );
        }
    }

    #[test]
    fn a_walk_over_a_missing_install_ends_in_a_gap() {
        let mut app = prepare_app();
        app.insert_resource(assets::SessionLoadOccupied(false))
            .add_systems(
                Update,
                adopt_menu_bank_walk.after(begin_menu_bank_walk),
            );
        app.update();
        // The walk runs on another thread: it may already have been adopted.
        let world = app.world();
        assert!(
            world.contains_resource::<FrontendAudioPrepare>()
                || world.contains_resource::<FrontendAudioGap>()
        );
        settle(&mut app);
        assert!(app.world().contains_resource::<FrontendAudioGap>());
        assert!(!app.world().contains_resource::<FrontendAudio>());
    }

    #[test]
    fn unknown_ui_sounds_are_recorded_once_and_empty_ones_ignored() {
        let bank = t6_catalog(&["mouse_over"]);
        let mut app = menu_app();
        with_bank(&mut app, &bank);
        with_play_resources(&mut app);
        for cue_id in ["", "no_such_click", "no_such_click"] {
            app.world_mut().write_message(MenuCueStart {
                alias: cue_id.to_owned(),
            });
        }
        app.update();
        assert_eq!(
            app.world().resource::<UnknownCueHoles>().aliases,
            ["no_such_click"]
        );
    }

    #[test]
    fn ui_sounds_without_a_bank_are_dropped_without_a_gap() {
        let mut app = menu_app();
        with_play_resources(&mut app);
        app.world_mut().write_message(MenuCueStart {
            alias: "mouse_over".to_owned(),
        });
        app.update();
        assert!(app.world().resource::<UnknownCueHoles>().is_empty());
    }

    fn send_music(app: &mut App, cue_id: &str) {
        app.world_mut().write_message(MenuSongStart {
            alias: cue_id.to_owned(),
        });
    }

    #[test]
    fn the_last_music_request_of_a_frame_is_remembered_without_a_bank() {
        let mut host = menu_app();
        host.add_systems(Update, menu_bed_driver);
        send_music(&mut host, "first");
        send_music(&mut host, "");
        send_music(&mut host, "second");
        host.update();
        assert_eq!(
            host.world().resource::<WantedMenuBed>().wanted.as_deref(),
            Some("second")
        );
    }

    #[test]
    fn stopping_music_clears_the_request_and_removes_every_bed() {
        let mut host = menu_app();
        host.add_systems(Update, menu_bed_driver);
        for alias in ["bed_a", "bed_b"] {
            host.world_mut().spawn(MenuBedVoice {
                track: alias.to_owned(),
            });
        }
        host.world_mut().resource_mut::<WantedMenuBed>().wanted = Some("bed_a".to_owned());
        host.world_mut().write_message(MenuSongHalt);
        host.update();
        assert!(host.world().resource::<WantedMenuBed>().wanted.is_none());
        assert!(beds(&mut host).is_empty());
    }

    #[test]
    fn the_bed_already_playing_is_kept_and_a_new_one_replaces_it() {
        let bank = t6_catalog(&[]);
        let mut host = menu_app();
        with_bank(&mut host, &bank);
        host.add_systems(Update, menu_bed_driver);
        host.world_mut().spawn(MenuBedVoice {
            track: "bed_a".to_owned(),
        });
        send_music(&mut host, "bed_a");
        host.update();
        assert_eq!(beds(&mut host), ["bed_a"]);
        send_music(&mut host, "missing_bed");
        host.update();
        assert!(beds(&mut host).is_empty());
        let world = host.world();
        assert!(world.resource::<WantedMenuBed>().wanted.is_none());
        assert_eq!(
            world.resource::<UnknownCueHoles>().aliases,
            ["missing_bed"]
        );
    }
}
