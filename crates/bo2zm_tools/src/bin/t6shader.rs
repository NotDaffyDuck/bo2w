//! bo2zm: dump a Black Ops II technique set's shaders, read-only.
//! `t6shader <technique set name prefix> [zone.ff] [out dir]` (default
//! Nuketown under `BO2MP_GAMES`, `shaders` here) lists the set's techniques, each pass's
//! shaders and register arguments, and writes every pass's pixel and vertex
//! shader bytecode (D3D11 DXBC) for a disassembler.

use std::path::PathBuf;

fn default_zone() -> PathBuf {
    bo2zm_tools::zone_file("zm_nuked.ff")
}
const DEFAULT_OUT: &str = "shaders";

/// A run that already said why on its own.
struct Failed;

/// "slot:name" for each filled technique slot, separated by spaces.
fn slot_labels<'a>(slots: impl IntoIterator<Item = Option<&'a str>>) -> String {
    slots
        .into_iter()
        .enumerate()
        .flat_map(|(slot, name)| name.map(|n| format!("{slot}:{n}")))
        .collect::<Vec<_>>()
        .join(" ")
}

/// File name for one pass's bytecode: slot, technique, pass and stage.
fn cso_name(slot: usize, technique: &str, pass: usize, stage: &str) -> String {
    format!("{slot:02}_{technique}_{pass}.{stage}.cso")
}

fn main() {
    std::process::exit(match drive() {
        Ok(()) => 0,
        Err(Failed) => 1,
    });
}

fn drive() -> Result<(), Failed> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let Some(prefix) = argv.first() else {
        eprintln!("usage: t6shader <technique set name prefix> [zone.ff] [out dir]");
        return Err(Failed);
    };
    let pick = |n: usize| argv.get(n).map(PathBuf::from);
    let (zone, out) = (pick(1).unwrap_or_else(default_zone), pick(2).unwrap_or_else(|| PathBuf::from(DEFAULT_OUT)));
    let capture = asset_t6::capture_zone_with_shaders(&zone).map_err(|e| {
        eprintln!("t6shader: {e}");
        Failed
    })?;
    // `--list`: every technique set in the zone, with its techniques.
    if prefix == "--list" {
        for set in &capture.technique_sets {
            let labels = slot_labels(set.techniques.iter().map(|t| t.as_ref().map(|t| t.name.as_str())));
            println!("{} | {labels}", set.name);
        }
        return Ok(());
    }
    let Some(set) = capture.technique_sets.iter().find(|t| t.name.starts_with(prefix.as_str())) else {
        eprintln!("t6shader: no technique set starts with {prefix}");
        return Err(Failed);
    };
    let dir = out.join(&set.name);
    std::fs::create_dir_all(&dir).map_err(|e| {
        eprintln!("t6shader: {}: {e}", dir.to_string_lossy());
        Failed
    })?;
    println!("{} (world vertex format {})", set.name, set.level_vertex_format);
    let filled = set
        .techniques
        .iter()
        .enumerate()
        .filter_map(|(i, t)| t.as_ref().map(|t| (i, t)));
    for (i, tech) in filled {
        println!("technique {i}: {} flags {:#x}", tech.name, tech.flags);
        for (p, pass) in tech.passes.iter().enumerate() {
            let (vs_len, ps_len) = (pass.vertex_program.len(), pass.pixel_program.len());
            println!(
                "   pass {p}: vs {} ({vs_len} bytes), ps {} ({ps_len} bytes)",
                pass.vertex_shader, pass.pixel_shader
            );
            pass.args.iter().for_each(|a| {
                println!(
                    "      arg type {} location {} size {} buffer {} u {:#010x}",
                    a.ty, a.location, a.size, a.buffer, a.u
                );
            });
            let stages = [("ps", &pass.pixel_program), ("vs", &pass.vertex_program)];
            for (stage, bytes) in stages.into_iter().filter(|(_, b)| !b.is_empty()) {
                let file = dir.join(cso_name(i, &tech.name, p, stage));
                if let Err(e) = std::fs::write(&file, bytes) {
                    eprintln!("t6shader: {}: {e}", file.display());
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_labels_skip_empty_slots_but_keep_numbering() {
        assert_eq!(slot_labels([]), "");
        assert_eq!(slot_labels([None, Some("lit"), None, Some("shadow")]), "1:lit 3:shadow");
    }

    #[test]
    fn cso_name_pads_the_slot_to_two_digits() {
        assert_eq!(cso_name(3, "lit", 0, "ps"), "03_lit_0.ps.cso");
        assert_eq!(cso_name(12, "shadow", 2, "vs"), "12_shadow_2.vs.cso");
    }
}
