//! `cargo xtask <command>`. Everything except `bundle-zip` reads `.env` at the
//! workspace root for hosts, keys and paths.

use std::process::ExitCode;

use xtask::dotenv::Env;
use xtask::shell::Res;
use xtask::{bundle_zip, certs, master, provision, publish, release, repo_root, windows};

const USAGE: &str = "usage: cargo xtask <command>
  windows [build|setup]
  certs <host>
  release <prod|dev|bundles>
  publish <prod|dev>
  provision
  logs <prod|dev> [--since 2h]
  master <install|update|status|logs|uninstall> user@host [--channel prod] [--since 10min]
  bundle-zip ARCHIVE FILE...";

/// Commands that need `.env` before they can do anything.
const NEEDS_ENV: [&str; 7] = [
    "windows",
    "certs",
    "release",
    "publish",
    "provision",
    "logs",
    "master",
];

fn main() -> ExitCode {
    let mut argv = std::env::args().skip(1);
    let Some(command) = argv.next() else {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    };
    let args: Vec<String> = argv.collect();
    match dispatch(&command, &args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(why) => {
            eprintln!("{command}: {why}");
            ExitCode::FAILURE
        }
    }
}

fn dispatch(command: &str, args: &[String]) -> Res<()> {
    if matches!(command, "help" | "-h" | "--help") {
        println!("{USAGE}");
        return Ok(());
    }
    if command == "bundle-zip" {
        return bundle_zip::run(args.to_vec());
    }
    if !NEEDS_ENV.contains(&command) {
        return Err(format!("unknown command\n{USAGE}"));
    }
    let root = repo_root();
    let env = Env::load(&root)?;
    match command {
        "windows" => windows::run_cli(&env, args),
        "certs" => certs::run_cli(&env, args),
        "release" => release::run_cli(&root, &env, args),
        "publish" => publish::run_cli(&root, &env, args),
        "provision" => provision::run_cli(&root, &env, args),
        "logs" => publish::logs(&env, args),
        _ => master::run_cli(&root, &env, args),
    }
}
