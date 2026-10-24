//! The command line: how to start (`map`, `serve`, `menu`, `play`) and, for a
//! render acceptance run, where its artifacts go.
//!
//! `--cmds <script>` belongs to the console, which takes it out first.

use std::{ffi::OsString, path::PathBuf};

use render::diag::acceptance::{ACCEPTANCE_ENV, ACCEPTANCE_FLAG, ACCEPTANCE_MAPS};

/// A render acceptance run: the folder it writes its artifacts to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcceptanceLaunch {
    pub dir: PathBuf,
}

/// Which way the process starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaunchMode {
    /// Black Ops II's multiplayer menus, with no map until a match starts.
    /// `menu` and `frontend t6mp` both ask for it.
    FrontendT6Mp,
    /// Load a zone and host a match on it from the window.
    Map(String),
    /// Load a zone and host a match on it with no window.
    Serve(String),
    /// Play a recorded demo back, on the zone it was recorded on unless told
    /// otherwise.
    Play {
        name: String,
        zone_override: Option<String>,
    },
}

const USAGE: &str = "usage: bo2mp [--cmds '<script>'] map <zone> | serve <zone> | menu | frontend t6mp | play <demo>\n       bo2mp --help";

/// The launch mode and acceptance run that `args` (without the program name)
/// ask for. The acceptance folder falls back to the environment.
pub fn parse_cli(
    args: impl Iterator<Item = String>,
) -> Result<(LaunchMode, Option<AcceptanceLaunch>), String> {
    parse_with_env(args, std::env::var_os(ACCEPTANCE_ENV))
}

fn parse_with_env(
    args: impl Iterator<Item = String>,
    env_dir: Option<OsString>,
) -> Result<(LaunchMode, Option<AcceptanceLaunch>), String> {
    let (rest, flagged) = take_acceptance_flag(args)?;
    let acceptance = flagged.or_else(|| {
        env_dir.map(|dir| AcceptanceLaunch {
            dir: PathBuf::from(dir),
        })
    });
    let mode = parse_launch_args(console::strip_cmds_flag(rest.into_iter()).into_iter())?;
    Ok((mode, acceptance))
}

/// Takes `--render-acceptance <dir>` or `--render-acceptance=<dir>` out of
/// `args`; the last one given wins.
fn take_acceptance_flag(
    args: impl Iterator<Item = String>,
) -> Result<(Vec<String>, Option<AcceptanceLaunch>), String> {
    let inline_prefix = format!("{ACCEPTANCE_FLAG}=");
    let mut rest = Vec::new();
    let mut found = None;
    let mut args = args;
    while let Some(arg) = args.next() {
        let dir = if arg == ACCEPTANCE_FLAG {
            match args.next() {
                Some(dir) if !dir.is_empty() && !dir.starts_with('-') => dir,
                _ => {
                    return Err(format!(
                        "usage: bo2mp {ACCEPTANCE_FLAG} <artifact-dir> map <zone>\n\
                         env alternative: {ACCEPTANCE_ENV}=<artifact-dir>\n\
                         expected maps later: {}",
                        ACCEPTANCE_MAPS.join(", ")
                    ));
                }
            }
        } else if let Some(dir) = arg.strip_prefix(&inline_prefix) {
            if dir.is_empty() {
                return Err(format!("empty {ACCEPTANCE_FLAG} path"));
            }
            dir.to_owned()
        } else {
            rest.push(arg);
            continue;
        };
        found = Some(AcceptanceLaunch {
            dir: PathBuf::from(dir),
        });
    }
    Ok((rest, found))
}

/// The launch mode `args` ask for, once the flags every mode shares are gone.
pub fn parse_launch_args(mut args: impl Iterator<Item = String>) -> Result<LaunchMode, String> {
    let Some(first) = args.next() else {
        return Err(USAGE.into());
    };
    match first.as_str() {
        "--help" | "-h" => Err(USAGE.into()),
        "map" => zone_argument(&mut args, "map").map(LaunchMode::Map),
        "serve" => zone_argument(&mut args, "serve").map(LaunchMode::Serve),
        "menu" if args.next().is_none() => Ok(LaunchMode::FrontendT6Mp),
        "menu" => Err("usage: bo2mp menu [--cmds '<script>']".into()),
        "play" => parse_play_args(args),
        "frontend" => match (args.next().as_deref(), args.next()) {
            (Some("t6mp"), None) => Ok(LaunchMode::FrontendT6Mp),
            _ => Err("usage: bo2mp frontend t6mp [--cmds '<script>']".into()),
        },
        other => Err(format!(
            "unknown launch args starting with `{other}` — expected: map <zone> | serve <zone> | menu | frontend t6mp | play <demo> [--cmds '<script>']"
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
fn parse_play_args(args: impl Iterator<Item = String>) -> Result<LaunchMode, String> {
    const PLAY_USAGE: &str = "usage: bo2mp play <demo> [--zone <zone>] [--cmds '<script>']";
    let mut name = None;
    let mut zone_override = None;
    let mut args = args;
    while let Some(arg) = args.next() {
        if arg == "--zone" {
            match args.next() {
                Some(zone) if !zone.is_empty() && !zone.starts_with('-') => {
                    zone_override = Some(zone);
                }
                _ => return Err(PLAY_USAGE.into()),
            }
        } else if let Some(zone) = arg.strip_prefix("--zone=") {
            if zone.is_empty() {
                return Err("empty --zone".into());
            }
            zone_override = Some(zone.to_owned());
        } else if arg.starts_with('-') {
            return Err(format!("unknown play flag `{arg}` — expected --zone"));
        } else if name.is_some() {
            return Err(format!("{PLAY_USAGE} (extra `{arg}`)"));
        } else {
            name = Some(replay::demo_stem(&arg).to_owned());
        }
    }
    match name {
        Some(name) if !name.is_empty() => Ok(LaunchMode::Play {
            name,
            zone_override,
        }),
        _ => Err(PLAY_USAGE.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(line: &str) -> impl Iterator<Item = String> + '_ {
        line.split_whitespace().map(str::to_owned)
    }

    fn mode(line: &str) -> Result<LaunchMode, String> {
        parse_launch_args(words(line))
    }

    fn cli(
        line: &str,
        env: Option<&str>,
    ) -> Result<(LaunchMode, Option<AcceptanceLaunch>), String> {
        parse_with_env(words(line), env.map(OsString::from))
    }

    fn acceptance(dir: &str) -> Option<AcceptanceLaunch> {
        Some(AcceptanceLaunch {
            dir: PathBuf::from(dir),
        })
    }

    #[test]
    fn map_and_serve_take_a_zone() {
        assert_eq!(
            mode("map mp_nuketown_2020"),
            Ok(LaunchMode::Map("mp_nuketown_2020".into()))
        );
        assert_eq!(
            mode("serve mp_raid"),
            Ok(LaunchMode::Serve("mp_raid".into()))
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
        assert_eq!(mode("menu"), Ok(LaunchMode::FrontendT6Mp));
        assert_eq!(mode("frontend t6mp"), Ok(LaunchMode::FrontendT6Mp));
    }

    #[test]
    fn menu_and_frontend_refuse_what_they_do_not_know() {
        assert!(mode("menu now").unwrap_err().contains("menu"));
        assert!(mode("frontend").unwrap_err().contains("frontend t6mp"));
        assert!(mode("frontend iw4").unwrap_err().contains("frontend t6mp"));
        assert!(
            mode("frontend t6mp extra")
                .unwrap_err()
                .contains("frontend t6mp")
        );
    }

    #[test]
    fn no_arguments_and_help_print_the_usage() {
        for line in ["", "--help", "-h"] {
            assert_eq!(mode(line), Err(USAGE.to_owned()), "{line:?}");
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
        let play = |name: &str| LaunchMode::Play {
            name: name.into(),
            zone_override: None,
        };
        assert_eq!(mode("play round1"), Ok(play("round1")));
        assert_eq!(mode("play round1.bo2mpdemo"), Ok(play("round1")));
        assert_eq!(mode("play demos/round1.bo2mpdemo"), Ok(play("round1")));
        assert_eq!(mode(r"play demos\round1"), Ok(play("round1")));
    }

    #[test]
    fn play_reads_the_zone_either_way_round() {
        let want = Ok(LaunchMode::Play {
            name: "round1".into(),
            zone_override: Some("mp_raid".into()),
        });
        assert_eq!(mode("play round1 --zone mp_raid"), want);
        assert_eq!(mode("play --zone mp_raid round1"), want);
        assert_eq!(mode("play round1 --zone=mp_raid"), want);
    }

    #[test]
    fn play_refuses_a_bad_line() {
        for line in [
            "play",
            "play --zone mp_raid",
            "play round1 --zone",
            "play round1 --zone --loud",
            "play round1 --zone=",
            "play round1 round2",
            "play round1 --speed",
        ] {
            assert!(mode(line).is_err(), "{line:?}");
        }
        assert!(mode("play a b").unwrap_err().contains("extra `b`"));
        assert!(mode("play a --speed").unwrap_err().contains("--speed"));
    }

    #[test]
    fn cmds_flags_are_the_consoles_and_do_not_reach_the_mode() {
        let (found, flagged) = cli("--cmds one;two map mp_raid --cmds=three", None).unwrap();
        assert_eq!(found, LaunchMode::Map("mp_raid".into()));
        assert_eq!(flagged, None);
    }

    #[test]
    fn the_acceptance_flag_takes_its_folder_either_way() {
        for line in [
            "--render-acceptance out map mp_raid",
            "map mp_raid --render-acceptance out",
            "--render-acceptance=out map mp_raid",
        ] {
            let (found, flagged) = cli(line, None).unwrap();
            assert_eq!(found, LaunchMode::Map("mp_raid".into()), "{line:?}");
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
        for line in [
            "map mp_raid --render-acceptance",
            "--render-acceptance --cmds x map mp_raid",
        ] {
            let error = cli(line, None).unwrap_err();
            assert!(error.contains(ACCEPTANCE_ENV), "{line:?}: {error}");
        }
        let error = cli("--render-acceptance= map mp_raid", None).unwrap_err();
        assert!(error.starts_with("empty"), "{error}");
    }
}
