//! Recording and playback commands: `demo` (alias `play`), `record`,
//! `stoprecord` and `clip`.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use replay::{
    CAPTURE_LOG_PATH, CLIP_CONTENTS_FILENAME, CLIP_SPAN_MILLIS, ClipLoopBuffer, RecordedGameLabel,
    StagedRecordingStart, DemoReplay, DemoRecorder, RecordingFault, PlaybackSession,
};
use ::session::SessionHandoverOrder;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use frame::StartupPersona;
use frame::ProcessDuty;
use net::{
    HostTimeKeeper, HostInputValve, HostSimulationWorld, ClientGuessState, ShownWorldFrame,
};
use render_frontend::prepare::scene::camera::GameplayCamera;

use super::echo::PromptReplyWriter;
use super::files::write_new_file;
use super::state_dump::state_dump_body;
use crate::TypedOrder;
use crate::TypedOrderRouter;
use replay::CLIP_RECORDING_FILE;
use std::io::ErrorKind;

/// The verb that closes an open recording.
const STOP_VERB: &str = "stoprecord";

pub(super) const HELP_BANNER: &[(&str, &str)] = &[
    (
        "demo",
        "demo <name> — stop whatever is running, then play bo2mp-artifacts/demos/<name>.bo2mpdemo or clips/<name>/clip.bo2mpdemo",
    ),
    ("play", "play <name> — launcher alias of demo"),
    (
        "record",
        "record [name] — start a demo under bo2mp-artifacts",
    ),
    (STOP_VERB, "stoprecord — close the demo being written"),
    (
        "clip",
        "clip — save last ≤45s available to this client as bo2mp-artifacts/clips/<ULID>/{clip.bo2mpdemo, dump.txt} (always-on ring)",
    ),
];

const NO_IDENTITY: &str = "launch identity missing (artifacts path unknown)";
const ID_TRIES: usize = 8;

/// Makes the folder for a new clip, with its name taken from `next_id`. When
/// the name is already in use another one is drawn, up to a handful of times.
fn allocate_clip_dir(
    root: &Path,
    mut next_id: impl FnMut() -> Result<String, String>,
) -> Result<(String, PathBuf), String> {
    for _ in 0..ID_TRIES {
        let id = next_id()?;
        let made = replay::make_clip_folder(root, &id);
        let taken = matches!(&made, Err(RecordingFault::Io(io)) if io.kind() == ErrorKind::AlreadyExists);
        if !taken {
            return made.map(|dir| (id, dir)).map_err(|why| format!("create clip folder: {why}"));
        }
    }
    Err(String::from("could not allocate a unique ULID directory"))
}

/// Text of a clip's manifest: what the replay crate writes, plus the origin of
/// the ticks and the role that kept them.
fn manifest_text(clip: &ClipFacts<'_>) -> String {
    let ClipFacts { id, ticks, span_ms, zone, recorded_epoch_nanos, source, role } = *clip;
    let mut text = replay::clip_contents_list(id, ticks, span_ms, zone, recorded_epoch_nanos);
    text += &format!("source = {source:?}\n");
    text += &format!("role = {role:?}\n");
    text
}

#[derive(Clone, Copy)]
struct ClipFacts<'a> {
    id: &'a str,
    ticks: u64,
    span_ms: u32,
    zone: &'a str,
    recorded_epoch_nanos: u128,
    source: &'a str,
    role: &'a str,
}

/// The material a clip is cut from. Only a process that runs the authority has
/// it; a plain client has the world the server sent.
struct Evidence<'a> {
    identity: &'a StartupPersona,
    authority: Option<&'a HostSimulationWorld>,
    clock: Option<&'a HostTimeKeeper>,
    prediction: Option<&'a sim::SimWorld>,
    presented: Option<&'a ShownWorldFrame>,
    ring: &'a ClipLoopBuffer,
}

/// Where a finished clip's files ended up.
struct StoredRecording {
    id: String,
    ticks: u64,
    demo: PathBuf,
    dump: PathBuf,
}

/// Nanoseconds since the UNIX epoch, now.
fn now_unix_ns() -> Result<u128, String> {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(since) => Ok(since.as_nanos()),
        Err(why) => Err(format!("system clock is before UNIX epoch: {why}")),
    }
}

fn text_of(error: impl std::fmt::Display) -> String {
    error.to_string()
}

impl Evidence<'_> {
    /// The world the clip's match identity is read from: the authority's when
    /// this process runs it, else the predicted one.
    fn world(&self) -> Result<&sim::SimWorld, String> {
        match (self.authority, self.prediction) {
            (Some(held), _) => Ok(&held.0),
            (None, Some(predicted)) => Ok(predicted),
            (None, None) => Err("no world to take the match identity from".to_owned()),
        }
    }

    /// Writes the ring as a demo at `path`; returns how many ticks it holds.
    fn write_demo(&self, path: &Path) -> Result<u64, String> {
        let identity = RecordedGameLabel::from_world_on_zone(self.world()?, &self.identity.zone);
        let mut demo = DemoRecorder::begin_at_location(path.to_path_buf(), identity).map_err(text_of)?;
        demo.append_ring(self.ring).map_err(text_of)?;
        demo.stop().map(|(ticks, _)| ticks).map_err(text_of)
    }

    /// Writes a clip's demo, state dump and manifest into `dir`.
    fn write_into(&self, dir: &Path, id: &str) -> Result<StoredRecording, String> {
        let recorded_epoch_nanos = now_unix_ns()?;
        self.world()?;
        let demo = dir.join(CLIP_RECORDING_FILE);
        let ticks = self.write_demo(&demo)?;

        let dump = dir.join(CAPTURE_LOG_PATH);
        let snapshot = state_dump_body(self.identity, recorded_epoch_nanos, self.clock, self.authority, self.presented, None);
        write_new_file(&dump, &snapshot)?;

        let facts = ClipFacts {
            id,
            ticks,
            span_ms: self.ring.span_ms(),
            zone: &self.identity.zone,
            recorded_epoch_nanos,
            source: if self.authority.is_some() { "authority" } else { "received" },
            role: &self.identity.duty_caption,
        };
        write_new_file(&dir.join(CLIP_CONTENTS_FILENAME), &manifest_text(&facts))?;
        replay::rewrite_latest_symlink(&self.identity.artifacts, id).map_err(text_of)?;
        Ok(StoredRecording { id: id.to_owned(), ticks, demo, dump })
    }

    /// Saves the ring as a new clip folder, removing the folder again if the
    /// clip could not be completed.
    fn save(&self, next_id: impl FnMut() -> Result<String, String>) -> Result<StoredRecording, String> {
        let (id, dir) = allocate_clip_dir(&self.identity.artifacts, next_id)?;
        self.write_into(&dir, &id).inspect_err(|_| {
            fs::remove_dir_all(&dir).ok();
        })
    }
}

/// Recording state the replay commands can change.
#[derive(SystemParam)]
pub(crate) struct Theater<'w> {
    persona: Option<Res<'w, StartupPersona>>,
    recorder: ResMut<'w, PlaybackSession>,
    arm: ResMut<'w, StagedRecordingStart>,
    swap: ResMut<'w, SessionHandoverOrder>,
}

/// Running-game state the replay commands only read.
#[derive(SystemParam)]
pub(crate) struct Live<'w> {
    authority: Option<Res<'w, HostSimulationWorld>>,
    camera: Res<'w, GameplayCamera>,
    gate: Res<'w, HostInputValve>,
    ring: Res<'w, ClipLoopBuffer>,
    duty: Res<'w, ProcessDuty>,
    prediction: Option<Res<'w, ClientGuessState>>,
    clock: Option<Res<'w, HostTimeKeeper>>,
    shown: Option<Res<'w, ShownWorldFrame>>,
}

impl Theater<'_> {
    /// The launch identity, or `<verb>: launch identity missing ...`.
    fn identity_for(&self, verb: &str) -> Result<&StartupPersona, String> {
        self.persona.as_deref().ok_or_else(|| format!("{verb}: {NO_IDENTITY}"))
    }

    /// Closes an open recording before playback, saying how that went.
    fn close_for_playback(&mut self) -> Option<String> {
        let open = self.recorder.0.take()?;
        let remark = open.stop().map_or_else(
            |why| format!("demo: stoprecord failed: {why}"),
            |(ticks, path)| {
                let shown = path.display();
                format!("demo: stoprecord {ticks} ticks in {shown} before playback")
            },
        );
        Some(remark)
    }

    /// Starts playing a recording. Side remarks go to `notes`; the `Err` text
    /// is the reply when playback cannot begin.
    fn begin_playback(
        &mut self,
        words: &[String],
        notes: &mut Vec<String>,
    ) -> Result<String, String> {
        let [name] = words else {
            return Err("usage: demo <name>".into());
        };
        let root = self.identity_for("demo")?.artifacts.clone();
        notes.extend(self.close_for_playback());
        let playback = DemoReplay::open(&root, name).map_err(|why| format!("demo: {why}"))?;
        let zone = match playback.identity().map_name() {
            Some(zone) => zone.to_owned(),
            None => {
                return Err(format!(
                    "demo: `{name}` carries no zone in its header — re-record it, or run: make play {name} ZONE=<map>"
                ));
            }
        };
        self.arm.exit_when_finished = false;
        self.arm.playback = Some(playback);
        let asked = self.swap.ask_for_recording(String::clone(name), String::clone(&zone), false);
        if asked.is_err() {
            // Nothing will come to play it.
            self.arm.playback = None;
        }
        asked
            .map(|swap| format!("demo: `{name}` zone `{zone}` (swap #{swap})"))
            .map_err(|why| format!("demo: {why}"))
    }

    fn begin_recording(&mut self, words: &[String], live: &Live) -> Result<String, String> {
        if words.len() > 1 {
            return Err("usage: record [name]".into());
        }
        if let Some(open) = self.recorder.0.as_ref() {
            let (name, ticks) = (open.name(), open.ticks_written());
            return Err(format!("record: already recording `{name}` ({ticks} ticks) — stoprecord first"));
        }
        let sim_owns_camera = live.camera.enabled && live.gate.local_cmds_enabled;
        if !sim_owns_camera {
            return Err(concat!(
                "record: the simulation does not own the camera yet ",
                "(no clip brushes, or an authored intermission view is showing); ",
                "there are no ticks to record",
            )
            .to_owned());
        }
        let persona = self.identity_for("record")?;
        let world = match live.authority.as_deref() {
            Some(authority) => &authority.0,
            None => return Err("record: authority world missing (match identity unavailable)".into()),
        };
        let stamp = RecordedGameLabel::from_world_on_zone(world, &persona.zone);
        let label = words.first().map(String::as_str).unwrap_or_default();
        let started = DemoRecorder::start_labelled(&persona.artifacts, label, stamp)
            .map_err(|why| format!("record: {why}"))?;
        let reply = format!("record: writing {}", started.path().display());
        self.recorder.0 = Some(started);
        Ok(reply)
    }

    fn end_recording(&mut self) -> Result<String, String> {
        let open = self.recorder.0.take().ok_or_else(|| String::from("stoprecord: not recording"))?;
        let (ticks, path) = open.stop().map_err(|why| format!("stoprecord: {why}"))?;
        let shown = path.display();
        Ok(format!("stoprecord: {ticks} ticks in {shown}"))
    }

    /// Saves the last stretch of play as a clip and returns what to say.
    fn save_clip(&self, words: &[String], live: &Live) -> Result<Vec<String>, String> {
        if let Some(refusal) = clip_refusal(words, &live.ring) {
            return Err(refusal.to_owned());
        }
        let identity = self.identity_for("clip")?;
        // A plain client keeps no authority of its own to cut from.
        let mine = live.duty.hosts_simulation();
        let evidence = Evidence {
            identity,
            authority: if mine { live.authority.as_deref() } else { None },
            clock: if mine { live.clock.as_deref() } else { None },
            prediction: live.prediction.as_deref().map(|state| state.0.world()),
            presented: live.shown.as_deref(),
            ring: &live.ring,
        };
        let saved = evidence
            .save(|| replay::fresh_sortable_id().map_err(text_of))
            .map_err(|why| format!("clip: {why}"))?;
        let seconds = f64::from(live.ring.span_ms()) / 1000.0;
        let window = CLIP_SPAN_MILLIS / 1000;
        let StoredRecording { id, ticks, demo, dump } = saved;
        let (demo, dump) = (demo.display(), dump.display());
        Ok(vec![
            format!("clip: {id}  {seconds:.2}s ({ticks} ticks) of {window}s"),
            format!("clip: demo → {demo}"),
            format!("clip: dump → {dump}"),
        ])
    }
}

/// Why `clip` will not run with these words and this ring, if it will not.
fn clip_refusal(args: &[String], ring: &ClipLoopBuffer) -> Option<&'static str> {
    if !args.is_empty() {
        Some("usage: clip")
    } else if ring.is_empty() {
        Some("clip: ring empty (0 ticks)")
    } else {
        None
    }
}

pub(crate) fn steer_playback_orders(
    mut commands: MessageReader<TypedOrder>,
    mut echo: PromptReplyWriter,
    mut theater: Theater,
    live: Live,
    mut dispatch: ResMut<TypedOrderRouter>,
) {
    let one = |line: String| vec![line];
    for order in commands.read() {
        let args = order.words.as_slice();
        let said: Result<Vec<String>, String> = match order.name.as_str() {
            "demo" | "play" => {
                let mut notes = Vec::new();
                let started = theater.begin_playback(args, &mut notes);
                notes.into_iter().for_each(|note| echo.write(note));
                // A demo that will not start must not leave the queue waiting.
                if started.is_err() {
                    dispatch.unblock();
                }
                started.map(one)
            }
            "record" => theater.begin_recording(args, &live).map(one),
            STOP_VERB => theater.end_recording().map(one),
            "clip" => theater.save_clip(args, &live),
            _ => continue,
        };
        let lines = said.unwrap_or_else(|refusal| vec![refusal]);
        lines.into_iter().for_each(|line| echo.write(line));
    }
}

#[cfg(test)]
mod tests {

    use super::super::files::fresh_folder;
    use crate::tool_cmds::replay::*;
    use crate::PromptOutputRow;
    use crate::PromptPanelOptions;
    use crate::PromptPanelStatus;
use bevy::app::Update;
use std::cell::Cell;

    fn zoned(zone: &str) -> RecordedGameLabel {
        let mut identity = RecordedGameLabel::default();
        identity.assign_region_label(zone);
        identity
    }

    /// A finished recording called `name` in `folder`, as a past `record` leaves it.
    fn recorded(folder: &Path, name: &str, identity: RecordedGameLabel) {
        DemoRecorder::start_labelled(folder, name, identity)
            .unwrap()
            .stop()
            .unwrap();
    }

    fn run(line: &str, folder: Option<&Path>, setup: impl FnOnce(&mut App)) -> App {
        let mut app = App::default();
        app.add_message::<TypedOrder>()
            .init_resource::<PromptPanelStatus>()
            .init_resource::<PromptPanelOptions>()
            .init_resource::<PromptOutputRow>()
            .init_resource::<PlaybackSession>()
            .init_resource::<StagedRecordingStart>()
            .init_resource::<SessionHandoverOrder>()
            .init_resource::<GameplayCamera>()
            .init_resource::<HostInputValve>()
            .init_resource::<ClipLoopBuffer>()
            .insert_resource(ProcessDuty::Listen)
            .insert_resource(TypedOrderRouter::in_flight())
            .add_systems(Update, steer_playback_orders);
        if let Some(folder) = folder {
            app.insert_resource(StartupPersona {
                artifacts: folder.to_path_buf(),
                zone: "mp_raid".into(),
                duty_caption: "host".into(),
                ..StartupPersona::default()
            });
        }
        setup(&mut app);
        for command in TypedOrder::parse_script(line) {
            app.world_mut().write_message(command);
        }
        app.update();
        app
    }

    fn log(app: &App) -> Vec<String> {
        app.world().resource::<PromptPanelStatus>().lines.clone()
    }

    fn queue_paused(app: &App) -> bool {
        app.world().resource::<TypedOrderRouter>().command_running
    }

    #[test]
    fn demo_needs_exactly_one_name_and_an_identity() {
        let folder = fresh_folder("demo-needs");
        for line in ["demo", "play a b"] {
            let app = run(line, Some(&folder), |_| {});
            assert_eq!(log(&app), ["usage: demo <name>"], "{line}");
            assert!(!queue_paused(&app), "{line}");
        }
        let app = run("demo x", None, |_| {});
        let actual = log(&app);
        assert_eq!(actual, ["demo: launch identity missing (artifacts path unknown)"]);
        assert!(!queue_paused(&app));
    }

    #[test]
    fn demo_plays_a_recording_on_the_map_it_was_made_on() {
        let folder = fresh_folder("demo-plays");
        recorded(&folder, "show", zoned("mp_raid"));
        let app = run("demo show", Some(&folder), |_| {});
        assert_eq!(log(&app), ["demo: `show` zone `mp_raid` (swap #1)"]);
        assert!(queue_paused(&app), "the queue waits for the swap");
        let arm = app.world().resource::<StagedRecordingStart>();
        assert!(arm.playback.is_some());
        assert!(!arm.exit_when_finished);
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn demo_refuses_a_recording_that_does_not_name_its_map() {
        let folder = fresh_folder("demo-no-zone");
        recorded(&folder, "old", RecordedGameLabel::default());
        let app = run("demo old", Some(&folder), |_| {});
        let expected = ["demo: `old` carries no zone in its header — re-record it, or run: make play old ZONE=<map>"];
        assert_eq!(log(&app), expected);
        assert!(!queue_paused(&app));
        assert!(
            app.world()
                .resource::<StagedRecordingStart>()
                .playback
                .is_none()
        );
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn demo_of_a_recording_that_is_not_there_lets_the_queue_go() {
        let folder = fresh_folder("demo-missing");
        let app = run("demo nothing", Some(&folder), |_| {});
        let said = log(&app);
        assert_eq!(said.len(), 1);
        assert!(said[0].starts_with("demo: "), "{said:?}");
        assert!(!queue_paused(&app));
    }

    #[test]
    fn a_second_demo_while_one_is_loading_is_refused_and_unarmed() {
        let folder = fresh_folder("demo-twice");
        recorded(&folder, "show", zoned("mp_raid"));
        let app = run("demo show; demo show", Some(&folder), |_| {});
        let expected = [
            "demo: `show` zone `mp_raid` (swap #1)",
            "demo: session swap already in progress"
        ];
        assert_eq!(log(&app), expected);
        assert!(!queue_paused(&app));
        assert!(
            app.world()
                .resource::<StagedRecordingStart>()
                .playback
                .is_none()
        );
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn demo_closes_a_recording_that_is_open() {
        let folder = fresh_folder("demo-closes");
        recorded(&folder, "show", zoned("mp_raid"));
        let app = run("demo show", Some(&folder), |app| {
            let open =
                DemoRecorder::start_labelled(&folder, "take", RecordedGameLabel::default())
                    .unwrap();
            app.world_mut().resource_mut::<PlaybackSession>().0 = Some(open);
        });
        let said = log(&app);
        assert_eq!(said.len(), 2, "{said:?}");
        assert!(
            said[0].starts_with("demo: stoprecord 0 ticks in ")
                && said[0].ends_with(" before playback"),
            "{said:?}"
        );
        assert!(app.world().resource::<PlaybackSession>().0.is_none());
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn record_says_what_stops_it() {
        let folder = fresh_folder("record-stops");
        let app = run("record a b", Some(&folder), |_| {});
        assert_eq!(log(&app), ["usage: record [name]"]);

        let app = run("record", Some(&folder), |_| {});
        let said = log(&app);
        assert!(
            said[0].starts_with("record: the simulation does not own the camera yet"),
            "{said:?}"
        );

        let owns_the_camera = |app: &mut App| {
            let sim_drives = true;
            app.insert_resource(GameplayCamera {
                enabled: sim_drives,
                ..GameplayCamera::default()
            })
            .insert_resource(HostInputValve {
                local_cmds_enabled: sim_drives,
            });
        };
        let app = run("record", None, owns_the_camera);
        let actual = log(&app);
        assert_eq!(actual, ["record: launch identity missing (artifacts path unknown)"]);
        let app = run("record", Some(&folder), owns_the_camera);
        let actual = log(&app);
        assert_eq!(actual, ["record: authority world missing (match identity unavailable)"]);
    }

    #[test]
    fn record_will_not_start_a_second_recording() {
        let folder = fresh_folder("record-twice");
        let app = run("record again", Some(&folder), |app| {
            let open =
                DemoRecorder::start_labelled(&folder, "take", RecordedGameLabel::default())
                    .unwrap();
            app.world_mut().resource_mut::<PlaybackSession>().0 = Some(open);
        });
        let expected = ["record: already recording `take` (0 ticks) — stoprecord first"];
        assert_eq!(log(&app), expected);
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn stoprecord_finishes_the_open_recording() {
        let folder = fresh_folder("stoprecord");
        let app = run("stoprecord", Some(&folder), |_| {});
        assert_eq!(log(&app), ["stoprecord: not recording"]);

        let app = run("stoprecord", Some(&folder), |app| {
            let open =
                DemoRecorder::start_labelled(&folder, "take", RecordedGameLabel::default())
                    .unwrap();
            app.world_mut().resource_mut::<PlaybackSession>().0 = Some(open);
        });
        let said = log(&app);
        assert!(said[0].starts_with("stoprecord: 0 ticks in "), "{said:?}");
        assert!(said[0].ends_with("take.bo2mpdemo"), "{said:?}");
        assert!(app.world().resource::<PlaybackSession>().0.is_none());
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn clip_needs_something_in_the_ring_and_a_place_to_put_it() {
        let folder = fresh_folder("clip-needs");
        let app = run("clip now", Some(&folder), |_| {});
        assert_eq!(log(&app), ["usage: clip"]);
        let app = run("clip", Some(&folder), |_| {});
        assert_eq!(log(&app), ["clip: ring empty (0 ticks)"]);
    }

    #[test]
    fn a_taken_id_is_replaced_by_a_fresh_one() {
        let folder = fresh_folder("clip-ids");
        let ids = ["TAKEN", "TAKEN", "FRESH"];
        let used = Cell::new(0);
        let next = || {
            used.set(used.get() + 1);
            Ok(ids[used.get() - 1].to_owned())
        };
        fs::create_dir_all(replay::clip_dir(&folder, "TAKEN")).unwrap();
        let (id, dir) = allocate_clip_dir(&folder, next).unwrap();
        assert_eq!(id, "FRESH");
        assert_eq!(dir, replay::clip_dir(&folder, "FRESH"));
        assert!(dir.is_dir());
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn ids_that_are_always_taken_give_up_after_a_few_tries() {
        let folder = fresh_folder("clip-ids-taken");
        fs::create_dir_all(replay::clip_dir(&folder, "SAME")).unwrap();
        let tries = Cell::new(0);
        let next = || {
            tries.set(tries.get() + 1);
            Ok("SAME".to_owned())
        };
        let expected = Err("could not allocate a unique ULID directory".into());
        assert_eq!(allocate_clip_dir(&folder, next), expected);
        assert_eq!(tries.get(), ID_TRIES);
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_failed_id_source_is_reported_as_it_is() {
        let folder = fresh_folder("clip-id-source");
        let expected = Err("no randomness".into());
        assert_eq!(allocate_clip_dir(&folder, || Err("no randomness".into())), expected);
    }

    #[test]
    fn the_manifest_says_where_the_ticks_came_from() {
        let text = manifest_text(&ClipFacts {
            id: "01ABC",
            ticks: 900,
            span_ms: 44_950,
            zone: "mp_raid",
            recorded_epoch_nanos: 12,
            source: "received",
            role: "client",
        });
        let expected = "id = \"01ABC\"\n\
         ticks = 900\n\
         duration_ms = 44950\n\
         zone = \"mp_raid\"\n\
         captured_unix_ns = 12\n\
         demo = \"clip.bo2mpdemo\"\n\
         dump = \"dump.txt\"\n\
         source = \"received\"\n\
         role = \"client\"\n";
        assert_eq!(text, expected);
    }

    #[test]
    fn a_clip_that_cannot_be_written_leaves_no_folder() {
        let folder = fresh_folder("clip-cleanup");
        let identity = StartupPersona {
            artifacts: folder.clone(),
            zone: "mp_raid".into(),
            ..StartupPersona::default()
        };
        let ring = ClipLoopBuffer::default();
        let evidence = Evidence {
            identity: &identity,
            authority: None,
            clock: None,
            prediction: None,
            presented: None,
            ring: &ring,
        };
        let saved = evidence.save(|| Ok("GONE".to_owned()));
        let expected = Some("no world to take the match identity from");
        assert_eq!(saved.err().as_deref(), expected);
        let expected = 0;
        assert_eq!(fs::read_dir(replay::clips_dir(&folder)).unwrap().count(), expected);
        let _ = fs::remove_dir_all(&folder);
    }
}
