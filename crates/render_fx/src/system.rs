//! The systems that keep the effect host in step with the client frame: clock, camera,
//! sound requests and the teardown after a match.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use frame::{EffectAudioPosted, GameSessionClosed, MatchSwapDone};
use fx::{EffectHoleReason, EffectClockMs, set_show_clock};
use net::{
    ClientStep, FrameClock, GameActive, NewestTakenSnapshot, step_presentation_clock,
    correct_forecast,
};
use render_anim::sync_camera_from_shown;
use render_scene::{FreeRoamCamera, LevelPresentFacts};

use crate::{
    EntityMarkBoard, EffectsEyePosition, EffectJournalCursor, FxModelDrawList, FxModelStagingPlan, EffectsDynamicLightSet,
    PostEffectLightSet, EffectsSystemHandle, ReadyFxBook, PreparedFxModelPlan, ReadyFxProps,
    ReadyHitFx, StagedStreaks, TracerRenderGuard, StreakLevel,
};

/// Schedules the effect host's per-frame bookkeeping inside the client frame sets.
pub fn install_effect_coordination(app: &mut App) {
    app.add_systems(
        Update,
        reset_fx_after_match
            .in_set(ClientStep::Load)
            .after(MatchSwapDone),
    );
    // The game is active before the frame clock moves; the presentation clock follows it.
    app.add_systems(
        Update,
        (
            set_game_active
                .after(correct_forecast)
                .before(step_presentation_clock),
            sync_presentation_clock.after(step_presentation_clock),
        )
            .in_set(ClientStep::Reconcile),
    );
    app.add_systems(
        Update,
        record_effects_eye_position
            .in_set(ClientStep::Present)
            .after(sync_camera_from_shown),
    );
    app.add_systems(
        Update,
        send_queued_fx_sounds
            .in_set(ClientStep::Effects)
            .before(EffectAudioPosted),
    );
}

/// Copies the fly camera's position, or the world origin when there is no camera.
pub fn record_effects_eye_position(
    eyes: Query<&Transform, With<FreeRoamCamera>>,
    mut origin: ResMut<EffectsEyePosition>,
) {
    let position = eyes.iter().next().map(|camera| camera.translation);
    origin.0 = position.map_or([0.0; 3], |at| at.to_array());
}

/// Everything the effect host keeps between frames that a finished match must not leave
/// behind.
#[derive(SystemParam)]
struct FxMatchState<'w> {
    host: ResMut<'w, EffectsSystemHandle>,
    journal: ResMut<'w, EffectJournalCursor>,
    lamp_list: ResMut<'w, EffectsDynamicLightSet>,
    after_lights: ResMut<'w, PostEffectLightSet>,
    tracers: ResMut<'w, StreakLevel>,
    tracer_gate: ResMut<'w, TracerRenderGuard>,
    marks: ResMut<'w, EntityMarkBoard>,
}

impl FxMatchState<'_> {
    fn start_over(&mut self) {
        *self.host = EffectsSystemHandle::default();
        self.journal.reset_createfx();
        *self.lamp_list = EffectsDynamicLightSet::default();
        *self.after_lights = PostEffectLightSet::default();
        *self.tracers = StreakLevel::default();
        *self.tracer_gate = TracerRenderGuard::default();
        *self.marks = EntityMarkBoard::default();
    }
}

/// After a match is torn down: a fresh host, empty queues, and no prepared fx data left
/// over from the old world.
fn reset_fx_after_match(
    mut torn_down: MessageReader<GameSessionClosed>,
    mut state: FxMatchState,
    mut commands: Commands,
) {
    if torn_down.read().count() == 0 {
        return;
    }
    state.start_over();
    commands.insert_resource(FxModelDrawList::without_geometry(0));
    commands.insert_resource(FxModelStagingPlan::default());
    commands.remove_resource::<ReadyFxBook>();
    commands.remove_resource::<ReadyFxProps>();
    commands.remove_resource::<PreparedFxModelPlan>();
    commands.remove_resource::<ReadyHitFx>();
    commands.remove_resource::<StagedStreaks>();
}

fn set_game_active(
    facts: Res<LevelPresentFacts>,
    adopted: Option<Res<NewestTakenSnapshot>>,
    mut active: ResMut<GameActive>,
) {
    let has_snapshot = adopted.is_some_and(|snapshot| snapshot.next().is_some());
    active.0 = GameActive::once_world_and_snapshot(facts.spawned, has_snapshot);
}

fn sync_presentation_clock(mut host: ResMut<EffectsSystemHandle>, clock: Res<FrameClock>) {
    set_show_clock(&mut host.0, EffectClockMs(clock.time()));
}

/// Turns the sounds the effect host queued this frame into alias commands. Requests older
/// than the previous frame are stale and dropped; ones whose alias cannot be found are
/// reported as gaps.
fn send_queued_fx_sounds(
    mut host: ResMut<EffectsSystemHandle>,
    recipe_book: Option<Res<ReadyFxBook>>,
    bank: Option<Res<audio::AudioCatalogRes>>,
    clock: Res<FrameClock>,
    mut commands_out: MessageWriter<audio::AliasCommand>,
) {
    let (Some(recipes), Some(bank)) = (recipe_book, bank) else {
        return;
    };
    let oldest_kept = clock.prior_ms();
    let requests = std::mem::take(&mut host.0.pending_sounds);
    for request in requests.into_iter().filter(|r| r.window_open_ms >= oldest_kept) {
        let elem = crate::effect_play::recipe_lookup(&recipes.0, request.ledger_index)
            .and_then(|parent| parent.elems.get(request.definition_slot as usize));
        let sound = elem.map(|elem| elem.sound_in_bank(request.dice_start, &bank.0));
        let playable = match sound {
            Some(asset_game::FxBankSound::Silent) => continue,
            Some(asset_game::FxBankSound::Play { namespace, alias }) => Some((namespace, alias)),
            Some(asset_game::FxBankSound::Gap) | None => None,
        };
        let Some((pack_ns, alias)) = playable else {
            host.0.gaps.log_hole(EffectHoleReason::ElemSoundSpawnSkipped {
                definition_slot: request.definition_slot,
            });
            continue;
        };
        commands_out.write(audio::AliasCommand::Play(audio::TriggerCueMsg {
            cue_ns: pack_ns,
            alias: alias.to_owned(),
            fallback: None,
            position_in_inches: Some(request.anchor),
            source_entity_no: Some(fx_t6::runtime::FX_ENTITYNUM_WORLD),
        }));
    }
}
