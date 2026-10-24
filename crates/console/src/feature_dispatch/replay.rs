//! `demo` (and its alias `play`), `record`, `stoprecord` and `clip`: playing
//! and keeping recordings of a match.

use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use ::replay::{
    CLIP_DEMO_FILE, CLIP_DUMP_FILE, CLIP_MANIFEST_FILE, CLIP_MS, ClipRing, MatchRecordIdentity,
    PendingReplayArm, Playback, Recording, ReplayError, ReplaySession,
};
use ::session::SessionSwapRequest;
use bevy::{ecs::system::SystemParam, prelude::*};
use frame::{LaunchIdentity, RuntimeRole};
use net::{
    AuthorityClock, AuthorityInputGate, AuthorityWorld, ClientPredictionState, PresentedSnapshot,
};
use render_frontend::prepare::scene::camera::SimCamera;

use super::{echo::ConsoleEcho, files::write_new_file, state_dump::state_dump_body};
use crate::{ConsoleCommand, ConsoleDispatch};

pub(super) const USAGE: &[(&str, &str)] = &[
    (
        "demo",
        "demo <name> — tear down the current occupancy, then play bo2mp-artifacts/demos/<name>.bo2mpdemo or clips/<name>/clip.bo2mpdemo",
    ),
    ("play", "play <name> — launcher alias of demo"),
    (
        "record",
        "record [name] — start a demo under bo2mp-artifacts",
    ),
    ("stoprecord", "stoprecord — finish the open demo"),
    (
        "clip",
        "clip — save last ≤45s available to this client as bo2mp-artifacts/clips/<ULID>/{clip.bo2mpdemo, dump.txt} (always-on ring)",
    ),
];

const NO_IDENTITY: &str = "launch identity missing (artifacts path unknown)";
const ID_TRIES: usize = 8;

/// A folder for a new clip under `clips`, named by an id from `next_id`. An
/// id that is already taken is replaced by a fresh one, a few times over.
fn allocate_clip_dir(
    artifacts: &Path,
    mut next_id: impl FnMut() -> Result<String, String>,
) -> Result<(String, PathBuf), String> {
    for _ in 0..ID_TRIES {
        let id = next_id()?;
        match ::replay::create_clip_dir(artifacts, &id) {
            Ok(dir) => return Ok((id, dir)),
            Err(ReplayError::Io(why)) if why.kind() == ErrorKind::AlreadyExists => {}
            Err(why) => return Err(format!("create clip folder: {why}")),
        }
    }
    Err("could not allocate a unique ULID directory".into())
}

/// The manifest of a clip: the replay crate's lines, then where the ticks
/// came from and who kept them.
fn manifest_text(clip: &ClipFacts<'_>) -> String {
    format!(
        "{}source = {:?}\nrole = {:?}\n",
        ::replay::clip_manifest(
            clip.id,
            clip.ticks,
            clip.duration_ms,
            clip.zone,
            clip.captured_unix_ns
        ),
        clip.source,
        clip.role
    )
}

struct ClipFacts<'a> {
    id: &'a str,
    ticks: u64,
    duration_ms: u32,
    zone: &'a str,
    captured_unix_ns: u128,
    source: &'a str,
    role: &'a str,
}

/// What a clip is made from. The authority is present only where this process
/// runs it; a client keeps what the server sent.
struct Evidence<'a> {
    identity: &'a LaunchIdentity,
    authority: Option<&'a AuthorityWorld>,
    clock: Option<&'a AuthorityClock>,
    prediction: Option<&'a sim::SimWorld>,
    presented: Option<&'a PresentedSnapshot>,
    ring: &'a ClipRing,
}

struct SavedClip {
    id: String,
    ticks: u64,
    demo: PathBuf,
    dump: PathBuf,
}

impl Evidence<'_> {
    /// Writes the three files of a clip into `dir`.
    fn write_into(&self, dir: &Path, id: &str) -> Result<SavedClip, String> {
        let captured = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|why| format!("system clock is before UNIX epoch: {why}"))?
            .as_nanos();
        let world = self
            .authority
            .map(|authority| &authority.0)
            .or(self.prediction)
            .ok_or("no world to take the match identity from")?;
        let record_identity = MatchRecordIdentity::from_world_on_zone(world, &self.identity.zone);

        let demo = dir.join(CLIP_DEMO_FILE);
        let mut recording = Recording::start_at_path(demo.clone(), record_identity)
            .map_err(|why| why.to_string())?;
        recording
            .record_clip_ring(self.ring)
            .map_err(|why| why.to_string())?;
        let (ticks, _) = recording.stop().map_err(|why| why.to_string())?;

        let dump = dir.join(CLIP_DUMP_FILE);
        let body = state_dump_body(
            self.identity,
            captured,
            self.clock,
            self.authority,
            self.presented,
            None,
        );
        write_new_file(&dump, &body)?;

        let manifest = manifest_text(&ClipFacts {
            id,
            ticks,
            duration_ms: self.ring.duration_ms(),
            zone: &self.identity.zone,
            captured_unix_ns: captured,
            source: if self.authority.is_some() {
                "authority"
            } else {
                "received"
            },
            role: &self.identity.role_label,
        });
        write_new_file(&dir.join(CLIP_MANIFEST_FILE), &manifest)?;

        ::replay::rewrite_latest_symlink(&self.identity.artifacts, id)
            .map_err(|why| why.to_string())?;
        Ok(SavedClip {
            id: id.to_owned(),
            ticks,
            demo,
            dump,
        })
    }

    /// Keeps the ring as a new clip. A clip that cannot be finished leaves no
    /// folder behind.
    fn save(&self, next_id: impl FnMut() -> Result<String, String>) -> Result<SavedClip, String> {
        let (id, dir) = allocate_clip_dir(&self.identity.artifacts, next_id)?;
        self.write_into(&dir, &id).inspect_err(|_| {
            let _ = fs::remove_dir_all(&dir);
        })
    }
}

/// What the replay commands read and change about recordings.
#[derive(SystemParam)]
pub(crate) struct Theater<'w> {
    identity: Option<Res<'w, LaunchIdentity>>,
    recorder: ResMut<'w, ReplaySession>,
    arm: ResMut<'w, PendingReplayArm>,
    swap: ResMut<'w, SessionSwapRequest>,
}

/// What the replay commands read about the running game.
#[derive(SystemParam)]
pub(crate) struct Live<'w> {
    authority: Option<Res<'w, AuthorityWorld>>,
    camera: Res<'w, SimCamera>,
    gate: Res<'w, AuthorityInputGate>,
    ring: Res<'w, ClipRing>,
    role: Res<'w, RuntimeRole>,
    prediction: Option<Res<'w, ClientPredictionState>>,
    clock: Option<Res<'w, AuthorityClock>>,
    presented: Option<Res<'w, PresentedSnapshot>>,
}

impl Theater<'_> {
    /// Plays a recording. `notes` gets what was said on the way; `Err` is
    /// what to say when playback cannot start.
    fn play_demo(&mut self, args: &[String], notes: &mut Vec<String>) -> Result<String, String> {
        let [name] = args else {
            return Err("usage: demo <name>".into());
        };
        let artifacts = self
            .identity
            .as_deref()
            .map(|identity| identity.artifacts.clone())
            .ok_or_else(|| format!("demo: {NO_IDENTITY}"))?;
        if let Some(open) = self.recorder.0.take() {
            notes.push(match open.stop() {
                Ok((ticks, path)) => format!(
                    "demo: stoprecord {ticks} ticks in {} before playback",
                    path.display()
                ),
                Err(why) => format!("demo: stoprecord failed: {why}"),
            });
        }
        let playback = Playback::open(&artifacts, name).map_err(|why| format!("demo: {why}"))?;
        let zone = playback
            .identity()
            .zone_name()
            .map(str::to_owned)
            .ok_or_else(|| {
                format!(
                    "demo: `{name}` has no zone in the header — re-record, or: make play {name} ZONE=<map>"
                )
            })?;
        self.arm.playback = Some(playback);
        self.arm.quit_on_end = false;
        match self.swap.request_demo(name.clone(), zone.clone(), false) {
            Ok(id) => Ok(format!("demo: `{name}` zone `{zone}` (swap #{id})")),
            Err(why) => {
                self.arm.playback = None;
                Err(format!("demo: {why}"))
            }
        }
    }

    fn start_recording(&mut self, args: &[String], live: &Live) -> Result<String, String> {
        if args.len() > 1 {
            return Err("usage: record [name]".into());
        }
        if let Some(open) = &self.recorder.0 {
            return Err(format!(
                "record: already recording `{}` ({} ticks) — stoprecord first",
                open.name(),
                open.ticks()
            ));
        }
        if !live.camera.enabled || !live.gate.local_cmds_enabled {
            return Err("record: the simulation does not own the camera yet \
                        (no clip brushes, or the authored intermission view is active); \
                        there are no ticks to record"
                .into());
        }
        let identity = self
            .identity
            .as_deref()
            .ok_or_else(|| format!("record: {NO_IDENTITY}"))?;
        let authority = live
            .authority
            .as_deref()
            .ok_or("record: authority world missing (match identity unavailable)")?;
        let match_identity = MatchRecordIdentity::from_world_on_zone(&authority.0, &identity.zone);
        let name = args.first().map_or("", String::as_str);
        let recording = Recording::start_with_identity(&identity.artifacts, name, match_identity)
            .map_err(|why| format!("record: {why}"))?;
        let said = format!("record: writing {}", recording.path().display());
        self.recorder.0 = Some(recording);
        Ok(said)
    }

    fn stop_recording(&mut self) -> Result<String, String> {
        let open = self.recorder.0.take().ok_or("stoprecord: not recording")?;
        open.stop()
            .map(|(ticks, path)| format!("stoprecord: {ticks} ticks in {}", path.display()))
            .map_err(|why| format!("stoprecord: {why}"))
    }

    /// Keeps the last stretch of play; the lines are what to say about it.
    fn keep_clip(&self, args: &[String], live: &Live) -> Result<Vec<String>, String> {
        if !args.is_empty() {
            return Err("usage: clip".into());
        }
        if live.ring.is_empty() {
            return Err("clip: ring empty (0 ticks)".into());
        }
        let identity = self
            .identity
            .as_deref()
            .ok_or_else(|| format!("clip: {NO_IDENTITY}"))?;
        let runs_authority = live.role.runs_authority();
        let evidence = Evidence {
            identity,
            authority: live.authority.as_deref().filter(|_| runs_authority),
            clock: live.clock.as_deref().filter(|_| runs_authority),
            prediction: live.prediction.as_deref().map(|state| state.0.world()),
            presented: live.presented.as_deref(),
            ring: &live.ring,
        };
        let saved = evidence
            .save(|| ::replay::new_ulid().map_err(|why| why.to_string()))
            .map_err(|why| format!("clip: {why}"))?;
        Ok(vec![
            format!(
                "clip: {}  {:.2}s ({} ticks) of {}s",
                saved.id,
                f64::from(live.ring.duration_ms()) / 1000.0,
                saved.ticks,
                CLIP_MS / 1000
            ),
            format!("clip: demo → {}", saved.demo.display()),
            format!("clip: dump → {}", saved.dump.display()),
        ])
    }
}

pub(crate) fn route_replay_commands(
    mut commands: MessageReader<ConsoleCommand>,
    mut echo: ConsoleEcho,
    mut theater: Theater,
    live: Live,
    mut dispatch: ResMut<ConsoleDispatch>,
) {
    for command in commands.read() {
        let args = command.args.as_slice();
        let said = match command.name.as_str() {
            "demo" | "play" => {
                let mut notes = Vec::new();
                let result = theater.play_demo(args, &mut notes);
                notes.into_iter().for_each(|note| echo.write(note));
                if result.is_err() {
                    dispatch.release();
                }
                result.map(|line| vec![line])
            }
            "record" => theater.start_recording(args, &live).map(|line| vec![line]),
            "stoprecord" => theater.stop_recording().map(|line| vec![line]),
            "clip" => theater.keep_clip(args, &live),
            _ => continue,
        };
        match said {
            Ok(lines) => lines.into_iter().for_each(|line| echo.write(line)),
            Err(line) => echo.write(line),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::{super::files::fresh_folder, *};
    use crate::{ConsoleLine, ConsoleSettings, ConsoleState};

    fn zoned(zone: &str) -> MatchRecordIdentity {
        let mut identity = MatchRecordIdentity::default();
        identity.set_zone(zone);
        identity
    }

    /// A finished recording called `name` in `folder`, as a past `record` leaves it.
    fn recorded(folder: &Path, name: &str, identity: MatchRecordIdentity) {
        Recording::start_with_identity(folder, name, identity)
            .unwrap()
            .stop()
            .unwrap();
    }

    fn run(line: &str, folder: Option<&Path>, setup: impl FnOnce(&mut App)) -> App {
        let mut app = App::new();
        app.add_message::<ConsoleCommand>()
            .init_resource::<ConsoleState>()
            .init_resource::<ConsoleSettings>()
            .init_resource::<ConsoleLine>()
            .init_resource::<ReplaySession>()
            .init_resource::<PendingReplayArm>()
            .init_resource::<SessionSwapRequest>()
            .init_resource::<SimCamera>()
            .init_resource::<AuthorityInputGate>()
            .init_resource::<ClipRing>()
            .insert_resource(RuntimeRole::Listen)
            .insert_resource(ConsoleDispatch::in_flight())
            .add_systems(Update, route_replay_commands);
        if let Some(folder) = folder {
            app.insert_resource(LaunchIdentity {
                artifacts: folder.to_path_buf(),
                zone: "mp_raid".into(),
                role_label: "host".into(),
                ..LaunchIdentity::default()
            });
        }
        setup(&mut app);
        for command in ConsoleCommand::parse_script(line) {
            app.world_mut().write_message(command);
        }
        app.update();
        app
    }

    fn log(app: &App) -> Vec<String> {
        app.world().resource::<ConsoleState>().log.clone()
    }

    fn queue_paused(app: &App) -> bool {
        app.world().resource::<ConsoleDispatch>().paused
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
        assert_eq!(
            log(&app),
            ["demo: launch identity missing (artifacts path unknown)"]
        );
        assert!(!queue_paused(&app));
    }

    #[test]
    fn demo_plays_a_recording_on_the_map_it_was_made_on() {
        let folder = fresh_folder("demo-plays");
        recorded(&folder, "show", zoned("mp_raid"));
        let app = run("demo show", Some(&folder), |_| {});
        assert_eq!(log(&app), ["demo: `show` zone `mp_raid` (swap #1)"]);
        assert!(queue_paused(&app), "the queue waits for the swap");
        let arm = app.world().resource::<PendingReplayArm>();
        assert!(arm.playback.is_some());
        assert!(!arm.quit_on_end);
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn demo_refuses_a_recording_that_does_not_name_its_map() {
        let folder = fresh_folder("demo-no-zone");
        recorded(&folder, "old", MatchRecordIdentity::default());
        let app = run("demo old", Some(&folder), |_| {});
        assert_eq!(
            log(&app),
            ["demo: `old` has no zone in the header — re-record, or: make play old ZONE=<map>"]
        );
        assert!(!queue_paused(&app));
        assert!(
            app.world()
                .resource::<PendingReplayArm>()
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
        assert_eq!(
            log(&app),
            [
                "demo: `show` zone `mp_raid` (swap #1)",
                "demo: session swap already in progress"
            ]
        );
        assert!(!queue_paused(&app));
        assert!(
            app.world()
                .resource::<PendingReplayArm>()
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
                Recording::start_with_identity(&folder, "take", MatchRecordIdentity::default())
                    .unwrap();
            app.world_mut().resource_mut::<ReplaySession>().0 = Some(open);
        });
        let said = log(&app);
        assert_eq!(said.len(), 2, "{said:?}");
        assert!(
            said[0].starts_with("demo: stoprecord 0 ticks in ")
                && said[0].ends_with(" before playback"),
            "{said:?}"
        );
        assert!(app.world().resource::<ReplaySession>().0.is_none());
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
            app.insert_resource(SimCamera {
                enabled: true,
                ..SimCamera::default()
            })
            .insert_resource(AuthorityInputGate {
                local_cmds_enabled: true,
            });
        };
        let app = run("record", None, owns_the_camera);
        assert_eq!(
            log(&app),
            ["record: launch identity missing (artifacts path unknown)"]
        );
        let app = run("record", Some(&folder), owns_the_camera);
        assert_eq!(
            log(&app),
            ["record: authority world missing (match identity unavailable)"]
        );
    }

    #[test]
    fn record_will_not_start_a_second_recording() {
        let folder = fresh_folder("record-twice");
        let app = run("record again", Some(&folder), |app| {
            let open =
                Recording::start_with_identity(&folder, "take", MatchRecordIdentity::default())
                    .unwrap();
            app.world_mut().resource_mut::<ReplaySession>().0 = Some(open);
        });
        assert_eq!(
            log(&app),
            ["record: already recording `take` (0 ticks) — stoprecord first"]
        );
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn stoprecord_finishes_the_open_recording() {
        let folder = fresh_folder("stoprecord");
        let app = run("stoprecord", Some(&folder), |_| {});
        assert_eq!(log(&app), ["stoprecord: not recording"]);

        let app = run("stoprecord", Some(&folder), |app| {
            let open =
                Recording::start_with_identity(&folder, "take", MatchRecordIdentity::default())
                    .unwrap();
            app.world_mut().resource_mut::<ReplaySession>().0 = Some(open);
        });
        let said = log(&app);
        assert!(said[0].starts_with("stoprecord: 0 ticks in "), "{said:?}");
        assert!(said[0].ends_with("take.bo2mpdemo"), "{said:?}");
        assert!(app.world().resource::<ReplaySession>().0.is_none());
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
        fs::create_dir_all(::replay::clip_dir(&folder, "TAKEN")).unwrap();
        let (id, dir) = allocate_clip_dir(&folder, next).unwrap();
        assert_eq!(id, "FRESH");
        assert_eq!(dir, ::replay::clip_dir(&folder, "FRESH"));
        assert!(dir.is_dir());
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn ids_that_are_always_taken_give_up_after_a_few_tries() {
        let folder = fresh_folder("clip-ids-taken");
        fs::create_dir_all(::replay::clip_dir(&folder, "SAME")).unwrap();
        let tries = Cell::new(0);
        let next = || {
            tries.set(tries.get() + 1);
            Ok("SAME".to_owned())
        };
        assert_eq!(
            allocate_clip_dir(&folder, next),
            Err("could not allocate a unique ULID directory".into())
        );
        assert_eq!(tries.get(), ID_TRIES);
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_failed_id_source_is_reported_as_it_is() {
        let folder = fresh_folder("clip-id-source");
        assert_eq!(
            allocate_clip_dir(&folder, || Err("no randomness".into())),
            Err("no randomness".into())
        );
    }

    #[test]
    fn the_manifest_says_where_the_ticks_came_from() {
        let text = manifest_text(&ClipFacts {
            id: "01ABC",
            ticks: 900,
            duration_ms: 44_950,
            zone: "mp_raid",
            captured_unix_ns: 12,
            source: "received",
            role: "client",
        });
        assert_eq!(
            text,
            "id = \"01ABC\"\n\
             ticks = 900\n\
             duration_ms = 44950\n\
             zone = \"mp_raid\"\n\
             captured_unix_ns = 12\n\
             demo = \"clip.bo2mpdemo\"\n\
             dump = \"dump.txt\"\n\
             source = \"received\"\n\
             role = \"client\"\n"
        );
    }

    #[test]
    fn a_clip_that_cannot_be_written_leaves_no_folder() {
        let folder = fresh_folder("clip-cleanup");
        let identity = LaunchIdentity {
            artifacts: folder.clone(),
            zone: "mp_raid".into(),
            ..LaunchIdentity::default()
        };
        let ring = ClipRing::default();
        let evidence = Evidence {
            identity: &identity,
            authority: None,
            clock: None,
            prediction: None,
            presented: None,
            ring: &ring,
        };
        let saved = evidence.save(|| Ok("GONE".to_owned()));
        assert_eq!(
            saved.err().as_deref(),
            Some("no world to take the match identity from")
        );
        assert_eq!(
            fs::read_dir(::replay::clips_dir(&folder)).unwrap().count(),
            0
        );
        let _ = fs::remove_dir_all(&folder);
    }
}
