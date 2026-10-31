//! bo2mp: what a gun keeps sounding while the trigger is held: the Death
//! Machine's barrel whirr (`spinLoopSound`) and its firing roar
//! (`fireLoopSound`). Black Ops II sustains them for as long as the trigger is
//! down, not once per shot. Each frame the frontend lists who is firing such a
//! gun (`WeaponLoops`); this brings the running emitters in line with that
//! list, the way `destructible_loops` does for a prop. A loop with no origin is
//! his own gun: it sits on his ear at full level.

use std::collections::{HashMap, HashSet};
use asset_core::AssetCategoryScope;
use bevy::prelude::*;

use crate::map_sound_set::LevelSoundSource;
use crate::loop_emitter::{self, Carry, Clip, LoopKit, Plan};
use crate::play_core::{BackgroundHearer, UnknownCueHoles};

/// One loop a gun is speaking this frame.
#[derive(Clone, Debug)]
pub struct WeaponLoopRow {
    /// The shooter's entity number.
    pub owner: u32,
    pub namespace: AssetCategoryScope,
    pub alias: String,
    /// Where the gun is; `None` for his own gun (heard on his ear).
    pub origin: Option<[f32; 3]>,
}

/// The gun loops speaking this frame (the frontend rewrites it every frame).
#[derive(Resource, Default)]
pub struct WeaponLoops(pub Vec<WeaponLoopRow>);

#[derive(Component)]
pub(crate) struct WeaponLoop {
    own: bool,
    shooter: u32,
    name: String,
}

impl WeaponLoop {
    fn bay(&self) -> (u32, &str, bool) {
        (self.shooter, self.name.as_str(), self.own)
    }
}

fn slot_of(row: &WeaponLoopRow) -> (u32, &str, bool) {
    (row.owner, row.alias.as_str(), row.origin.is_none())
}

pub(crate) fn sync_weapon_loops(
    mut commands: Commands,
    firing: Res<WeaponLoops>,
    ears: Query<&Transform, (With<BackgroundHearer>, Without<WeaponLoop>)>,
    mut running: Query<(Entity, &WeaponLoop, &mut LevelSoundSource, &mut Transform)>,
    mut kit: LoopKit,
    mut gaps: ResMut<UnknownCueHoles>,
) {
    let ear = ears.iter().next().map(|t| t.translation.to_array());
    let spot = |row: &WeaponLoopRow| row.origin.or(ear);

    let mut wanted: HashMap<(u32, &str, bool), &WeaponLoopRow> = HashMap::new();
    for row in &firing.0 {
        wanted.entry(slot_of(row)).or_insert(row);
    }
    for (ent, loop_tag, mut emit_state, mut transform) in &mut running {
        match wanted.get(&loop_tag.bay()) {
            Some(row) => {
                if let Some(at) = spot(row) {
                    loop_emitter::follow(&mut emit_state, &mut transform, at);
                }
            }
            None => {
                commands.entity(ent).try_despawn();
                if loop_emitter::logging() {
                    diag::info!(
                        Audio,
                        "audio: gun loop `{}` on {} stopped",
                        loop_tag.name,
                        loop_tag.shooter
                    );
                }
            }
        }
    }

    let Some(catalog) = kit.catalog() else {
        return;
    };
    let alive: HashSet<(u32, &str, bool)> = running.iter().map(|(_, tag, ..)| tag.bay()).collect();
    for bank_row in firing.0.iter().filter(|row| !alive.contains(&slot_of(row))) {
        let Some(at) = spot(bank_row) else {
            continue;
        };
        let name = bank_row.alias.as_str();
        let clip = match kit.reel(bank_row.namespace, name) {
            Clip::Done(clip) => clip,
            Clip::Decoding => continue,
            Clip::Unknown => {
                gaps.note_miss(name);
                continue;
            }
        };
        let Some(sound) = loop_emitter::lead_row(&catalog, bank_row.namespace, name) else {
            gaps.note_miss(name);
            continue;
        };
        let own = bank_row.origin.is_none();
        let gain = sound.volume_low.max(0.0);
        let plan = Plan {
            row: sound,
            at,
            gain,
            speed: 1.0,
            knots: kit.knots_for(sound, if own { Carry::Ear } else { Carry::World }),
            far: loop_emitter::widened(sound),
        };
        let tag = WeaponLoop {
            own,
            shooter: bank_row.owner,
            name: name.to_owned(),
        };
        kit.spawn(&mut commands, tag, clip, plan);
        if loop_emitter::logging() {
            diag::info!(
                Audio,
                "audio: gun loop `{name}` on {} vol={gain} own={own}",
                bank_row.owner
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip_bank::ClipRepository;
    use crate::play_core::{CommonPlaybackStore, AudioCatalogRes};
    use crate::sample_feed::ClipPcm;
    use asset_audio::{SnapshotSoundAlias, SnapshotSoundEntry, AudioRoster};
    use bevy::ecs::system::RunSystemOnce;
    use std::sync::Arc;

    fn row(owner: u32, cue_name: &str, eye_pos: Option<[f32; 3]>) -> WeaponLoopRow {
        WeaponLoopRow {
            owner,
            namespace: AssetCategoryScope::T6,
            alias: cue_name.to_owned(),
            origin: eye_pos,
        }
    }

    fn world_with(rows: Vec<WeaponLoopRow>) -> World {
        let mut world = World::new();
        world.insert_resource(WeaponLoops(rows));
        world.insert_resource(Assets::<ClipPcm>::default());
        world.insert_resource(CommonPlaybackStore::default());
        world.insert_resource(UnknownCueHoles::default());
        world
    }

    fn catalog_with(cue_id: &str) -> Arc<asset_audio::AudioRoster> {
        let mut catalog = AudioRoster::default();
        catalog.add_sound(SnapshotSoundEntry {
            name: cue_id.to_owned(),
            aliases: vec![SnapshotSoundAlias {
                streamed: Some(("nobank".to_owned(), "0001".to_owned())),
                ..SnapshotSoundAlias::default()
            }],
        });
        Arc::new(catalog)
    }

    fn install_bank(world: &mut World, catalog: Arc<asset_audio::AudioRoster>) {
        world.insert_resource(ClipRepository::start(Arc::clone(&catalog), None));
        world.insert_resource(AudioCatalogRes(catalog));
    }

    fn put_loop(world: &mut World, owner: u32, cue_key: &str, own: bool, at: [f32; 3]) -> Entity {
        world
            .spawn((
                WeaponLoop {
                    own,
                    shooter: owner,
                    name: cue_key.to_owned(),
                },
                LevelSoundSource {
                    position_in_inches: at,
                    reach_lower: 1.0,
                    reach_upper: 2.0,
                    knots: Arc::from(vec![[0.0, 1.0]]),
                    rest_gain: 1.0,
                    pcm: Handle::default(),
                    speed: 1.0,
                    active_stereo_handle: Option::default(),
                    fixed_pan: None,
                    category: None,
                    line_end_inches: None,
                },
                Transform::from_translation(Vec3::from_array(at)),
            ))
            .id()
    }

    fn run(world: &mut World) {
        world.run_system_once(sync_weapon_loops).unwrap();
    }

    fn origin_of(world: &World, ent: Entity) -> ([f32; 3], Vec3) {
        let emitter = world.get::<LevelSoundSource>(ent).unwrap();
        let transform = world.get::<Transform>(ent).unwrap();
        (emitter.position_in_inches, transform.translation)
    }

    #[test]
    fn a_loop_nobody_fires_any_more_is_despawned() {
        let mut world = world_with(vec![row(3, "held", Some([0.0; 3]))]);
        let stale = put_loop(&mut world, 4, "held", false, [0.0; 3]);
        let other_alias = put_loop(&mut world, 3, "other", false, [0.0; 3]);
        let kept = put_loop(&mut world, 3, "held", false, [0.0; 3]);
        run(&mut world);
        assert!(world.get_entity(stale).is_err());
        assert!(world.get_entity(other_alias).is_err());
        assert!(world.get_entity(kept).is_ok());
    }

    #[test]
    fn his_own_loop_and_another_shooters_loop_are_different_loops() {
        // The row is on a world origin; the standing loop is the `own` one.
        let mut world = world_with(vec![row(3, "held", Some([1.0; 3]))]);
        let own_loop = put_loop(&mut world, 3, "held", true, [0.0; 3]);
        run(&mut world);
        assert!(world.get_entity(own_loop).is_err());
        // A row without an origin keeps an `own` loop and drops a world one.
        let mut world = world_with(vec![row(3, "held", None)]);
        let own_loop = put_loop(&mut world, 3, "held", true, [0.0; 3]);
        let world_loop = put_loop(&mut world, 3, "held", false, [0.0; 3]);
        run(&mut world);
        assert!(world.get_entity(own_loop).is_ok());
        assert!(world.get_entity(world_loop).is_err());
    }

    #[test]
    fn a_kept_loop_follows_its_gun_and_his_own_follows_his_ear() {
        let mut world = world_with(vec![
            row(3, "held", Some([5.0, 6.0, 7.0])),
            row(9, "own", None),
        ]);
        world.spawn((BackgroundHearer, Transform::from_xyz(10.0, 20.0, 30.0)));
        let theirs = put_loop(&mut world, 3, "held", false, [0.0; 3]);
        let mine = put_loop(&mut world, 9, "own", true, [0.0; 3]);
        run(&mut world);
        assert_eq!(
            origin_of(&world, theirs),
            ([5.0, 6.0, 7.0], Vec3::new(5.0, 6.0, 7.0))
        );
        assert_eq!(
            origin_of(&world, mine),
            ([10.0, 20.0, 30.0], Vec3::new(10.0, 20.0, 30.0))
        );
    }

    #[test]
    fn his_own_loop_stays_put_when_there_is_no_ear() {
        let mut world = world_with(vec![row(9, "own", None)]);
        let mine = put_loop(&mut world, 9, "own", true, [4.0, 4.0, 4.0]);
        run(&mut world);
        assert_eq!(origin_of(&world, mine).0, [4.0, 4.0, 4.0]);
    }

    #[test]
    fn an_alias_the_bank_lacks_is_a_gap_once_and_spawns_nothing() {
        let mut world = world_with(vec![
            row(1, "nothing_here", Some([0.0; 3])),
            row(2, "nothing_here", Some([1.0; 3])),
            row(3, "no_ear_no_gap", None),
        ]);
        install_bank(&mut world, catalog_with("present"));
        run(&mut world);
        run(&mut world);
        let gaps = world.resource::<UnknownCueHoles>();
        assert_eq!(gaps.aliases, vec!["nothing_here".to_owned()]);
        assert_eq!(world.query::<&WeaponLoop>().iter(&world).count(), 0);
    }

    #[test]
    fn a_clip_that_is_not_ready_spawns_nothing_and_is_not_a_gap() {
        let mut world = world_with(vec![row(1, "present", Some([0.0; 3]))]);
        install_bank(&mut world, catalog_with("present"));
        run(&mut world);
        assert_eq!(world.query::<&WeaponLoop>().iter(&world).count(), 0);
        assert!(world.resource::<UnknownCueHoles>().aliases.is_empty());
    }
}
