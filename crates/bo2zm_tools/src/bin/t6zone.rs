//! bo2zm: open Black Ops II zones read-only and print what is in them.
//!
//! `t6zone <file.ff>...` reports each file; `t6zone --nuketown [BO2 folder]`
//! reports the nine zones Nuketown Zombies loads, from `<folder>/zone/all`.
//! Per zone: envelope, record count, inflated image vs the XFile header's
//! size, block sizes, script strings, dependencies, and the asset count by
//! type. Nothing is written anywhere.

use std::collections::BTreeMap;
use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::time::Instant;

use asset_transport::{T6ZoneMemory, ZoneTitleKind, mount_zone};
use fastfile_t6::{
    XAssetKind, MAX_ZONE_FILE_TOTAL, WalkSink, Walker, XChunks, XFILE_BLOCK_NAMES,
    ZoneByteReader,
};

/// The Nuketown Zombies load set, in load order.
const NUKETOWN_ZONES: [&str; 9] = [
    "code_pre_gfx_zm",
    "code_post_gfx_zm",
    "common_zm",
    "patch_zm",
    "ui_zm",
    "patch_ui_zm",
    "dlczm0_load_zm",
    "zm_nuked",
    "zm_nuked_patch",
];

/// Counts what a walk loaded and remembers where it was.
#[derive(Default)]
struct WalkCensus {
    current: Option<(usize, XAssetKind)>,
    loaded: BTreeMap<XAssetKind, usize>,
    /// (stream cursor, VIRTUAL watermark, index, type) at each asset start.
    starts: Vec<(usize, usize, usize, XAssetKind)>,
}

impl WalkCensus {
    fn asset_at(&self, cursor: usize) -> Option<&(usize, usize, usize, XAssetKind)> {
        self.starts.iter().rev().find(|s| s.0 <= cursor)
    }
}

impl WalkSink for WalkCensus {
    fn begin_asset(&mut self, s: &ZoneByteReader<'_>, asset_ix: usize, ty: XAssetKind) {
        self.current = Some((asset_ix, ty));
        self.starts.push((
            s.cursor(),
            s.write_pos(fastfile_t6::ARCHIVE_SEGMENT_HEAPED as u8),
            asset_ix,
            ty,
        ));
    }

    fn asset_loaded(
        &mut self,
        _s: &ZoneByteReader<'_>,
        loaded: fastfile_t6::Loaded,
    ) -> fastfile_t6::Result<()> {
        *self.loaded.entry(loaded.ty).or_default() += 1;
        Ok(())
    }
}

struct ZoneReport {
    name: String,
    counts: BTreeMap<XAssetKind, usize>,
    assets: usize,
}

fn text_of(e: impl Display) -> String {
    e.to_string()
}

/// What the command line asks for: walk the assets too, and which files.
/// `None` when nothing was asked for.
fn parse_command(args: &[String]) -> Option<(bool, Vec<PathBuf>)> {
    let (walk, rest) = match args {
        [flag, rest @ ..] if flag == "--walk" => (true, rest),
        _ => (false, args),
    };
    let files = match rest {
        [] => return None,
        [flag, root @ ..] if flag == "--nuketown" => {
            let dir = root.first().map_or_else(bo2zm_tools::game_dir, PathBuf::from).join("zone").join("all");
            NUKETOWN_ZONES.map(|zone| dir.join(format!("{zone}.ff"))).to_vec()
        }
        named => named.iter().map(PathBuf::from).collect(),
    };
    Some((walk, files))
}

/// The zone's own name, else the file's stem.
fn shown_label(own: Option<&str>, file: &Path) -> String {
    let stem = || file.file_stem().map(|s| s.to_string_lossy().into_owned());
    own.map(str::to_owned).or_else(stem).unwrap_or_default()
}

/// Reports one zone; a failure is printed here, where the zone's report would have been.
fn report_or_say_why(path: &Path, walk: bool) -> Result<ZoneReport, String> {
    let outcome = report(path, walk);
    if let Err(error) = &outcome {
        println!("== {}\n   FAILED: {error}\n", path.to_string_lossy());
    }
    outcome
}

fn main() {
    let code = run(&std::env::args().skip(1).collect::<Vec<_>>());
    std::process::exit(code);
}

/// Everything `main` does after the arguments are in hand; the exit code.
fn run(args: &[String]) -> i32 {
    let Some((walk, files)) = parse_command(args) else {
        eprintln!("usage: t6zone <zone.ff>... | t6zone --nuketown [BO2 folder]");
        return 2;
    };

    let outcomes: Vec<_> = files.iter().map(|path| report_or_say_why(path, walk)).collect();
    let failed = outcomes.iter().filter(|o| o.is_err()).count();
    let reports: Vec<ZoneReport> = outcomes.into_iter().flatten().collect();

    if reports.len() > 1 {
        print_totals(&reports);
    }
    let note = if failed == 0 { "" } else { " - SOME FAILED" };
    println!("opened {opened}/{total} zones{note}", opened = reports.len(), total = files.len());
    i32::from(failed > 0)
}

fn report(path: &Path, do_walk: bool) -> Result<ZoneReport, String> {
    let clock = Instant::now();
    let file = std::fs::read(path).map_err(|e| format!("reading: {e}"))?;
    let header = fastfile_t6::read_ff_preamble(&file).map_err(text_of)?;
    let records = XChunks::new(&file, header.body_offset);
    let mut record_count = 0usize;
    let mut walk = records.clone();
    for record in walk.by_ref() {
        record.map_err(text_of)?;
        record_count += 1;
    }
    let records_end = walk.position();

    let image = mount_zone(path).map_err(text_of)?;
    // T6ZONE_DUMP=dir: write the inflated image there as <file stem>.img.
    if let Ok(dir) = std::env::var("T6ZONE_DUMP") {
        let stem = path.file_stem().unwrap_or_default().to_string_lossy();
        let _ = std::fs::write(Path::new(&dir).join(format!("{stem}.img")), &image.bytes);
    }
    if image.game != ZoneTitleKind::T6 {
        return Err(format!("opened as {:?}, not T6", image.game));
    }
    let xfile = image.t6_header().map_err(text_of)?;
    let content_len = image.bytes.len() - fastfile_t6::ZONE_HEAD_BYTES;

    let mut memory = T6ZoneMemory::for_header(&xfile);
    let mut zs = memory.stream(&image.bytes).map_err(text_of)?;
    let table = fastfile_t6::read_asset_directory(&mut zs).map_err(text_of)?;
    let list_end = zs.cursor();

    let mut counts: BTreeMap<XAssetKind, usize> = BTreeMap::default();
    let mut unknown: BTreeMap<u32, usize> = BTreeMap::default();
    for i in 0..table.count() {
        let raw = table.raw_kind(&zs, i).map_err(text_of)?;
        match XAssetKind::from_u32(raw) {
            Some(ty) => *counts.entry(ty).or_default() += 1,
            None => *unknown.entry(raw).or_default() += 1,
        }
    }

    let name = shown_label(header.signed_name(), path);
    println!("== {name}  ({})", path.to_string_lossy());
    println!(
        "   file {} bytes, {:?}, {} records (records end at {:#x}, file pads {} bytes after)",
        file.len(),
        header.signing,
        record_count,
        records_end,
        file.len().saturating_sub(records_end)
    );
    println!(
        "   image {} bytes = 40-byte XFile header + {} content; XFile says size {} external {}  [{}]",
        image.bytes.len(),
        content_len,
        xfile.size,
        xfile.outside_extent,
        if content_len == xfile.size as usize {
            "content length matches"
        } else {
            "CONTENT LENGTH MISMATCH"
        }
    );
    let blocks: Vec<String> = (0..MAX_ZONE_FILE_TOTAL)
        .filter(|&i| xfile.block_size[i] != 0)
        .map(|i| format!("{}={}", XFILE_BLOCK_NAMES[i], xfile.block_size[i]))
        .collect();
    println!("   blocks: {}", blocks.join(" "));
    let depends: Vec<&str> = (0..table.depends.count())
        .filter_map(|i| table.depends.get(&zs, i))
        .collect();
    println!(
        "   script strings {}, dependencies {}{}",
        table.strings.count(),
        table.depends.count(),
        if depends.is_empty() {
            String::new()
        } else {
            format!(" ({})", depends.join(", "))
        }
    );
    let sample: Vec<&str> = (0..table.strings.count().min(8))
        .filter_map(|i| table.strings.get(&zs, i))
        .collect();
    if !sample.is_empty() {
        println!("   first script strings: {}", sample.join(" | "));
    }
    println!(
        "   asset list ends at image offset {list_end:#x}; {} bytes of asset bodies follow",
        image.bytes.len() - list_end
    );
    println!("   {count} assets:", count = table.count());
    for (ty, n) in &counts {
        println!("      {n:>6}  {name}", name = ty.name());
    }
    for (raw, n) in &unknown {
        println!("      {n:>6}  UNKNOWN pool id {raw:#x}");
    }
    if do_walk {
        let walk_started = Instant::now();
        let mut tallies = WalkCensus::default();
        let result = {
            let mut walker = Walker::new(&mut zs, &mut tallies);
            let r = walker.walk_assets(&table);
            let trail: Vec<&str> = walker.trail().collect();
            r.map_err(|e| (e, trail))
        };
        match result {
            Ok(()) => {
                let end = zs.cursor();
                let blocks: Vec<String> = (0..MAX_ZONE_FILE_TOTAL)
                    .filter(|&i| xfile.block_size[i] != 0 || zs.write_pos(i as u8) != 0)
                    .map(|i| {
                        format!(
                            "{} {}/{}",
                            XFILE_BLOCK_NAMES[i],
                            zs.write_pos(i as u8),
                            xfile.block_size[i]
                        )
                    })
                    .collect();
                let loaded: usize = tallies.loaded.values().sum();
                println!(
                    "   WALK: {} at image offset {end:#x} of {:#x}; {loaded} asset loads; unsettled offsets {}; {:.2?}",
                    if end == image.bytes.len() {
                        "reached the exact end"
                    } else {
                        "STOPPED SHORT"
                    },
                    image.bytes.len(),
                    zs.open_fixup_count(),
                    walk_started.elapsed()
                );
                println!("   blocks used/declared: {}", blocks.join(", "));
                if let Some((ptr, mark, cursor)) = zs.earliest_pending() {
                    let at = tallies.asset_at(cursor);
                    println!(
                        "   first forward offset: block {} offset {:#x} while block was at {:#x} (stream {:#x}) in asset {:?}",
                        ptr.seg,
                        ptr.offset,
                        mark,
                        cursor,
                        at.map(|a| (a.2, a.3.name(), a.1))
                    );
                }
                if end != image.bytes.len() {
                    return Err(format!(
                        "walk ended at {end:#x}, image is {:#x}",
                        image.bytes.len()
                    ));
                }
            }
            Err((error, trail)) => {
                let (index, ty) = tallies
                    .current
                    .unwrap_or((usize::MAX, XAssetKind::XModelPieces));
                println!(
                    "   WALK FAILED at asset {index} ({}) image offset {:#x}: {error}\n   inside: {}",
                    ty.name(),
                    zs.cursor(),
                    trail.join(" > ")
                );
                // T6ZONE_STARTS=n: where the last n assets started.
                if let Some(n) = std::env::var("T6ZONE_STARTS").ok().and_then(|v| v.parse().ok()) {
                    let from = tallies.starts.len().saturating_sub(n);
                    for (cursor, virt, i, t) in &tallies.starts[from..] {
                        println!("   start asset {i} {} at {cursor:#x} virtual {virt:#x}", t.name());
                    }
                }
                return Err(format!("walk failed at asset {index} ({name})", name = ty.name()));
            }
        }
    }
    println!("   read in {:.2?}\n", clock.elapsed());

    if !unknown.is_empty() {
        return Err(format!(
            "{} assets with unknown pool ids",
            unknown.values().sum::<usize>()
        ));
    }
    Ok(ZoneReport {
        name,
        assets: table.count(),
        counts,
    })
}

fn print_totals(reports: &[ZoneReport]) {
    let mut all: BTreeMap<XAssetKind, usize> = BTreeMap::default();
    for r in reports {
        for (ty, n) in &r.counts {
            *all.entry(*ty).or_default() += n;
        }
    }
    println!("== totals by type across {} zones", reports.len());
    let width = reports.iter().map(|r| r.name.len()).max().unwrap_or(4);
    for (ty, total) in &all {
        let per: Vec<String> = reports
            .iter()
            .filter_map(|r| r.counts.get(ty).map(|n| format!("{}={n}", r.name)))
            .collect();
        println!("   {:>6}  {:<18} {}", total, ty.name(), per.join(" "));
    }
    let assets: usize = reports.iter().map(|r| r.assets).sum();
    println!("   {assets:>6}  assets total");
    for r in reports {
        println!("   {:<width$} {:>6} assets", r.name, r.assets);
    }
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn command_line_forms() {
        assert_eq!(parse_command(&words(&[])), None);
        assert_eq!(parse_command(&words(&["--walk"])), None);
        let (walk, files) = parse_command(&words(&["a.ff", "b.ff"])).unwrap();
        assert!(!walk);
        assert_eq!(files, vec![PathBuf::from("a.ff"), PathBuf::from("b.ff")]);
        let (walk, files) = parse_command(&words(&["--walk", "--nuketown", "root"])).unwrap();
        assert!(walk);
        assert_eq!(files.len(), NUKETOWN_ZONES.len());
        assert_eq!(files[0], Path::new("root").join("zone").join("all").join("code_pre_gfx_zm.ff"));
    }

    #[test]
    fn name_falls_back_to_the_file_stem() {
        assert_eq!(shown_label(Some("zm_nuked"), Path::new("x/y.ff")), "zm_nuked");
        assert_eq!(shown_label(None, Path::new("x/y.ff")), "y");
        assert_eq!(shown_label(None, Path::new("")), "");
    }
}
