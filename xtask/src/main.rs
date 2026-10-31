//! `cargo xtask <command>`. Everything except `bundle-zip` reads `.env` at the
//! workspace root for hosts, keys and paths.

use std::path::Path;

use xtask::dot_vars::DotVars;
use xtask::{certs, host_setup, lobby_host, publish, release, windows, zip_pack};

const HELP_BANNER: &str = "usage: cargo xtask <command>
  windows [build|setup]
  certs <host>
  release <prod|dev|bundles>
  publish <prod|dev>
  provision
  logs <prod|dev> [--since 2h]
  master <install|update|status|logs|uninstall> user@host [--channel prod] [--since 10min]
  bundle-zip ARCHIVE FILE...";

/// Commands that need `.env` before they can do anything, and who runs them.
const WITH_ENV: [(&str, fn(&Path, &DotVars, &[String]) -> Result<(), String>); 7] = [
    ("windows", |_, dot_conf, cli_words| windows::dispatch_subcommand(dot_conf, cli_words)),
    ("certs", |_, dot_conf, cli_words| certs::dispatch_subcommand(dot_conf, cli_words)),
    ("release", release::dispatch_subcommand),
    ("publish", publish::dispatch_subcommand),
    ("provision", host_setup::dispatch_subcommand),
    ("logs", |_, dot_conf, cli_words| publish::tail_logs(dot_conf, cli_words)),
    ("master", lobby_host::dispatch_subcommand),
];

fn main() -> std::process::ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let ending = match argv.split_first() {
        None => {
            println!("{HELP_BANNER}");
            Ok(())
        }
        Some((command, cli_words)) => {
            dispatch(command, cli_words).map_err(|why| format!("{command}: {why}"))
        }
    };
    match ending {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn dispatch(command: &str, cli_words: &[String]) -> Result<(), String> {
    let route = WITH_ENV.iter().find(|(title, _)| *title == command);
    match (command, route) {
        ("help" | "-h" | "--help", _) => {
            println!("{HELP_BANNER}");
            Ok(())
        }
        ("bundle-zip", _) => zip_pack::launch(cli_words.to_vec()),
        (_, Some((_, handler))) => {
            let repo_root = xtask::workspace_top_dir();
            handler(&repo_root, &DotVars::load(&repo_root)?, cli_words)
        }
        (_, None) => Err(format!("unknown command\n{HELP_BANNER}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_with_an_env_is_in_the_usage_text() {
        for (title, _) in WITH_ENV {
            assert!(HELP_BANNER.contains(&format!("\n  {title} ")) || HELP_BANNER.contains(&format!("\n  {title}\n")), "{title}");
        }
        assert!(HELP_BANNER.contains("\n  bundle-zip ARCHIVE FILE..."));
    }

    #[test]
    fn unknown_commands_get_the_usage() {
        let error = dispatch("nonsense", &[]).unwrap_err();
        assert!(error.starts_with("unknown command\nusage: cargo xtask <command>"), "{error}");
    }

    #[test]
    fn help_and_bundle_zip_need_no_env() {
        for word in ["help", "-h", "--help"] {
            assert!(dispatch(word, &[]).is_ok());
        }
        let error = dispatch("bundle-zip", &[]).unwrap_err();
        assert_eq!(error, "usage: cargo xtask bundle-zip ARCHIVE FILE...");
    }
}
