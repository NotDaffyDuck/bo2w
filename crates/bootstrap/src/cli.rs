//! The command line: how to start (`map`, `serve`, `menu`, `play`) and, for a
//! render acceptance run, where its artifacts go.
//!
//! `--cmds <script>` belongs to the console, which takes it out first.

use std::{ffi::OsString, path::PathBuf};

use render::checks::acceptance::{RENDER_CHECK_ENV, SMOKE_CHECK_CLI_SWITCH, TRIAL_MAPS};

/// A render acceptance run: the folder it writes its artifacts to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignoffStartup { pub dir: PathBuf }

impl SignoffStartup {
    /// Sign-off output goes under `folder`.
    fn into_folder(folder: impl Into<PathBuf>) -> Self {
        Self { dir: folder.into() }
    }
}

/// Which way the process starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartupFlavor {
    /// Black Ops II's multiplayer menus, with no map until a match starts.
    /// `menu` and `frontend t6mp` both ask for it.
    FrontendT6Mp,
    /// Black Ops II's menus over a loaded zone (`frontend t6:zm_nuked`): the
    /// zone's match is hosted from them.
    Frontend(String),
    /// Load a zone and host a match on it from the window.
    Hosted(String),
    /// Load a zone and host a match on it with no window.
    Serve(String),
    /// Play a recorded demo back, on the zone it was recorded on unless told
    /// otherwise.
    Play {
        name: String,
        forced_pack: Option<String>,
    },
}

const HELP_BANNER: &str = "usage: bo2mp [--cmds '<script>'] map <zone> | serve <zone> | menu | frontend t6mp | frontend <zone> | play <demo>\n       bo2mp --help";

/// The launch mode and acceptance run that `args` (without the program name)
/// ask for. The acceptance folder falls back to the environment.
pub fn read_flags(words: impl Iterator<Item = String>) -> Result<(StartupFlavor, Option<SignoffStartup>), String> {
    parse_with_env(words, std::env::var_os(RENDER_CHECK_ENV))
}

fn parse_with_env(words: impl Iterator<Item = String>, from_env: Option<OsString>) -> Result<(StartupFlavor, Option<SignoffStartup>), String> {
    let (left, flagged) = take_acceptance_flag(words)?;
    // The command line beats the environment.
    let signoff = flagged.or_else(|| from_env.map(SignoffStartup::into_folder));
    let without_cmds = console::strip_cmds_flag(left.into_iter());
    Ok((parse_launch_args(without_cmds.into_iter())?, signoff))
}

/// Takes `--render-acceptance <dir>` or `--render-acceptance=<dir>` out of
/// `args`; the last one given wins.
fn take_acceptance_flag(mut words: impl Iterator<Item = String>) -> Result<(Vec<String>, Option<SignoffStartup>), String> {
    let switch = SMOKE_CHECK_CLI_SWITCH;
    let env_name = RENDER_CHECK_ENV;
    let inline_prefix = format!("{switch}=");
    let (mut left, mut found) = (Vec::new(), None);
    while let Some(word) = words.next() {
        let folder = if word == switch {
            match words.next().filter(|next| is_value(next)) {
                Some(next) => next,
                None => {
                    return Err(format!(
                        "usage: bo2mp {switch} <artifact-dir> map <zone>\n\
                         env alternative: {env_name}=<artifact-dir>\n\
                         expected maps later: {}",
                        TRIAL_MAPS.join(", ")
                    ));
                }
            }
        } else if let Some(inline) = word.strip_prefix(&inline_prefix) {
            if inline.is_empty() {
                return Err(format!("empty {switch} path"));
            }
            inline.to_owned()
        } else {
            left.push(word);
            continue;
        };
        // A later flag replaces an earlier one.
        found = Some(SignoffStartup::into_folder(folder));
    }
    Ok((left, found))
}

/// A flag's value: something, and not the next flag.
fn is_value(word: &str) -> bool {
    !(word.is_empty() || word.starts_with('-'))
}

/// The launch mode `args` ask for, once the flags every mode shares are gone.
/// No arguments (a double-click on the exe) opens the multiplayer front end,
/// as `frontend t6mp` does.
pub fn parse_launch_args(mut args: impl Iterator<Item = String>) -> Result<StartupFlavor, String> {
    let Some(first) = args.next() else {
        return Ok(StartupFlavor::FrontendT6Mp);
    };
    match first.as_str() {
        "--help" | "-h" => Err(HELP_BANNER.into()),
        "map" => zone_argument(&mut args, "map").map(StartupFlavor::Hosted),
        "serve" => zone_argument(&mut args, "serve").map(StartupFlavor::Serve),
        "menu" if args.next().is_none() => Ok(StartupFlavor::FrontendT6Mp),
        "menu" => Err("usage: bo2mp menu [--cmds '<script>']".into()),
        "play" => parse_play_args(args),
        "frontend" => match (args.next().as_deref(), args.next()) {
            (Some("t6mp"), None) => Ok(StartupFlavor::FrontendT6Mp),
            (Some(zone), None) if !zone.is_empty() => Ok(StartupFlavor::Frontend(zone.to_owned())),
            _ => Err("usage: bo2mp frontend t6mp | frontend <zone> [--cmds '<script>']".into()),
        },
        unknown => Err(format!(
            "unknown launch args starting with `{unknown}` — expected: map <zone> | serve <zone> | menu | frontend t6mp | frontend <zone> | play <demo> [--cmds '<script>']"
        )),
    }
}

/// The zone name after `map` or `serve`.
fn zone_argument(args: &mut impl Iterator<Item = String>, verb: &str) -> Result<String, String> {
    args.next()
        .filter(|zone| !zone.is_empty())
        .ok_or_else(|| format!("usage: bo2mp {verb} <zone> [--cmds '<script>']"))
}

/// `play <demo> [--zone <zone> | --zone=<zone>]`. The demo may be given as a
/// path or with its extension; only its name is kept.
fn parse_play_args(mut words: impl Iterator<Item = String>) -> Result<StartupFlavor, String> {
    const PLAY_USAGE: &str = "usage: bo2mp play <demo> [--zone <zone>] [--cmds '<script>']";
    let (mut demo, mut forced_pack): (Option<String>, Option<String>) = (None, None);
    while let Some(word) = words.next() {
        if word == "--zone" {
            let Some(pack) = words.next().filter(|next| is_value(next)) else {
                return Err(PLAY_USAGE.into());
            };
            forced_pack = Some(pack);
        } else if let Some(pack) = word.strip_prefix("--zone=") {
            match pack {
                "" => return Err("empty --zone".into()),
                _ => forced_pack = Some(pack.to_owned()),
            }
        } else if word.starts_with('-') {
            return Err(format!("unknown play flag `{word}` — expected --zone"));
        } else if demo.is_none() {
            demo = Some(replay::recording_stem(&word).to_owned());
        } else {
            return Err(format!("{PLAY_USAGE} (extra `{word}`)"));
        }
    }
    let name = demo.filter(|n| !n.is_empty()).ok_or_else(|| String::from(PLAY_USAGE))?;
    Ok(StartupFlavor::Play { name, forced_pack })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(line: &str) -> impl Iterator<Item = String> + '_ {
        line.split_whitespace().map(str::to_owned)
    }

    fn mode(line: &str) -> Result<StartupFlavor, String> {
        parse_launch_args(words(line))
    }

    fn cli(line: &str, env: Option<&str>) -> Result<(StartupFlavor, Option<SignoffStartup>), String> {
        parse_with_env(words(line), env.map(OsString::from))
    }

    fn acceptance(dir: &str) -> Option<SignoffStartup> {
        Some(SignoffStartup::into_folder(dir))
    }

    #[test]
    fn map_and_serve_take_a_zone() {
        assert_eq!(
            mode("map mp_nuketown_2020"),
            Ok(StartupFlavor::Hosted("mp_nuketown_2020".into()))
        );
        assert_eq!(
            mode("serve mp_raid"),
            Ok(StartupFlavor::Serve("mp_raid".into()))
        );
    }

    #[test]
    fn map_and_serve_without_a_zone_say_how_to_ask() {
        assert!(mode("map").unwrap_err().contains("map <zone>"));
        assert!(mode("serve").unwrap_err().contains("serve <zone>"));
        let blank = parse_launch_args(["map".to_owned(), String::new()].into_iter());
        assert!(blank.unwrap_err().contains("map <zone>"));
    }

    #[test]
    fn menu_and_frontend_t6mp_open_the_front_end() {
        assert_eq!(mode("menu"), Ok(StartupFlavor::FrontendT6Mp));
        assert_eq!(mode("frontend t6mp"), Ok(StartupFlavor::FrontendT6Mp));
    }

    #[test]
    fn frontend_with_a_zone_opens_the_menus_over_that_zone() {
        assert_eq!(
            mode("frontend t6:zm_nuked"),
            Ok(StartupFlavor::Frontend("t6:zm_nuked".to_owned()))
        );
    }

    #[test]
    fn menu_and_frontend_refuse_what_they_do_not_know() {
        assert!(mode("menu now").unwrap_err().contains("menu"));
        assert!(mode("frontend").unwrap_err().contains("frontend t6mp"));
        assert!(mode("frontend a b").unwrap_err().contains("frontend t6mp"));
        assert!(
            mode("frontend t6mp extra")
                .unwrap_err()
                .contains("frontend t6mp")
        );
    }

    #[test]
    fn no_arguments_open_the_multiplayer_front_end() {
        assert_eq!(mode(""), Ok(StartupFlavor::FrontendT6Mp));
    }

    #[test]
    fn help_prints_the_usage() {
        for line in ["--help", "-h"] {
            assert_eq!(mode(line), Err(HELP_BANNER.to_owned()), "{line:?}");
        }
    }

    #[test]
    fn an_unknown_word_is_named() {
        let error = mode("fly away").unwrap_err();
        assert!(error.contains("`fly`"), "{error}");
        assert!(error.contains("map <zone>"), "{error}");
    }

    #[test]
    fn play_keeps_only_the_demo_name() {
        let play = |name: &str| StartupFlavor::Play {
            name: name.into(),
            forced_pack: None,
        };
        assert_eq!(mode("play round1"), Ok(play("round1")));
        assert_eq!(mode("play round1.bo2mpdemo"), Ok(play("round1")));
        assert_eq!(mode("play demos/round1.bo2mpdemo"), Ok(play("round1")));
        assert_eq!(mode(r"play demos\round1"), Ok(play("round1")));
    }

    #[test]
    fn play_reads_the_zone_either_way_round() {
        let want = Ok(StartupFlavor::Play {
            name: "round1".into(),
            forced_pack: Some("mp_raid".into()),
        });
        assert_eq!(mode("play round1 --zone mp_raid"), want);
        assert_eq!(mode("play --zone mp_raid round1"), want);
        assert_eq!(mode("play round1 --zone=mp_raid"), want);
    }

    #[test]
    fn play_refuses_a_bad_line() {
        let typed = [
            "play",
            "play --zone mp_raid",
            "play round1 --zone",
            "play round1 --zone --loud",
            "play round1 --zone=",
            "play round1 round2",
            "play round1 --speed",
        ];
        for line in typed {
            assert!(mode(line).is_err(), "{line:?}");
        }
        assert!(mode("play a b").unwrap_err().contains("extra `b`"));
        assert!(mode("play a --speed").unwrap_err().contains("--speed"));
    }

    #[test]
    fn cmds_flags_are_the_consoles_and_do_not_reach_the_mode() {
        let (found, flagged) = cli("--cmds one;two map mp_raid --cmds=three", None).unwrap();
        assert_eq!(found, StartupFlavor::Hosted("mp_raid".into()));
        assert_eq!(flagged, None);
    }

    #[test]
    fn the_acceptance_flag_takes_its_folder_either_way() {
        let typed = [
            "--render-acceptance out map mp_raid",
            "map mp_raid --render-acceptance out",
            "--render-acceptance=out map mp_raid",
        ];
        for line in typed {
            let (found, flagged) = cli(line, None).unwrap();
            assert_eq!(found, StartupFlavor::Hosted("mp_raid".into()), "{line:?}");
            assert_eq!(flagged, acceptance("out"), "{line:?}");
        }
    }

    #[test]
    fn the_acceptance_flag_beats_the_environment_which_otherwise_fills_in() {
        let (_, flagged) = cli("--render-acceptance flag map mp_raid", Some("env")).unwrap();
        assert_eq!(flagged, acceptance("flag"));
        let (_, from_env) = cli("map mp_raid", Some("env")).unwrap();
        assert_eq!(from_env, acceptance("env"));
        let (_, none) = cli("map mp_raid", None).unwrap();
        assert_eq!(none, None);
    }

    #[test]
    fn the_acceptance_flag_needs_a_folder() {
        let typed = [
            "map mp_raid --render-acceptance",
            "--render-acceptance --cmds x map mp_raid",
        ];
        for line in typed {
            let error = cli(line, None).unwrap_err();
            assert!(error.contains(RENDER_CHECK_ENV), "{line:?}: {error}");
        }
        let error = cli("--render-acceptance= map mp_raid", None).unwrap_err();
        assert!(error.starts_with("empty"), "{error}");
    }
}
