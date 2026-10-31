//! bo2mp: where a Black Ops II sound alias's clip lives.
//!
//! `t6snd <filter> <sound folder> <zone.ff>...` prints every alias whose name
//! holds `filter` in the zones' sound banks: the bank, the alias, each
//! variant's asset id and file name, and which `.sab*` file in the sound
//! folder carries that id (every file there is opened). Nothing is written.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Aliases listed in full per bank; later matches are only counted.
const SHOWN_PER_BANK: usize = 12;

/// A run that already said why on its own.
struct Failed;

/// Which bank files carry each asset id (comma-separated, in folder order), and how many files the folder held.
struct SabIndex {
    files_seen: usize,
    banks_by_id: HashMap<u32, String>,
}

impl SabIndex {
    /// The bank files holding `id`, comma-separated, or "-" when none does.
    fn banks_for(&self, id: u32) -> String {
        match self.banks_by_id.get(&id) {
            Some(banks) => banks.clone(),
            None => "-".to_owned(),
        }
    }
}

/// Every entry of the folder, in name order.
fn sorted_entries(folder: &str) -> Vec<PathBuf> {
    let Ok(listing) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = listing.flatten().map(|entry| entry.path()).collect();
    found.sort();
    found
}

/// Whether a file name belongs to a sound bank (`.sabs`, `.sabl`, ...).
fn is_bank_name(file_name: &str) -> bool {
    file_name.contains(".sab")
}

/// Notes that bank file `name` carries each of `ids`.
fn record_ids(banks_by_id: &mut HashMap<u32, String>, name: &str, sound_ids: impl IntoIterator<Item = u32>) {
    for id in sound_ids {
        let banks = banks_by_id.entry(id).or_default();
        if !banks.is_empty() {
            banks.push(',');
        }
        banks.push_str(name);
    }
}

fn index_banks(folder: &str) -> SabIndex {
    let entries = sorted_entries(folder);
    let mut banks_by_id = HashMap::new();
    let bank_files = entries.iter().filter_map(|path| {
        let name = path.file_name()?.to_str()?;
        is_bank_name(name).then(|| (name.to_owned(), path))
    });
    for (name, path) in bank_files {
        let ids = match sab_t6::BankFile::open(path) {
            Ok(bank) => bank.entries.iter().map(|entry| entry.id).collect::<Vec<u32>>(),
            Err(e) => {
                eprintln!("{name}: {e}");
                continue;
            }
        };
        record_ids(&mut banks_by_id, &name, ids);
    }
    SabIndex { files_seen: entries.len(), banks_by_id }
}

/// How many of the matches get printed in full, given how many came before.
fn is_listed(match_number: usize) -> bool {
    match_number <= SHOWN_PER_BANK
}

fn print_bank(zone: &str, bank: &asset_t6::m2::SndBankRef, needle: &str, index: &SabIndex) {
    let mut matched = 0usize;
    for list in bank.aliases.iter().filter(|l| l.name.to_ascii_lowercase().contains(needle)) {
        matched += 1;
        if !is_listed(matched) {
            continue;
        }
        for alias in &list.entries {
            println!(
                "{zone} | {} | {} | id {:08x} file '{}' | in {} | secondary '{}' vol {}-{} flags {:08x} {:08x} fade {}-{} ms",
                bank.name,
                list.name,
                alias.asset_id,
                alias.asset_file,
                index.banks_for(alias.asset_id),
                alias.secondary,
                alias.vol_min,
                alias.vol_max,
                alias.flags[0],
                alias.flags[1],
                alias.ease_on,
                alias.ease_off
            );
        }
    }
    match matched {
        0 => {}
        total => println!("{zone} | {} | {total} aliases match", bank.name),
    }
}

fn usage() -> Failed {
    eprintln!("usage: t6snd <filter> <sound folder> <zone.ff>...");
    Failed
}

fn main() {
    std::process::exit(match run(std::env::args().skip(1).collect()) {
        Ok(()) => 0,
        Err(Failed) => 1,
    });
}

fn run(args: Vec<String>) -> Result<(), Failed> {
    let [filter, folder, first_zone, more_zones @ ..] = args.as_slice() else {
        return Err(usage());
    };
    let zones = std::iter::once(first_zone).chain(more_zones);
    let needle = filter.to_ascii_lowercase();
    let index = index_banks(folder);
    println!("{} sab files, {} ids", index.files_seen, index.banks_by_id.len());
    for zone in zones {
        match asset_t6::capture_zone(Path::new(zone)) {
            Ok(captured) => {
                for bank in &captured.sound_banks {
                    print_bank(&captured.zone, bank, &needle, &index);
                }
            }
            Err(e) => eprintln!("{zone}: {e}"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_sab_files_count_as_banks() {
        assert!(is_bank_name("en_mp.all.sabs"));
        assert!(is_bank_name("common.sabl"));
        assert!(!is_bank_name("readme.txt"));
    }

    #[test]
    fn listing_stops_after_the_twelfth_match() {
        assert!(is_listed(1));
        assert!(is_listed(SHOWN_PER_BANK));
        assert!(!is_listed(SHOWN_PER_BANK + 1));
    }

    #[test]
    fn banks_for_joins_or_dashes() {
        let mut banks_by_id = HashMap::new();
        banks_by_id.insert(7, "a.sabs,b.sabl".to_owned());
        let index = SabIndex { files_seen: 2, banks_by_id };
        assert_eq!(index.banks_for(7), "a.sabs,b.sabl");
        assert_eq!(index.banks_for(8), "-");
    }

    #[test]
    fn record_ids_appends_the_bank_to_each_id() {
        let mut index = SabIndex { files_seen: 2, banks_by_id: HashMap::new() };
        record_ids(&mut index.banks_by_id, "a.sabs", [1, 2]);
        record_ids(&mut index.banks_by_id, "b.sabs", [2]);
        assert_eq!(index.banks_for(1), "a.sabs");
        assert_eq!(index.banks_for(2), "a.sabs,b.sabs");
    }

    #[test]
    fn a_missing_folder_lists_nothing() {
        assert!(sorted_entries("no/such/folder/for/t6snd").is_empty());
    }
}
