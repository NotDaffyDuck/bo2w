//! bo2zm: the Zombies music bed. Black Ops II's scripts set a music state
//! (`setmusicstate("WAVE")`) that its client scripts turn into a looping
//! track; the server sends the track's alias as the client dvar
//! `bo2zm_music` (empty for silence) and this keeps that one loop playing
//! (2D, the alias's own volume) for the match.

use asset_core::AssetCategoryScope;
use bevy::audio::Volume;
use bevy::prelude::*;
use net::{HostViewClient, ShownWorldFrame};

use crate::backend::{SoundAudience, GameRoundSerial};
use crate::clip_bank::{ClipRepository, waveforms_for_cue};
use crate::sample_feed::LoopedPcm;
use crate::play_core::{UnknownCueHoles, AudioCatalogRes};

#[derive(Component)]
pub(crate) struct ZmMusicBed {
    bed_cue: String,
    /// The alias's own volume (the music slider goes over it).
    bed_gain: f32,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn sync_zm_music(
    mut commands: Commands,
    presented: Option<Res<ShownWorldFrame>>,
    viewer: Option<Res<HostViewClient>>,
    bank: Option<Res<AudioCatalogRes>>,
    mut clips: Option<ResMut<ClipRepository>>,
    mut looping_assets: ResMut<Assets<LoopedPcm>>,
    mut gaps: ResMut<UnknownCueHoles>,
    epoch: Res<GameRoundSerial>,
    beds: Query<(Entity, &ZmMusicBed)>,
    mut sink_q: Query<&mut bevy::audio::AudioSink, With<ZmMusicBed>>,
) {
    // The music slider, live.
    for (ent, bed) in &beds {
        if let Ok(mut sink) = sink_q.get_mut(ent) {
            use bevy::audio::AudioSinkPlayback;
            sink.set_volume(Volume::Linear(bed.bed_gain * crate::bus_gain::music()));
        }
    }
    let (Some(shown), Some(viewer)) = (presented, viewer) else {
        return;
    };
    let Some(frame_snap) = shown.snapshot() else {
        return;
    };
    let wanted = frame_snap
        .meta
        .script_dvars(viewer.0)
        .string("bo2zm_music")
        .unwrap_or_default()
        .to_owned();
    for (ent_id, bed) in &beds {
        if bed.bed_cue != wanted {
            crate::backend::silence_cue(&mut commands, ent_id);
            diag::info!(Audio, "sound: zm music `{}` off", bed.bed_cue);
        }
    }
    if wanted.is_empty() || beds.iter().any(|(_, bed)| bed.bed_cue == wanted) {
        return;
    }
    let (Some(snd_bank), Some(clip_repo)) = (bank, clips.as_mut()) else {
        return;
    };
    let Some(key) = waveforms_for_cue(&snd_bank.0, AssetCategoryScope::T6, &wanted)
        .into_iter()
        .next()
    else {
        gaps.note_miss(&wanted);
        return;
    };
    clip_repo.req_in(key.clone());
    let Some(Ok(pcm)) = clip_repo.ready_pcm(&key) else {
        return;
    };
    let bank_row = snd_bank
        .0
        .entry_in(AssetCategoryScope::T6, &wanted)
        .and_then(|row| row.aliases.first());
    let volume = bank_row.map_or(0.8, |a| a.volume_low.max(0.0));
    let pcm = crate::sample_feed::with_stereo_pan(pcm, bank_row.and_then(|a| a.stereo_pan));
    let handle = looping_assets.add(pcm.looped());
    let entity = crate::backend::begin_repeating_cue(
        &mut commands,
        handle,
        Volume::Linear(volume * crate::bus_gain::music()),
        epoch.0,
        SoundAudience::Match,
    );
    commands.entity(entity).insert(ZmMusicBed {
        bed_cue: wanted.clone(),
        bed_gain: volume,
    });
    crate::duck::tag(&mut commands.entity(entity), bank_row, 0);
    if let Some(category) = bank_row.and_then(|a| a.menu_category) {
        commands.entity(entity).insert(crate::MenuVolume {
            cat_ix: category,
            base: volume,
        });
    }
    diag::info!(Audio, "sound: zm music `{wanted}` vol {volume:.2}");
}
