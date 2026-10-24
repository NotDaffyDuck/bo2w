//! bo2mp: where a Black Ops II sound alias's clip lives.
//!
//! `t6snd <filter> <sound folder> <zone.ff>...` prints every alias whose name
//! holds `filter` in the zones' sound banks: the bank, the alias, each
//! variant's asset id and file name, and which `.sab*` file in the sound
//! folder carries that id (every file there is opened). Nothing is written.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 3 {
        eprintln!("usage: t6snd <filter> <sound folder> <zone.ff>...");
        return ExitCode::FAILURE;
    }
    let filter = args[0].to_ascii_lowercase();
    let mut by_id: HashMap<u32, Vec<String>> = HashMap::new();
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&args[1])
        .map(|r| r.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    paths.sort();
    for path in &paths {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_owned();
        if !name.contains(".sab") {
            continue;
        }
        match sab_t6::BankFile::open(path) {
            Ok(file) => {
                for e in &file.entries {
                    by_id.entry(e.id).or_default().push(name.clone());
                }
            }
            Err(e) => eprintln!("{name}: {e}"),
        }
    }
    println!("{} sab files, {} ids", paths.len(), by_id.len());
    for zone in &args[2..] {
        let cap = match asset_t6::capture_zone(std::path::Path::new(zone)) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("{zone}: {e}");
                continue;
            }
        };
        for bank in &cap.sound_banks {
            let mut shown = 0usize;
            for list in &bank.aliases {
                if !list.name.to_ascii_lowercase().contains(&filter) {
                    continue;
                }
                shown += 1;
                if shown > 12 {
                    continue;
                }
                for a in &list.entries {
                    let files = by_id.get(&a.asset_id).map(|v| v.join(",")).unwrap_or_else(|| "-".to_owned());
                    println!(
                        "{} | {} | {} | id {:08x} file '{}' | in {} | secondary '{}' vol {}-{} flags {:08x} {:08x} fade {}-{} ms",
                        cap.zone, bank.name, list.name, a.asset_id, a.asset_file, files,
                        a.secondary, a.vol_min, a.vol_max, a.flags[0], a.flags[1], a.fade_in, a.fade_out
                    );
                }
            }
            if shown > 0 {
                println!("{} | {} | {} aliases match", cap.zone, bank.name, shown);
            }
        }
    }
    ExitCode::SUCCESS
}
