//! bo2zm: Origins' upgraded staffs, the charge loop the charging player
//! hears (zm_tomb_amb.csc sndchargeshot): `playloopsound(alias + "loop",
//! 1.5)` fades it in over 1.5 s, and 0.5 s after the last charge call
//! `stoploopsound(0.1)` fades it out over 0.1 s. The server names the loop
//! in the client dvar `bo2zm_staffloop` (empty = stopped; sim
//! t6/tomb_staff_charge.rs); each level's own sound comes as a local sound.

use bevy::audio::Volume;
use bevy::prelude::*;
use net::{LocalPresentClient, PresentedSnapshot};

use crate::backend::{AudioScope, MatchEpoch};
use crate::clip_store::ClipStore;
use crate::laststand::{clip, fade_gain};
use crate::pcm::LoopingPcmAudio;
use crate::playback::{MissingAliasGaps, SoundBank};

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
pub(crate) fn update(
    mut commands: Commands,
    presented: Option<Res<PresentedSnapshot>>,
    local: Option<Res<LocalPresentClient>>,
    bank: Option<Res<SoundBank>>,
    mut clips: Option<ResMut<ClipStore>>,
    mut looping_assets: ResMut<Assets<LoopingPcmAudio>>,
    mut gaps: ResMut<MissingAliasGaps>,
    epoch: Res<MatchEpoch>,
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
            crate::backend::stop(&mut commands, e);
        }
        *st = StaffLoop {
            client,
            epoch: epoch.0,
            ..default()
        };
    }
    let want = match (presented.as_ref(), id) {
        (Some(p), Some(c)) => p
            .snapshot()
            .and_then(|snap| snap.meta.script_dvars(c).string("bo2zm_staffloop").map(str::to_owned))
            .unwrap_or_default(),
        _ => String::new(),
    };
    if want != st.want {
        if want.is_empty() {
            // stoploopsound(0.1), from wherever the fade in had got to.
            let from = fade_gain(st.fade, now);
            st.fade = (now, from, 0.0, OUT_SECS);
        } else {
            // playloopsound on the same entity: a new staff's loop takes
            // the old one's place, fading in.
            if let Some((_, e, _)) = st.looper.take() {
                crate::backend::stop(&mut commands, e);
            }
            st.fade = (now, 0.0, 1.0, IN_SECS);
        }
        diag::info!(Audio, "audio: staff charge loop `{want}`");
        st.want = want;
    }
    let (Some(bank), Some(clips)) = (bank, clips.as_mut()) else {
        return;
    };
    if !st.want.is_empty()
        && st.looper.is_none()
        && let Some((pcm, own)) = clip(&bank, clips, &mut gaps, &st.want)
    {
        let handle = looping_assets.add(pcm.into_looping());
        let e = crate::backend::spawn_loop(
            &mut commands,
            handle,
            Volume::Linear(0.0),
            epoch.0,
            AudioScope::Match,
        );
        st.looper = Some((st.want.clone(), e, own));
    }
    if let Some((_, e, own)) = st.looper.clone() {
        let g = fade_gain(st.fade, now);
        if st.want.is_empty() && g <= 0.0 {
            crate::backend::stop(&mut commands, e);
            st.looper = None;
        } else if let Ok(mut sink) = sinks.get_mut(e) {
            sink.set_volume(Volume::Linear(own * g * crate::bus_gain::sfx()));
        }
    }
}
