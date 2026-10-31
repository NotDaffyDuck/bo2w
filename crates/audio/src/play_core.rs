//! Starting sounds: the systems that turn alias messages into voices, the
//! queue for sounds still waiting on a decode, and the 3D placement maths.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::f32::consts::FRAC_PI_4;
use std::ops::ControlFlow;
use std::sync::Arc;
use std::time::Instant;

use asset_audio::{SnapshotSoundAlias, AudioRoster, mix_interval, unit_random};
use asset_core::AssetCategoryScope;
use bevy::{
    audio::{AudioSink, AudioSinkPlayback, AddAudioSource, Volume},
    ecs::system::SystemParam,
    prelude::*,
};
use frame::{ClientStep, EffectAudioPosted, GameSessionClosed, MatchSwapDone};
use net::{NewestTakenSnapshot, HostLocalSoundMsg};

use crate::attenuation::{CURVE_KNOT_CAP, distance_attenuation};
use crate::backend::GameRoundSerial;
use crate::clip_bank::{
    WaveformHandle, ClipRepository, QueuedSingleCue, QueuedVoiceStarts, waveform_for_variant, waveforms_for_cue,
    due_time_for,
};
use crate::alerts::{
    AliasCommand, BoundWeaponSound, FootfallMsg, TouchdownNoiseMsg, TriggerCueMsg, SOUND_SOURCE_SELF,
    FirstPersonAnimCues, GunNoiseMsg,
};
use crate::sample_feed::{PanHandle, LoopedPcm, ClipPcm};
use crate::distance::{gap_in_inches, inches_of};
use crate::playback_gate::NoiseCategory;
use crate::playback_gate::{PlaybackVerdictEntry, PlaybackVerdictLog, PlaybackBlocker, PlaybackVerdict, MuteCause};
use crate::voice_slots::{ChannelBooking, ChannelUsageTally, has_free_voice, free_spent_channels};

/// How many layers deep a sound may chain to another before the chain is cut.
const LAYER_DEPTH_CAP: u8 = 10;

/// Where the random pick state starts.
const PICK_SEED: u32 = 0x00a5_5a5a;

/// Marks the entity whose transform is the player's ear.
#[derive(Component, Default)]
pub struct BackgroundHearer;

/// A positional sound that is retuned every frame from where the ear is.
#[derive(Component)]
pub struct SpatialVoice {
    pub position_in_inches: [f32; 3],
    pub reach_lower: f32,
    pub reach_upper: f32,
    pub knots: Arc<[[f32; 2]]>,
    pub near_knots: Option<Arc<[[f32; 2]]>>,
    pub priority: Option<asset_audio::VoicePriority>,
    pub rest_volume: f32,

    pub active_stereo_handle: PanHandle,
    /// bo2mp: a Black Ops II 3D sound with a pan of its own keeps those stereo
    /// gains: it falls off with distance but does not turn with the listener.
    pub fixed_pan: Option<(f32, f32)>,
    /// bo2mp: the Black Ops II menu volume this sound answers to.
    pub category: Option<u8>,
}

impl SpatialVoice {
    /// What the falloff leaves of the sound at `distance`. A channel with no
    /// curve at all is silent.
    fn falloff_at(&self, distance: f32) -> f32 {
        if self.knots.is_empty() {
            return 0.0;
        }
        let near = self.near_knots.as_deref();
        distance_attenuation(&self.knots, near, distance, self.reach_lower, self.reach_upper).max(0.0)
    }

    /// Turns the sound toward where it now sits relative to the ear.
    fn face(&self, ear: Vec3, right: Vec3) {
        let source = Vec3::from_array(self.position_in_inches);
        let (to_left, to_right) = oneshot_lr_gains(ear, right, source, 1.0);
        self.active_stereo_handle.store(to_left, to_right);
    }
}

#[derive(Resource, Clone)]
pub struct AudioCatalogRes(pub Arc<AudioRoster>);

/// Random state for choosing between an alias's variants.
#[derive(Resource)]
pub struct CuePickState {
    recent: HashMap<(AssetCategoryScope, String), usize>,
    rng: u32,
}

impl Default for CuePickState {
    fn default() -> Self {
        let (recent, rng) = (HashMap::default(), PICK_SEED);
        Self { recent, rng }
    }
}

/// What a playing voice was started for, so a later stop can find it.
#[derive(Component)]
pub struct AliasPlayback {
    pub namespace: AssetCategoryScope,
    pub source_entity_no: Option<u32>,
    pub alias: String,
}

/// Aliases that were asked for and could not be found, each listed once.
#[derive(Resource, Default, Debug)]
pub struct UnknownCueHoles {
    pub aliases: Vec<String>,
}

impl UnknownCueHoles {
    pub fn note_miss(&mut self, cue_name: &str) {
        if self.aliases.iter().all(|listed| listed != cue_name) {
            diag::warn!(Audio, "sound: no cue named `{cue_name}` (typed gap)");
            self.aliases.push(cue_name.to_string());
        }
    }

    pub fn len(&self) -> usize {
        self.aliases.as_slice().len()
    }

    pub fn is_empty(&self) -> bool {
        self.aliases.first().is_none()
    }
}

/// Assets shared by every play of the same clip or falloff curve.
#[derive(Resource, Default)]
pub(crate) struct CommonPlaybackStore {
    dry: HashMap<WaveformHandle, Handle<ClipPcm>>,
    falloff_curves: HashMap<String, Arc<[[f32; 2]]>>,
    common_dry_reuses: usize,
}

impl CommonPlaybackStore {
    /// The asset for a clip's unpanned sound: the one already made this match,
    /// else the resident one from a previous match, else a new one.
    fn plain_handle(
        &mut self,
        key: &WaveformHandle,
        sound: ClipPcm,
        assets: &mut Assets<ClipPcm>,
        store: Option<&ClipRepository>,
    ) -> Handle<ClipPcm> {
        if let Some(made) = self.dry.get(key) {
            return made.clone();
        }
        let fresh = match self.borrow_resident(key, &sound, assets, store) {
            Some(resident) => resident,
            None => assets.add(sound),
        };
        self.dry.entry(key.clone()).or_insert(fresh).clone()
    }

    /// The previous match's asset for the clip, if the store still holds one.
    fn borrow_resident(
        &mut self,
        key: &WaveformHandle,
        sound: &ClipPcm,
        assets: &mut Assets<ClipPcm>,
        store: Option<&ClipRepository>,
    ) -> Option<Handle<ClipPcm>> {
        let (resident, reused) = store?.resident_dry_handle(key, assets, sound)?;
        if reused {
            self.count_reuse(key);
        }
        Some(resident)
    }

    fn count_reuse(&mut self, reel: &WaveformHandle) {
        if self.common_dry_reuses == 0 {
            diag::info!(Audio, "sound: dry handle {reel:?} shared again");
        }
        self.common_dry_reuses += 1;
    }

    /// One shared copy of a named falloff curve, cut to the engine's knot limit.
    pub(crate) fn curve_knots(&mut self, name: &str, points: &[(f32, f32)]) -> Arc<[[f32; 2]]> {
        let shared = self.falloff_curves.entry(name.to_owned()).or_insert_with(|| {
            let kept = points.len().min(CURVE_KNOT_CAP);
            points[..kept].iter().map(|&(at, level)| [at, level]).collect()
        });
        Arc::clone(shared)
    }
}

pub(crate) struct PlayerNoisePlugin;

impl Plugin for PlayerNoisePlugin {
    fn build(&self, app: &mut App) {
        register_audio_sources(app);
        register_start_state(app);
        register_message_types(app);
        register_effect_systems(app);
        register_load_systems(app);
    }
}

fn register_audio_sources(app: &mut App) {
    if !app.is_plugin_added::<bevy::audio::AudioPlugin>() {
        // BO2MP_HEADLESS=1: there is no sound device, so nothing plays the
        // clips. They stay assets and the volume setting keeps a home.
        app.init_asset::<ClipPcm>();
        app.init_asset::<LoopedPcm>();
        app.init_resource::<bevy::audio::GlobalVolume>();
        return;
    }
    app.add_audio_source::<ClipPcm>();
    app.add_audio_source::<LoopedPcm>();
    crate::mix_probe::install(app);
}

fn register_start_state(app: &mut App) {
    app.init_resource::<CuePickState>();
    app.init_resource::<crate::clip_bank::ResidentClipCache>();
    app.init_resource::<UnknownCueHoles>();
    app.init_resource::<CommonPlaybackStore>();
    app.init_resource::<PlaybackVerdictLog>();
    app.init_resource::<QueuedVoiceStarts>();
    app.init_resource::<ChannelUsageTally>();
    app.init_resource::<crate::ent_sounds::NotetrackSoundTable>();
    // Ambient bed and the loops that run beside the one-shots.
    app.init_resource::<crate::map_sound_set::AmbientBedStarted>();
    app.init_resource::<crate::map_sound_set::BankLoadTried>();
    app.init_resource::<crate::map_sound_set::ResidentSoundBank>();
    app.init_resource::<crate::StrideRhythmWatch>();
    app.init_resource::<crate::WeaponLoops>();
    app.init_resource::<crate::slide_loop::SlideLoop>();
}

fn register_message_types(app: &mut App) {
    app.add_message::<AliasCommand>();
    app.add_message::<FootfallMsg>();
    app.add_message::<TouchdownNoiseMsg>();
    app.add_message::<GunNoiseMsg>();
    app.add_message::<BoundWeaponSound>();
    app.add_message::<FirstPersonAnimCues>();
}

/// The stages in which sounds get started each frame, in the order they run.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum StartPhase {
    /// Systems that feed the alias queue.
    Intake,
    /// Voices that ended last frame give their slots back.
    Reclaim,
    /// Alias messages become voices.
    Aliases,
    /// Starts that waited on a decode go out.
    Parked,
    /// Footsteps and landings.
    Surfaces,
    /// Weapon, notetrack and ADS sounds.
    Weapons,
}

fn register_effect_systems(host: &mut App) {
    use StartPhase::{Aliases, Intake, Parked, Reclaim, Surfaces, Weapons};
    use crate::map_sound_set::boot_ambient;
    let effects = || ClientStep::Effects;
    host.configure_sets(
        Update,
        (
            Intake,
            Reclaim,
            Aliases.after(EffectAudioPosted),
            Parked,
            Surfaces,
            Weapons,
        )
            .chain()
            .in_set(effects()),
    );
    host.add_systems(
        Update,
        (
            boot_ambient,
            crate::map_sound_set::play_random_point_sounds,
            route_local_sound_commands.run_if(resource_exists::<NewestTakenSnapshot>),
            crate::shock_sound::update_tinnitus
                .run_if(not(crate::shock_sound::bank_is_t6)),
        )
            .in_set(Intake),
    );
    // bo2zm: the Sound sliders' gains, before anything plays.
    host.add_systems(Update, crate::bus_gain::sync.in_set(Intake));
    host.add_systems(Update, free_spent_channels.in_set(Reclaim));
    host.add_systems(Update, start_alias_commands.in_set(Aliases));
    host.add_systems(Update, start_parked_sounds.in_set(Parked));
    host.add_systems(
        Update,
        (start_footsteps, start_landings).in_set(Surfaces),
    );
    host.add_systems(
        Update,
        (
            crate::ent_sounds::voice_first_person_anim_cues,
            crate::ads_sounds::play_ads_sounds,
            start_weapon_sounds,
            start_bound_weapon_sounds,
        )
            .chain()
            .in_set(Weapons),
    );
    // Loops, and the gain pass that follows the ducking.
    let emitter_gain = crate::map_sound_set::map_emitter_gain_tick;
    host.add_systems(
        Update,
        (
            (
                crate::destructible_loops::sync_destructible_loops,
                crate::weapon_loops::sync_weapon_loops,
            )
                .before(emitter_gain),
            // The slide's start sound is cut a frame after it began.
            crate::slide_loop::sync_slide_loop.after(Aliases).before(emitter_gain),
            crate::zm_music::sync_zm_music.after(crate::bus_gain::sync),
            // bo2zm: the down, dive-to-prone and Origins staff sounds, and the room echo.
            (
                crate::dive_vox::sync_dive_vox,
                crate::laststand::sync_laststand,
                crate::staffloop::sync_staff_loop,
                crate::room_echo::sync_room_echo,
            )
                .after(crate::bus_gain::sync)
                .before(Aliases),
            emitter_gain.after(crate::duck::DuckSet).after(crate::bus_gain::sync),
            retune_channels.after(crate::duck::DuckSet),
        )
            .in_set(effects()),
    );
}

fn register_load_systems(app: &mut App) {
    use crate::map_sound_set::{
        install_sound_bank, start_sound_bank_compose, stop_map_ambient_on_match_end,
    };
    app.add_systems(
        Update,
        (
            (
                stop_map_ambient_on_match_end.after(MatchSwapDone),
                start_sound_bank_compose,
                install_sound_bank,
                crate::ent_sounds::bind_notetrack_sounds,
                crate::map_sound_set::boot_silent_map_effects,
            )
                .chain(),
            forget_clip_prep_after_match,
        )
            .in_set(ClientStep::Load),
    );
}

fn forget_clip_prep_after_match(
    mut ended: MessageReader<GameSessionClosed>,
    mut waiting: ResMut<QueuedVoiceStarts>,
    mut assets: ResMut<CommonPlaybackStore>,
) {
    if ended.read().count() == 0 {
        return;
    }
    waiting.clear();
    let CommonPlaybackStore {
        common_dry_reuses: reused,
        ..
    } = std::mem::take(&mut *assets);
    diag::info!(
        Audio,
        "audio: common dry handles reused this match={reused}"
    );
}

/// The ear's position and its right-hand direction. Two ears is a bug.
fn listener_pose(ears: &Query<&Transform, With<BackgroundHearer>>) -> Option<(Vec3, Vec3)> {
    let mut found = ears.iter();
    let only = found.next()?;
    assert!(found.next().is_none(), "more than one ambient listener");
    Some((only.translation, only.rotation * Vec3::X))
}

/// Retunes every live positional sound: priority, falloff, pan and level.
fn retune_channels(
    ears: Query<&Transform, With<BackgroundHearer>>,
    settings: Res<frame::PlayerPrefsBundle>,
    ducks: Res<crate::DuckMixer>,
    mut occupancy: ResMut<ChannelUsageTally>,
    mut channels: Query<(
        Entity,
        &SpatialVoice,
        &mut AudioSink,
        Option<&crate::DuckGroup>,
    )>,
) {
    let Some((listener_at, right)) = listener_pose(&ears) else {
        return;
    };
    let ear_at = inches_of(listener_at);
    for (ent_id, channel, mut sink, group) in &mut channels {
        let distance = gap_in_inches(ear_at, channel.position_in_inches);
        if let Some(curve) = &channel.priority {
            occupancy.update_alias_priority(ent_id, curve.priority_for(Some(distance)));
        }
        if channel.fixed_pan.is_none() {
            channel.face(listener_at, right);
        }
        let menu = settings.menu_volume(channel.category) * ducks.linear(group.map(|g| g.0));
        let level = channel.rest_volume * channel.falloff_at(distance) * settings.master_gain();
        sink.set_volume(Volume::Linear((level * menu).max(0.0)));
    }
}

/// Plays or stops the sounds the server names by configstring index.
fn route_local_sound_commands(
    mut incoming: MessageReader<HostLocalSoundMsg>,
    snapshot: Res<NewestTakenSnapshot>,
    announcer: Res<crate::match_voices::AnnouncerRoutes>,
    mut play: MessageWriter<crate::AliasCommand>,
) {
    for command in incoming.read() {
        let Some(name) = snapshot.sound_cue_name(command.index) else {
            diag::warn!(
                Audio,
                "audio: configstring {} of a local sound names nothing (typed gap)",
                command.index
            );
            continue;
        };
        // bo2zm: a game's own alias (`t6:name`, as the zombies staffs send) goes to that game;
        // the rest through the announcer routes.
        let own_game = name.split_once(':').and_then(|(ns, alias)| {
            asset_core::AssetCategoryScope::parse(ns).map(|ns| (ns, alias))
        });
        let (namespace, alias) = own_game.unwrap_or_else(|| announcer.route(name));
        play.write(local_sound_message(
            namespace,
            alias.to_owned(),
            command.stop,
        ));
    }
}

/// The message that carries a server-named sound to the entity of the local
/// player: a stop when the server asked for one, otherwise a plain play.
fn local_sound_message(namespace: AssetCategoryScope, cue_name: String, stop: bool) -> AliasCommand {
    let source_entity_no = Some(SOUND_SOURCE_SELF);
    match stop {
        true => AliasCommand::Stop {
            namespace,
            alias: cue_name,
            source_entity_no,
        },
        false => AliasCommand::Play(bare_play(namespace, cue_name, source_entity_no)),
    }
}

/// A request to play `alias` that is not placed in the world and has no stand-in.
fn bare_play(namespace: AssetCategoryScope, cue_key: String, source_entity_no: Option<u32>) -> TriggerCueMsg {
    TriggerCueMsg {
        cue_ns: namespace,
        alias: cue_key,
        source_entity_no,
        fallback: Option::default(),
        position_in_inches: Option::default(),
    }
}

/// bo2zm: what a legacy movement sound becomes on a Black Ops II map.
enum T6Movement {
    /// Play BO2's own sound of that name.
    Play(&'static str),
    /// Nothing: BO2 chains the gear sound off each step alias's secondary, and
    /// has none for some (walking gear, quiet steps).
    Skip,
    /// Not a movement sound, or not a BO2 map.
    NotT6,
}

fn t6_movement(bank: &AudioRoster, namespace: AssetCategoryScope, cue_id: &str) -> T6Movement {
    let t6_map = namespace == AssetCategoryScope::T6
        && bank
            .slot_in(AssetCategoryScope::T6, "fly_step_run_plr_default")
            .is_some();
    let own_name = t6_map
        .then(|| crate::cue_names::t6_movement_alias(cue_id))
        .flatten();
    match own_name {
        None => T6Movement::NotT6,
        Some(name) if name.starts_with("fly_gear_") && !name.starts_with("fly_gear_mantle") => {
            T6Movement::Skip
        }
        Some(name) if bank.slot_in(AssetCategoryScope::T6, name).is_some() => T6Movement::Play(name),
        Some(_) => T6Movement::Skip,
    }
}

/// Where a movement alias really plays, or `None` when it is dropped.
fn movement_target<'x>(
    bank: &AudioRoster,
    namespace: AssetCategoryScope,
    alias: &'x str,
) -> Option<(AssetCategoryScope, &'x str)> {
    match t6_movement(bank, namespace, alias) {
        T6Movement::Play(name) => Some((AssetCategoryScope::T6, name)),
        T6Movement::Skip => None,
        T6Movement::NotT6 => Some((namespace, alias)),
    }
}

/// Everything the sound-starting systems share, so each one declares a single
/// parameter instead of a dozen.
#[derive(SystemParam)]
struct SoundDesk<'w, 's> {
    commands: Commands<'w, 's>,
    pcm_assets: ResMut<'w, Assets<ClipPcm>>,
    shared: ResMut<'w, CommonPlaybackStore>,
    pick: ResMut<'w, CuePickState>,
    gaps: ResMut<'w, UnknownCueHoles>,
    clips: Option<ResMut<'w, ClipRepository>>,
    queued: ResMut<'w, QueuedVoiceStarts>,
    occupancy: ResMut<'w, ChannelUsageTally>,
    decisions: ResMut<'w, PlaybackVerdictLog>,
    bank: Option<Res<'w, AudioCatalogRes>>,
    epoch: Res<'w, GameRoundSerial>,
    listeners: Query<'w, 's, &'static Transform, With<BackgroundHearer>>,
}

impl<'w, 's> SoundDesk<'w, 's> {
    /// Splits the desk into a stage for starting sounds and the list of
    /// missing aliases. `None` while there is no sound bank.
    fn open<'a>(&'a mut self) -> Option<(Stage<'a, 'w, 's>, &'a mut UnknownCueHoles)> {
        let catalog: &AudioRoster = self.bank.as_ref().map(|bank| &*bank.0)?;
        let pose = listener_pose(&self.listeners);
        let stock = Stock::new(
            &mut self.pcm_assets,
            &mut self.shared,
            self.clips.as_deref_mut(),
        );
        let books = Books::new(
            &mut self.pick,
            &mut self.queued,
            &mut self.occupancy,
            &mut self.decisions,
        );
        let stage = Stage::over(
            &mut self.commands,
            catalog,
            stock,
            books,
            pose,
            self.epoch.0,
        );
        Some((stage, &mut *self.gaps))
    }

    /// Ends the voices a `Stop` names, and cancels starts still waiting on a
    /// decode for the same sound.
    fn stop_alias(&mut self, namespace: AssetCategoryScope, cue_id: &str, source_entity_no: Option<u32>) {
        let epoch = self.epoch.0;
        self.queued.cancel_alias(namespace, cue_id, source_entity_no, epoch);
        let target = AliasPlayback {
            namespace,
            source_entity_no,
            alias: cue_id.to_owned(),
        };
        // Queued rather than done now, so it lands after the spawns this same
        // batch already issued; a Play that comes later still makes a new voice.
        self.commands.queue(move |ecs: &mut World| {
            let mut voices = ecs.query::<(Entity, &AliasPlayback, &crate::backend::PlayingCue)>();
            let doomed: Vec<Entity> = voices
                .iter(ecs)
                .filter_map(|(entity, tag, voice)| {
                    let same_sound = (tag.namespace, &tag.alias, tag.source_entity_no)
                        == (target.namespace, &target.alias, target.source_entity_no);
                    let in_match = voice.scope == crate::backend::SoundAudience::Match;
                    (same_sound && in_match && voice.cue_epoch == epoch).then_some(entity)
                })
                .collect();
            doomed.into_iter().for_each(|ent| {
                ecs.despawn(ent);
            });
        });
    }
}

/// What a batch of starts draws on: the asset stores and the clip store.
struct Stock<'a> {
    pcm_assets: &'a mut Assets<ClipPcm>,
    shared: &'a mut CommonPlaybackStore,
    clips: Option<&'a mut ClipRepository>,
}

impl<'a> Stock<'a> {
    fn new(
        pcm_store: &'a mut Assets<ClipPcm>,
        shared: &'a mut CommonPlaybackStore,
        clips: Option<&'a mut ClipRepository>,
    ) -> Self {
        Self {
            pcm_assets: pcm_store,
            shared,
            clips,
        }
    }
}

/// What a batch of starts keeps count in: picks, waiting starts, live voices
/// and the decision log.
struct Books<'a> {
    pick: &'a mut CuePickState,
    pending: &'a mut QueuedVoiceStarts,
    occupancy: &'a mut ChannelUsageTally,
    decisions: &'a mut PlaybackVerdictLog,
}

impl<'a> Books<'a> {
    fn new(
        pick: &'a mut CuePickState,
        pending: &'a mut QueuedVoiceStarts,
        occupancy: &'a mut ChannelUsageTally,
        decisions: &'a mut PlaybackVerdictLog,
    ) -> Self {
        Self {
            pick,
            pending,
            occupancy,
            decisions,
        }
    }
}

/// The borrowed parts of the world that one batch of sound starts works on.
struct Stage<'a, 'w, 's> {
    commands: &'a mut Commands<'w, 's>,
    bank: &'a AudioRoster,
    stock: Stock<'a>,
    books: Books<'a>,
    /// The ear's position and right-hand direction, when there is an ear.
    pose: Option<(Vec3, Vec3)>,
    /// The match the stage is working in.
    gen_no: u64,
}

/// One sound to start.
#[derive(Clone, Copy)]
struct Want<'a> {
    namespace: AssetCategoryScope,
    alias: &'a str,
    /// The alias's index in the bank, when the caller already bound it.
    bound: Option<usize>,
    spot: Option<[f32; 3]>,
    source_entity_no: Option<u32>,
    class: NoiseCategory,
    tilt_multiplier: f32,
    epoch: u64,
}

impl<'a> Want<'a> {
    fn new(namespace: AssetCategoryScope, cue_id: &'a str, class: NoiseCategory, epoch: u64) -> Self {
        Self {
            namespace,
            alias: cue_id,
            bound: None,
            class,
            spot: None,
            tilt_multiplier: 1.0,
            source_entity_no: None,
            epoch,
        }
    }

    /// The same request, placed in the world and tied to a sound entity.
    fn at(self, eye_pos: Option<[f32; 3]>, source_entity_no: Option<u32>) -> Self {
        Self {
            spot: eye_pos,
            source_entity_no,
            ..self
        }
    }

    fn pitched(self, tilt_multiplier: f32) -> Self {
        Self {
            tilt_multiplier,
            ..self
        }
    }

    fn bound_to(self, index: usize) -> Self {
        Self {
            bound: Some(index),
            ..self
        }
    }

    /// The request a waiting start was parked with.
    fn from_parked(row: &'a QueuedSingleCue) -> Self {
        Self::new(row.namespace, &row.alias, row.class, row.gen_tag)
            .at(row.position_in_inches, row.source_entity_no)
            .with_binding(row.bound)
    }

    fn with_binding(self, bound: Option<usize>) -> Self {
        Self { bound, ..self }
    }

    /// This request as a start that waits for `clip` to decode.
    fn parked(&self, variant_ix: usize, reel: &WaveformHandle, tune: &Tune) -> QueuedSingleCue {
        let Self { class, epoch, .. } = *self;
        let due = due_time_for(class);
        QueuedSingleCue {
            namespace: self.namespace,
            alias: self.alias.to_owned(),
            bound: self.bound,
            variant_ix,
            gain: tune.gain,
            pitch_mul: tune.rate,
            position_in_inches: self.spot,
            source_entity_no: self.source_entity_no,
            reel: reel.clone(),
            mix_layer: tune.next_cue.clone(),
            class,
            gen_tag: epoch,
            due_at: due,
        }
    }
}

/// The volume and pitch a pick settled on, and the sound it chains to.
struct Tune {
    gain: f32,
    rate: f32,
    next_cue: Option<String>,
}

impl Tune {
    const NEUTRAL: Self = Self { gain: 1.0, rate: 1.0, next_cue: None };
}

/// A clip that decoded, with the numbers it will be played at.
struct Chosen<'a> {
    samples: ClipPcm,
    reel: &'a WaveformHandle,
    voice_ix: usize,
    gain: f32,
    pitch: f32,
    layer: Option<&'a str>,
}

impl<'a> Chosen<'a> {
    /// A parked start whose clip has now decoded.
    fn from_parked(entry: &'a QueuedSingleCue, samples: ClipPcm) -> Self {
        Self {
            samples,
            reel: &entry.reel,
            gain: entry.gain,
            voice_ix: entry.variant_ix,
            layer: entry.mix_layer.as_deref(),
            pitch: entry.pitch_mul,
        }
    }
}

/// How far a 3D sound is from the ear and what its falloff leaves of it.
struct Reach {
    dist: f32,
    reach_lower: f32,
    reach_upper: f32,
    atten: f32,
}

impl Reach {
    fn measure(row: &SnapshotSoundAlias, dist: f32, atten: f32) -> Self {
        let (reach_lower, reach_upper) = (row.reach_lower, row.reach_upper);
        Self {
            dist,
            reach_lower,
            reach_upper,
            atten,
        }
    }

    fn describe(&self) -> String {
        falloff_detail(self.dist, self.reach_lower, self.reach_upper, self.atten)
    }
}

/// Where a 3D sound sits and how loud its falloff leaves it.
struct Placed {
    pos: [f32; 3],
    reach: Reach,
    knots: Arc<[[f32; 2]]>,
    near_knots: Option<Arc<[[f32; 2]]>>,
    fixed_pan: Option<(f32, f32)>,
    gains: (f32, f32),
}

/// What asking the clip store for a clip came to.
enum ClipFetch {
    Decoded(ClipPcm),
    Pending,
    Declined(PlaybackBlocker),
}

/// What one attempt to start a sound came to.
struct StartReport {
    result: PlaybackVerdict,
    picked: Option<usize>,
    layered: Option<(String, PlaybackVerdict)>,
    detail: Option<String>,
}

impl StartReport {
    fn of(result: PlaybackVerdict, picked: Option<usize>) -> Self {
        Self {
            result,
            picked,
            layered: None,
            detail: None,
        }
    }

    fn failed(why: PlaybackBlocker) -> Self {
        Self::of(PlaybackVerdict::Failed(why), None)
    }

    fn with_variant(self, pick_ix: usize) -> Self {
        Self {
            picked: Some(pick_ix),
            ..self
        }
    }

    fn chained(self, layered: Option<(String, PlaybackVerdict)>) -> Self {
        Self { layered, ..self }
    }

    fn with_detail(self, detail: Option<String>) -> Self {
        Self { detail, ..self }
    }

    fn suppressed(reason: MuteCause, pick_ix: usize, detail: Option<String>) -> Self {
        Self::of(PlaybackVerdict::Suppressed(reason), Some(pick_ix)).with_detail(detail)
    }
}

fn falloff_detail(dist: f32, reach_lower: f32, reach_upper: f32, atten: f32) -> String {
    format!("dist={dist:.0} dist_min={reach_lower:.0} dist_max={reach_upper:.0} atten={atten:.3}")
}

fn sound_for<'b>(
    bank: &'b AudioRoster,
    want: &Want<'_>,
) -> Option<&'b asset_audio::SnapshotSoundEntry> {
    match want.bound {
        Some(index) => bank.sound_at(index),
        None => bank.entry_in(want.namespace, want.alias),
    }
}

/// Whether the sound sits in the world rather than in the player's head.
fn is_positional(
    bank: &AudioRoster,
    namespace: AssetCategoryScope,
    row: Option<&SnapshotSoundAlias>,
    bay: Option<u32>,
) -> bool {
    // bo2zm: Black Ops II keeps Black Ops' pan-type bit (flags bit 1 = 3D).
    if matches!(namespace, AssetCategoryScope::T5 | AssetCategoryScope::T6) {
        return row
            .and_then(|row| row.alias_bits)
            .is_some_and(|flags| flags & 2 != 0);
    }
    bay.and_then(|bay| bank.actor_lane(bay))
        .is_none_or(|info| info.has_depth_axis)
}

/// A decision-log entry for a start that carries no layer and no detail.
fn bare_decision(
    namespace: AssetCategoryScope,
    cue_id: &str,
    picked: Option<usize>,
    result: PlaybackVerdict,
) -> PlaybackVerdictEntry {
    PlaybackVerdictEntry {
        namespace,
        alias: cue_id.to_owned(),
        variant_ix: picked,
        outcome: result,
        alt_slot: None,
        detail: None,
    }
}

/// What became of a start that was parked while its clip decoded.
enum ParkedFate {
    /// Its match is over.
    Stale,
    /// It waited past its deadline.
    Expired,
    /// The clip is not ready yet.
    Waiting,
    /// The clip failed to decode.
    Broken,
    /// The clip is ready.
    Decoded(ClipPcm),
}

impl<'a, 'w, 's> Stage<'a, 'w, 's> {
    fn over(
        commands: &'a mut Commands<'w, 's>,
        bank: &'a AudioRoster,
        stock: Stock<'a>,
        books: Books<'a>,
        pose: Option<(Vec3, Vec3)>,
        gen_no: u64,
    ) -> Self {
        Self {
            commands,
            bank,
            stock,
            books,
            pose,
            gen_no,
        }
    }

    /// A plain request for `alias`, to be adjusted with the `Want` builders.
    fn want<'x>(&self, namespace: AssetCategoryScope, cue_id: &'x str, class: NoiseCategory) -> Want<'x> {
        Want::new(namespace, cue_id, class, self.gen_no)
    }

    /// Starts a sound and logs how it went.
    fn play(&mut self, want: &Want<'_>) -> PlaybackVerdict {
        let report = self.start(want, 0);
        self.log_start(want, report)
    }

    fn log_start(&mut self, want: &Want<'_>, report: StartReport) -> PlaybackVerdict {
        let StartReport {
            result,
            picked,
            layered,
            detail,
        } = report;
        if let Some((link, linked)) = layered.as_ref().filter(|(_, linked)| !linked.is_open()) {
            diag::warn!(
                Audio,
                "audio: layer `{link}` chained from `{}:{}` came to {linked}",
                want.namespace.as_str(),
                want.alias
            );
        }
        let entry = PlaybackVerdictEntry {
            alt_slot: layered,
            detail,
            ..bare_decision(want.namespace, want.alias, picked, result.clone())
        };
        self.books.decisions.miss_row(entry);
        result
    }

    /// A footstep or landing: tries Black Ops II's own movement names first,
    /// then the legacy names the surface table lists for the alias.
    fn play_surface(&mut self, step: (&str, &str, Option<[f32; 3]>, Option<u32>), class: NoiseCategory) -> PlaybackVerdict {
        let (alias, fallback, spot, source_entity_no) = step;
        let bank = self.bank;
        let own_names = [alias, fallback]
            .into_iter()
            .filter_map(crate::cue_names::t6_movement_alias)
            .filter(|name| bank.slot_in(AssetCategoryScope::T6, name).is_some());
        let legacy_names = crate::cue_names::surface_cue_list(alias, fallback)
            .iter()
            .copied();
        let verdict = own_names.chain(legacy_names).try_fold(
            PlaybackVerdict::Failed(PlaybackBlocker::UnknownCue),
            |_, name| {
                let want = self
                    .want(AssetCategoryScope::T6, name, class)
                    .at(spot, source_entity_no);
                let outcome = self.play(&want);
                match outcome.allows_binding_fallback() {
                    true => ControlFlow::Continue(outcome),
                    false => ControlFlow::Break(outcome),
                }
            },
        );
        let (ControlFlow::Continue(last) | ControlFlow::Break(last)) = verdict;
        last
    }

    /// Plays the sound a layer chains to, one level deeper.
    fn follow_layer(&mut self, want: &Want<'_>, link: Option<&str>, depth: u8) -> Option<(String, PlaybackVerdict)> {
        // bo2zm: BO2 writes "null" for no second layer.
        let name = link.filter(|name| !name.is_empty() && !name.eq_ignore_ascii_case("null"))?;
        let outcome = if depth >= LAYER_DEPTH_CAP {
            PlaybackVerdict::Failed(PlaybackBlocker::NoPcm)
        } else {
            let chained = Want::new(want.namespace, name, want.class, want.epoch)
                .at(want.spot, want.source_entity_no);
            self.start(&chained, depth + 1).result
        };
        Some((name.to_owned(), outcome))
    }

    /// Like `follow_layer`, but Black Ops chains its layer before the main
    /// sound, so it never follows one afterwards.
    fn follow_layer_after(&mut self, want: &Want<'_>, link: Option<&str>, depth: u8) -> Option<(String, PlaybackVerdict)> {
        if want.namespace == AssetCategoryScope::T5 {
            return None;
        }
        self.follow_layer(want, link, depth)
    }

    /// Draws which variant of the sound to play, steering clear of the one
    /// played last.
    fn draw_variant(&mut self, want: &Want<'_>) -> Option<asset_audio::ResidentSelectionResult<'a>> {
        let bank = self.bank;
        let key = (want.namespace, want.alias.to_owned());
        let avoid = self.books.pick.recent.get(&key).copied();
        let rng = &mut self.books.pick.rng;
        match want.bound {
            Some(index) => bank.pick_loaded_outcome_at(index, rng, avoid),
            None => bank.choose_read_result(want.namespace, want.alias, rng, avoid),
        }
    }

    /// The row of the alias that a variant index names.
    fn alias_row(&self, want: &Want<'_>, variant: usize) -> Option<&'a SnapshotSoundAlias> {
        sound_for(self.bank, want)?.aliases.get(variant)
    }

    /// The volume, pitch and chained layer for a pick: the loaded clip's own,
    /// else drawn from the row's ranges. The pitch follows the request's scale.
    fn tune_for(
        &mut self,
        want: &Want<'_>,
        drawn: &asset_audio::ResidentSelectionResult<'_>,
        row: Option<&SnapshotSoundAlias>,
    ) -> Tune {
        let base = match (drawn.in_memory.as_ref(), row) {
            (Some(picked), _) => Tune {
                gain: picked.gain,
                rate: picked.pitch_ratio,
                next_cue: picked.layer_name.clone(),
            },
            (None, Some(row)) => {
                let (gain, rate) =
                    streamed_row_volume_pitch(want.namespace, row, &mut self.books.pick.rng);
                Tune {
                    gain,
                    rate,
                    next_cue: row.layered_alias.clone(),
                }
            }
            (None, None) => Tune::NEUTRAL,
        };
        let scale = Some(want.tilt_multiplier)
            .filter(|scale| scale.is_finite() && *scale > 0.0)
            .unwrap_or(1.0);
        Tune {
            rate: base.rate * scale,
            ..base
        }
    }

    /// Picks a variant of the sound and starts it, or queues it while its
    /// clip decodes.
    fn start(&mut self, want: &Want<'_>, depth: u8) -> StartReport {
        let Some(drawn) = self.draw_variant(want) else {
            return StartReport::failed(PlaybackBlocker::UnknownCue);
        };
        let slot_no = drawn.chosen_row;
        let row = self.alias_row(want, slot_no);
        let chain = row.and_then(|row| row.layered_alias.as_deref());
        let is_t5 = want.namespace == AssetCategoryScope::T5;

        let leading = is_t5
            .then(|| self.follow_layer(want, chain, depth))
            .flatten();

        let loaded = drawn.in_memory.as_ref().map(|picked| {
            let zone = AssetCategoryScope::from_zone_game(picked.sound.game);
            (picked.sound.name.as_str(), zone)
        });
        let (loaded_as, loaded_ns) = loaded.unzip();
        let found = waveform_for_variant(
            self.bank,
            want.namespace,
            want.alias,
            want.bound,
            slot_no,
            loaded_as,
            loaded_ns,
        );
        let Some(clip) = found else {
            // bo2mp: a Black Ops II alias with no sound of its own can exist
            // only for its secondary (a jump's launch exertion carries the
            // gear sound), and that still plays.
            let trailing = (want.namespace == AssetCategoryScope::T6)
                .then(|| self.follow_layer(want, chain, depth))
                .flatten();
            return StartReport::failed(PlaybackBlocker::NoPcm)
                .with_variant(slot_no)
                .chained(trailing);
        };

        let tune = self.tune_for(want, &drawn, row);
        match self.claim_clip(want, slot_no, &clip, &tune) {
            ClipFetch::Decoded(samples) => {
                let chosen = Chosen {
                    samples,
                    reel: &clip,
                    voice_ix: slot_no,
                    gain: tune.gain,
                    pitch: tune.rate,
                    layer: tune.next_cue.as_deref(),
                };
                let started = self.submit(want, depth, chosen);
                if is_t5 {
                    started.chained(leading)
                } else {
                    started
                }
            }
            ClipFetch::Pending => StartReport::of(PlaybackVerdict::Pending, Some(slot_no)),
            ClipFetch::Declined(failure) => StartReport::failed(failure).with_variant(slot_no),
        }
    }

    /// Asks the clip store for the clip (and the ones its layer chains to) and
    /// takes it if it is ready; otherwise parks the start until it is.
    fn claim_clip(
        &mut self,
        want: &Want<'_>,
        bay: usize,
        reel: &WaveformHandle,
        tune: &Tune,
    ) -> ClipFetch {
        let bank = self.bank;
        let Some(store) = self.stock.clips.as_deref_mut() else {
            return ClipFetch::Declined(PlaybackBlocker::NoPcm);
        };
        let linked = tune
            .next_cue
            .iter()
            .flat_map(|link| waveforms_for_cue(bank, want.namespace, link));
        for wanted in std::iter::once(reel.clone()).chain(linked) {
            store.req_in(wanted);
        }
        match store.ready_pcm(reel) {
            Some(Ok(pcm)) => ClipFetch::Decoded(pcm),
            Some(Err(_)) => ClipFetch::Declined(PlaybackBlocker::UnreadableBytes),
            None => {
                self.books.pending.queue_oneshot(want.parked(bay, reel, tune));
                ClipFetch::Pending
            }
        }
    }

    /// Frees the voices the sound's channel and alias limits require, or says
    /// why there is no room.
    fn claim_voice(
        &mut self,
        want: &Want<'_>,
        bay: Option<u32>,
        limit_name: &str,
        row: Option<&SnapshotSoundAlias>,
        priority: f32,
    ) -> Result<(), MuteCause> {
        let (commands, occupancy) = (&mut *self.commands, &mut *self.books.occupancy);
        clear_channel(commands, occupancy, self.bank, bay, want.source_entity_no)?;
        prepare_alias_voice(
            commands,
            occupancy,
            want.namespace,
            limit_name,
            row,
            want.source_entity_no,
            priority,
        )
    }

    /// How a voice's priority reads from where the sound sits.
    fn priority_for(&self, row: Option<&SnapshotSoundAlias>, spot: Option<[f32; 3]>) -> f32 {
        let Some(curve) = row.and_then(|row| row.voice_priority.as_ref()) else {
            return 0.0;
        };
        let from_ear = spot
            .zip(self.pose)
            .map(|(pos_src, (ear, _))| gap_in_inches(inches_of(ear), pos_src));
        curve.priority_for(from_ear)
    }

    /// Works out where a 3D sound sits and what its falloff leaves of it.
    /// `Err` carries the start that came to nothing instead.
    fn place(
        &mut self,
        want: &Want<'_>,
        chosen: &Chosen<'_>,
        row: Option<&SnapshotSoundAlias>,
        pos: [f32; 3],
        layer_depth: u8,
    ) -> Result<Placed, StartReport> {
        let variant = chosen.voice_ix;
        let refuse = |failure: PlaybackBlocker| StartReport::failed(failure).with_variant(variant);
        let (ear, right) = self.pose.ok_or_else(|| {
            diag::warn!(
                Audio,
                "audio: world alias `{}` has no listener (typed gap)",
                want.alias
            );
            refuse(PlaybackBlocker::NoEars)
        })?;
        let row = row.ok_or_else(|| refuse(PlaybackBlocker::NoPcm))?;
        let fall_curve = row.volume_falloff.as_ref().ok_or_else(|| {
            diag::warn!(
                Audio,
                "audio: world alias `{}` has no falloff curve (typed gap)",
                want.alias
            );
            refuse(PlaybackBlocker::AttenuationCurveAbsent)
        })?;

        let dist = gap_in_inches(inches_of(ear), pos);
        let curve_pts = self.stock.shared.curve_knots(&fall_curve.name, &fall_curve.knots);
        let near_knots = row
            .near_falloff
            .as_ref()
            .map(|near| self.stock.shared.curve_knots(&near.name, &near.knots));
        let atten = distance_attenuation(
            &curve_pts,
            near_knots.as_deref(),
            dist,
            row.reach_lower,
            row.reach_upper,
        );
        let reach = Reach::measure(row, dist, atten);
        match atten.partial_cmp(&0.0) {
            Some(Ordering::Less) => {
                diag::warn!(
                    Audio,
                    "audio: world alias `{}` falloff curve `{}` failed evaluation (typed gap)",
                    want.alias,
                    fall_curve.name
                );
                Err(refuse(PlaybackBlocker::AttenuationMath))
            }
            Some(Ordering::Equal) => {
                // bo2mp: out of this layer's reach, but its secondary is a sound
                // of its own (a gun's npc shot chains to its LFE, then to the
                // distant shot and its decay, each with a longer reach), so it
                // still starts and far gunfire is heard.
                let secondary = self.follow_layer_after(want, chosen.layer, layer_depth);
                let silent = StartReport::suppressed(
                    MuteCause::Inaudible,
                    variant,
                    Some(reach.describe()),
                );
                Err(silent.chained(secondary))
            }
            _ => {
                // bo2mp: a Black Ops II 3D sound with a pan of its own keeps that pan.
                let fixed_pan = row
                    .stereo_pan
                    .map(|pan| pan.gains(chosen.samples.lane_count()));
                let gains = fixed_pan.unwrap_or_else(|| {
                    oneshot_lr_gains(ear, right, Vec3::from_array(pos), 1.0)
                });
                Ok(Placed {
                    pos,
                    reach,
                    knots: curve_pts,
                    near_knots,
                    fixed_pan,
                    gains,
                })
            }
        }
    }

    /// The asset a voice plays: the clip's shared plain one, or, when the
    /// sound is panned, a copy of its own with a live pan control.
    fn pcm_handle(
        &mut self,
        key: &WaveformHandle,
        snd: ClipPcm,
        pan: Option<(f32, f32)>,
        echo: Option<(std::sync::Arc<crate::room_echo::EchoParams>, f32)>,
    ) -> (Handle<ClipPcm>, Option<PanHandle>) {
        let stock = &mut self.stock;
        // bo2zm: a play that rings into the room's echo is its own asset (the plain one is
        // still cached for later plays).
        if let (None, Some((params, send))) = (pan, echo.clone()) {
            let store = stock.clips.as_deref();
            stock.shared.plain_handle(key, snd.clone(), stock.pcm_assets, store);
            return (stock.pcm_assets.add(snd.with_echo(params, send)), None);
        }
        let Some((pan_l, right)) = pan else {
            let plain =
                stock
                    .shared
                    .plain_handle(key, snd, stock.pcm_assets, stock.clips.as_deref());
            return (plain, None);
        };
        // The clip's plain asset is still cached so later unpanned plays can
        // share it.
        let store = stock.clips.as_deref();
        stock
            .shared
            .plain_handle(key, snd.clone(), stock.pcm_assets, store);
        let panned = match echo {
            Some((params, send)) => snd.with_pan_handle().with_echo(params, send),
            None => snd.with_pan_handle(),
        };
        let control = panned.pan_handle().expect("with_pan_handle").clone();
        control.store(pan_l, right);
        (stock.pcm_assets.add(panned), Some(control))
    }

    /// Spawns the voice for a clip that is ready, and the layer it chains to.
    fn submit(&mut self, want: &Want<'_>, depth: u8, chosen: Chosen<'_>) -> StartReport {
        let snd_bank = self.bank;
        let lane_no = chosen.voice_ix;
        let sound = sound_for(snd_bank, want);
        let row = sound.and_then(|sound| sound.aliases.get(lane_no));
        let channel = sound.and_then(|sound| sound.channel_of_row(lane_no));
        let limit_name = sound.map_or(want.alias, |sound| sound.name.as_str());

        let spot = want
            .spot
            .filter(|_| is_positional(snd_bank, want.namespace, row, channel));
        let priority = self.priority_for(row, spot);
        let placed = match spot.map(|pos| self.place(want, &chosen, row, pos, depth)) {
            None => None,
            Some(Ok(placed)) => Some(placed),
            Some(Err(refused)) => return refused,
        };
        let detail = placed.as_ref().map(|placed| placed.reach.describe());
        if let Err(reason) = self.claim_voice(want, channel, limit_name, row, priority) {
            return StartReport::suppressed(reason, lane_no, detail);
        }
        self.books
            .pick
            .recent
            .insert((want.namespace, want.alias.to_owned()), lane_no);

        let clip_len = crate::duck::clip_ms(&chosen.samples);
        let Chosen {
            samples,
            reel: clip,
            layer,
            pitch,
            gain,
            ..
        } = chosen;
        // bo2zm: Black Ops II's Sound sliders (music, effects, voice) on its bus.
        let gain = if want.namespace == AssetCategoryScope::T6 {
            gain * crate::bus_gain::t6_alias(row.and_then(|r| r.alias_bits))
        } else {
            gain
        };
        let pan = match &placed {
            Some(placed) => Some(placed.gains),
            None => row
                .and_then(|row| row.stereo_pan)
                .map(|pan| pan.gains(samples.lane_count()))
                .filter(|gains| *gains != (1.0, 1.0)),
        };
        let echo = room_echo_for(want.namespace, row);
        let (handle, active_stereo_handle) = self.pcm_handle(clip, samples, pan, echo);
        let reach_gain = placed.as_ref().map_or(1.0, |placed| placed.reach.atten);

        let booking = book_voice(snd_bank, channel, want.source_entity_no);
        let voice = crate::backend::begin_single_cue(
            self.commands,
            handle,
            Volume::Linear((gain * reach_gain).max(0.0)),
            pitch,
            want.epoch,
            want.class.scope(),
        );
        let base = gain.max(0.0);
        let category = row.and_then(|row| row.menu_category);
        let channel_3d = placed
            .zip(active_stereo_handle)
            .map(|(placed, live)| channel_around(placed, live, row, base));
        let mut spawned = self.commands.entity(voice);
        spawned.insert(AliasPlayback {
            namespace: want.namespace,
            source_entity_no: want.source_entity_no,
            alias: want.alias.to_owned(),
        });
        if let Some(world_voice) = channel_3d {
            spawned.insert(world_voice);
        }
        crate::duck::tag(&mut spawned, row, clip_len);
        if let Some(category) = category {
            spawned.insert(crate::MenuVolume { cat_ix: category, base });
        }
        if let Some(held) = booking {
            spawned.insert(held);
            self.books.occupancy.hold_entity(voice, held);
        }
        if want.namespace == AssetCategoryScope::T5 {
            let alias_lease = self.books.occupancy.track_alias(
                voice,
                want.namespace,
                limit_name,
                want.source_entity_no,
                priority,
            );
            spawned.insert(alias_lease);
        }

        let secondary = self.follow_layer_after(want, layer, depth);
        StartReport::of(PlaybackVerdict::Handed, Some(lane_no))
            .chained(secondary)
            .with_detail(detail)
    }

    /// How a parked start stands now.
    fn fate_of(&self, entry: &QueuedSingleCue, now: Instant) -> ParkedFate {
        let other_match =
            entry.class.scope() == crate::backend::SoundAudience::Match && entry.gen_tag != self.gen_no;
        if other_match {
            return ParkedFate::Stale;
        }
        if entry.due_at <= now {
            return ParkedFate::Expired;
        }
        let store = self.stock.clips.as_deref();
        match store.and_then(|store| store.ready_pcm(&entry.reel)) {
            None => ParkedFate::Waiting,
            Some(Err(_)) => ParkedFate::Broken,
            Some(Ok(pcm)) => ParkedFate::Decoded(pcm),
        }
    }

    /// Starts the sounds whose clips finished decoding since the last frame,
    /// and gives up on the ones that waited too long.
    fn drain_pending(&mut self) {
        let now = Instant::now();
        let parked = std::mem::take(&mut self.books.pending.entries);
        for row in parked {
            let failure = match self.fate_of(&row, now) {
                ParkedFate::Stale => continue,
                ParkedFate::Waiting => {
                    self.books.pending.entries.push(row);
                    continue;
                }
                ParkedFate::Decoded(pcm) => {
                    let want = Want::from_parked(&row);
                    let started = self.submit(&want, 0, Chosen::from_parked(&row, pcm));
                    self.log_start(&want, started);
                    continue;
                }
                ParkedFate::Expired => {
                    let space = row.namespace.as_str();
                    diag::warn!(
                        Audio,
                        "audio: parked `{space}:{}` gave up waiting for its clip (typed gap)",
                        row.alias
                    );
                    PlaybackBlocker::Expired
                }
                ParkedFate::Broken => PlaybackBlocker::UnreadableBytes,
            };
            note_failure(
                self.books.decisions,
                (row.namespace, &row.alias),
                Some(row.variant_ix),
                failure,
            );
        }
    }
}

/// The retuning component for a voice that sits in the world.
fn channel_around(
    placed: Placed,
    active_stereo_handle: PanHandle,
    row: Option<&SnapshotSoundAlias>,
    rest_volume: f32,
) -> SpatialVoice {
    SpatialVoice {
        position_in_inches: placed.pos,
        reach_lower: placed.reach.reach_lower,
        reach_upper: placed.reach.reach_upper,
        knots: placed.knots,
        near_knots: placed.near_knots,
        priority: row.and_then(|row| row.voice_priority.clone()),
        rest_volume,
        active_stereo_handle,
        fixed_pan: placed.fixed_pan,
        category: row.and_then(|row| row.menu_category),
    }
}

fn note_failure(
    log: &mut PlaybackVerdictLog,
    who: (AssetCategoryScope, &str),
    picked: Option<usize>,
    failure: PlaybackBlocker,
) {
    let (namespace, alias) = who;
    let entry = bare_decision(namespace, alias, picked, PlaybackVerdict::Failed(failure));
    log.miss_row(entry);
}

/// Logs and records the sounds asked for while there is no sound bank.
fn drop_without_bank<'x>(aliases: impl Iterator<Item = &'x str>, log: &mut PlaybackVerdictLog) {
    let noisy = !crate::AudioSilent::active();
    for name in aliases {
        if noisy {
            diag::warn!(
                Audio,
                "audio: no SoundBank, so alias `{name}` is dropped (typed gap)"
            );
        }
        note_failure(
            log,
            (AssetCategoryScope::T6, name),
            None,
            PlaybackBlocker::NoCatalog,
        );
    }
}

/// The alias to play and the pitch scale it asks for, unless it is a stop.
fn playable(command: &AliasCommand) -> Option<(&TriggerCueMsg, f32)> {
    let (sound, pitch) = match command {
        AliasCommand::Stop { .. } => return None,
        AliasCommand::PlayPitched { sound, pitch } => (sound, *pitch),
        AliasCommand::Play(sound) => (sound, 1.0),
    };
    Some((sound, pitch))
}

fn start_alias_commands(mut incoming: MessageReader<AliasCommand>, mut desk: SoundDesk) {
    for command in incoming.read() {
        if let AliasCommand::Stop {
            namespace,
            alias,
            source_entity_no,
        } = command
        {
            desk.stop_alias(*namespace, alias, *source_entity_no);
            continue;
        }
        let Some((event, tilt_multiplier)) = playable(command) else {
            continue;
        };
        match desk.open() {
            Some((mut stage, unfound)) => {
                play_alias_event(&mut stage, unfound, event, tilt_multiplier);
            }
            None => drop_without_bank(std::iter::once(event.alias.as_str()), &mut desk.decisions),
        }
    }
}

/// Plays one alias message, falling back to the alias it names as a stand-in,
/// and notes the names that turned out to be missing.
fn play_alias_event(
    stage: &mut Stage<'_, '_, '_>,
    unfound: &mut UnknownCueHoles,
    ev: &TriggerCueMsg,
    tilt_multiplier: f32,
) {
    let Some((cue_ns, alias)) = movement_target(stage.bank, ev.cue_ns, &ev.alias)
    else {
        return;
    };
    let first = stage
        .want(cue_ns, alias, NoiseCategory::World)
        .at(ev.position_in_inches, ev.source_entity_no)
        .pitched(tilt_multiplier);
    if !stage.play(&first).allows_binding_fallback() {
        return;
    }
    if let Some(stand_in) = ev.fallback.as_deref() {
        let second = Want {
            namespace: ev.cue_ns,
            alias: stand_in,
            ..first
        };
        let outcome = stage.play(&second);
        if outcome.is_open() {
            return;
        }
        let unmatched = outcome.allows_binding_fallback();
        if unmatched {
            unfound.note_miss(stand_in);
        }
    }
    unfound.note_miss(&ev.alias);
}

/// Plays a footstep or landing event's surface chain and notes what was
/// missing.
fn play_surface_event(
    stage: &mut Stage<'_, '_, '_>,
    unfound: &mut UnknownCueHoles,
    step: (&str, &str, Option<[f32; 3]>, Option<u32>),
) {
    let (alias, fallback, ..) = step;
    if stage
        .play_surface(step, NoiseCategory::World)
        .allows_binding_fallback()
    {
        unfound.note_miss(alias);
        unfound.note_miss(fallback);
    }
}

/// Plays a batch of footsteps or landings, or drops them all when there is no
/// sound bank.
fn play_surface_batch<'x>(desk: &mut SoundDesk, events: impl Iterator<Item = (&'x str, &'x str, Option<[f32; 3]>, Option<u32>)>) {
    let Some((mut stage, unfound)) = desk.open() else {
        drop_without_bank(events.map(|(alias, ..)| alias), &mut desk.decisions);
        return;
    };
    events.for_each(|step| play_surface_event(&mut stage, unfound, step));
}

fn start_footsteps(mut events: MessageReader<FootfallMsg>, mut desk: SoundDesk) {
    let steps = events
        .read()
        .map(|step| (step.alias, step.fallback, step.position_in_inches, step.source_entity_no));
    play_surface_batch(&mut desk, steps);
}

fn start_landings(mut events: MessageReader<TouchdownNoiseMsg>, mut desk: SoundDesk) {
    let landings = events
        .read()
        .map(|land| (land.alias, land.fallback, land.position_in_inches, land.source_entity_no));
    play_surface_batch(&mut desk, landings);
}

fn start_weapon_sounds(mut events: MessageReader<GunNoiseMsg>, mut desk: SoundDesk) {
    let Some((mut stage, unfound)) = desk.open() else {
        let names = events.read().map(|event| event.alias.as_str());
        drop_without_bank(names, &mut desk.decisions);
        return;
    };
    let bank = stage.bank;
    let routed = events.read().filter_map(|ev| {
        let target = movement_target(bank, ev.cue_ns, &ev.alias)?;
        Some((ev, target))
    });
    for (event, (namespace, alias)) in routed {
        let want = stage
            .want(namespace, alias, NoiseCategory::Weapon)
            .at(event.position_in_inches, event.source_entity_no);
        if stage.play(&want).allows_binding_fallback() {
            unfound.note_miss(alias);
        }
    }
}

fn start_bound_weapon_sounds(mut events: MessageReader<BoundWeaponSound>, mut desk: SoundDesk) {
    let Some((mut stage, unfound)) = desk.open() else {
        let _ = events.read().count();
        return;
    };
    let bank = stage.bank;
    let revision = bank.revision();
    let current = events
        .read()
        .filter(|event| event.catalog_edition == revision)
        .filter_map(|event| Some((event, bank.label_at(event.index)?)));
    for (event, alias) in current {
        let namespace = bank.space_of_slot(event.index);
        let want = stage
            .want(namespace, alias, NoiseCategory::Weapon)
            .at(event.position_in_inches, event.source_entity_no)
            .bound_to(event.index);
        if stage.play(&want).allows_binding_fallback() {
            unfound.note_miss(alias);
        }
    }
}

fn start_parked_sounds(mut desk: SoundDesk) {
    let waiting = !desk.queued.entries.is_empty();
    match (waiting, desk.bank.is_some(), desk.clips.is_some()) {
        (false, ..) | (true, true, false) => {}
        (true, false, _) => desk.queued.clear(),
        (true, true, true) => {
            if let Some((mut stage, _)) = desk.open() {
                stage.drain_pending();
            }
        }
    }
}

/// Plays one alias through the full start path, for callers that hold the
/// pieces themselves instead of a `SoundDesk`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn fire_single_cue(
    commands: &mut Commands,
    pcm_assets: &mut Assets<ClipPcm>,
    shared: &mut CommonPlaybackStore,
    bank: &AudioRoster,
    namespace: AssetCategoryScope,
    cue_id: &str,
    position_in_inches: Option<[f32; 3]>,
    listener: Option<(Vec3, Vec3)>,
    pick_state: &mut CuePickState,
    clips: Option<&mut ClipRepository>,
    pending: &mut QueuedVoiceStarts,
    occupancy: &mut ChannelUsageTally,
    log: &mut PlaybackVerdictLog,
    source_entity_no: Option<u32>,
    class: NoiseCategory,
    epoch: u64,
) -> PlaybackVerdict {
    let stock = Stock::new(pcm_assets, shared, clips);
    let books = Books::new(pick_state, pending, occupancy, log);
    let mut stage = Stage::over(commands, bank, stock, books, listener, epoch);
    let want = stage
        .want(namespace, cue_id, class)
        .at(position_in_inches, source_entity_no);
    stage.play(&want)
}

/// Which way a world sound lies from the ear: -1 is hard left, 1 hard right.
pub fn oneshot_pan(hear_at: Vec3, right: Vec3, source: Vec3) -> f32 {
    let toward = source - hear_at;
    if toward.length_squared() < 1e-8 {
        return 0.0;
    }
    toward.normalize().dot(right).clamp(-1.0, 1.0)
}

/// The left and right gains of a world sound, an equal-power pan scaled by
/// `atten`.
pub fn oneshot_lr_gains(
    ear: Vec3,
    right: Vec3,
    source: Vec3,
    atten: f32,
) -> (f32, f32) {
    let angle = (oneshot_pan(ear, right, source) + 1.0) * FRAC_PI_4;
    let scaled = |trig: f32| atten * trig;
    (scaled(angle.cos()), scaled(angle.sin()))
}

/// Stops the voices that hold the sound's entity channel when that channel
/// is restricted, then checks that the channel has a free voice.
fn clear_channel(
    commands: &mut Commands,
    occupancy: &mut ChannelUsageTally,
    bank: &AudioRoster,
    bay: Option<u32>,
    source_entity_no: Option<u32>,
) -> Result<(), MuteCause> {
    let Some((chan, info)) = bay.and_then(|ch| Some((ch, bank.actor_lane(ch)?))) else {
        return Ok(());
    };
    if let Some(ent) = source_entity_no.filter(|_| info.is_limited) {
        occupancy
            .take_ent_channel(ent, chan)
            .into_iter()
            .for_each(|entity| crate::backend::silence_cue(commands, entity));
    }
    match has_free_voice(occupancy.voices_on(chan), 0, info.simultaneous_channel_cap) {
        true => Ok(()),
        false => Err(MuteCause::AtVoiceCap),
    }
}

/// Black Ops' per-alias voice limits: a global one and a per-entity one, each
/// in one of several modes packed into the alias flags.
fn prepare_alias_voice(
    commands: &mut Commands,
    occupancy: &mut ChannelUsageTally,
    namespace: AssetCategoryScope,
    cue_id: &str,
    row: Option<&SnapshotSoundAlias>,
    source_entity_no: Option<u32>,
    priority: f32,
) -> Result<(), MuteCause> {
    let Some(row) = row.filter(|_| namespace == AssetCategoryScope::T5) else {
        return Ok(());
    };
    let flags = row.alias_bits.unwrap_or(0);
    let mode_at = |shift: u32| (flags >> shift) & 3;
    [
        (mode_at(25), row.cap_total, false),
        (mode_at(27), row.actor_cap_total, true),
    ]
    .into_iter()
    .filter_map(|(mode, count, per_entity)| Some(((mode, count?), per_entity)))
    .try_for_each(|(limit, per_entity)| {
        let evicted =
            occupancy.limit_alias(namespace, cue_id, source_entity_no, limit, per_entity, priority)?;
        evicted
            .into_iter()
            .for_each(|entity| crate::backend::silence_cue(commands, entity));
        Ok(())
    })
}

/// bo2zm: the room echo a Black Ops II alias's play sends into (its `reverbSend`), when the
/// player is in a room that has one.
fn room_echo_for(
    namespace: AssetCategoryScope,
    row: Option<&asset_audio::SnapshotSoundAlias>,
) -> Option<(std::sync::Arc<crate::room_echo::EchoParams>, f32)> {
    if namespace != AssetCategoryScope::T6 {
        return None;
    }
    let send = row?.reverb_send;
    if send <= 0.0 {
        return None;
    }
    Some((crate::room_echo::current()?, send))
}

fn book_voice(bank: &AudioRoster, bay: Option<u32>, source_entity_no: Option<u32>) -> Option<ChannelBooking> {
    bay.filter(|&ch| bank.actor_lane(ch).is_some())
        .map(|channel| ChannelBooking { channel, source_entity_no })
}

/// The volume and pitch a streamed row asks for, drawn from its ranges. A
/// zero range means "unset" and plays at 1.
fn streamed_row_volume_pitch(
    namespace: AssetCategoryScope,
    entry: &SnapshotSoundAlias,
    stream: &mut u32,
) -> (f32, f32) {
    let draw_volume = unit_random(stream);
    let draw_pitch = unit_random(stream);
    let ranged = |low: f32, high: f32, draw: f32, unset_means_one: bool| {
        if unset_means_one && low == 0.0 && high == 0.0 {
            1.0
        } else {
            mix_interval(low, high, draw)
        }
    };
    (
        ranged(
            entry.volume_low,
            entry.volume_high,
            draw_volume,
            namespace != AssetCategoryScope::T5,
        ),
        ranged(entry.pitch_lower_bound, entry.pitch_upper_bound, draw_pitch, true),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cue_names::{FootfallPace, footfall_cue_names, gear_alias, gear_jingle_cue};
    use crate::backend::{SoundAudience, PlayingCue};
    use crate::voice_slots::AliasVoiceLease;
    use asset_audio::{SnapshotSoundAlias, SnapshotAudioCurve, SnapshotSoundEntry, StereoPan};
    use bevy::audio::PlaybackSettings;
    use bevy::ecs::world::CommandQueue;
    use std::time::Duration;

    const T6: AssetCategoryScope = AssetCategoryScope::T6;
    const T5: AssetCategoryScope = AssetCategoryScope::T5;
    const NOWHERE: Option<[f32; 3]> = None;
    const FULL: f32 = 1.0;
    const WORLD: NoiseCategory = NoiseCategory::World;
    const UI: NoiseCategory = NoiseCategory::Ui;
    const ASPHALT: u32 = 22 << 20;

    fn row() -> SnapshotSoundAlias {
        SnapshotSoundAlias {
            streamed: Some(("bank.xpak".to_owned(), "0001".to_owned())),
            volume_low: 1.0,
            volume_high: 1.0,
            pitch_lower_bound: 1.0,
            pitch_upper_bound: 1.0,
            reach_lower: 100.0,
            reach_upper: 1100.0,
            chance_weight: 1.0,
            ..SnapshotSoundAlias::default()
        }
    }

    fn clipless() -> SnapshotSoundAlias {
        SnapshotSoundAlias {
            streamed: None,
            ..row()
        }
    }

    fn linear() -> SnapshotAudioCurve {
        SnapshotAudioCurve {
            name: "line".to_owned(),
            knots: vec![(0.0, 1.0), (1.0, 0.0)],
        }
    }

    fn sound(name: &str, rows: Vec<SnapshotSoundAlias>) -> SnapshotSoundEntry {
        SnapshotSoundEntry {
            name: name.to_owned(),
            aliases: rows,
        }
    }

    fn catalog(sounds: Vec<SnapshotSoundEntry>) -> AudioRoster {
        let mut catalog = AudioRoster::default();
        for entry in sounds {
            catalog.add_sound(entry);
        }
        catalog.publish();
        catalog
    }

    fn key() -> WaveformHandle {
        WaveformHandle::Streamed {
            ns: T6,
            folder: "bank.xpak".to_owned(),
            name: "0001".to_owned(),
        }
    }

    fn pcm(channels: u16) -> ClipPcm {
        ClipPcm::from_parts(Arc::from(vec![0.25f32; 64]), channels, 44_100).expect("pcm")
    }

    fn want(cue_id: &str) -> Want<'_> {
        Want {
            namespace: T6,
            alias: cue_id,
            bound: None,
            spot: None,
            source_entity_no: None,
            class: NoiseCategory::World,
            tilt_multiplier: 1.0,
            epoch: 0,
        }
    }

    fn chosen<'a>(reel: &'a WaveformHandle, layer: Option<&'a str>, channels: u16) -> Chosen<'a> {
        Chosen {
            samples: pcm(channels),
            reel,
            voice_ix: 0,
            gain: 0.8,
            pitch: 1.5,
            layer,
        }
    }

    fn waiting(
        cue_id: &str,
        source_entity_no: Option<u32>,
        class: NoiseCategory,
        epoch: u64,
        wait: Duration,
    ) -> QueuedSingleCue {
        QueuedSingleCue {
            namespace: T6,
            alias: cue_id.to_owned(),
            bound: None,
            variant_ix: 0,
            gain: 1.0,
            pitch_mul: 1.0,
            position_in_inches: NOWHERE,
            source_entity_no,
            reel: key(),
            mix_layer: None,
            class,
            gen_tag: epoch,
            due_at: Instant::now() + wait,
        }
    }

    struct Rig {
        world: World,
        queue: CommandQueue,
        assets: Assets<ClipPcm>,
        shared: CommonPlaybackStore,
        pick: CuePickState,
        pending: QueuedVoiceStarts,
        occupancy: ChannelUsageTally,
        decisions: PlaybackVerdictLog,
        clips: Option<ClipRepository>,
        bank: AudioRoster,
        pose: Option<(Vec3, Vec3)>,
    }

    impl Rig {
        fn new(sounds: Vec<SnapshotSoundEntry>) -> Self {
            Self {
                world: World::new(),
                queue: CommandQueue::default(),
                assets: Assets::default(),
                shared: CommonPlaybackStore::default(),
                pick: CuePickState::default(),
                pending: QueuedVoiceStarts::default(),
                occupancy: ChannelUsageTally::default(),
                decisions: PlaybackVerdictLog::default(),
                clips: None,
                bank: catalog(sounds),
                pose: None,
            }
        }

        fn with_ear(mut self) -> Self {
            self.pose = Some((Vec3::ZERO, Vec3::X));
            self
        }

        fn run<R>(&mut self, job: impl FnOnce(&mut Stage<'_, '_, '_>) -> R) -> R {
            let mut commands = Commands::new(&mut self.queue, &self.world);
            let stock = Stock::new(&mut self.assets, &mut self.shared, self.clips.as_mut());
            let books = Books::new(
                &mut self.pick,
                &mut self.pending,
                &mut self.occupancy,
                &mut self.decisions,
            );
            let mut stage = Stage::over(&mut commands, &self.bank, stock, books, self.pose, 0);
            let out = job(&mut stage);
            self.queue.apply(&mut self.world);
            out
        }

        fn voices(&mut self) -> Vec<Entity> {
            let mut found = self.world.query_filtered::<Entity, With<AliasPlayback>>();
            found.iter(&self.world).collect()
        }

        fn log(&self) -> Vec<String> {
            self.decisions.lines().collect()
        }
    }

    #[test]
    fn a_missing_alias_is_listed_once() {
        let mut gaps = UnknownCueHoles::default();
        assert!(gaps.is_empty());
        gaps.note_miss("a");
        gaps.note_miss("b");
        gaps.note_miss("a");
        assert_eq!(gaps.aliases, ["a", "b"]);
        assert_eq!(gaps.len(), 2);
        assert!(!gaps.is_empty());
    }

    #[test]
    fn pick_state_starts_from_its_fixed_seed() {
        let state = CuePickState::default();
        assert_eq!(state.rng, 0x00a5_5a5a);
        assert!(state.recent.is_empty());
    }

    #[test]
    fn curves_are_shared_by_name_and_cut_to_the_knot_limit() {
        let mut shared = CommonPlaybackStore::default();
        let long: Vec<(f32, f32)> = (0..12).map(|n| (n as f32, 1.0)).collect();
        let first = shared.curve_knots("fall", &long);
        assert_eq!(first.len(), CURVE_KNOT_CAP);
        assert_eq!(first[3], [3.0, 1.0]);
        let again = shared.curve_knots("fall", &[(0.0, 0.0)]);
        assert!(Arc::ptr_eq(&first, &again));
        let other = shared.curve_knots("rise", &[(0.0, 0.0), (1.0, 1.0)]);
        assert_eq!(other.len(), 2);
    }

    #[test]
    fn pan_follows_the_side_the_emitter_is_on() {
        let ear = Vec3::new(10.0, 0.0, 0.0);
        assert_eq!(oneshot_pan(ear, Vec3::X, ear), 0.0);
        assert_eq!(
            oneshot_pan(ear, Vec3::X, ear + Vec3::new(50.0, 0.0, 0.0)),
            1.0
        );
        assert_eq!(
            oneshot_pan(ear, Vec3::X, ear - Vec3::new(5.0, 0.0, 0.0)),
            -1.0
        );
        let ahead = oneshot_pan(ear, Vec3::X, ear + Vec3::Y * 9.0);
        assert!(ahead.abs() < 1e-6);
        let diagonal = oneshot_pan(ear, Vec3::X, ear + Vec3::new(1.0, 1.0, 0.0));
        assert!((diagonal - FRAC_PI_4.cos()).abs() < 1e-5);
    }

    #[test]
    fn channel_gains_are_equal_power_and_scale_with_attenuation() {
        let (left, right) = oneshot_lr_gains(Vec3::ZERO, Vec3::X, Vec3::Y, 1.0);
        assert!((left - FRAC_PI_4.cos()).abs() < 1e-6);
        assert!((right - FRAC_PI_4.sin()).abs() < 1e-6);
        let (left, right) = oneshot_lr_gains(Vec3::ZERO, Vec3::X, Vec3::X * 4.0, 0.5);
        assert!(left.abs() < 1e-6);
        assert!((right - 0.5).abs() < 1e-6);
        let (left, right) = oneshot_lr_gains(Vec3::ZERO, Vec3::X, -Vec3::X, 2.0);
        assert!((left - 2.0).abs() < 1e-6);
        assert!(right.abs() < 1e-6);
    }

    #[test]
    fn streamed_rows_draw_both_numbers_and_unset_ranges_play_at_one() {
        let mut rng = 7;
        let mut twin = 7;
        let unset = SnapshotSoundAlias::default();
        assert_eq!(streamed_row_volume_pitch(T6, &unset, &mut rng), (1.0, 1.0));
        unit_random(&mut twin);
        unit_random(&mut twin);
        assert_eq!(rng, twin);

        let ranged = SnapshotSoundAlias {
            volume_low: 0.5,
            volume_high: 0.5,
            pitch_lower_bound: 0.9,
            pitch_upper_bound: 0.9,
            ..SnapshotSoundAlias::default()
        };
        assert_eq!(streamed_row_volume_pitch(T6, &ranged, &mut rng), (0.5, 0.9));
        let (loudness, pitch) = streamed_row_volume_pitch(AssetCategoryScope::T5, &unset, &mut rng);
        assert_eq!((loudness, pitch), (0.0, 1.0));
    }

    #[test]
    fn a_voice_lease_needs_a_channel_the_bank_knows() {
        let bank = catalog(vec![]);
        assert!(book_voice(&bank, None, Some(3)).is_none());
        assert!(book_voice(&bank, Some(2), Some(3)).is_none());
    }

    #[test]
    fn channel_preparation_passes_without_channel_info() {
        let bank = catalog(vec![]);
        let mut world = World::new();
        let mut queue = CommandQueue::default();
        let mut commands = Commands::new(&mut queue, &world);
        let mut chan_use = ChannelUsageTally::default();
        assert_eq!(
            clear_channel(&mut commands, &mut chan_use, &bank, None, Some(1)),
            Ok(())
        );
        assert_eq!(
            clear_channel(&mut commands, &mut chan_use, &bank, Some(4), Some(1)),
            Ok(())
        );
        queue.apply(&mut world);
    }

    #[test]
    fn alias_limits_only_bind_old_namespace_rows() {
        let mut world = World::new();
        let mut queue = CommandQueue::default();
        let mut busy_map = ChannelUsageTally::default();
        let first = world.spawn_empty().id();
        busy_map.track_alias(first, AssetCategoryScope::T5, "shot", None, 0.0);
        let limited = SnapshotSoundAlias {
            alias_bits: Some(1 << 25),
            cap_total: Some(1),
            ..SnapshotSoundAlias::default()
        };
        {
            let mut commands = Commands::new(&mut queue, &world);
            let same = prepare_alias_voice(
                &mut commands,
                &mut busy_map,
                T6,
                "shot",
                Some(&limited),
                None,
                0.0,
            );
            assert_eq!(same, Ok(()));
            let no_row = prepare_alias_voice(
                &mut commands,
                &mut busy_map,
                T5,
                "shot",
                None,
                None,
                0.0,
            );
            assert_eq!(no_row, Ok(()));
        }
        queue.apply(&mut world);
        assert!(world.get_entity(first).is_ok());

        {
            let mut commands = Commands::new(&mut queue, &world);
            let cut = prepare_alias_voice(
                &mut commands,
                &mut busy_map,
                T5,
                "shot",
                Some(&limited),
                None,
                0.0,
            );
            assert_eq!(cut, Ok(()));
        }
        queue.apply(&mut world);
        assert!(world.get_entity(first).is_err());
    }

    #[test]
    fn falloff_detail_prints_whole_distances_and_three_decimals() {
        assert_eq!(
            falloff_detail(600.4, 100.0, 1100.0, 0.5),
            "dist=600 dist_min=100 dist_max=1100 atten=0.500"
        );
    }

    #[test]
    fn movement_names_follow_the_bank() {
        let (step, _) = footfall_cue_names(FootfallPace::Run, ASPHALT, true, false);
        let gear = gear_jingle_cue(FootfallPace::Sprint, true);
        let bank = catalog(vec![
            sound("fly_step_run_plr_default", vec![row()]),
            sound("fly_gear_sprint_plr", vec![row()]),
            sound("fly_gear_mantle_plr", vec![row()]),
        ]);
        assert!(matches!(t6_movement(&bank, T6, step), T6Movement::Skip));
        assert!(matches!(t6_movement(&bank, T6, gear), T6Movement::Skip));
        assert!(matches!(
            t6_movement(&bank, T6, gear_alias(true)),
            T6Movement::Play("fly_gear_mantle_plr")
        ));
        assert!(matches!(
            t6_movement(&bank, T6, "not_a_step"),
            T6Movement::NotT6
        ));
        assert!(matches!(
            t6_movement(&bank, AssetCategoryScope::T5, step),
            T6Movement::NotT6
        ));

        let with_step = catalog(vec![
            sound("fly_step_run_plr_default", vec![row()]),
            sound("fly_step_run_plr_asphalt", vec![row()]),
        ]);
        assert!(matches!(
            t6_movement(&with_step, T6, step),
            T6Movement::Play("fly_step_run_plr_asphalt")
        ));
        let no_default = catalog(vec![sound("fly_step_run_plr_asphalt", vec![row()])]);
        assert!(matches!(
            t6_movement(&no_default, T6, step),
            T6Movement::NotT6
        ));
    }

    #[test]
    fn a_plain_sound_is_spawned_with_its_tags() {
        let mut rig = Rig::new(vec![sound("beep", vec![row()])]);
        let clip = key();
        let started = rig.run(|stage| stage.submit(&want("beep"), 0, chosen(&clip, None, 1)));
        assert_eq!(started.result, PlaybackVerdict::Handed);
        assert_eq!(started.picked, Some(0));
        assert!(started.layered.is_none());
        assert!(started.detail.is_none());

        let voices = rig.voices();
        assert_eq!(voices.len(), 1);
        let entity = voices[0];
        let tag = rig.world.get::<AliasPlayback>(entity).expect("tag");
        assert_eq!(
            (tag.namespace, tag.alias.as_str(), tag.source_entity_no),
            (T6, "beep", None)
        );
        let voice = rig.world.get::<PlayingCue>(entity).expect("voice");
        assert_eq!((voice.cue_epoch, voice.scope), (0, SoundAudience::Match));
        let settings = rig.world.get::<PlaybackSettings>(entity).expect("settings");
        assert!((settings.volume.to_linear() - 0.8).abs() < 1e-6);
        assert!((settings.speed - 1.5).abs() < 1e-6);
        assert!(rig.world.get::<SpatialVoice>(entity).is_none());
        assert!(rig.world.get::<crate::MenuVolume>(entity).is_none());
        assert!(rig.world.get::<AliasVoiceLease>(entity).is_none());
        assert_eq!(
            rig.pick.recent.get(&(T6, "beep".to_owned())),
            Some(&0)
        );
        assert_eq!(rig.assets.len(), 1);
    }

    #[test]
    fn sounds_of_one_clip_share_one_unpanned_asset() {
        let mut rig = Rig::new(vec![sound("beep", vec![row()])]);
        let clip = key();
        rig.run(|stage| stage.submit(&want("beep"), 0, chosen(&clip, None, 1)));
        rig.run(|stage| stage.submit(&want("beep"), 0, chosen(&clip, None, 1)));
        assert_eq!(rig.voices().len(), 2);
        assert_eq!(rig.assets.len(), 1);
    }

    #[test]
    fn a_ui_sound_belongs_to_the_menus_and_a_menu_category_is_tagged() {
        let tagged = SnapshotSoundAlias {
            menu_category: Some(3),
            ..row()
        };
        let mut rig = Rig::new(vec![sound("click", vec![tagged])]);
        let clip = key();
        let ui = Want {
            class: NoiseCategory::Ui,
            epoch: 4,
            ..want("click")
        };
        rig.run(|stage| stage.submit(&ui, 0, chosen(&clip, None, 1)));
        let entity = rig.voices()[0];
        let voice = rig.world.get::<PlayingCue>(entity).expect("voice");
        assert_eq!((voice.cue_epoch, voice.scope), (4, SoundAudience::Menu));
        let menu = rig
            .world
            .get::<crate::MenuVolume>(entity)
            .expect("menu volume");
        assert_eq!(menu.cat_ix, 3);
        assert!((menu.base - 0.8).abs() < 1e-6);
    }

    #[test]
    fn a_fixed_pan_gets_its_own_asset_beside_the_shared_one() {
        let panned = SnapshotSoundAlias {
            stereo_pan: Some(StereoPan {
                mono: [0.25, 0.75],
                stereo: [0.5, 0.5],
            }),
            ..row()
        };
        let mut rig = Rig::new(vec![sound("pan", vec![panned])]);
        let clip = key();
        rig.run(|stage| stage.submit(&want("pan"), 0, chosen(&clip, None, 1)));
        assert_eq!(rig.assets.len(), 2);
        assert_eq!(rig.voices().len(), 1);

        let level = SnapshotSoundAlias {
            stereo_pan: Some(StereoPan {
                mono: [1.0, 1.0],
                stereo: [1.0, 1.0],
            }),
            ..row()
        };
        let mut plain = Rig::new(vec![sound("pan", vec![level])]);
        plain.run(|stage| stage.submit(&want("pan"), 0, chosen(&clip, None, 1)));
        assert_eq!(plain.assets.len(), 1);
    }

    fn world_row() -> SnapshotSoundAlias {
        SnapshotSoundAlias {
            alias_bits: Some(2),
            volume_falloff: Some(linear()),
            ..row()
        }
    }

    #[test]
    fn a_world_sound_follows_its_falloff_and_keeps_a_live_channel() {
        let mut rig = Rig::new(vec![sound("boom", vec![world_row()])]).with_ear();
        let clip = key();
        let at = Want {
            spot: Some([0.0, 600.0, 0.0]),
            source_entity_no: Some(7),
            ..want("boom")
        };
        let started = rig.run(|stage| stage.submit(&at, 0, chosen(&clip, None, 1)));
        assert_eq!(started.result, PlaybackVerdict::Handed);
        assert_eq!(
            started.detail.as_deref(),
            Some("dist=600 dist_min=100 dist_max=1100 atten=0.500")
        );
        let entity = rig.voices()[0];
        let channel = rig.world.get::<SpatialVoice>(entity).expect("channel");
        assert_eq!(channel.position_in_inches, [0.0, 600.0, 0.0]);
        assert_eq!((channel.reach_lower, channel.reach_upper), (100.0, 1100.0));
        assert!((channel.rest_volume - 0.8).abs() < 1e-6);
        assert!(channel.fixed_pan.is_none());
        assert!(channel.near_knots.is_none());
        assert_eq!(&*channel.knots, &[[0.0, 1.0], [1.0, 0.0]]);
        let settings = rig.world.get::<PlaybackSettings>(entity).expect("settings");
        assert!((settings.volume.to_linear() - 0.4).abs() < 1e-6);
        assert_eq!(
            rig.world.get::<AliasPlayback>(entity).expect("tag").source_entity_no,
            Some(7)
        );
    }

    #[test]
    fn a_world_sound_with_a_fixed_pan_keeps_those_gains() {
        let fixed = SnapshotSoundAlias {
            stereo_pan: Some(StereoPan {
                mono: [0.3, 0.6],
                stereo: [0.1, 0.2],
            }),
            ..world_row()
        };
        let mut rig = Rig::new(vec![sound("boom", vec![fixed])]).with_ear();
        let clip = key();
        let at = Want {
            spot: Some([0.0, 100.0, 0.0]),
            ..want("boom")
        };
        rig.run(|stage| stage.submit(&at, 0, chosen(&clip, None, 1)));
        let entity = rig.voices()[0];
        let channel = rig.world.get::<SpatialVoice>(entity).expect("channel");
        assert_eq!(channel.fixed_pan, Some((0.3, 0.6)));
    }

    #[test]
    fn a_world_sound_that_cannot_be_placed_says_why() {
        let clip = key();
        let at = || Want {
            spot: Some([0.0, 600.0, 0.0]),
            ..want("boom")
        };

        let mut deaf = Rig::new(vec![sound("boom", vec![world_row()])]);
        let started = deaf.run(|stage| stage.submit(&at(), 0, chosen(&clip, None, 1)));
        assert_eq!(
            started.result,
            PlaybackVerdict::Failed(PlaybackBlocker::NoEars)
        );
        assert_eq!(started.picked, Some(0));
        assert!(deaf.voices().is_empty());

        let mut bare = world_row();
        bare.volume_falloff = None;
        let mut no_curve = Rig::new(vec![sound("boom", vec![bare])]).with_ear();
        let started = no_curve.run(|stage| stage.submit(&at(), 0, chosen(&clip, None, 1)));
        assert_eq!(
            started.result,
            PlaybackVerdict::Failed(PlaybackBlocker::AttenuationCurveAbsent)
        );

        let broken = SnapshotSoundAlias {
            volume_falloff: Some(SnapshotAudioCurve {
                name: "one".to_owned(),
                knots: vec![(0.0, 1.0)],
            }),
            ..world_row()
        };
        let mut bad_curve = Rig::new(vec![sound("boom", vec![broken])]).with_ear();
        let started = bad_curve.run(|stage| stage.submit(&at(), 0, chosen(&clip, None, 1)));
        assert_eq!(
            started.result,
            PlaybackVerdict::Failed(PlaybackBlocker::AttenuationMath)
        );
        assert!(bad_curve.voices().is_empty());
    }

    #[test]
    fn a_sound_out_of_reach_is_suppressed_but_its_layer_still_starts() {
        let mut rig = Rig::new(vec![
            sound("far", vec![world_row()]),
            sound("tail", vec![clipless()]),
        ])
        .with_ear();
        let clip = key();
        let at = Want {
            spot: Some([0.0, 5000.0, 0.0]),
            ..want("far")
        };
        let started = rig.run(|stage| stage.submit(&at, 0, chosen(&clip, Some("tail"), 1)));
        assert_eq!(
            started.result,
            PlaybackVerdict::Suppressed(MuteCause::Inaudible)
        );
        assert_eq!(
            started.detail.as_deref(),
            Some("dist=5000 dist_min=100 dist_max=1100 atten=0.000")
        );
        assert_eq!(
            started.layered,
            Some(("tail".to_owned(), PlaybackVerdict::Failed(PlaybackBlocker::NoPcm)))
        );
        assert!(rig.voices().is_empty());
        assert!(rig.pick.recent.is_empty());
    }

    #[test]
    fn the_layer_after_a_sound_is_followed_to_a_depth_limit() {
        let mut rig = Rig::new(vec![
            sound("main", vec![row()]),
            sound("tail", vec![clipless()]),
        ]);
        let clip = key();
        let started =
            rig.run(|stage| stage.submit(&want("main"), 0, chosen(&clip, Some("tail"), 1)));
        assert_eq!(started.result, PlaybackVerdict::Handed);
        assert_eq!(
            started.layered,
            Some(("tail".to_owned(), PlaybackVerdict::Failed(PlaybackBlocker::NoPcm)))
        );

        let deep = rig.run(|stage| stage.submit(&want("main"), 10, chosen(&clip, Some("tail"), 1)));
        assert_eq!(
            deep.layered,
            Some(("tail".to_owned(), PlaybackVerdict::Failed(PlaybackBlocker::NoPcm)))
        );
        let empty = rig.run(|stage| stage.submit(&want("main"), 0, chosen(&clip, Some(""), 1)));
        assert!(empty.layered.is_none());
        let none = rig.run(|stage| stage.submit(&want("main"), 0, chosen(&clip, None, 1)));
        assert!(none.layered.is_none());
    }

    #[test]
    fn an_old_namespace_sound_claims_an_alias_slot_and_the_limit_cuts_the_oldest() {
        let limited = SnapshotSoundAlias {
            alias_bits: Some(1 << 25),
            cap_total: Some(1),
            ..row()
        };
        let mut rig = Rig::new(vec![sound("shot", vec![limited])]);
        let clip = key();
        let old = Want {
            namespace: T5,
            bound: Some(0),
            ..want("shot")
        };
        rig.run(|stage| stage.submit(&old, 0, chosen(&clip, None, 1)));
        let first = rig.voices();
        assert_eq!(first.len(), 1);
        assert!(rig.world.get::<AliasVoiceLease>(first[0]).is_some());

        let again = rig.run(|stage| stage.submit(&old, 0, chosen(&clip, None, 1)));
        assert_eq!(again.result, PlaybackVerdict::Handed);
        let now = rig.voices();
        assert_eq!(now.len(), 1);
        assert_ne!(now[0], first[0]);
    }

    #[test]
    fn starting_a_sound_reports_each_way_it_can_miss() {
        let chained = SnapshotSoundAlias {
            layered_alias: Some("silent".to_owned()),
            ..clipless()
        };
        let mut rig = Rig::new(vec![
            sound("silent", vec![clipless()]),
            sound("chained", vec![chained]),
            sound("heard", vec![row()]),
        ]);
        let gone = rig.run(|stage| stage.start(&want("nothing"), 0));
        assert_eq!(
            gone.result,
            PlaybackVerdict::Failed(PlaybackBlocker::UnknownCue)
        );
        assert_eq!(gone.picked, None);

        let no_clip = rig.run(|stage| stage.start(&want("silent"), 0));
        assert_eq!(no_clip.result, PlaybackVerdict::Failed(PlaybackBlocker::NoPcm));
        assert_eq!(no_clip.picked, Some(0));
        assert!(no_clip.layered.is_none());

        let layered = rig.run(|stage| stage.start(&want("chained"), 0));
        assert_eq!(layered.result, PlaybackVerdict::Failed(PlaybackBlocker::NoPcm));
        assert_eq!(
            layered.layered,
            Some((
                "silent".to_owned(),
                PlaybackVerdict::Failed(PlaybackBlocker::NoPcm)
            ))
        );

        let no_store = rig.run(|stage| stage.start(&want("heard"), 0));
        assert_eq!(no_store.result, PlaybackVerdict::Failed(PlaybackBlocker::NoPcm));
        assert_eq!(no_store.picked, Some(0));
        assert!(rig.pending.entries.is_empty());
    }

    #[test]
    fn a_clip_still_decoding_is_parked_with_its_numbers() {
        let mut rig = Rig::new(vec![sound("heard", vec![row()])]);
        rig.clips = Some(ClipRepository::start(Arc::new(rig.bank.clone()), None));
        let slow = Want {
            spot: Some([1.0, 2.0, 3.0]),
            source_entity_no: Some(5),
            class: NoiseCategory::Weapon,
            tilt_multiplier: 2.0,
            epoch: 3,
            ..want("heard")
        };
        let started = rig.run(|stage| stage.start(&slow, 0));
        match started.result {
            PlaybackVerdict::Pending => {
                let queued = &rig.pending.entries[0];
                assert_eq!(queued.alias, "heard");
                assert_eq!((queued.variant_ix, queued.gain, queued.pitch_mul), (0, 1.0, 2.0));
                assert_eq!(queued.position_in_inches, Some([1.0, 2.0, 3.0]));
                assert_eq!((queued.source_entity_no, queued.gen_tag), (Some(5), 3));
                assert_eq!(queued.class, NoiseCategory::Weapon);
                assert!(queued.mix_layer.is_none());
            }
            PlaybackVerdict::Failed(PlaybackBlocker::UnreadableBytes) => {
                assert!(rig.pending.entries.is_empty())
            }
            other => panic!("unexpected outcome {other}"),
        }
    }

    #[test]
    fn a_bad_clip_ends_a_waiting_start_as_a_decode_failure() {
        let mut rig = Rig::new(vec![sound("heard", vec![row()])]);
        let mut store = ClipRepository::start(Arc::new(rig.bank.clone()), None);
        store.req_in(key());
        rig.clips = Some(store);
        let give_up = Instant::now() + Duration::from_secs(20);
        rig.pending.queue_oneshot(waiting(
            "heard",
            None,
            WORLD,
            0,
            Duration::from_secs(60),
        ));
        loop {
            rig.run(|stage| stage.drain_pending());
            if !rig.decisions.is_empty() {
                break;
            }
            assert!(Instant::now() < give_up, "the clip never settled");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(rig.pending.entries.is_empty());
        let log = rig.log();
        assert_eq!(log.len(), 1);
        assert!(
            log[0].contains("`t6:heard` variant=0 result=FailedDecode"),
            "{log:?}"
        );
    }

    #[test]
    fn the_pending_queue_drops_old_matches_expires_late_ones_and_keeps_the_rest() {
        let mut rig = Rig::new(vec![sound("heard", vec![row()])]);
        rig.clips = Some(ClipRepository::start(Arc::new(rig.bank.clone()), None));
        let minute = Duration::from_secs(60);
        for (tag, class, epoch, wait) in [
            (0, NoiseCategory::World, 9, minute),
            (1, NoiseCategory::Ui, 9, minute),
            (2, NoiseCategory::World, 0, Duration::ZERO),
            (3, NoiseCategory::World, 0, minute),
        ] {
            rig.pending
                .queue_oneshot(waiting("heard", Some(tag), class, epoch, wait));
        }
        rig.run(|stage| stage.drain_pending());
        let log = rig.log();
        assert!(
            log.iter()
                .any(|l| l.contains("result=ExpiredAwaitingDecode")),
            "{log:?}"
        );
        let kept: Vec<_> = rig.pending.entries.iter().map(|e| e.source_entity_no).collect();
        assert!(
            !kept.contains(&Some(0)) && !kept.contains(&Some(2)),
            "{kept:?}"
        );
        assert!(
            kept.contains(&Some(1)) || log.len() >= 2,
            "{kept:?} {log:?}"
        );
    }

    #[test]
    fn a_surface_chain_tries_black_ops_names_first_and_reports_the_last_miss() {
        let (step, fallback) = footfall_cue_names(FootfallPace::Run, ASPHALT, true, false);
        let mut rig = Rig::new(vec![]);
        let nothing =
            rig.run(|stage| stage.play_surface((step, fallback, None, None), NoiseCategory::World));
        assert_eq!(nothing, PlaybackVerdict::Failed(PlaybackBlocker::UnknownCue));
        assert!(rig.voices().is_empty());

        let mut rig = Rig::new(vec![sound("fly_step_run_plr_asphalt", vec![clipless()])]);
        let one =
            rig.run(|stage| stage.play_surface((step, fallback, None, None), NoiseCategory::World));
        assert_eq!(one, PlaybackVerdict::Failed(PlaybackBlocker::UnknownCue));
        let log = rig.log();
        assert!(log[0].contains("result=FailedNoPcm"), "{log:?}");
        assert!(log[0].contains("`t6:fly_step_run_plr_asphalt`"), "{log:?}");
    }

    #[test]
    fn a_plain_play_logs_one_decision_and_returns_the_outcome() {
        let mut rig = Rig::new(vec![sound("silent", vec![clipless()])]);
        let outcome = rig.run(|stage| stage.play(&want("silent")));
        assert_eq!(outcome, PlaybackVerdict::Failed(PlaybackBlocker::NoPcm));
        let log = rig.log();
        assert_eq!(log.len(), 1);
        assert!(
            log[0].contains("`t6:silent` variant=0 result=FailedNoPcm"),
            "{log:?}"
        );
    }

    #[test]
    fn play_alias_oneshot_runs_the_whole_start_path() {
        let mut world = World::new();
        let mut queue = CommandQueue::default();
        let mut commands = Commands::new(&mut queue, &world);
        let bank = catalog(vec![sound("silent", vec![clipless()])]);
        let mut assets = Assets::<ClipPcm>::default();
        let mut shared = CommonPlaybackStore::default();
        let mut pick = CuePickState::default();
        let mut backlog = QueuedVoiceStarts::default();
        let mut chan_busy = ChannelUsageTally::default();
        let mut verdicts = PlaybackVerdictLog::default();
        let result: PlaybackVerdict = fire_single_cue(
            &mut commands,
            &mut assets,
            &mut shared,
            &bank,
            T6,
            "silent",
            Some([0.0; 3]),
            None,
            &mut pick,
            None,
            &mut backlog,
            &mut chan_busy,
            &mut verdicts,
            Some(2),
            UI,
            0,
        );
        assert_eq!(result, PlaybackVerdict::Failed(PlaybackBlocker::NoPcm));
        assert_eq!(verdicts.len(), 1);
        queue.apply(&mut world);
    }

    fn app(sounds: Option<Vec<SnapshotSoundEntry>>) -> App {
        let mut harness = App::new();
        register_message_types(&mut harness);
        harness.insert_resource(Assets::<ClipPcm>::default());
        harness.insert_resource(CuePickState::default());
        harness.insert_resource(CommonPlaybackStore::default());
        harness.insert_resource(UnknownCueHoles::default());
        harness.insert_resource(QueuedVoiceStarts::default());
        harness.insert_resource(ChannelUsageTally::default());
        harness.insert_resource(PlaybackVerdictLog::default());
        harness.init_resource::<GameRoundSerial>();
        harness.add_systems(
            Update,
            (
                start_alias_commands,
                start_footsteps,
                start_landings,
                start_weapon_sounds,
                start_bound_weapon_sounds,
                start_parked_sounds,
            ),
        );
        if let Some(list) = sounds {
            harness.insert_resource(AudioCatalogRes(Arc::new(catalog(list))));
        }
        harness
    }

    fn gaps(app: &App) -> Vec<String> {
        app.world().resource::<UnknownCueHoles>().aliases.clone()
    }

    fn log(app: &App) -> Vec<String> {
        app.world().resource::<PlaybackVerdictLog>().lines().collect()
    }

    fn play(cue_id: &str, fallback: Option<&str>) -> AliasCommand {
        AliasCommand::Play(TriggerCueMsg {
            cue_ns: T6,
            alias: cue_id.to_owned(),
            fallback: fallback.map(str::to_owned),
            position_in_inches: NOWHERE,
            source_entity_no: None,
        })
    }

    fn weapon(cue_id: &str) -> GunNoiseMsg {
        GunNoiseMsg {
            cue_ns: T6,
            alias: cue_id.to_owned(),
            position_in_inches: NOWHERE,
            source_entity_no: None,
        }
    }

    #[test]
    fn every_request_is_logged_as_dropped_while_there_is_no_bank() {
        let mut app = app(None);
        app.world_mut().write_message(play("a", None));
        app.world_mut().write_message(FootfallMsg {
            alias: "step",
            fallback: "step_default",
            position_in_inches: NOWHERE,
            source_entity_no: None,
        });
        app.world_mut().write_message(TouchdownNoiseMsg {
            alias: "land",
            fallback: "land_default",
            position_in_inches: NOWHERE,
            source_entity_no: None,
        });
        app.world_mut().write_message(weapon("gun"));
        app.update();
        let lines = log(&app);
        assert_eq!(lines.len(), 4, "{lines:?}");
        for alias in ["a", "step", "land", "gun"] {
            let want = format!("`t6:{alias}` variant=- result=FailedBankMissing");
            assert!(
                lines.iter().any(|line| line.contains(&want)),
                "{alias}: {lines:?}"
            );
        }
        assert!(gaps(&app).is_empty());
    }

    #[test]
    fn an_alias_with_nothing_to_play_tries_its_fallback_then_lists_both() {
        let mut app = app(Some(vec![sound("a", vec![clipless()])]));
        app.world_mut().write_message(play("a", Some("b")));
        app.world_mut().write_message(play("c", None));
        app.update();
        assert_eq!(gaps(&app), ["b", "a", "c"]);
        let lines = log(&app);
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(lines[0].contains("`t6:a` variant=0 result=FailedNoPcm"));
        assert!(lines[1].contains("`t6:b` variant=- result=FailedMissingAlias"));
        assert!(lines[2].contains("`t6:c` variant=- result=FailedMissingAlias"));
    }

    #[test]
    fn a_pitched_play_starts_like_a_plain_one() {
        let mut app = app(Some(vec![sound("a", vec![clipless()])]));
        app.world_mut().write_message(AliasCommand::PlayPitched {
            sound: TriggerCueMsg {
                cue_ns: T6,
                alias: "a".to_owned(),
                fallback: Option::default(),
                position_in_inches: NOWHERE,
                source_entity_no: None,
            },
            pitch: 1.2,
        });
        app.update();
        assert_eq!(gaps(&app), ["a"]);
        assert_eq!(log(&app).len(), 1);
    }

    #[test]
    fn legacy_movement_names_play_as_black_ops_names_or_not_at_all() {
        let (step, _) = footfall_cue_names(FootfallPace::Run, ASPHALT, true, false);
        let gear = gear_jingle_cue(FootfallPace::Sprint, true);
        let mut app = app(Some(vec![
            sound("fly_step_run_plr_default", vec![clipless()]),
            sound("fly_step_run_plr_asphalt", vec![clipless()]),
            sound("fly_gear_sprint_plr", vec![clipless()]),
        ]));
        app.world_mut().write_message(play(gear, None));
        app.update();
        assert!(log(&app).is_empty());
        assert!(gaps(&app).is_empty());

        app.world_mut().write_message(play(step, None));
        app.world_mut().write_message(weapon(step));
        app.update();
        let lines = log(&app);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(
            lines
                .iter()
                .all(|line| line.contains("`t6:fly_step_run_plr_asphalt`"))
        );
    }

    #[test]
    fn footsteps_and_landings_list_what_the_bank_lacks() {
        let mut app = app(Some(vec![]));
        app.world_mut().write_message(FootfallMsg {
            alias: "step_x",
            fallback: "step_y",
            position_in_inches: NOWHERE,
            source_entity_no: None,
        });
        app.world_mut().write_message(TouchdownNoiseMsg {
            alias: "land_x",
            fallback: "land_y",
            position_in_inches: NOWHERE,
            source_entity_no: None,
        });
        app.update();
        let mut listed = gaps(&app);
        listed.sort();
        assert_eq!(listed, ["land_x", "land_y", "step_x", "step_y"]);
    }

    #[test]
    fn weapon_sounds_note_a_missing_alias_once() {
        let mut app = app(Some(vec![sound("gun", vec![clipless()])]));
        for alias in ["gun", "gun", "nope"] {
            app.world_mut().write_message(weapon(alias));
        }
        app.update();
        assert_eq!(gaps(&app), ["gun", "nope"]);
        assert_eq!(log(&app).len(), 3);
    }

    #[test]
    fn bound_weapon_sounds_check_the_bank_revision_and_slot() {
        let mut app = app(Some(vec![sound("gun", vec![clipless()])]));
        let revision = app.world().resource::<AudioCatalogRes>().0.revision();
        for (catalog_edition, index) in [(revision + 1, 0), (revision, 5), (revision, 0)] {
            app.world_mut().write_message(BoundWeaponSound {
                catalog_edition,
                index,
                position_in_inches: NOWHERE,
                source_entity_no: Some(4),
            });
        }
        app.update();
        let lines = log(&app);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains("`t6:gun` variant=0 result=FailedNoPcm"));
        assert_eq!(gaps(&app), ["gun"]);
    }

    #[test]
    fn a_stop_cancels_waiting_starts_and_ends_matching_voices_only() {
        let mut app = app(Some(vec![]));
        let minute = Duration::from_secs(60);
        let mut starts = QueuedVoiceStarts::default();
        starts.queue_oneshot(waiting("loop_a", Some(1), NoiseCategory::World, 0, minute));
        starts.queue_oneshot(waiting("loop_a", Some(2), NoiseCategory::World, 0, minute));
        starts.queue_oneshot(waiting("loop_a", Some(1), NoiseCategory::Ui, 0, minute));
        starts.queue_oneshot(waiting("loop_a", Some(1), NoiseCategory::World, 5, minute));
        app.insert_resource(starts);

        let spawn = |app: &mut App, cue_id: &str, source_entity_no, scope, epoch| {
            let tag = AliasPlayback {
                namespace: T6,
                source_entity_no,
                alias: cue_id.to_owned(),
            };
            let mut queue = CommandQueue::default();
            let entity = {
                let mut commands = Commands::new(&mut queue, app.world());
                crate::backend::begin_single_cue(
                    &mut commands,
                    Handle::default(),
                    Volume::Linear(FULL),
                    1.0,
                    epoch,
                    scope,
                )
            };
            queue.apply(app.world_mut());
            app.world_mut().entity_mut(entity).insert(tag);
            entity
        };
        let doomed = spawn(&mut app, "loop_a", Some(1), SoundAudience::Match, 0);
        let other_ent = spawn(&mut app, "loop_a", Some(2), SoundAudience::Match, 0);
        let other_alias = spawn(&mut app, "loop_b", Some(1), SoundAudience::Match, 0);
        let other_epoch = spawn(&mut app, "loop_a", Some(1), SoundAudience::Match, 5);
        let menu = spawn(&mut app, "loop_a", Some(1), SoundAudience::Menu, 0);

        app.world_mut().write_message(AliasCommand::Stop {
            namespace: T6,
            alias: "loop_a".to_owned(),
            source_entity_no: Some(1),
        });
        app.update();

        let left: Vec<_> = app
            .world()
            .resource::<QueuedVoiceStarts>()
            .entries
            .iter()
            .map(|entry| (entry.source_entity_no, entry.gen_tag, entry.class))
            .collect();
        assert_eq!(
            left,
            [
                (Some(2), 0, NoiseCategory::World),
                (Some(1), 0, NoiseCategory::Ui),
                (Some(1), 5, NoiseCategory::World)
            ]
        );
        assert!(app.world().get_entity(doomed).is_err());
        for survivor in [other_ent, other_alias, other_epoch, menu] {
            assert!(app.world().get_entity(survivor).is_ok());
        }
    }

    #[test]
    fn the_pending_queue_is_cleared_without_a_bank_and_kept_without_clips() {
        let minute = Duration::from_secs(60);
        let mut no_bank = app(None);
        no_bank
            .world_mut()
            .resource_mut::<QueuedVoiceStarts>()
            .queue_oneshot(waiting("x", None, NoiseCategory::World, 0, minute));
        no_bank.update();
        assert!(
            no_bank
                .world()
                .resource::<QueuedVoiceStarts>()
                .entries
                .is_empty()
        );

        let mut no_clips = app(Some(vec![]));
        no_clips
            .world_mut()
            .resource_mut::<QueuedVoiceStarts>()
            .queue_oneshot(waiting("x", None, NoiseCategory::World, 0, minute));
        no_clips.update();
        assert_eq!(
            no_clips.world().resource::<QueuedVoiceStarts>().entries.len(),
            1
        );
        assert!(log(&no_clips).is_empty());
    }

    #[test]
    #[should_panic(expected = "more than one ambient listener")]
    fn two_ears_are_a_bug() {
        let mut app = app(Some(vec![]));
        let world = app.world_mut();
        for _ in 0..2 {
            world.spawn((BackgroundHearer, Transform::default()));
        }
        app.world_mut().write_message(play("a", None));
        app.update();
    }

    #[test]
    fn a_server_local_sound_becomes_a_play_or_a_stop_for_the_local_entity() {
        let play = local_sound_message(T6, "ui_ping".to_owned(), false);
        let AliasCommand::Play(event) = play else {
            panic!("expected a play, got {play:?}");
        };
        assert_eq!(event.alias, "ui_ping");
        assert_eq!(event.source_entity_no, Some(SOUND_SOURCE_SELF));
        assert!(event.fallback.is_none() && event.position_in_inches.is_none());

        let stop = local_sound_message(T6, "ui_ping".to_owned(), true);
        assert!(matches!(
            stop,
            AliasCommand::Stop { alias, source_entity_no: Some(SOUND_SOURCE_SELF), .. } if alias == "ui_ping"
        ));
    }

    #[test]
    fn only_play_commands_have_something_to_play() {
        let stop = AliasCommand::Stop {
            namespace: T6,
            alias: "a".to_owned(),
            source_entity_no: None,
        };
        assert!(playable(&stop).is_none());
        let plain = AliasCommand::Play(bare_play(T6, "a".to_owned(), Some(3)));
        assert_eq!(playable(&plain).map(|(_, pitch)| pitch), Some(1.0));
        let pitched = AliasCommand::PlayPitched {
            sound: bare_play(T6, "a".to_owned(), None),
            pitch: 1.25,
        };
        assert_eq!(playable(&pitched).map(|(_, pitch)| pitch), Some(1.25));
    }

    #[test]
    fn a_world_sound_on_the_ears_axis_is_centred_and_loses_nothing_to_distance_scale() {
        let (left, right) = oneshot_lr_gains(Vec3::ZERO, Vec3::X, Vec3::Y, 1.0);
        assert!((left - right).abs() < 1e-6);
        let (quiet_left, quiet_right) =
            oneshot_lr_gains(Vec3::ZERO, Vec3::X, Vec3::Y, 0.0);
        assert_eq!((quiet_left, quiet_right), (0.0, 0.0));
    }
}
