//! bo2mp: Black Ops II's ground sounds for sliding, driven on the client by
//! the server's slide start and stop events (the event's parm is the slide
//! class of the ground).
//!
//! BO2 keeps a single slide slot per local client, shared by all players. A
//! start fills it (or reuses it), pins it where the slider stands (it does not
//! follow him afterwards) and loops `fly_dtp_slide_loop_<plr|npc>_<class>` on
//! it; a stop on an empty slot does nothing, otherwise it empties the slot.
//! Both also play a sound on the slider: the start one, named for the loop,
//! and the stop one. `plr` is the player you are looking through, `npc` is
//! anyone else.
//!
//! The start sound is itself a looping alias that is played once. BO2 cuts a
//! looping sound that nothing keeps alive on the following frame, so here it
//! is cut once a frame has gone by. A class with no alias stays silent, as in
//! BO2 (no stand-in).

use asset_core::AssetCategoryScope;
use bevy::prelude::*;

use crate::map_sound_set::LevelSoundSource;
use crate::backend::GameRoundSerial;
use crate::loop_emitter::{self, Carry, Clip, LoopKit, Plan};
use crate::play_core::BackgroundHearer;
use crate::{AliasCommand, TriggerCueMsg};

/// The nine kinds of ground the slide sounds tell apart, by the events' parm.
const GROUNDS: [&str; 9] = [
    "default", "metal", "concrete", "wood", "snow", "dirt", "gravel", "grass", "mud",
];

/// Black Ops II's pan-type bit in an alias's flags: set means a 3D sound.
const PAN_3D: u32 = 2;

/// A start sound is cut once it is this many frames old.
const START_LIFE: u64 = 2;

/// What the slot loops, and where.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SlideSlot {
    pub(crate) spot: [f32; 3],
    pub(crate) alias: String,
}

/// A start sound waiting to be cut.
#[derive(Clone, Debug, PartialEq)]
struct Awaiting {
    ent: u32,
    born: u64,
    cue_name: String,
}

#[derive(Resource, Default)]
pub(crate) struct SlideLoop {
    pub(crate) bay: Option<SlideSlot>,
    awaiting: Vec<Awaiting>,
    age: u64,
    seen_epoch: u64,
}

#[derive(Component)]
pub(crate) struct SlideLoopEmitter {
    positional: bool,
    loop_cue: String,
}

/// A slide start or stop on `sound_ent` (at `origin`): updates the slot and
/// gives the sound to play on the slider, if any.
pub(crate) fn slide_event(
    slide: &mut SlideLoop,
    start: bool,
    first_person: bool,
    class: i32,
    spot: [f32; 3],
    sound_ent: u32,
) -> Option<TriggerCueMsg> {
    let ground = usize::try_from(class)
        .ok()
        .and_then(|n| GROUNDS.get(n).copied())
        .unwrap_or(GROUNDS[0]);
    let viewer = if first_person { "plr" } else { "npc" };
    let named = |stem: &str| format!("fly_{stem}_{viewer}_{ground}");
    let alias = if start {
        let on_slider = named("pslide_loop");
        slide.bay = Some(SlideSlot {
            spot,
            alias: named("dtp_slide_loop"),
        });
        slide.awaiting.push(Awaiting {
            ent: sound_ent,
            born: slide.age,
            cue_name: on_slider.clone(),
        });
        on_slider
    } else {
        slide.bay.take()?;
        named("pslide_stop")
    };
    Some(TriggerCueMsg {
        position_in_inches: (!first_person).then_some(spot),
        source_entity_no: Some(sound_ent),
        fallback: Option::default(),
        cue_ns: AssetCategoryScope::T6,
        alias,
    })
}

/// A new match starts with an empty slot and no start sound to cut.
fn forget_old_match(slide: &mut SlideLoop, epoch: u64) {
    if slide.seen_epoch == epoch {
        return;
    }
    slide.seen_epoch = epoch;
    slide.bay = None;
    slide.awaiting.clear();
}

/// Cuts the start sounds that have lived out their frames, oldest first.
fn cut_spent_starts(slide: &mut SlideLoop, play: &mut MessageWriter<AliasCommand>) {
    let now = slide.age;
    let (spent, live): (Vec<_>, Vec<_>) = std::mem::take(&mut slide.awaiting)
        .into_iter()
        .partition(|start| now.wrapping_sub(start.born) >= START_LIFE);
    slide.awaiting = live;
    for start in spent {
        play.write(AliasCommand::Stop {
            namespace: AssetCategoryScope::T6,
            alias: start.cue_name,
            source_entity_no: Some(start.ent),
        });
    }
}

/// Cuts spent start sounds, and keeps one emitter on the slot's loop (in the
/// world at the slot when its alias is 3D, otherwise on the ear).
pub(crate) fn sync_slide_loop(
    mut commands: Commands,
    mut slide: ResMut<SlideLoop>,
    epoch: Res<GameRoundSerial>,
    mut play: MessageWriter<AliasCommand>,
    ears: Query<&Transform, (With<BackgroundHearer>, Without<SlideLoopEmitter>)>,
    mut running: Query<(Entity, &SlideLoopEmitter, &mut LevelSoundSource, &mut Transform)>,
    mut kit: LoopKit,
) {
    let slide = &mut *slide;
    forget_old_match(slide, epoch.0);
    slide.age = slide.age.wrapping_add(1);
    cut_spent_starts(slide, &mut play);

    let ear = ears.iter().next().map(|t| t.translation.to_array());
    let slot = slide.bay.as_ref();
    let mut holder = None;
    for (ent_id, tag, mut emit_state, mut transform) in &mut running {
        let fits = holder.is_none() && slot.is_some_and(|slot| slot.alias == tag.loop_cue);
        if !fits {
            commands.entity(ent_id).try_despawn();
            continue;
        }
        holder = Some(ent_id);
        let to = if tag.positional {
            slot.map(|slot| slot.spot)
        } else {
            ear
        };
        if let Some(to) = to {
            loop_emitter::follow(&mut emit_state, &mut transform, to);
        }
    }
    if holder.is_some() {
        return;
    }

    let (Some(slot), Some(catalog)) = (slot, kit.catalog()) else {
        return;
    };
    let name = slot.alias.as_str();
    let Some(sound) = loop_emitter::lead_row(&catalog, AssetCategoryScope::T6, name) else {
        return;
    };
    let Clip::Done(clip) = kit.reel(AssetCategoryScope::T6, name) else {
        return;
    };
    let positional = sound.alias_bits.unwrap_or(0) & PAN_3D != 0;
    let Some(at) = (if positional { Some(slot.spot) } else { ear }) else {
        return;
    };
    let gain = sound.volume_low.max(0.0);
    let carry = if positional { Carry::World } else { Carry::Ear };
    let plan = Plan {
        row: sound,
        at,
        gain,
        speed: 1.0,
        knots: kit.knots_for(sound, carry),
        far: loop_emitter::widened(sound),
    };
    let tag = SlideLoopEmitter {
        positional,
        loop_cue: name.to_owned(),
    };
    kit.spawn(&mut commands, tag, clip, plan);
    if loop_emitter::logging() {
        diag::info!(
            Audio,
            "audio: slide loop `{name}` vol={gain} positional={positional}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use crate::clip_bank::ClipRepository;
    use crate::sample_feed::ClipPcm;
    use crate::play_core::{CommonPlaybackStore, AudioCatalogRes};
    use asset_audio::{SnapshotSoundAlias, SnapshotSoundEntry, AudioRoster};
    use bevy::ecs::system::RunSystemOnce;

    fn world_at(epoch: u64) -> World {
        let mut world = World::new();
        world.insert_resource(SlideLoop {
            seen_epoch: epoch,
            ..SlideLoop::default()
        });
        world.insert_resource(GameRoundSerial(epoch));
        world.init_resource::<Messages<AliasCommand>>();
        world.insert_resource(Assets::<ClipPcm>::default());
        world.insert_resource(CommonPlaybackStore::default());
        world
    }

    fn bay(cue_id: &str, eye_pos: [f32; 3]) -> Option<SlideSlot> {
        Some(SlideSlot {
            alias: cue_id.to_owned(),
            spot: eye_pos,
        })
    }

    fn put_emitter(world: &mut World, cue_key: &str, positional: bool, at: [f32; 3]) -> Entity {
        world
            .spawn((
                SlideLoopEmitter {
                    loop_cue: cue_key.to_owned(),
                    positional,
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
        world.run_system_once(sync_slide_loop).unwrap();
    }

    fn stops(world: &mut World) -> Vec<(String, Option<u32>)> {
        world
            .resource_mut::<Messages<AliasCommand>>()
            .drain()
            .map(|command| match command {
                AliasCommand::Stop { alias, source_entity_no, .. } => (alias, source_entity_no),
                other => panic!("unexpected {other:?}"),
            })
            .collect()
    }

    fn count_emitters(world: &mut World) -> usize {
        world.query::<&SlideLoopEmitter>().iter(world).count()
    }

    #[test]
    fn a_start_sound_is_stopped_once_a_frame_has_passed() {
        let mut world = world_at(0);
        world.resource_mut::<SlideLoop>().awaiting = vec![Awaiting {
            cue_name: "fly_pslide_loop_plr_wood".to_owned(),
            ent: 6,
            born: 0,
        }];
        run(&mut world);
        assert!(stops(&mut world).is_empty());
        assert_eq!(world.resource::<SlideLoop>().awaiting.len(), 1);
        run(&mut world);
        assert_eq!(
            stops(&mut world),
            vec![("fly_pslide_loop_plr_wood".to_owned(), Some(6))]
        );
        assert!(world.resource::<SlideLoop>().awaiting.is_empty());
        assert_eq!(world.resource::<SlideLoop>().age, 2);
    }

    #[test]
    fn a_new_match_empties_the_slot_and_forgets_the_start_sounds() {
        let mut world = world_at(1);
        {
            let mut slide = world.resource_mut::<SlideLoop>();
            slide.bay = bay("fly_dtp_slide_loop_plr_wood", [0.0; 3]);
            slide.awaiting = vec![Awaiting {
                cue_name: "a".to_owned(),
                ent: 1,
                born: 0,
            }];
        }
        world.insert_resource(GameRoundSerial(2));
        let loop_entity = put_emitter(&mut world, "fly_dtp_slide_loop_plr_wood", true, [0.0; 3]);
        run(&mut world);
        let slide = world.resource::<SlideLoop>();
        assert_eq!((slide.bay.clone(), slide.seen_epoch), (None, 2));
        assert!(slide.awaiting.is_empty());
        assert!(stops(&mut world).is_empty());
        assert!(world.get_entity(loop_entity).is_err());
    }

    #[test]
    fn the_slots_emitter_follows_it_in_3d_or_rides_the_ear() {
        let mut world = world_at(0);
        world.spawn((BackgroundHearer, Transform::from_xyz(7.0, 8.0, 9.0)));
        world.resource_mut::<SlideLoop>().bay = bay("loop_a", [1.0, 2.0, 3.0]);
        let positional = put_emitter(&mut world, "loop_a", true, [0.0; 3]);
        run(&mut world);
        let at = |entity| world.get::<LevelSoundSource>(entity).unwrap().position_in_inches;
        assert_eq!(at(positional), [1.0, 2.0, 3.0]);
        assert_eq!(
            world.get::<Transform>(positional).unwrap().translation,
            Vec3::new(1.0, 2.0, 3.0)
        );
        let mut world = world_at(0);
        world.spawn((BackgroundHearer, Transform::from_xyz(7.0, 8.0, 9.0)));
        world.resource_mut::<SlideLoop>().bay = bay("loop_a", [1.0, 2.0, 3.0]);
        let flat = put_emitter(&mut world, "loop_a", false, [0.0; 3]);
        run(&mut world);
        let emitter = world.get::<LevelSoundSource>(flat).unwrap();
        assert_eq!(emitter.position_in_inches, [7.0, 8.0, 9.0]);
    }

    #[test]
    fn only_one_emitter_stays_and_only_for_the_slots_alias() {
        let mut world = world_at(0);
        world.resource_mut::<SlideLoop>().bay = bay("loop_a", [0.0; 3]);
        put_emitter(&mut world, "loop_a", true, [0.0; 3]);
        put_emitter(&mut world, "loop_a", true, [0.0; 3]);
        put_emitter(&mut world, "loop_b", true, [0.0; 3]);
        run(&mut world);
        let left: Vec<String> = world
            .query::<&SlideLoopEmitter>()
            .iter(&world)
            .map(|tag| tag.loop_cue.clone())
            .collect();
        assert_eq!(left, vec!["loop_a".to_owned()]);
    }

    #[test]
    fn an_empty_slot_clears_every_emitter() {
        let mut world = world_at(0);
        put_emitter(&mut world, "loop_a", true, [0.0; 3]);
        put_emitter(&mut world, "loop_b", false, [0.0; 3]);
        run(&mut world);
        assert_eq!(count_emitters(&mut world), 0);
    }

    #[test]
    fn nothing_spawns_without_a_bank_or_a_playable_clip() {
        let mut world = world_at(0);
        world.resource_mut::<SlideLoop>().bay = bay("loop_a", [0.0; 3]);
        run(&mut world);
        assert_eq!(count_emitters(&mut world), 0);
        // A bank that holds the alias, but whose clip is not decoded yet.
        let mut catalog = AudioRoster::default();
        catalog.add_sound(SnapshotSoundEntry {
            name: "loop_a".to_owned(),
            aliases: vec![SnapshotSoundAlias {
                streamed: Some(("nobank".to_owned(), "0001".to_owned())),
                ..SnapshotSoundAlias::default()
            }],
        });
        let catalog = Arc::new(catalog);
        world.insert_resource(ClipRepository::start(Arc::clone(&catalog), None));
        world.insert_resource(AudioCatalogRes(catalog));
        world.spawn((BackgroundHearer, Transform::default()));
        run(&mut world);
        assert_eq!(count_emitters(&mut world), 0);
    }

    #[test]
    fn the_classes_run_by_the_events_parm() {
        let mut slide = SlideLoop::default();
        let names: Vec<String> = (0..10)
            .map(|class| {
                slide_event(&mut slide, true, true, class, [0.0; 3], 1)
                    .unwrap()
                    .alias
            })
            .collect();
        assert_eq!(names[8], "fly_pslide_loop_plr_mud");
        assert_eq!(names[9], "fly_pslide_loop_plr_default");
        assert_eq!(names[4], "fly_pslide_loop_plr_snow");
        let negative = slide_event(&mut slide, false, true, -1, [0.0; 3], 1).unwrap();
        assert_eq!(negative.alias, "fly_pslide_stop_plr_default");
        assert_eq!(negative.source_entity_no, Some(1));
        assert!(negative.fallback.is_none());
        assert_eq!(negative.cue_ns, AssetCategoryScope::T6);
    }

    #[test]
    fn a_start_fills_the_slot_and_a_stop_empties_it_once() {
        let mut slide = SlideLoop::default();
        let start = slide_event(&mut slide, true, false, 2, [1.0, 2.0, 3.0], 7).unwrap();
        assert_eq!(start.alias, "fly_pslide_loop_npc_concrete");
        assert_eq!(start.position_in_inches, Some([1.0, 2.0, 3.0]));
        assert_eq!(start.source_entity_no, Some(7));
        assert_eq!(
            slide.bay,
            Some(SlideSlot {
                alias: "fly_dtp_slide_loop_npc_concrete".to_owned(),
                spot: [1.0, 2.0, 3.0],
            })
        );
        assert_eq!(slide.awaiting.len(), 1);
        // The stop names the ground it ends on.
        let stop = slide_event(&mut slide, false, false, 0, [4.0, 5.0, 6.0], 7).unwrap();
        assert_eq!(stop.alias, "fly_pslide_stop_npc_default");
        assert_eq!(slide.bay, None);
        // The slot is empty: no sound.
        assert!(slide_event(&mut slide, false, false, 0, [4.0, 5.0, 6.0], 7).is_none());
    }

    #[test]
    fn his_own_slide_is_plr_on_his_ear_and_a_bad_class_is_default() {
        let mut slide = SlideLoop::default();
        let start = slide_event(&mut slide, true, true, 9, [1.0, 2.0, 3.0], 0).unwrap();
        assert_eq!(start.alias, "fly_pslide_loop_plr_default");
        assert_eq!(start.position_in_inches, None);
        // Another player's start takes the one slot over.
        slide_event(&mut slide, true, false, 8, [9.0, 9.0, 9.0], 3).unwrap();
        assert_eq!(
            slide.bay.as_ref().map(|slot| slot.alias.as_str()),
            Some("fly_dtp_slide_loop_npc_mud")
        );
    }
}
