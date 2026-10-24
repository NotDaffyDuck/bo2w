//! Starting sounds: the systems that turn alias messages into voices, the
//! queue for sounds still waiting on a decode, and the 3D placement maths.

use std::collections::HashMap;
use std::f32::consts::FRAC_PI_4;
use std::sync::Arc;
use std::time::Instant;

use asset_audio::{SoundCatalog, lerp_range, unit_random};
use asset_core::AssetNamespace;
use bevy::{
    audio::{AddAudioSource, AudioSink, AudioSinkPlayback, Volume},
    ecs::system::SystemParam,
    prelude::*,
};
use frame::{ClientSet, FxSoundPublished, MatchTornDown, SessionSwapApplied};
use net::{LastAdoptedSnapshot, SvcLocalSound};

use crate::attenuation::{SND_CURVE_MAX_KNOTS, distance_attenuation};
use crate::backend::MatchEpoch;
use crate::clip_store::{
    ClipKey, ClipStore, PendingOneshot, PendingStarts, clip_key_for_variant, clip_keys_for_alias,
    deadline_for,
};
use crate::messages::{
    AliasCommand, BoundWeaponSound, Footstep, LandSound, PlayAlias, SND_ENT_LOCAL,
    ViewmodelNotetracks, WeaponSound,
};
use crate::pcm::{LoopingPcmAudio, PcmAudio};
use crate::space::{distance_inches, transform_inches};
use crate::start::{
    SoundClass, StartDecision, StartDecisions, StartFailure, StartOutcome, SuppressReason,
};
use crate::voice::{VoiceLease, VoiceOccupancy, has_free_voice, reclaim_finished_voices};

/// Marks the entity whose transform is the player's ear.
#[derive(Component, Default)]
pub struct AmbientListener;

/// A positional sound that is retuned every frame from where the ear is.
#[derive(Component)]
pub struct Channel3d {
    pub origin_inches: [f32; 3],
    pub dist_min: f32,
    pub dist_max: f32,
    pub knots: Arc<[[f32; 2]]>,
    pub near_knots: Option<Arc<[[f32; 2]]>>,
    pub priority: Option<asset_audio::VoicePriority>,
    pub base_volume: f32,

    pub live_pan: crate::pcm::LivePan,
    /// bo2mp: a Black Ops II 3D sound with a pan of its own keeps those stereo
    /// gains: it falls off with distance but does not turn with the listener.
    pub fixed_pan: Option<(f32, f32)>,
    /// bo2mp: the Black Ops II menu volume this sound answers to.
    pub category: Option<u8>,
}

#[derive(Resource, Clone)]
pub struct SoundBank(pub Arc<SoundCatalog>);

/// Random state for choosing between an alias's variants.
#[derive(Resource)]
pub struct SoundPickState {
    pub lcg: u32,
    pub last_variant: HashMap<(AssetNamespace, String), usize>,
}

impl Default for SoundPickState {
    fn default() -> Self {
        Self {
            lcg: 0x00a5_5a5a,
            last_variant: HashMap::new(),
        }
    }
}

/// What a playing voice was started for, so a later stop can find it.
#[derive(Component)]
pub struct AliasPlayback {
    pub namespace: AssetNamespace,
    pub snd_ent: Option<u32>,
    pub alias: String,
}

/// Aliases that were asked for and could not be found, each listed once.
#[derive(Resource, Default, Debug)]
pub struct MissingAliasGaps {
    pub aliases: Vec<String>,
}

impl MissingAliasGaps {
    pub fn record(&mut self, alias: &str) {
        if self.aliases.iter().all(|known| known != alias) {
            diag::warn!(Audio, "audio: missing alias `{alias}` (typed gap)");
            self.aliases.push(alias.to_owned());
        }
    }

    pub fn len(&self) -> usize {
        self.aliases.len()
    }

    pub fn is_empty(&self) -> bool {
        self.aliases.is_empty()
    }
}

/// Assets shared by every play of the same clip or falloff curve.
#[derive(Resource, Default)]
pub(crate) struct SharedPlayAssets {
    dry: HashMap<ClipKey, Handle<PcmAudio>>,
    curves: HashMap<String, Arc<[[f32; 2]]>>,
    common_dry_reuses: usize,
}

impl SharedPlayAssets {
    /// The asset for a clip's unpanned sound: the one already made this match,
    /// else the resident one from a previous match, else a new one.
    fn dry_handle(
        &mut self,
        pcm_assets: &mut Assets<PcmAudio>,
        clip: &ClipKey,
        pcm: PcmAudio,
        clips: Option<&ClipStore>,
    ) -> Handle<PcmAudio> {
        if let Some(handle) = self.dry.get(clip) {
            return handle.clone();
        }
        let resident = clips.and_then(|store| store.resident_dry_handle(clip, pcm_assets, &pcm));
        let handle = match resident {
            Some((handle, reused)) => {
                if reused {
                    if self.common_dry_reuses == 0 {
                        diag::info!(Audio, "audio: first common dry handle reused: {clip:?}");
                    }
                    self.common_dry_reuses += 1;
                }
                handle
            }
            None => pcm_assets.add(pcm),
        };
        self.dry.insert(clip.clone(), handle.clone());
        handle
    }

    /// One shared copy of a named falloff curve, cut to the engine's knot limit.
    pub(crate) fn intern_curve(&mut self, name: &str, knots: &[(f32, f32)]) -> Arc<[[f32; 2]]> {
        Arc::clone(self.curves.entry(name.to_owned()).or_insert_with(|| {
            knots
                .iter()
                .take(SND_CURVE_MAX_KNOTS)
                .map(|&(x, y)| [x, y])
                .collect()
        }))
    }
}

pub(crate) struct PlayerSoundPlugin;

impl Plugin for PlayerSoundPlugin {
    fn build(&self, app: &mut App) {
        if app.is_plugin_added::<bevy::audio::AudioPlugin>() {
            app.add_audio_source::<PcmAudio>()
                .add_audio_source::<LoopingPcmAudio>();
        } else {
            // BO2MP_HEADLESS=1: there is no sound device, so nothing plays the
            // clips. They stay assets and the volume setting keeps a home.
            app.init_asset::<PcmAudio>()
                .init_asset::<LoopingPcmAudio>()
                .init_resource::<bevy::audio::GlobalVolume>();
        }
        app.init_resource::<SoundPickState>()
            .init_resource::<crate::clip_store::ResidentClipCache>()
            .init_resource::<MissingAliasGaps>()
            .init_resource::<SharedPlayAssets>()
            .init_resource::<StartDecisions>()
            .init_resource::<PendingStarts>()
            .init_resource::<VoiceOccupancy>()
            .init_resource::<crate::ambient::MapAmbientBooted>()
            .init_resource::<crate::ambient::SoundBankLoadAttempted>()
            .init_resource::<crate::ambient::ResidentSoundBank>()
            .init_resource::<crate::BobCycleTracker>()
            .init_resource::<crate::WeaponLoops>()
            .init_resource::<crate::slide_loop::SlideLoop>()
            .add_message::<AliasCommand>()
            .add_message::<Footstep>()
            .add_message::<WeaponSound>()
            .add_message::<BoundWeaponSound>()
            .add_message::<ViewmodelNotetracks>()
            .init_resource::<crate::entity_events::NotetrackSoundTable>()
            .add_message::<LandSound>()
            .add_systems(
                Update,
                (
                    crate::ambient::boot_map_ambient_once,
                    crate::ambient::play_random_point_sounds.before(play_alias_messages),
                    reclaim_finished_voices
                        .before(drain_pending_oneshots)
                        .before(play_alias_messages)
                        .before(play_footstep_messages)
                        .before(play_weapon_sound_messages)
                        .before(play_bound_weapon_sounds)
                        .before(play_land_sound_messages),
                    drain_pending_oneshots
                        .after(play_alias_messages)
                        .before(play_footstep_messages)
                        .before(play_weapon_sound_messages)
                        .before(play_land_sound_messages),
                    apply_svc_local_sound
                        .before(play_alias_messages)
                        .run_if(resource_exists::<LastAdoptedSnapshot>),
                    crate::shellshock::update_shellshock_tinnitus
                        .before(play_alias_messages)
                        .run_if(not(crate::shellshock::bank_is_t6)),
                    play_alias_messages.after(FxSoundPublished),
                    play_footstep_messages,
                    crate::entity_events::play_viewmodel_notetrack_messages
                        .before(play_weapon_sound_messages)
                        .before(play_bound_weapon_sounds),
                    crate::ads_sounds::play_ads_sounds.before(play_weapon_sound_messages),
                    play_weapon_sound_messages,
                    play_bound_weapon_sounds.after(drain_pending_oneshots),
                    play_land_sound_messages,
                    (
                        crate::destructible_loops::update,
                        crate::weapon_loops::update,
                    )
                        .before(crate::ambient::update_map_emitter_gain),
                    // The slide's start sound is cut a frame after
                    // play_alias_messages begins it.
                    crate::slide_loop::update
                        .after(play_alias_messages)
                        .before(crate::ambient::update_map_emitter_gain),
                    crate::zm_music::update,
                    crate::ambient::update_map_emitter_gain.after(crate::duck::DuckSet),
                    update_all_channels.after(crate::duck::DuckSet),
                )
                    .in_set(ClientSet::Effects),
            )
            .add_systems(
                Update,
                (
                    crate::ambient::start_sound_bank_compose
                        .after(crate::ambient::stop_map_ambient_on_match_end),
                    crate::ambient::install_sound_bank
                        .after(crate::ambient::start_sound_bank_compose),
                    crate::entity_events::bind_notetrack_sounds
                        .after(crate::ambient::install_sound_bank),
                    crate::ambient::stop_map_ambient_on_match_end.after(SessionSwapApplied),
                    reset_clip_prep_on_match_torn_down,
                )
                    .in_set(ClientSet::Load),
            );
    }
}

fn reset_clip_prep_on_match_torn_down(
    mut torn: MessageReader<MatchTornDown>,
    mut pending: ResMut<PendingStarts>,
    mut shared: ResMut<SharedPlayAssets>,
) {
    if torn.read().next().is_none() {
        return;
    }
    torn.clear();
    pending.clear();
    diag::info!(
        Audio,
        "audio: common dry handles reused this match={}",
        shared.common_dry_reuses
    );
    *shared = SharedPlayAssets::default();
}

/// The ear's position and its right-hand direction. Two ears is a bug.
fn listener_pose(listeners: &Query<&Transform, With<AmbientListener>>) -> Option<(Vec3, Vec3)> {
    let mut ears = listeners.iter();
    let first = ears.next()?;
    assert!(ears.next().is_none(), "more than one ambient listener");
    Some((first.translation, first.rotation * Vec3::X))
}

/// Retunes every live positional sound: priority, falloff, pan and level.
fn update_all_channels(
    listeners: Query<&Transform, With<AmbientListener>>,
    mut channels: Query<(Entity, &Channel3d, &mut AudioSink, Option<&crate::DuckGroup>)>,
    mut occupancy: ResMut<VoiceOccupancy>,
    settings: Res<frame::GameSettings>,
    ducks: Res<crate::DuckMixer>,
) {
    let Some((ear, right)) = listener_pose(&listeners) else {
        return;
    };
    let ear_inches = transform_inches(ear);
    for (entity, channel, mut sink, group) in &mut channels {
        let distance = distance_inches(ear_inches, channel.origin_inches);
        if let Some(priority) = &channel.priority {
            occupancy.update_alias_priority(entity, priority.evaluate(Some(distance)));
        }

        let falloff = if channel.knots.is_empty() {
            0.0
        } else {
            distance_attenuation(
                &channel.knots,
                channel.near_knots.as_deref(),
                distance,
                channel.dist_min,
                channel.dist_max,
            )
            .max(0.0)
        };
        if channel.fixed_pan.is_none() {
            let source = Vec3::from_array(channel.origin_inches);
            let (left, right_gain) = world_oneshot_channel_gains(ear, right, source, 1.0);
            channel.live_pan.set(left, right_gain);
        }

        let menu = settings.menu_volume(channel.category) * ducks.linear(group.map(|g| g.0));
        let level = channel.base_volume * falloff * settings.master_gain() * menu;
        sink.set_volume(Volume::Linear(level.max(0.0)));
    }
}

/// Plays or stops the sounds the server names by configstring index.
fn apply_svc_local_sound(
    mut commands_in: MessageReader<SvcLocalSound>,
    adopted: Res<LastAdoptedSnapshot>,
    announcer: Res<crate::match_voices::AnnouncerRoutes>,
    mut play: MessageWriter<crate::AliasCommand>,
) {
    for command in commands_in.read() {
        let routed = adopted
            .sound_alias_name(command.index)
            .map(|name| announcer.route(name))
            .map(|(namespace, alias)| (namespace, alias.to_owned()));
        let Some((namespace, alias)) = routed else {
            diag::warn!(
                Audio,
                "audio: svc local sound CS index {} is unresolved (typed gap)",
                command.index
            );
            continue;
        };
        let message = if command.stop {
            AliasCommand::Stop {
                namespace,
                alias,
                snd_ent: Some(SND_ENT_LOCAL),
            }
        } else {
            AliasCommand::Play(PlayAlias {
                namespace,
                alias,
                fallback: None,
                origin_inches: None,
                snd_ent: Some(SND_ENT_LOCAL),
            })
        };
        play.write(message);
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

fn t6_movement(bank: &SoundCatalog, namespace: AssetNamespace, alias: &str) -> T6Movement {
    let on_t6_map = namespace == AssetNamespace::T6
        && bank
            .index_in(AssetNamespace::T6, "fly_step_run_plr_default")
            .is_some();
    if !on_t6_map {
        return T6Movement::NotT6;
    }
    let Some(t6) = crate::aliases::t6_movement_alias(alias) else {
        return T6Movement::NotT6;
    };
    let is_gear = t6.starts_with("fly_gear_") && !t6.starts_with("fly_gear_mantle");
    if !is_gear && bank.index_in(AssetNamespace::T6, t6).is_some() {
        T6Movement::Play(t6)
    } else {
        T6Movement::Skip
    }
}

/// Everything the sound-starting systems share, so each one declares a single
/// parameter instead of a dozen.
#[derive(SystemParam)]
struct SoundDesk<'w, 's> {
    commands: Commands<'w, 's>,
    pcm_assets: ResMut<'w, Assets<PcmAudio>>,
    shared: ResMut<'w, SharedPlayAssets>,
    pick: ResMut<'w, SoundPickState>,
    gaps: ResMut<'w, MissingAliasGaps>,
    clips: Option<ResMut<'w, ClipStore>>,
    pending: ResMut<'w, PendingStarts>,
    occupancy: ResMut<'w, VoiceOccupancy>,
    decisions: ResMut<'w, StartDecisions>,
    bank: Option<Res<'w, SoundBank>>,
    epoch: Res<'w, MatchEpoch>,
    listeners: Query<'w, 's, &'static Transform, With<AmbientListener>>,
}

impl<'w, 's> SoundDesk<'w, 's> {
    /// Splits the desk into a stage for starting sounds and the list of
    /// missing aliases. `None` while there is no sound bank.
    fn open<'a>(&'a mut self) -> Option<(Stage<'a, 'w, 's>, &'a mut MissingAliasGaps)> {
        let bank: &SoundCatalog = &self.bank.as_ref()?.0;
        let pose = listener_pose(&self.listeners);
        let stage = Stage {
            commands: &mut self.commands,
            pcm_assets: &mut *self.pcm_assets,
            shared: &mut *self.shared,
            bank,
            pick: &mut *self.pick,
            clips: self.clips.as_deref_mut(),
            pending: &mut *self.pending,
            occupancy: &mut *self.occupancy,
            decisions: &mut *self.decisions,
            pose,
            epoch: self.epoch.0,
        };
        Some((stage, &mut *self.gaps))
    }

    /// Ends the voices a `Stop` names, and cancels starts still waiting on a
    /// decode for the same sound.
    fn stop_alias(&mut self, namespace: AssetNamespace, alias: &str, snd_ent: Option<u32>) {
        let epoch = self.epoch.0;
        self.pending.cancel_alias(namespace, alias, snd_ent, epoch);
        let alias = alias.to_owned();
        // Queued rather than done now, so it lands after the spawns this same
        // batch already issued; a Play that comes later still makes a new voice.
        self.commands.queue(move |world: &mut World| {
            let doomed: Vec<Entity> = world
                .query::<(Entity, &AliasPlayback, &crate::backend::Voice)>()
                .iter(world)
                .filter(|(_, tag, voice)| {
                    tag.namespace == namespace
                        && tag.alias == alias
                        && tag.snd_ent == snd_ent
                        && voice.epoch == epoch
                        && voice.scope == crate::backend::AudioScope::Match
                })
                .map(|(entity, ..)| entity)
                .collect();
            for entity in doomed {
                world.despawn(entity);
            }
        });
    }
}

/// The borrowed parts of the world that one batch of sound starts works on.
struct Stage<'a, 'w, 's> {
    commands: &'a mut Commands<'w, 's>,
    pcm_assets: &'a mut Assets<PcmAudio>,
    shared: &'a mut SharedPlayAssets,
    bank: &'a SoundCatalog,
    pick: &'a mut SoundPickState,
    clips: Option<&'a mut ClipStore>,
    pending: &'a mut PendingStarts,
    occupancy: &'a mut VoiceOccupancy,
    decisions: &'a mut StartDecisions,
    /// The ear's position and right-hand direction, when there is an ear.
    pose: Option<(Vec3, Vec3)>,
    /// The match the stage is working in.
    epoch: u64,
}

/// One sound to start.
#[derive(Clone, Copy)]
struct Want<'a> {
    namespace: AssetNamespace,
    alias: &'a str,
    /// The alias's index in the bank, when the caller already bound it.
    bound: Option<usize>,
    origin: Option<[f32; 3]>,
    snd_ent: Option<u32>,
    class: SoundClass,
    pitch_scale: f32,
    epoch: u64,
}

/// A clip that decoded, with the numbers it will be played at.
struct Chosen<'a> {
    pcm: PcmAudio,
    clip: &'a ClipKey,
    variant: usize,
    volume: f32,
    pitch: f32,
    layer: Option<&'a str>,
}

/// Where a 3D sound sits and how loud its falloff leaves it.
struct Placed {
    pos: [f32; 3],
    dist: f32,
    dist_min: f32,
    dist_max: f32,
    atten: f32,
    knots: Arc<[[f32; 2]]>,
    near_knots: Option<Arc<[[f32; 2]]>>,
    fixed_pan: Option<(f32, f32)>,
    gains: (f32, f32),
}

enum ClipTake {
    Ready(PcmAudio),
    Pending,
    Failed(StartFailure),
}

/// What one attempt to start a sound came to.
struct OneshotStart {
    outcome: StartOutcome,
    variant: Option<usize>,
    secondary: Option<(String, StartOutcome)>,
    detail: Option<String>,
}

impl OneshotStart {
    fn failed(failure: StartFailure) -> Self {
        Self {
            outcome: StartOutcome::Failed(failure),
            variant: None,
            secondary: None,
            detail: None,
        }
    }

    fn with_variant(mut self, variant: usize) -> Self {
        self.variant = Some(variant);
        self
    }

    fn suppressed(reason: SuppressReason, variant: usize, detail: Option<String>) -> Self {
        Self {
            outcome: StartOutcome::Suppressed(reason),
            variant: Some(variant),
            secondary: None,
            detail,
        }
    }
}

fn falloff_detail(dist: f32, dist_min: f32, dist_max: f32, atten: f32) -> String {
    format!("dist={dist:.0} dist_min={dist_min:.0} dist_max={dist_max:.0} atten={atten:.3}")
}

fn sound_for<'b>(bank: &'b SoundCatalog, want: &Want<'_>) -> Option<&'b asset_audio::CapturedSound> {
    match want.bound {
        Some(index) => bank.sound_at(index),
        None => bank.sound_in(want.namespace, want.alias),
    }
}

impl<'a, 'w, 's> Stage<'a, 'w, 's> {
    /// A plain request for `alias`, to be adjusted with struct update syntax.
    fn want<'x>(&self, namespace: AssetNamespace, alias: &'x str, class: SoundClass) -> Want<'x> {
        Want {
            namespace,
            alias,
            bound: None,
            origin: None,
            snd_ent: None,
            class,
            pitch_scale: 1.0,
            epoch: self.epoch,
        }
    }

    /// Starts a sound and logs how it went.
    fn play(&mut self, want: &Want<'_>) -> StartOutcome {
        let started = self.start(want, 0);
        self.record_start(want.namespace, want.alias, started)
    }

    fn record_start(
        &mut self,
        namespace: AssetNamespace,
        alias: &str,
        started: OneshotStart,
    ) -> StartOutcome {
        if let Some((layer, result)) = &started.secondary
            && !result.is_open()
        {
            diag::warn!(
                Audio,
                "audio: secondary `{layer}` of `{}:{alias}` result={result}",
                namespace.as_str()
            );
        }
        let outcome = started.outcome.clone();
        self.decisions.record(StartDecision {
            namespace,
            alias: alias.to_owned(),
            variant: started.variant,
            outcome: started.outcome,
            secondary: started.secondary,
            detail: started.detail,
        });
        outcome
    }

    /// A footstep or landing: tries Black Ops II's own movement names first,
    /// then the legacy names the surface table lists for the alias.
    fn play_surface(
        &mut self,
        alias: &str,
        fallback: &str,
        origin: Option<[f32; 3]>,
        snd_ent: Option<u32>,
        class: SoundClass,
    ) -> StartOutcome {
        let bank = self.bank;
        let own_names = [alias, fallback]
            .into_iter()
            .filter_map(crate::aliases::t6_movement_alias)
            .filter(|name| bank.index_in(AssetNamespace::T6, name).is_some());
        let legacy = crate::aliases::surface_alias_candidates(alias, fallback)
            .iter()
            .copied();
        let mut last = StartOutcome::Failed(StartFailure::MissingAlias);
        for candidate in own_names.chain(legacy) {
            let want = Want {
                origin,
                snd_ent,
                ..self.want(AssetNamespace::T6, candidate, class)
            };
            let outcome = self.play(&want);
            if outcome.is_open() || !outcome.allows_binding_fallback() {
                return outcome;
            }
            last = outcome;
        }
        last
    }

    /// Plays the sound a layer chains to, one level deeper.
    fn follow_layer(
        &mut self,
        want: &Want<'_>,
        layer: Option<&str>,
        depth: u8,
    ) -> Option<(String, StartOutcome)> {
        let name = layer.filter(|name| !name.is_empty())?;
        if depth >= 10 {
            return Some((name.to_owned(), StartOutcome::Failed(StartFailure::NoPcm)));
        }
        let chained = Want {
            alias: name,
            bound: None,
            pitch_scale: 1.0,
            ..*want
        };
        let started = self.start(&chained, depth + 1);
        Some((name.to_owned(), started.outcome))
    }

    /// Like `follow_layer`, but Black Ops chains its layer before the main
    /// sound, so it never follows one afterwards.
    fn follow_layer_after(
        &mut self,
        want: &Want<'_>,
        layer: Option<&str>,
        depth: u8,
    ) -> Option<(String, StartOutcome)> {
        if want.namespace == AssetNamespace::T5 {
            None
        } else {
            self.follow_layer(want, layer, depth)
        }
    }

    /// Picks a variant of the sound and starts it, or queues it while its
    /// clip decodes.
    fn start(&mut self, want: &Want<'_>, depth: u8) -> OneshotStart {
        let bank = self.bank;
        let avoid = self
            .pick
            .last_variant
            .get(&(want.namespace, want.alias.to_owned()))
            .copied();
        let drawn = match want.bound {
            Some(index) => bank.pick_loaded_outcome_at(index, &mut self.pick.lcg, avoid),
            None => bank.pick_loaded_outcome(want.namespace, want.alias, &mut self.pick.lcg, avoid),
        };
        let Some(drawn) = drawn else {
            return OneshotStart::failed(StartFailure::MissingAlias);
        };
        let variant = drawn.variant_index;
        let row = sound_for(bank, want).and_then(|sound| sound.aliases.get(variant));

        let t5_secondary = if want.namespace == AssetNamespace::T5 {
            self.follow_layer(want, row.and_then(|row| row.secondary.as_deref()), depth)
        } else {
            None
        };

        let (loaded_name, loaded_ns) = match &drawn.picked {
            Some(picked) => (
                Some(picked.sound.name.as_str()),
                Some(AssetNamespace::from_zone_game(picked.sound.game)),
            ),
            None => (None, None),
        };
        let Some(clip) = clip_key_for_variant(
            bank,
            want.namespace,
            want.alias,
            want.bound,
            variant,
            loaded_name,
            loaded_ns,
        ) else {
            // bo2mp: a Black Ops II alias with no sound of its own can exist
            // only for its secondary (a jump's launch exertion carries the
            // gear sound), and that still plays.
            let mut failed = OneshotStart::failed(StartFailure::NoPcm).with_variant(variant);
            if want.namespace == AssetNamespace::T6 {
                let layer = row.and_then(|row| row.secondary.as_deref());
                failed.secondary = self.follow_layer(want, layer, depth);
            }
            return failed;
        };

        let (volume, pitch, layer) = match &drawn.picked {
            Some(picked) => (picked.volume, picked.pitch, picked.layer.clone()),
            None => match row {
                Some(row) => {
                    let (volume, pitch) =
                        streamed_row_volume_pitch(want.namespace, row, &mut self.pick.lcg);
                    (volume, pitch, row.secondary.clone())
                }
                None => (1.0, 1.0, None),
            },
        };
        let pitch = pitch
            * if want.pitch_scale.is_finite() && want.pitch_scale > 0.0 {
                want.pitch_scale
            } else {
                1.0
            };

        let pcm = match self.claim_clip(want, variant, &clip, volume, pitch, layer.as_deref()) {
            ClipTake::Ready(pcm) => pcm,
            ClipTake::Pending => {
                return OneshotStart {
                    outcome: StartOutcome::Pending,
                    variant: Some(variant),
                    secondary: None,
                    detail: None,
                };
            }
            ClipTake::Failed(failure) => {
                return OneshotStart::failed(failure).with_variant(variant);
            }
        };
        let mut started = self.submit(
            want,
            depth,
            Chosen {
                pcm,
                clip: &clip,
                variant,
                volume,
                pitch,
                layer: layer.as_deref(),
            },
        );
        if want.namespace == AssetNamespace::T5 {
            started.secondary = t5_secondary;
        }
        started
    }

    /// Asks the clip store for the clip (and the one its layer chains to) and
    /// takes it if it is ready; otherwise parks the start until it is.
    fn claim_clip(
        &mut self,
        want: &Want<'_>,
        variant: usize,
        clip: &ClipKey,
        volume: f32,
        pitch: f32,
        layer: Option<&str>,
    ) -> ClipTake {
        let bank = self.bank;
        let Some(store) = self.clips.as_deref_mut() else {
            return ClipTake::Failed(StartFailure::NoPcm);
        };
        store.request(clip.clone());
        if let Some(layer) = layer {
            for key in clip_keys_for_alias(bank, want.namespace, layer) {
                store.request(key);
            }
        }
        match store.ready(clip) {
            Some(Ok(pcm)) => ClipTake::Ready(pcm),
            Some(Err(_)) => ClipTake::Failed(StartFailure::DecodeFailed),
            None => {
                self.pending.push_oneshot(PendingOneshot {
                    namespace: want.namespace,
                    alias: want.alias.to_owned(),
                    bound: want.bound,
                    variant,
                    volume,
                    pitch,
                    origin_inches: want.origin,
                    snd_ent: want.snd_ent,
                    clip: clip.clone(),
                    layer: layer.map(str::to_owned),
                    class: want.class,
                    epoch: want.epoch,
                    deadline: deadline_for(want.class),
                });
                ClipTake::Pending
            }
        }
    }

    /// Frees the voices the sound's channel and alias limits require, or says
    /// why there is no room.
    fn claim_voice(
        &mut self,
        want: &Want<'_>,
        channel: Option<u32>,
        limit_name: &str,
        row: Option<&asset_audio::CapturedAlias>,
        priority: f32,
    ) -> Result<(), SuppressReason> {
        prepare_voice(
            self.commands,
            self.occupancy,
            self.bank,
            channel,
            want.snd_ent,
        )?;
        prepare_alias_voice(
            self.commands,
            self.occupancy,
            want.namespace,
            limit_name,
            row,
            want.snd_ent,
            priority,
        )
    }

    /// Works out where a 3D sound sits and what its falloff leaves of it.
    /// `Err` carries the start that came to nothing instead.
    fn place(
        &mut self,
        want: &Want<'_>,
        chosen: &Chosen<'_>,
        row: Option<&asset_audio::CapturedAlias>,
        pos: [f32; 3],
        layer_depth: u8,
    ) -> Result<Placed, OneshotStart> {
        let variant = chosen.variant;
        let Some((ear, right)) = self.pose else {
            diag::warn!(
                Audio,
                "audio: world alias `{}` has no listener (typed gap)",
                want.alias
            );
            return Err(OneshotStart::failed(StartFailure::NoListener).with_variant(variant));
        };
        let Some(row) = row else {
            return Err(OneshotStart::failed(StartFailure::NoPcm).with_variant(variant));
        };
        let dist = distance_inches(transform_inches(ear), pos);
        let Some(curve) = row.volume_falloff.as_ref() else {
            diag::warn!(
                Audio,
                "audio: world alias `{}` has no falloff curve (typed gap)",
                want.alias
            );
            return Err(OneshotStart::failed(StartFailure::NoFalloffCurve).with_variant(variant));
        };
        let knots = self.shared.intern_curve(&curve.name, &curve.knots);
        let near_knots = row
            .near_falloff
            .as_ref()
            .map(|near| self.shared.intern_curve(&near.name, &near.knots));
        let atten = distance_attenuation(
            &knots,
            near_knots.as_deref(),
            dist,
            row.dist_min,
            row.dist_max,
        );
        // bo2mp: a Black Ops II 3D sound with a pan of its own keeps that pan.
        let fixed_pan = row
            .stereo_pan
            .map(|pan| pan.gains(chosen.pcm.channel_count()));
        let gains = fixed_pan.unwrap_or_else(|| {
            world_oneshot_channel_gains(ear, right, Vec3::from_array(pos), 1.0)
        });
        if atten < 0.0 {
            diag::warn!(
                Audio,
                "audio: world alias `{}` falloff curve `{}` failed evaluation (typed gap)",
                want.alias,
                curve.name
            );
            return Err(OneshotStart::failed(StartFailure::FalloffEval).with_variant(variant));
        }
        if atten == 0.0 {
            // bo2mp: out of this layer's reach, but its secondary is a sound
            // of its own (a gun's npc shot chains to its LFE, then to the
            // distant shot and its decay, each with a longer reach), so it
            // still starts and far gunfire is heard.
            let secondary = self.follow_layer_after(want, chosen.layer, layer_depth);
            return Err(OneshotStart {
                outcome: StartOutcome::Suppressed(SuppressReason::Inaudible),
                variant: Some(variant),
                secondary,
                detail: Some(falloff_detail(dist, row.dist_min, row.dist_max, atten)),
            });
        }
        Ok(Placed {
            pos,
            dist,
            dist_min: row.dist_min,
            dist_max: row.dist_max,
            atten,
            knots,
            near_knots,
            fixed_pan,
            gains,
        })
    }

    /// Spawns the voice for a clip that is ready, and the layer it chains to.
    fn submit(&mut self, want: &Want<'_>, depth: u8, chosen: Chosen<'_>) -> OneshotStart {
        let bank = self.bank;
        let variant = chosen.variant;
        let volume = chosen.volume;
        let sound = sound_for(bank, want);
        let row = sound.and_then(|sound| sound.aliases.get(variant));
        let limit_name = sound.map_or(want.alias, |sound| sound.name.as_str());
        let channel = sound.and_then(|sound| sound.ent_channel(variant));

        // bo2zm: Black Ops II keeps Black Ops' pan-type bit (flags bit 1 = 3D).
        let positional = if matches!(want.namespace, AssetNamespace::T5 | AssetNamespace::T6) {
            row.and_then(|row| row.flags)
                .is_some_and(|flags| flags & 2 != 0)
        } else {
            channel
                .and_then(|ch| bank.ent_channel(ch))
                .is_none_or(|info| info.is_3d)
        };
        let spot = want.origin.filter(|_| positional);
        let priority = row
            .and_then(|row| row.voice_priority.as_ref())
            .map_or(0.0, |priority| {
                let distance = spot
                    .zip(self.pose)
                    .map(|(origin, (ear, _))| distance_inches(transform_inches(ear), origin));
                priority.evaluate(distance)
            });

        let placed = match spot {
            Some(pos) => match self.place(want, &chosen, row, pos, depth) {
                Ok(placed) => Some(placed),
                Err(refused) => return refused,
            },
            None => None,
        };
        let detail = placed
            .as_ref()
            .map(|p| falloff_detail(p.dist, p.dist_min, p.dist_max, p.atten));
        if let Err(reason) = self.claim_voice(want, channel, limit_name, row, priority) {
            return OneshotStart::suppressed(reason, variant, detail);
        }
        self.pick
            .last_variant
            .insert((want.namespace, want.alias.to_owned()), variant);

        let total_ms = crate::duck::clip_ms(&chosen.pcm);
        let Chosen { pcm, clip, layer, pitch, .. } = chosen;
        // A panned sound gets its own asset; the clip's plain one is still
        // cached so later unpanned plays can share it.
        let panned = match &placed {
            Some(placed) => Some(placed.gains),
            None => row
                .and_then(|row| row.stereo_pan)
                .map(|pan| pan.gains(pcm.channel_count()))
                .filter(|&gains| gains != (1.0, 1.0)),
        };
        let (handle, live_pan) = match panned {
            Some((left, right)) => {
                let _ = self
                    .shared
                    .dry_handle(self.pcm_assets, clip, pcm.clone(), self.clips.as_deref());
                let live = pcm.with_live_pan();
                let control = live.live_pan().expect("with_live_pan").clone();
                control.set(left, right);
                (self.pcm_assets.add(live), Some(control))
            }
            None => (
                self.shared
                    .dry_handle(self.pcm_assets, clip, pcm, self.clips.as_deref()),
                None,
            ),
        };
        let level = match &placed {
            Some(placed) => (volume * placed.atten).max(0.0),
            None => volume.max(0.0),
        };

        let lease = voice_lease(bank, channel, want.snd_ent);
        let entity = crate::backend::spawn_oneshot(
            self.commands,
            handle,
            Volume::Linear(level),
            pitch,
            want.epoch,
            want.class.scope(),
        );
        {
            let mut spawned = self.commands.entity(entity);
            spawned.insert(AliasPlayback {
                namespace: want.namespace,
                snd_ent: want.snd_ent,
                alias: want.alias.to_owned(),
            });
            if let (Some(placed), Some(live_pan)) = (placed, live_pan) {
                spawned.insert(Channel3d {
                    origin_inches: placed.pos,
                    dist_min: placed.dist_min,
                    dist_max: placed.dist_max,
                    knots: placed.knots,
                    near_knots: placed.near_knots,
                    priority: row.and_then(|row| row.voice_priority.clone()),
                    base_volume: volume.max(0.0),
                    live_pan,
                    fixed_pan: placed.fixed_pan,
                    category: row.and_then(|row| row.menu_category),
                });
            }
            crate::duck::tag(&mut spawned, row, total_ms);
            if let Some(category) = row.and_then(|row| row.menu_category) {
                spawned.insert(crate::MenuVolume {
                    category,
                    base: volume.max(0.0),
                });
            }
            if let Some(lease) = lease {
                spawned.insert(lease);
            }
        }
        if let Some(lease) = lease {
            self.occupancy.track(entity, lease);
        }
        if want.namespace == AssetNamespace::T5 {
            let alias_lease = self.occupancy.track_alias(
                entity,
                want.namespace,
                limit_name,
                want.snd_ent,
                priority,
            );
            self.commands.entity(entity).insert(alias_lease);
        }

        let secondary = self.follow_layer_after(want, layer, depth);
        OneshotStart {
            outcome: StartOutcome::Submitted,
            variant: Some(variant),
            secondary,
            detail,
        }
    }

    /// Starts the sounds whose clips finished decoding since the last frame,
    /// and gives up on the ones that waited too long.
    fn drain_pending(&mut self) {
        let now = Instant::now();
        let waiting = std::mem::take(&mut self.pending.entries);
        for entry in waiting {
            let stale = entry.class.scope() == crate::backend::AudioScope::Match
                && entry.epoch != self.epoch;
            if stale {
                continue;
            }
            if now >= entry.deadline {
                diag::warn!(
                    Audio,
                    "audio: alias `{}:{}` expired awaiting decode (typed gap)",
                    entry.namespace.as_str(),
                    entry.alias
                );
                note_failure(
                    self.decisions,
                    entry.namespace,
                    &entry.alias,
                    Some(entry.variant),
                    StartFailure::Expired,
                );
                continue;
            }
            let state = self
                .clips
                .as_deref()
                .and_then(|clips| clips.ready(&entry.clip));
            match state {
                None => self.pending.entries.push(entry),
                Some(Err(_)) => note_failure(
                    self.decisions,
                    entry.namespace,
                    &entry.alias,
                    Some(entry.variant),
                    StartFailure::DecodeFailed,
                ),
                Some(Ok(pcm)) => {
                    let want = Want {
                        namespace: entry.namespace,
                        alias: &entry.alias,
                        bound: entry.bound,
                        origin: entry.origin_inches,
                        snd_ent: entry.snd_ent,
                        class: entry.class,
                        pitch_scale: 1.0,
                        epoch: entry.epoch,
                    };
                    let started = self.submit(
                        &want,
                        0,
                        Chosen {
                            pcm,
                            clip: &entry.clip,
                            variant: entry.variant,
                            volume: entry.volume,
                            pitch: entry.pitch,
                            layer: entry.layer.as_deref(),
                        },
                    );
                    self.record_start(entry.namespace, &entry.alias, started);
                }
            }
        }
    }
}

fn note_failure(
    decisions: &mut StartDecisions,
    namespace: AssetNamespace,
    alias: &str,
    variant: Option<usize>,
    failure: StartFailure,
) {
    decisions.record(StartDecision {
        namespace,
        alias: alias.to_owned(),
        variant,
        outcome: StartOutcome::Failed(failure),
        secondary: None,
        detail: None,
    });
}

/// Logs and records the sounds asked for while there is no sound bank.
fn drop_without_bank<'x>(aliases: impl Iterator<Item = &'x str>, decisions: &mut StartDecisions) {
    for alias in aliases {
        if !crate::AudioSilent::active() {
            diag::warn!(
                Audio,
                "audio: alias `{alias}` dropped — no SoundBank (typed gap)"
            );
        }
        note_failure(
            decisions,
            AssetNamespace::T6,
            alias,
            None,
            StartFailure::BankMissing,
        );
    }
}

fn play_alias_messages(mut events: MessageReader<AliasCommand>, mut desk: SoundDesk) {
    for command in events.read() {
        let (event, pitch_scale) = match command {
            AliasCommand::Play(event) => (event, 1.0),
            AliasCommand::PlayPitched { sound, pitch } => (sound, *pitch),
            AliasCommand::Stop {
                namespace,
                alias,
                snd_ent,
            } => {
                desk.stop_alias(*namespace, alias, *snd_ent);
                continue;
            }
        };
        let Some((mut stage, gaps)) = desk.open() else {
            drop_without_bank(std::iter::once(event.alias.as_str()), &mut desk.decisions);
            continue;
        };
        let (namespace, alias) = match t6_movement(stage.bank, event.namespace, &event.alias) {
            T6Movement::Play(t6) => (AssetNamespace::T6, t6),
            T6Movement::Skip => continue,
            T6Movement::NotT6 => (event.namespace, event.alias.as_str()),
        };
        let first = Want {
            origin: event.origin_inches,
            snd_ent: event.snd_ent,
            pitch_scale,
            ..stage.want(namespace, alias, SoundClass::World)
        };
        let outcome = stage.play(&first);
        if outcome.is_open() || !outcome.allows_binding_fallback() {
            continue;
        }
        if let Some(fallback) = event.fallback.as_deref() {
            let second = stage.play(&Want {
                namespace: event.namespace,
                alias: fallback,
                ..first
            });
            if second.is_open() {
                continue;
            }
            if second.allows_binding_fallback() {
                gaps.record(fallback);
            }
        }
        gaps.record(&event.alias);
    }
}

/// Plays a footstep or landing event's surface chain and notes what was
/// missing.
fn play_surface_event(
    stage: &mut Stage<'_, '_, '_>,
    gaps: &mut MissingAliasGaps,
    alias: &str,
    fallback: &str,
    origin: Option<[f32; 3]>,
    snd_ent: Option<u32>,
) {
    let outcome = stage.play_surface(alias, fallback, origin, snd_ent, SoundClass::World);
    if outcome.allows_binding_fallback() {
        gaps.record(alias);
        gaps.record(fallback);
    }
}

fn play_footstep_messages(mut events: MessageReader<Footstep>, mut desk: SoundDesk) {
    let Some((mut stage, gaps)) = desk.open() else {
        drop_without_bank(events.read().map(|event| event.alias), &mut desk.decisions);
        return;
    };
    for event in events.read() {
        play_surface_event(
            &mut stage,
            gaps,
            event.alias,
            event.fallback,
            event.origin_inches,
            event.snd_ent,
        );
    }
}

fn play_land_sound_messages(mut events: MessageReader<LandSound>, mut desk: SoundDesk) {
    let Some((mut stage, gaps)) = desk.open() else {
        drop_without_bank(events.read().map(|event| event.alias), &mut desk.decisions);
        return;
    };
    for event in events.read() {
        play_surface_event(
            &mut stage,
            gaps,
            event.alias,
            event.fallback,
            event.origin_inches,
            event.snd_ent,
        );
    }
}

fn play_weapon_sound_messages(mut events: MessageReader<WeaponSound>, mut desk: SoundDesk) {
    let Some((mut stage, gaps)) = desk.open() else {
        drop_without_bank(
            events.read().map(|event| event.alias.as_str()),
            &mut desk.decisions,
        );
        return;
    };
    for event in events.read() {
        let (namespace, alias) = match t6_movement(stage.bank, event.namespace, &event.alias) {
            T6Movement::Play(t6) => (AssetNamespace::T6, t6),
            T6Movement::Skip => continue,
            T6Movement::NotT6 => (event.namespace, event.alias.as_str()),
        };
        let want = Want {
            origin: event.origin_inches,
            snd_ent: event.snd_ent,
            ..stage.want(namespace, alias, SoundClass::Weapon)
        };
        if stage.play(&want).allows_binding_fallback() {
            gaps.record(alias);
        }
    }
}

fn play_bound_weapon_sounds(mut events: MessageReader<BoundWeaponSound>, mut desk: SoundDesk) {
    let Some((mut stage, gaps)) = desk.open() else {
        events.clear();
        return;
    };
    let bank = stage.bank;
    for event in events.read() {
        if event.bank_revision != bank.revision() {
            continue;
        }
        let Some(alias) = bank.name_at(event.index) else {
            continue;
        };
        let want = Want {
            bound: Some(event.index),
            origin: event.origin_inches,
            snd_ent: event.snd_ent,
            ..stage.want(bank.namespace_of_alias(event.index), alias, SoundClass::Weapon)
        };
        if stage.play(&want).allows_binding_fallback() {
            gaps.record(alias);
        }
    }
}

fn drain_pending_oneshots(mut desk: SoundDesk) {
    if desk.pending.entries.is_empty() {
        return;
    }
    if desk.bank.is_none() {
        desk.pending.clear();
        return;
    }
    if desk.clips.is_none() {
        return;
    }
    if let Some((mut stage, _)) = desk.open() {
        stage.drain_pending();
    }
}

/// Plays one alias through the full start path, for callers that hold the
/// pieces themselves instead of a `SoundDesk`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn play_alias_oneshot(
    commands: &mut Commands,
    pcm_assets: &mut Assets<PcmAudio>,
    shared: &mut SharedPlayAssets,
    bank: &SoundCatalog,
    namespace: AssetNamespace,
    alias: &str,
    origin_inches: Option<[f32; 3]>,
    listener: Option<(Vec3, Vec3)>,
    pick: &mut SoundPickState,
    clips: Option<&mut ClipStore>,
    pending: &mut PendingStarts,
    occupancy: &mut VoiceOccupancy,
    decisions: &mut StartDecisions,
    snd_ent: Option<u32>,
    class: SoundClass,
    epoch: u64,
) -> StartOutcome {
    let mut stage = Stage {
        commands,
        pcm_assets,
        shared,
        bank,
        pick,
        clips,
        pending,
        occupancy,
        decisions,
        pose: listener,
        epoch,
    };
    let want = Want {
        origin: origin_inches,
        snd_ent,
        ..stage.want(namespace, alias, class)
    };
    stage.play(&want)
}

/// Which way a world sound lies from the ear: -1 is hard left, 1 hard right.
pub fn world_oneshot_pan(ear: Vec3, listener_right: Vec3, emitter: Vec3) -> f32 {
    let toward = emitter - ear;
    if toward.length_squared() < 1e-8 {
        return 0.0;
    }
    toward.normalize().dot(listener_right).clamp(-1.0, 1.0)
}

/// The left and right gains of a world sound, an equal-power pan scaled by
/// `atten`.
pub fn world_oneshot_channel_gains(
    ear: Vec3,
    listener_right: Vec3,
    emitter: Vec3,
    atten: f32,
) -> (f32, f32) {
    let angle = (world_oneshot_pan(ear, listener_right, emitter) + 1.0) * FRAC_PI_4;
    (atten * angle.cos(), atten * angle.sin())
}

/// Stops the voices that hold the sound's entity channel when that channel
/// is restricted, then checks that the channel has a free voice.
fn prepare_voice(
    commands: &mut Commands,
    occupancy: &mut VoiceOccupancy,
    bank: &SoundCatalog,
    channel: Option<u32>,
    snd_ent: Option<u32>,
) -> Result<(), SuppressReason> {
    let Some(ch) = channel else {
        return Ok(());
    };
    let Some(info) = bank.ent_channel(ch) else {
        return Ok(());
    };
    if info.is_restricted
        && let Some(ent) = snd_ent
    {
        for entity in occupancy.take_entity_channel(ent, ch) {
            crate::backend::stop(commands, entity);
        }
    }
    if has_free_voice(occupancy.voice_count(ch), 0, info.max_voices) {
        Ok(())
    } else {
        Err(SuppressReason::VoiceLimit)
    }
}

/// Black Ops' per-alias voice limits: a global one and a per-entity one, each
/// in one of several modes packed into the alias flags.
fn prepare_alias_voice(
    commands: &mut Commands,
    occupancy: &mut VoiceOccupancy,
    namespace: AssetNamespace,
    alias: &str,
    row: Option<&asset_audio::CapturedAlias>,
    snd_ent: Option<u32>,
    priority: f32,
) -> Result<(), SuppressReason> {
    if namespace != AssetNamespace::T5 {
        return Ok(());
    }
    let Some(row) = row else {
        return Ok(());
    };
    let flags = row.flags.unwrap_or(0);
    let limits = [
        ((flags >> 25) & 3, row.limit_count, false),
        ((flags >> 27) & 3, row.entity_limit_count, true),
    ];
    for (mode, count, per_entity) in limits {
        let Some(count) = count else {
            continue;
        };
        let evicted = occupancy.limit_alias(
            namespace,
            alias,
            snd_ent,
            (mode, count),
            per_entity,
            priority,
        )?;
        if let Some(entity) = evicted {
            crate::backend::stop(commands, entity);
        }
    }
    Ok(())
}

fn voice_lease(
    bank: &SoundCatalog,
    channel: Option<u32>,
    snd_ent: Option<u32>,
) -> Option<VoiceLease> {
    let channel = channel?;
    bank.ent_channel(channel)?;
    Some(VoiceLease { channel, snd_ent })
}

/// The volume and pitch a streamed row asks for, drawn from its ranges. A
/// zero range means "unset" and plays at 1.
fn streamed_row_volume_pitch(
    namespace: AssetNamespace,
    row: &asset_audio::CapturedAlias,
    rng: &mut u32,
) -> (f32, f32) {
    let draw_volume = unit_random(rng);
    let draw_pitch = unit_random(rng);
    let volume_unset = row.vol_min == 0.0 && row.vol_max == 0.0;
    let volume = if namespace != AssetNamespace::T5 && volume_unset {
        1.0
    } else {
        lerp_range(row.vol_min, row.vol_max, draw_volume)
    };
    let pitch = if row.pitch_min == 0.0 && row.pitch_max == 0.0 {
        1.0
    } else {
        lerp_range(row.pitch_min, row.pitch_max, draw_pitch)
    };
    (volume, pitch)
}
