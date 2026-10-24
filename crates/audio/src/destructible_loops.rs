//! Loops spoken by destructibles that are in a damage stage: gas hissing out
//! of a punctured tank, a propane fire burning on its cap, a helicopter's
//! engine. The host publishes which alias each prop is speaking and where;
//! this keeps the positional emitters that carry them in step with that set.
//! A loop on something that moves follows it, and its volume and pitch follow
//! the published row (the script's `setloopstate`).

use std::sync::Arc;

use asset_core::AssetNamespace;
use bevy::prelude::*;
use net::PresentedSnapshot;
use sim::DestructibleLoopSound;

use crate::ambient::{MapAmbient, MapEmitter, SoundBankNamespace};
use crate::clip_store::{ClipStore, clip_keys_for_alias};
use crate::pcm::PcmAudio;
use crate::playback::{MissingAliasGaps, SharedPlayAssets, SoundBank};

#[derive(Component)]
pub(crate) struct DestructibleLoop {
    owner: u32,
    alias: String,
    /// The alias's own level (`vol_min`); the row's volume scales it.
    vol: f32,
    /// Level last written to the log: volume, pitch, time (BO2MP_SOUND_LOG).
    logged: (u8, u8, f64),
    /// Whether the log last saw it holding a voice, i.e. among the loudest
    /// map emitters and above the audible floor.
    voiced: bool,
}

fn sound_log() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("BO2MP_SOUND_LOG").is_some())
}

fn rounded(point: [f32; 3]) -> [f32; 3] {
    point.map(f32::round)
}

/// Emitter gain for a row: the alias level scaled by the row's percentage.
fn row_gain(vol: f32, row: &DestructibleLoopSound) -> f32 {
    vol * f32::from(row.volume) / 100.0
}

/// Playback speed for a row; its pitch is a percentage and never below 1.
fn row_speed(row: &DestructibleLoopSound) -> f32 {
    f32::from(row.pitch.max(1)) / 100.0
}

pub(crate) fn update(
    mut commands: Commands,
    presented: Res<PresentedSnapshot>,
    mut playing: Query<(
        Entity,
        &mut DestructibleLoop,
        &mut MapEmitter,
        &mut Transform,
        Has<crate::backend::Voice>,
    )>,
    time: Res<Time>,
    bank: Option<Res<SoundBank>>,
    namespace: Option<Res<SoundBankNamespace>>,
    mut clips: Option<ResMut<ClipStore>>,
    mut pcm: ResMut<Assets<PcmAudio>>,
    mut shared: ResMut<SharedPlayAssets>,
    mut gaps: ResMut<MissingAliasGaps>,
) {
    let Some(snapshot) = presented.snapshot() else {
        return;
    };

    // The rows being spoken now, each with its alias name.
    let speaking: Vec<(&DestructibleLoopSound, &str)> = snapshot
        .meta
        .world_objects
        .destructible_loop_sounds
        .iter()
        .filter_map(|row| {
            let (_, name) = snapshot
                .meta
                .sound_aliases
                .iter()
                .find(|(index, _)| *index == row.alias_index)?;
            Some((row, name.as_str()))
        })
        .collect();

    let now = time.elapsed_secs_f64();

    // Loops already playing: retire the ones no longer spoken, retune the rest.
    for (entity, mut current, mut emitter, mut transform, has_voice) in &mut playing {
        let row = speaking
            .iter()
            .find(|(row, alias)| row.owner.to_wire() == current.owner && *alias == current.alias)
            .map(|(row, _)| *row);
        let Some(row) = row else {
            commands.entity(entity).try_despawn();
            diag::info!(
                Audio,
                "audio: destructible loop `{}` on {:#x} stopped at {:?}",
                current.alias,
                current.owner,
                emitter.origin_inches.map(f32::round)
            );
            continue;
        };

        if sound_log() && has_voice != current.voiced {
            current.voiced = has_voice;
            let state = if has_voice {
                "playing"
            } else {
                "dropped (out of range or not loudest)"
            };
            diag::info!(
                Audio,
                "audio: destructible loop `{}` on {:#x} voice {} at {:?}",
                current.alias,
                current.owner,
                state,
                emitter.origin_inches.map(f32::round)
            );
        }

        if emitter.origin_inches != row.origin {
            emitter.origin_inches = row.origin;
            transform.translation = Vec3::from_array(row.origin);
        }
        let gain = row_gain(current.vol, row);
        if emitter.base_gain != gain {
            emitter.base_gain = gain;
        }
        let speed = row_speed(row);
        if emitter.speed != speed {
            emitter.speed = speed;
        }

        let level_changed = (row.volume, row.pitch) != (current.logged.0, current.logged.1);
        if sound_log() && level_changed && now - current.logged.2 >= 1.0 {
            current.logged = (row.volume, row.pitch, now);
            diag::info!(
                Audio,
                "audio: destructible loop `{}` on {:#x} level vol={} pitch={} at {:?}",
                current.alias,
                current.owner,
                row.volume,
                row.pitch,
                rounded(row.origin)
            );
        }
    }

    // Spoken loops with no emitter yet: start them once their clip is decoded.
    let (Some(bank), Some(clips)) = (bank, clips.as_mut()) else {
        return;
    };
    let namespace = namespace.map_or(AssetNamespace::T6, |map| map.namespace);
    for (row, alias) in speaking {
        let owner = row.owner.to_wire();
        let started = playing
            .iter()
            .any(|(_, existing, _, _, _)| existing.owner == owner && existing.alias == alias);
        if started {
            continue;
        }

        let Some(key) = clip_keys_for_alias(&bank.0, namespace, alias)
            .into_iter()
            .next()
        else {
            gaps.record(alias);
            continue;
        };
        clips.request(key.clone());
        let Some(Ok(audio)) = clips.ready(&key) else {
            continue;
        };

        let definition = bank
            .0
            .sound_in(namespace, alias)
            .or_else(|| {
                let index = bank.0.index_unique(alias)?;
                bank.0.sounds.get(index)
            })
            .and_then(|sound| sound.aliases.first());
        let Some(definition) = definition else {
            gaps.record(alias);
            continue;
        };

        let knots = match &definition.volume_falloff {
            Some(curve) => shared.intern_curve(&curve.name, &curve.knots),
            None => Arc::from(Vec::<[f32; 2]>::new()),
        };
        if knots.is_empty() {
            diag::warn!(
                Audio,
                "audio: destructible loop `{alias}` has no falloff curve (typed gap)"
            );
        }

        let vol = definition.vol_min.max(0.0);
        let mut spawned = commands.spawn((
            DestructibleLoop {
                owner,
                alias: alias.to_owned(),
                vol,
                logged: (row.volume, row.pitch, now),
                voiced: false,
            },
            MapAmbient,
            MapEmitter {
                origin_inches: row.origin,
                dist_min: definition.dist_min,
                dist_max: definition.dist_max,
                knots,
                base_gain: row_gain(vol, row),
                pcm: pcm.add(audio),
                speed: row_speed(row),
                live_pan: None,
                fixed_pan: definition.stereo_pan,
                category: definition.menu_category,
            },
            Transform::from_translation(Vec3::from_array(row.origin)),
        ));
        crate::duck::tag(&mut spawned, Some(definition), 0);
        diag::info!(
            Audio,
            "audio: destructible loop `{alias}` on {:#x} vol={} pitch={} at {:?}",
            owner,
            row.volume,
            row.pitch,
            rounded(row.origin)
        );
    }
}
