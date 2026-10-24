//! bo2mp: Black Ops II's slide ground sounds on the client, from the
//! server's slide start and stop events (their parm is the ground's slide
//! class).
//!
//! BO2 keeps one slide slot per local client, shared by every player: a
//! start fills the slot (or reuses it), puts it where the slider is (it does
//! not follow him after that) and loops `fly_dtp_slide_loop_<plr|npc>_<class>`
//! on it; a stop with the slot empty does nothing, otherwise it empties the
//! slot. Each also plays its own sound on the slider: the start
//! `fly_pslide_loop_*`, the stop `fly_pslide_stop_*` (`plr` for the player
//! you are looking through, `npc` for anyone else).
//!
//! The start's `fly_pslide_loop_*` is a looping alias played once. BO2 stops
//! a looping sound that nothing keeps alive the next frame, so it is stopped
//! here once a frame has passed. A class with no alias stays silent, as in
//! BO2 (no stand-in).

use crate::ambient::{MapAmbient, MapEmitter};
use crate::backend::MatchEpoch;
use crate::clip_store::{ClipStore, clip_keys_for_alias};
use crate::pcm::PcmAudio;
use crate::playback::{AmbientListener, SharedPlayAssets, SoundBank};
use crate::{AliasCommand, PlayAlias};
use asset_core::AssetNamespace;
use bevy::prelude::*;
use std::sync::Arc;

/// The slide sounds' nine ground classes, by the events' parm.
const CLASSES: [&str; 9] = [
    "default", "metal", "concrete", "wood", "snow", "dirt", "gravel", "grass", "mud",
];

/// What the slot loops, and where.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SlideSlot {
    pub(crate) alias: String,
    pub(crate) origin: [f32; 3],
}

/// A start sound waiting for its stop.
#[derive(Clone, Debug, PartialEq)]
struct StartSound {
    alias: String,
    snd_ent: u32,
    frame: u64,
}

#[derive(Resource, Default)]
pub(crate) struct SlideLoop {
    pub(crate) slot: Option<SlideSlot>,
    started: Vec<StartSound>,
    frame: u64,
    epoch: u64,
}

#[derive(Component)]
pub(crate) struct SlideLoopEmitter {
    alias: String,
    positional: bool,
}

/// A slide start or stop on `snd_ent` (at `origin`): updates the slot and
/// gives the sound to play on the slider, if any.
pub(crate) fn slide_event(
    slide: &mut SlideLoop,
    start: bool,
    player_view: bool,
    class: i32,
    origin: [f32; 3],
    snd_ent: u32,
) -> Option<PlayAlias> {
    let class = usize::try_from(class)
        .ok()
        .and_then(|index| CLASSES.get(index))
        .copied()
        .unwrap_or(CLASSES[0]);
    let who = if player_view { "plr" } else { "npc" };
    let alias = if start {
        slide.slot = Some(SlideSlot {
            alias: format!("fly_dtp_slide_loop_{who}_{class}"),
            origin,
        });
        let alias = format!("fly_pslide_loop_{who}_{class}");
        slide.started.push(StartSound {
            alias: alias.clone(),
            snd_ent,
            frame: slide.frame,
        });
        alias
    } else {
        slide.slot.take()?;
        format!("fly_pslide_stop_{who}_{class}")
    };
    Some(PlayAlias {
        namespace: AssetNamespace::T6,
        alias,
        fallback: None,
        origin_inches: (!player_view).then_some(origin),
        snd_ent: Some(snd_ent),
    })
}

fn sound_log() -> bool {
    static LOG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *LOG.get_or_init(|| std::env::var_os("BO2MP_SOUND_LOG").is_some())
}

/// Stops the start sounds a frame old, and keeps one emitter on the slot's
/// loop (3D at the slot when the alias is, otherwise on the ear).
#[allow(clippy::too_many_arguments)]
pub(crate) fn update(
    mut commands: Commands,
    mut slide: ResMut<SlideLoop>,
    epoch: Res<MatchEpoch>,
    mut play: MessageWriter<AliasCommand>,
    listeners: Query<&Transform, (With<AmbientListener>, Without<SlideLoopEmitter>)>,
    mut playing: Query<(Entity, &SlideLoopEmitter, &mut MapEmitter, &mut Transform)>,
    bank: Option<Res<SoundBank>>,
    mut clips: Option<ResMut<ClipStore>>,
    mut pcm: ResMut<Assets<PcmAudio>>,
    mut shared: ResMut<SharedPlayAssets>,
) {
    let slide = &mut *slide;
    if slide.epoch != epoch.0 {
        slide.epoch = epoch.0;
        slide.slot = None;
        slide.started.clear();
    }
    slide.frame = slide.frame.wrapping_add(1);
    let frame = slide.frame;
    slide.started.retain(|sound| {
        if frame.wrapping_sub(sound.frame) < 2 {
            return true;
        }
        play.write(AliasCommand::Stop {
            namespace: AssetNamespace::T6,
            alias: sound.alias.clone(),
            snd_ent: Some(sound.snd_ent),
        });
        false
    });

    let ear = listeners.iter().next().map(|t| t.translation.to_array());
    let mut kept = false;
    for (entity, tag, mut emitter, mut transform) in &mut playing {
        let slot = slide.slot.as_ref().filter(|slot| slot.alias == tag.alias);
        let Some(slot) = slot.filter(|_| !kept) else {
            commands.entity(entity).try_despawn();
            continue;
        };
        kept = true;
        let origin = if tag.positional { Some(slot.origin) } else { ear };
        if let Some(origin) = origin
            && emitter.origin_inches != origin
        {
            emitter.origin_inches = origin;
            transform.translation = Vec3::from_array(origin);
        }
    }
    if kept {
        return;
    }
    let (Some(slot), Some(bank), Some(clips)) = (slide.slot.as_ref(), bank, clips.as_mut()) else {
        return;
    };
    let alias = slot.alias.as_str();
    let Some(sound) = bank
        .0
        .sound_in(AssetNamespace::T6, alias)
        .and_then(|s| s.aliases.first())
    else {
        return;
    };
    let Some(key) = clip_keys_for_alias(&bank.0, AssetNamespace::T6, alias)
        .into_iter()
        .next()
    else {
        return;
    };
    clips.request(key.clone());
    let Some(Ok(audio)) = clips.ready(&key) else {
        return;
    };
    // Black Ops II's pan-type bit (flags bit 1 = 3D).
    let positional = sound.flags.is_some_and(|flags| flags & 2 != 0);
    let Some(origin) = (if positional { Some(slot.origin) } else { ear }) else {
        return;
    };
    let knots = if positional {
        sound
            .volume_falloff
            .as_ref()
            .map(|curve| shared.intern_curve(&curve.name, &curve.knots))
            .unwrap_or_else(|| Arc::from(vec![[0.0, 1.0], [1.0, 0.0]]))
    } else {
        Arc::from(vec![[0.0, 1.0], [1.0, 1.0]])
    };
    let vol = sound.vol_min.max(0.0);
    let mut spawned = commands.spawn((
        SlideLoopEmitter {
            alias: alias.to_owned(),
            positional,
        },
        MapAmbient,
        MapEmitter {
            origin_inches: origin,
            dist_min: sound.dist_min,
            dist_max: sound.dist_max.max(sound.dist_min + 1.0),
            knots,
            base_gain: vol,
            pcm: pcm.add(audio),
            speed: 1.0,
            live_pan: None,
            fixed_pan: sound.stereo_pan,
            category: sound.menu_category,
        },
        Transform::from_translation(Vec3::from_array(origin)),
    ));
    crate::duck::tag(&mut spawned, Some(sound), 0);
    if sound_log() {
        diag::info!(
            Audio,
            "audio: slide loop `{alias}` vol={vol} positional={positional}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_start_fills_the_slot_and_a_stop_empties_it_once() {
        let mut slide = SlideLoop::default();
        let start = slide_event(&mut slide, true, false, 2, [1.0, 2.0, 3.0], 7).unwrap();
        assert_eq!(start.alias, "fly_pslide_loop_npc_concrete");
        assert_eq!(start.origin_inches, Some([1.0, 2.0, 3.0]));
        assert_eq!(start.snd_ent, Some(7));
        assert_eq!(
            slide.slot,
            Some(SlideSlot {
                alias: "fly_dtp_slide_loop_npc_concrete".to_owned(),
                origin: [1.0, 2.0, 3.0],
            })
        );
        assert_eq!(slide.started.len(), 1);
        // The stop names the ground it ends on.
        let stop = slide_event(&mut slide, false, false, 0, [4.0, 5.0, 6.0], 7).unwrap();
        assert_eq!(stop.alias, "fly_pslide_stop_npc_default");
        assert_eq!(slide.slot, None);
        // The slot is empty: no sound.
        assert!(slide_event(&mut slide, false, false, 0, [4.0, 5.0, 6.0], 7).is_none());
    }

    #[test]
    fn his_own_slide_is_plr_on_his_ear_and_a_bad_class_is_default() {
        let mut slide = SlideLoop::default();
        let start = slide_event(&mut slide, true, true, 9, [1.0, 2.0, 3.0], 0).unwrap();
        assert_eq!(start.alias, "fly_pslide_loop_plr_default");
        assert_eq!(start.origin_inches, None);
        // Another player's start takes the one slot over.
        slide_event(&mut slide, true, false, 8, [9.0, 9.0, 9.0], 3).unwrap();
        assert_eq!(
            slide.slot.as_ref().map(|slot| slot.alias.as_str()),
            Some("fly_dtp_slide_loop_npc_mud")
        );
    }
}
