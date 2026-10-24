//! gun_trace SCRIPT [FACTS_DUMP]: run a weapon_t6 input script and print the frame trace.
//! Without a dump it uses the demo table (round test numbers, not BO2's); `demo:dual` is the
//! dual-pistol table (window 8).
//! Exit code 1 when an `expect` line fails, 2 on a script or file error.

use std::process::ExitCode;

use weapon_t6::facts::FactsTable;
use weapon_t6::trace::{Runner, demo_facts};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(script_path) = args.first() else {
        eprintln!("usage: gun_trace SCRIPT [FACTS_DUMP]");
        return ExitCode::from(2);
    };
    let script = match std::fs::read_to_string(script_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{script_path}: {e}");
            return ExitCode::from(2);
        }
    };
    let facts = match args.get(1) {
        Some(p) if p == "demo:dual" => weapon_t6::pm::dual::demo_facts(),
        Some(p) => match std::fs::read_to_string(p) {
            Ok(text) => FactsTable::from_dump(&text),
            Err(e) => {
                eprintln!("{p}: {e}");
                return ExitCode::from(2);
            }
        },
        None => demo_facts(),
    };
    match Runner::new(&facts).run_script(&script) {
        Ok(r) => {
            for l in &r.lines {
                println!("{l}");
            }
            println!("{} expect failure(s)", r.failures);
            if r.failures == 0 {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(e) => {
            eprintln!("{script_path}: {e}");
            ExitCode::from(2)
        }
    }
}
