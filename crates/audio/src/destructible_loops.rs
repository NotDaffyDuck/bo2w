//! Props in a damage stage that speak a loop: gas hissing from a punctured
//! tank, a propane fire on its cap, a helicopter's engine. The host publishes
//! which alias each prop speaks and where it stands; the positional emitters
//! that carry those loops are kept in step with that list here. A loop on a
//! prop that moves goes with it, and its level and pitch follow the published
//! row (the script's `setloopstate`).

use asset_audio::{SnapshotSoundAlias, AudioRoster};
use asset_core::AssetCategoryScope;
use bevy::prelude::*;
use net::ShownWorldFrame;
use sim::{DestructibleLoopSound, Snapshot};

use crate::map_sound_set::{LevelSoundSource, CatalogNameSpace};
use crate::backend::PlayingCue;
use crate::loop_emitter::{self, Clip, LoopKit, Plan};
use crate::play_core::UnknownCueHoles;

/// A prop's loop that is running, with what the log last heard of it.
#[derive(Component)]
pub(crate) struct DestructibleLoop {
    prop: u32,
    name: String,
    /// The alias's own level (`vol_min`), which the row's volume scales.
    level: f32,
    /// Volume, pitch and time of the last level line (BO2MP_SOUND_LOG).
    last_said: (u8, u8, f64),
    /// Whether the log saw it holding a voice last time: among the loudest map
    /// emitters, and above the audible floor.
    heard: bool,
}

/// A published row together with the name of the alias it speaks.
struct Voiced<'a> {
    row: &'a DestructibleLoopSound,
    name: &'a str,
}

impl DestructibleLoop {
    fn is_for(&self, said: &Voiced) -> bool {
        self.prop == said.row.owner.encode_for_net() && self.name == said.name
    }

    fn report_stop(&self, at: [f32; 3]) {
        diag::info!(
            Audio,
            "audio: destructible loop `{}` on {:#x} stopped at {:?}",
            self.name,
            self.prop,
            rounded(at)
        );
    }

    fn report_voice(&mut self, has_voice: bool, at: [f32; 3]) {
        if !loop_emitter::logging() || has_voice == self.heard {
            return;
        }
        self.heard = has_voice;
        let state = if has_voice {
            "playing"
        } else {
            "dropped (out of range or not loudest)"
        };
        diag::info!(
            Audio,
            "audio: destructible loop `{}` on {:#x} voice {} at {:?}",
            self.name,
            self.prop,
            state,
            rounded(at)
        );
    }

    fn report_level(&mut self, row: &DestructibleLoopSound, now: f64) {
        let moved = (row.volume, row.pitch) != (self.last_said.0, self.last_said.1);
        if !(loop_emitter::logging() && moved && now - self.last_said.2 >= 1.0) {
            return;
        }
        self.last_said = (row.volume, row.pitch, now);
        diag::info!(
            Audio,
            "audio: destructible loop `{}` on {:#x} level vol={} pitch={} at {:?}",
            self.name,
            self.prop,
            row.volume,
            row.pitch,
            rounded(row.origin)
        );
    }
}

fn rounded(point: [f32; 3]) -> [f32; 3] {
    point.map(f32::round)
}

/// Emitter gain for a row: the alias level scaled by the row's percentage.
fn row_gain(level: f32, row: &DestructibleLoopSound) -> f32 {
    level * f32::from(row.volume) / 100.0
}

/// Playback speed for a row; its pitch is a percentage and never below 1.
fn row_speed(row: &DestructibleLoopSound) -> f32 {
    f32::from(row.pitch.max(1)) / 100.0
}

/// The published rows whose alias index has a name, with that name.
fn voiced_rows(snapshot: &Snapshot) -> Vec<Voiced<'_>> {
    let meta = &snapshot.meta;
    let rows = meta.world_objects.destructible_loop_sounds.iter();
    rows.filter_map(|row| {
        let entry = meta.sound_aliases.iter().find(|e| e.0 == row.alias_index)?;
        Some(Voiced {
            row,
            name: entry.1.as_str(),
        })
    })
    .collect()
}

/// The variant a new loop plays: the namespace's own alias, else the only
/// alias of that name anywhere in the bank.
fn definition_of<'a>(
    catalog: &'a AudioRoster,
    scope: AssetCategoryScope,
    name: &str,
) -> Option<&'a SnapshotSoundAlias> {
    let sound = catalog
        .entry_in(scope, name)
        .or_else(|| catalog.sounds.get(catalog.slot_unique(name)?))?;
    sound.aliases.first()
}

/// Starts the loop for a spoken row, once its clip has decoded.
fn begin(
    commands: &mut Commands,
    kit: &mut LoopKit,
    unfound: &mut UnknownCueHoles,
    ns: AssetCategoryScope,
    catalog: &AudioRoster,
    said: &Voiced,
    now: f64,
) {
    let Voiced { row, name } = *said;
    let clip = match kit.reel(ns, name) {
        Clip::Unknown => return unfound.note_miss(name),
        Clip::Decoding => return,
        Clip::Done(clip) => clip,
    };
    let Some(lead) = definition_of(catalog, ns, name) else {
        return unfound.note_miss(name);
    };
    let knots = kit.fade(lead).unwrap_or_default();
    if knots.first().is_none() {
        diag::warn!(
            Audio,
            "audio: destructible loop `{name}` has no falloff curve (typed gap)"
        );
    }
    let level = lead.volume_low.max(0.0);
    let plan = Plan {
        row: lead,
        at: row.origin,
        gain: row_gain(level, row),
        speed: row_speed(row),
        knots,
        far: lead.reach_upper,
    };
    let tag = DestructibleLoop {
        prop: row.owner.encode_for_net(),
        name: name.to_owned(),
        level,
        last_said: (row.volume, row.pitch, now),
        heard: false,
    };
    kit.spawn(commands, tag, clip, plan);
    diag::info!(
        Audio,
        "audio: destructible loop `{name}` on {:#x} vol={} pitch={} at {:?}",
        row.owner.encode_for_net(),
        row.volume,
        row.pitch,
        rounded(row.origin)
    );
}

pub(crate) fn sync_destructible_loops(
    mut commands: Commands,
    shown: Res<ShownWorldFrame>,
    mut standing: Query<'_, '_, (Entity, &'static mut DestructibleLoop, &'static mut LevelSoundSource, &'static mut Transform, Has<PlayingCue>)>,
    clock: Res<Time>,
    bank_ns: Option<Res<CatalogNameSpace>>,
    mut gaps: ResMut<UnknownCueHoles>,
    mut kit: LoopKit,
) {
    let Some(frame_snap) = shown.snapshot() else {
        return;
    };
    let spoken = voiced_rows(frame_snap);
    let now = clock.elapsed_secs_f64();

    for (ent, mut held, mut emit_state, mut transform, has_voice) in &mut standing {
        let Some(row) = spoken.iter().find(|said| held.is_for(said)).map(|said| said.row) else {
            commands.entity(ent).try_despawn();
            held.report_stop(emit_state.position_in_inches);
            continue;
        };
        held.report_voice(has_voice, emit_state.position_in_inches);
        loop_emitter::follow(&mut emit_state, &mut transform, row.origin);
        loop_emitter::retune(&mut emit_state, row_gain(held.level, row), row_speed(row));
        held.report_level(row, now);
    }

    let Some(catalog) = kit.catalog() else {
        return;
    };
    let ns = bank_ns.map_or(AssetCategoryScope::T6, |map| map.namespace);
    for said in &spoken {
        if standing.iter().any(|(_, held, ..)| held.is_for(said)) {
            continue;
        }
        begin(&mut commands, &mut kit, &mut gaps, ns, &catalog, said, now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip_bank::ClipRepository;
    use crate::sample_feed::ClipPcm;
    use crate::play_core::{CommonPlaybackStore, AudioCatalogRes};
    use std::sync::Arc;
    use asset_audio::{SnapshotSoundAlias, SnapshotSoundEntry, AudioRoster};
    use bevy::ecs::system::RunSystemOnce;
    use sim::{ScriptModelId, Snapshot, Tick};

    fn spoken(owner: u32, alias_index: u8, eye_pos: [f32; 3], volume: u8, pitch: u8) -> DestructibleLoopSound {
        DestructibleLoopSound {
            owner: ScriptModelId::decode_net(owner),
            alias_index,
            origin: eye_pos,
            volume,
            pitch,
        }
    }

    fn world_speaking(rows: Vec<DestructibleLoopSound>, names: &[(u8, &str)]) -> World {
        let mut world = World::new();
        let mut snapshot = Snapshot::unpublished(Tick(1));
        snapshot.meta.world_objects.destructible_loop_sounds = rows;
        snapshot.meta.sound_aliases = names
            .iter()
            .map(|(index, name)| (*index, (*name).to_owned()))
            .collect();
        let mut presented = ShownWorldFrame::default();
        presented.announce_parsed(snapshot);
        world.insert_resource(presented);
        world.insert_resource(Time::<()>::default());
        world.insert_resource(Assets::<ClipPcm>::default());
        world.insert_resource(CommonPlaybackStore::default());
        world.insert_resource(UnknownCueHoles::default());
        world
    }

    fn put_loop(world: &mut World, owner: u32, cue_name: &str, vol: f32) -> Entity {
        world
            .spawn((
                DestructibleLoop {
                    prop: owner,
                    name: cue_name.to_owned(),
                    level: vol,
                    last_said: (100, 100, 0.0),
                    heard: false,
                },
                LevelSoundSource {
                    position_in_inches: [0.0; 3],
                    reach_lower: 1.0,
                    reach_upper: 2.0,
                    knots: Arc::from(vec![[0.0, 1.0]]),
                    rest_gain: 0.0,
                    pcm: Handle::default(),
                    speed: 1.0,
                    active_stereo_handle: Option::default(),
                    fixed_pan: None,
                    category: None,
                    line_end_inches: None,
                },
                Transform::default(),
            ))
            .id()
    }

    fn run(world: &mut World) {
        world.run_system_once(sync_destructible_loops).unwrap();
    }

    fn install_bank(world: &mut World, cue_key: &str) {
        let mut catalog = AudioRoster::default();
        catalog.add_sound(SnapshotSoundEntry {
            name: cue_key.to_owned(),
            aliases: vec![SnapshotSoundAlias {
                streamed: Some(("nobank".to_owned(), "0001".to_owned())),
                ..SnapshotSoundAlias::default()
            }],
        });
        let catalog = Arc::new(catalog);
        world.insert_resource(ClipRepository::start(Arc::clone(&catalog), None));
        world.insert_resource(AudioCatalogRes(catalog));
    }

    #[test]
    fn a_rows_gain_and_speed_follow_its_percentages() {
        let row = spoken(1, 0, [0.0; 3], 50, 150);
        assert_eq!(row_gain(0.8, &row), 0.4);
        assert_eq!(row_speed(&row), 1.5);
        // A pitch of 0 never stops the clip.
        assert_eq!(row_speed(&spoken(1, 0, [0.0; 3], 0, 0)), 0.01);
        assert_eq!(rounded([1.4, -1.6, 2.5]), [1.0, -2.0, 3.0]);
    }

    #[test]
    fn with_no_snapshot_every_loop_is_left_alone() {
        let mut world = world_speaking(Vec::new(), &[]);
        world.insert_resource(ShownWorldFrame::default());
        let kept = put_loop(&mut world, 5, "hiss", 1.0);
        run(&mut world);
        assert!(world.get_entity(kept).is_ok());
    }

    #[test]
    fn a_loop_no_longer_spoken_is_despawned() {
        let mut world = world_speaking(vec![spoken(5, 2, [0.0; 3], 100, 100)], &[(2, "hiss")]);
        let other_owner = put_loop(&mut world, 6, "hiss", 1.0);
        let other_alias = put_loop(&mut world, 5, "roar", 1.0);
        let kept = put_loop(&mut world, 5, "hiss", 1.0);
        run(&mut world);
        assert!(world.get_entity(other_owner).is_err());
        assert!(world.get_entity(other_alias).is_err());
        assert!(world.get_entity(kept).is_ok());
    }

    #[test]
    fn a_row_with_no_alias_name_speaks_nothing() {
        let mut world = world_speaking(vec![spoken(5, 9, [0.0; 3], 100, 100)], &[(2, "hiss")]);
        let kept = put_loop(&mut world, 5, "hiss", 1.0);
        run(&mut world);
        assert!(world.get_entity(kept).is_err());
    }

    #[test]
    fn a_kept_loop_follows_the_published_row() {
        let mut world = world_speaking(
            vec![spoken(5, 2, [3.0, 4.0, 5.0], 50, 200)],
            &[(2, "hiss")],
        );
        let kept = put_loop(&mut world, 5, "hiss", 0.8);
        run(&mut world);
        let emitter = world.get::<LevelSoundSource>(kept).unwrap();
        assert_eq!(emitter.position_in_inches, [3.0, 4.0, 5.0]);
        assert_eq!(emitter.rest_gain, 0.4);
        assert_eq!(emitter.speed, 2.0);
        let transform = world.get::<Transform>(kept).unwrap();
        assert_eq!(transform.translation, Vec3::new(3.0, 4.0, 5.0));
    }

    #[test]
    fn a_spoken_alias_the_bank_lacks_is_a_gap_and_spawns_nothing() {
        let mut world = world_speaking(
            vec![spoken(5, 2, [0.0; 3], 100, 100), spoken(6, 2, [0.0; 3], 100, 100)],
            &[(2, "hiss")],
        );
        install_bank(&mut world, "on_disk");
        run(&mut world);
        assert_eq!(
            world.resource::<UnknownCueHoles>().aliases,
            vec!["hiss".to_owned()]
        );
        assert_eq!(world.query::<&DestructibleLoop>().iter(&world).count(), 0);
    }

    #[test]
    fn a_clip_still_decoding_spawns_nothing_and_is_not_a_gap() {
        let mut world = world_speaking(vec![spoken(5, 2, [0.0; 3], 100, 100)], &[(2, "hiss")]);
        install_bank(&mut world, "hiss");
        run(&mut world);
        assert_eq!(world.query::<&DestructibleLoop>().iter(&world).count(), 0);
        assert!(world.resource::<UnknownCueHoles>().aliases.is_empty());
    }
}
