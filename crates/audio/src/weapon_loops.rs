//! bo2mp: a gun's held loops while it fires: the Death Machine's barrel
//! whirring (`spinLoopSound`) and its firing roar (`fireLoopSound`), which
//! Black Ops II plays for as long as the trigger is down instead of a sound
//! per shot. The frontend lists who is firing such a gun each frame
//! (`WeaponLoops`); this reconciles that list against the emitters that
//! carry the loops, as `destructible_loops` does for a prop's loops. His own
//! gun's loops (no origin) sit on his ear, at full level.

use crate::ambient::{MapAmbient, MapEmitter};
use crate::clip_store::{ClipStore, clip_keys_for_alias};
use crate::pcm::PcmAudio;
use crate::playback::{AmbientListener, MissingAliasGaps, SharedPlayAssets, SoundBank};
use asset_core::AssetNamespace;
use bevy::prelude::*;
use std::sync::Arc;

/// One loop a gun is speaking this frame.
#[derive(Clone, Debug)]
pub struct WeaponLoopRow {
    /// The shooter's entity number.
    pub owner: u32,
    pub namespace: AssetNamespace,
    pub alias: String,
    /// Where the gun is; `None` for his own gun (heard on his ear).
    pub origin: Option<[f32; 3]>,
}

/// The gun loops speaking this frame (the frontend rewrites it every frame).
#[derive(Resource, Default)]
pub struct WeaponLoops(pub Vec<WeaponLoopRow>);

#[derive(Component)]
pub(crate) struct WeaponLoop {
    owner: u32,
    alias: String,
    own: bool,
}

fn sound_log() -> bool {
    static LOG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *LOG.get_or_init(|| std::env::var_os("BO2MP_SOUND_LOG").is_some())
}

pub(crate) fn update(
    mut commands: Commands,
    loops: Res<WeaponLoops>,
    listeners: Query<&Transform, (With<AmbientListener>, Without<WeaponLoop>)>,
    mut playing: Query<(Entity, &WeaponLoop, &mut MapEmitter, &mut Transform)>,
    bank: Option<Res<SoundBank>>,
    mut clips: Option<ResMut<ClipStore>>,
    mut pcm: ResMut<Assets<PcmAudio>>,
    mut shared: ResMut<SharedPlayAssets>,
    mut gaps: ResMut<MissingAliasGaps>,
) {
    let ear = listeners.iter().next().map(|t| t.translation.to_array());
    let place = |row: &WeaponLoopRow| row.origin.or(ear);

    for (entity, playing_loop, mut emitter, mut transform) in &mut playing {
        let Some(row) = loops.0.iter().find(|row| {
            row.owner == playing_loop.owner
                && row.alias == playing_loop.alias
                && row.origin.is_none() == playing_loop.own
        }) else {
            commands.entity(entity).try_despawn();
            if sound_log() {
                diag::info!(
                    Audio,
                    "audio: gun loop `{}` on {} stopped",
                    playing_loop.alias,
                    playing_loop.owner
                );
            }
            continue;
        };
        if let Some(origin) = place(row)
            && emitter.origin_inches != origin
        {
            emitter.origin_inches = origin;
            transform.translation = Vec3::from_array(origin);
        }
    }

    let (Some(bank), Some(clips)) = (bank, clips.as_mut()) else {
        return;
    };
    for row in &loops.0 {
        if playing.iter().any(|(_, playing_loop, _, _)| {
            playing_loop.owner == row.owner
                && playing_loop.alias == row.alias
                && playing_loop.own == row.origin.is_none()
        }) {
            continue;
        }
        let Some(origin) = place(row) else {
            continue;
        };
        let alias = row.alias.as_str();
        let Some(key) = clip_keys_for_alias(&bank.0, row.namespace, alias)
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
        let Some(sound) = bank
            .0
            .sound_in(row.namespace, alias)
            .and_then(|s| s.aliases.first())
        else {
            gaps.record(alias);
            continue;
        };
        // His own gun: flat (it is on his ear). Another's: the alias's
        // own falloff.
        let knots = if row.origin.is_none() {
            Arc::from(vec![[0.0, 1.0], [1.0, 1.0]])
        } else {
            sound
                .volume_falloff
                .as_ref()
                .map(|curve| shared.intern_curve(&curve.name, &curve.knots))
                .unwrap_or_else(|| Arc::from(vec![[0.0, 1.0], [1.0, 0.0]]))
        };
        let vol = sound.vol_min.max(0.0);
        let mut spawned = commands.spawn((
            WeaponLoop {
                owner: row.owner,
                alias: alias.to_owned(),
                own: row.origin.is_none(),
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
                "audio: gun loop `{alias}` on {} vol={vol} own={}",
                row.owner,
                row.origin.is_none()
            );
        }
    }
}
