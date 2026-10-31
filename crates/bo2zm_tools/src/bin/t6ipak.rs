//! bo2zm: open Black Ops II image packs read-only. `t6ipak <pack.ipak>...`
//! lists each pack's sections and entries; `--decode` also decodes every
//! entry, parses it as an IWI v27 image and checks the image's own size
//! table. Nothing is written.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::fmt::{Debug, Display};
use std::path::Path;
use std::time::Instant;

use ipak_t6::{IpakHeader, decode_entry, parse_header, parse_index, parse_iwi};

fn read_at(file: &mut File, at: u64, len: usize) -> std::io::Result<Vec<u8>> {
    file.seek(SeekFrom::Start(at))?;
    let mut chunk = vec![0u8; len];
    file.read_exact(&mut chunk)?;
    Ok(chunk)
}

fn text_of(e: impl Display) -> String {
    e.to_string()
}

/// Split the command line into (decode?, packs); `None` without any pack.
fn parse_args(raw: Vec<String>) -> Option<(bool, Vec<String>)> {
    let (decode, packs) = match raw.split_first() {
        Some((flag, tail)) if flag == "--decode" => (true, tail.to_vec()),
        _ => (false, raw),
    };
    (!packs.is_empty()).then_some((decode, packs))
}

/// "Format", plus " cube" and " volume" for those kinds of image.
fn kind_label(format: &dyn Debug, cube: bool, volume: bool) -> String {
    let mut label = format!("{format:?}");
    for (on, word) in [(cube, " cube"), (volume, " volume")] {
        if on {
            label.push_str(word);
        }
    }
    label
}

/// What decoding a pack's entries added up to.
#[derive(Default)]
struct Tally {
    formats: BTreeMap<String, usize>,
    bad: usize,
    out_bytes: u64,
    /// (name hash, data hash, message) of the first entry that failed.
    first_error: Option<(u32, u32, String)>,
}

impl Tally {
    fn record(&mut self, hashes: (u32, u32), result: Result<String, String>, out_len: usize) {
        match result {
            Ok(kind) => {
                *self.formats.entry(kind).or_default() += 1;
                self.out_bytes += out_len as u64;
            }
            Err(e) => {
                self.bad += 1;
                self.first_error.get_or_insert((hashes.0, hashes.1, e));
            }
        }
    }
}

fn main() {
    let Some((decode, packs)) = parse_args(std::env::args().skip(1).collect()) else {
        eprintln!("usage: t6ipak [--decode] <pack.ipak>...");
        std::process::exit(2);
    };
    let failures = packs
        .iter()
        .filter(|path| match report(Path::new(path), decode) {
            Ok(()) => false,
            Err(e) => {
                println!("== {path}\n   FAILED: {e}");
                true
            }
        })
        .count();
    std::process::exit(i32::from(failures > 0));
}

fn report(path: &Path, decode: bool) -> Result<(), String> {
    let clock = Instant::now();
    let mut file = File::open(path).map_err(text_of)?;
    let file_len = file.metadata().map_err(text_of)?.len();
    let head = read_at(&mut file, 0, 256.min(file_len as usize)).map_err(text_of)?;
    let header: IpakHeader = parse_header(&head).map_err(text_of)?;
    let index_bytes = read_at(
        &mut file,
        header.index.starts_at as u64,
        header.index.item_count as usize * ipak_t6::INDEX_ENTRY_LEN,
    )
    .map_err(text_of)?;
    let index = parse_index(&index_bytes, header.index.item_count as usize).map_err(text_of)?;
    let stored: u64 = index.iter().map(|e| e.stored_len as u64).sum();
    println!(
        "== {}  ({} bytes; header says {})\n   {} entries, {} stored bytes; data section at {:#x} size {:#x}",
        path.to_string_lossy(),
        file_len,
        header.file_size,
        index.len(),
        stored,
        header.data.starts_at,
        header.data.byte_len
    );
    if !decode {
        return Ok(());
    }
    let mut tally = Tally::default();
    let mut plain = Vec::new();
    // File order: the index is sorted by key, which jumps around the disk.
    let mut by_offset: Vec<_> = index.iter().collect();
    by_offset.sort_by_key(|e| e.data_at);
    for entry in by_offset {
        let at = header.data.starts_at as u64 + entry.data_at as u64;
        let bytes = read_at(&mut file, at, entry.stored_len as usize).map_err(text_of)?;
        plain.clear();
        let result = decode_entry(&bytes, at, &mut plain)
            .map_err(text_of)
            .and_then(|()| {
                let img = parse_iwi(&plain).map_err(text_of)?;
                Ok(kind_label(&img.format, img.is_cube(), img.is_volume()))
            });
        // T6IPAK_FIND=<name hash>: that image's format and size.
        if let Ok(want) = std::env::var("T6IPAK_FIND")
            && u32::from_str_radix(want.trim_start_matches("0x"), 16).ok()
                == Some(entry.label_digest())
        {
            println!(
                "   found {:#010x}: {:?} ({} bytes)",
                entry.label_digest(),
                result,
                plain.len()
            );
            if let Ok(write_target) = std::env::var("T6IPAK_OUT") {
                let _ = std::fs::write(&write_target, &plain);
            }
            if let Ok(img) = parse_iwi(&plain) {
                println!("      {}x{} format {:?}", img.texel_width, img.height, img.format);
            }
        }
        tally.record((entry.label_digest(), entry.data_hash()), result, plain.len());
    }
    println!(
        "   decoded {} of {} entries to {} bytes of IWI images in {:.2?}",
        index.len() - tally.bad,
        index.len(),
        tally.out_bytes,
        clock.elapsed()
    );
    for (k, n) in &tally.formats {
        println!("      {n:>6}  {k}");
    }
    let bad = tally.bad;
    tally.first_error.map_or(Ok(()), |(name, data, e)| {
        println!("   first failure: name hash {name:#010x} data hash {data:#010x}: {e}");
        Err(format!("{bad} entries failed"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn args_split_the_decode_flag_from_the_packs() {
        assert_eq!(parse_args(args(&[])), None);
        assert_eq!(parse_args(args(&["--decode"])), None);
        assert_eq!(parse_args(args(&["a.ipak"])), Some((false, args(&["a.ipak"]))));
        assert_eq!(
            parse_args(args(&["--decode", "a.ipak", "b.ipak"])),
            Some((true, args(&["a.ipak", "b.ipak"])))
        );
    }

    #[test]
    fn kind_label_marks_cube_and_volume() {
        assert_eq!(kind_label(&"Dxt1", false, false), "\"Dxt1\"");
        assert_eq!(kind_label(&7u8, true, false), "7 cube");
        assert_eq!(kind_label(&7u8, true, true), "7 cube volume");
    }

    #[test]
    fn tally_keeps_the_first_failure_only() {
        let mut t = Tally::default();
        t.record((1, 2), Ok("A".into()), 10);
        t.record((3, 4), Err("bad".into()), 99);
        t.record((5, 6), Err("worse".into()), 99);
        t.record((7, 8), Ok("A".into()), 5);
        assert_eq!((t.bad, t.out_bytes, t.formats["A"]), (2, 15, 2));
        assert_eq!(t.first_error, Some((3, 4, "bad".to_owned())));
    }
}
