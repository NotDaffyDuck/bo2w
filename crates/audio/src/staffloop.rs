//! bo2zm: Origins' upgraded staffs, the charge loop the charging player
//! hears (zm_tomb_amb.csc sndchargeshot): `playloopsound(alias + "loop",
//! 1.5)` fades it in over 1.5 s, and 0.5 s after the last charge call
//! `stoploopsound(0.1)` fades it out over 0.1 s. The server names the loop
//! in the client dvar `bo2zm_staffloop` (empty = stopped; sim
//! t6/tomb_staff_charge.rs); each level's own sound comes as a local sound.

use bevy::audio::Volume;
use bevy::prelude::*;
use net::{HostViewClient, ShownWorldFrame};

use crate::backend::{SoundAudience, GameRoundSerial};
use crate::clip_bank::ClipRepository;
use crate::laststand::{reel, fade_gain};
use crate::sample_feed::LoopedPcm;
use crate::play_core::{UnknownCueHoles, AudioCatalogRes};

const IN_SECS: f32 = 1.5;
const OUT_SECS: f32 = 0.1;

#[derive(Default)]
pub(crate) struct StaffLoop {
    client: Option<u32>,
    epoch: u64,
    /// The loop the server names ("" = none).
    want: String,
    /// The voice: its alias, entity and own volume.
    looper: Option<(String, Entity, f32)>,
    /// (start time, from, to, seconds).
    fade: (f32, f32, f32, f32),
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn sync_staff_loop(
    mut commands: Commands,
    presented: Option<Res<ShownWorldFrame>>,
    local: Option<Res<HostViewClient>>,
    bank: Option<Res<AudioCatalogRes>>,
    mut clips: Option<ResMut<ClipRepository>>,
    mut looping_assets: ResMut<Assets<LoopedPcm>>,
    mut gaps: ResMut<UnknownCueHoles>,
    epoch: Res<GameRoundSerial>,
    time: Res<Time>,
    mut st: Local<StaffLoop>,
    mut sinks: Query<&mut bevy::audio::AudioSink>,
) {
    use bevy::audio::AudioSinkPlayback;
    let now = time.elapsed_secs();
    let id = local.as_ref().map(|l| l.0);
    let client = id.map(|c| c.0);
    if st.client != client || st.epoch != epoch.0 {
        if let Some((_, e, _)) = st.looper.take() {
            crate::backend::silence_cue(&mut commands, e);
        }
        *st = StaffLoop {
            client,
            epoch: epoch.0,
            ..default()
        };
    }
    let want_loop = match (presented.as_ref(), id) {
        (Some(p), Some(c)) => p
            .snapshot()
            .and_then(|snap| snap.meta.script_dvars(c).string("bo2zm_staffloop").map(str::to_owned))
            .unwrap_or_default(),
        _ => String::new(),
    };
    if want_loop != st.want {
        if want_loop.is_empty() {
            // stoploopsound(0.1), from wherever the fade in had got to.
            let from = fade_gain(st.fade, now);
            st.fade = (now, from, 0.0, OUT_SECS);
        } else {
            // playloopsound on the same entity: a new staff's loop takes
            // the old one's place, fading in.
            if let Some((_, e, _)) = st.looper.take() {
                crate::backend::silence_cue(&mut commands, e);
            }
            st.fade = (now, 0.0, 1.0, IN_SECS);
        }
        diag::info!(Audio, "sound: staff charging loop `{want_loop}`");
        st.want = want_loop;
    }
    let (Some(snd_bank), Some(clips)) = (bank, clips.as_mut()) else {
        return;
    };
    if !st.want.is_empty()
        && st.looper.is_none()
        && let Some((pcm, own)) = reel(&snd_bank, clips, &mut gaps, &st.want)
    {
        let handle = looping_assets.add(pcm.looped());
        let e = crate::backend::begin_repeating_cue(
            &mut commands,
            handle,
            Volume::Linear(0.0),
            epoch.0,
            SoundAudience::Match,
        );
        st.looper = Some((st.want.clone(), e, own));
    }
    if let Some((_, e, own)) = st.looper.clone() {
        let g = fade_gain(st.fade, now);
        if st.want.is_empty() && g <= 0.0 {
            crate::backend::silence_cue(&mut commands, e);
            st.looper = None;
        } else if let Ok(mut sink) = sinks.get_mut(e) {
            sink.set_volume(Volume::Linear(own * g * crate::bus_gain::sfx()));
        }
    }
}
