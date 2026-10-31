//! bo2zm: read a Black Ops II map's compiled scripts, read-only.
//! `t6gsc [zone.ff]...` (default the Nuketown patch zone) checks the
//! decoder on every script (string references and import call sites read
//! where the object's tables say) and prints what the map scripts say
//! about effects and ambience. `T6GSC_RAW_OUT=<dir>` also writes every raw
//! file there.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use asset_t6::{GscObject, MapScriptFacts, capture_zone};

fn default_zone() -> PathBuf {
    bo2zm_tools::zone_file("zm_nuked_patch.ff")
}

/// A run that already said why on its own.
struct Failed;

/// How many scripts fell in each category, and what the decoder's check summed to.
#[derive(Default)]
struct Totals {
    strings: (usize, usize),
    imports: (usize, usize),
    parse_failures: usize,
}

impl Totals {
    /// Adds one script's (strings read, strings, imports read, imports);
    /// true when the decoder read everything the object's tables list.
    fn merge(&mut self, (s_hit, s_all, i_hit, i_all): (usize, usize, usize, usize)) -> bool {
        self.strings = (self.strings.0 + s_hit, self.strings.1 + s_all);
        self.imports = (self.imports.0 + i_hit, self.imports.1 + i_all);
        s_hit == s_all && i_hit == i_all
    }
}

/// How often each label occurs.
fn tally<'a>(labels: impl Iterator<Item = &'a str>) -> BTreeMap<&'a str, usize> {
    labels.fold(BTreeMap::new(), |mut counts, label| {
        *counts.entry(label).or_insert(0) += 1;
        counts
    })
}

/// The zones named on the command line, or the default one.
fn zones_or_default(named: impl Iterator<Item = String>) -> Vec<PathBuf> {
    let zones: Vec<PathBuf> = named.map(PathBuf::from).collect();
    if let [] = zones.as_slice() {
        return vec![default_zone()];
    }
    zones
}

/// One decoded instruction as a line of text.
fn insn_text(insn: &gsc_t6::Insn) -> String {
    use gsc_t6::{Insn, op_name};
    match insn {
        Insn::Plain(op) => op_name(*op).to_owned(),
        Insn::Int(op, n) => format!("{} {n}", op_name(*op)),
        Insn::Str(op, t) => format!("{} \"{t}\"", op_name(*op)),
        Insn::Call(op, i) => format!("{} {}({})", op_name(*op), i.full(), i.params),
        Insn::Jump(op, to) => format!("{} -> {to:#x}", op_name(*op)),
        other => format!("{other:?}"),
    }
}

/// `T6GSC_DIS=<compiled .gsc>` [`T6GSC_FN=<function>`]: print the file's
/// functions as decoded instructions (reading BO2's own scripts).
fn disassemble(path: &str) -> Result<(), Failed> {
    let bytes = std::fs::read(path).map_err(|_| {
        eprintln!("cannot read {path}");
        Failed
    })?;
    let obj = gsc_t6::ScriptObject::parse(&bytes).map_err(|e| {
        eprintln!("{path}: {e}");
        Failed
    })?;
    let want = std::env::var("T6GSC_FN").ok();
    for f in &obj.functions {
        if want.as_deref().is_some_and(|w| !f.name.eq_ignore_ascii_case(w)) {
            continue;
        }
        println!("function {}({} params)", f.name, f.params);
        for (at, insn) in &f.code {
            println!("  {at:#06x} {}", insn_text(insn));
        }
    }
    Ok(())
}

fn main() {
    std::process::exit(match drive() {
        Ok(()) => 0,
        Err(Failed) => 1,
    });
}

fn drive() -> Result<(), Failed> {
    if let Ok(path) = std::env::var("T6GSC_DIS") {
        return disassemble(&path);
    }
    let zones = zones_or_default(std::env::args().skip(1));
    let mut scripts: HashMap<String, Vec<u8>> = HashMap::new();
    for zone in &zones {
        match capture_zone(zone) {
            Ok(c) => {
                // T6GSC_RAW_OUT=<dir>: write every raw file (UI scripts,
                // tables, configs) there, `/` as `__` (a later zone wins).
                if let Ok(dir) = std::env::var("T6GSC_RAW_OUT") {
                    for (name, bytes) in &c.raw_files {
                        let flat = name.trim_start_matches("script:").replace(['/', '\\'], "__");
                        let _ = std::fs::write(std::path::Path::new(&dir).join(flat), bytes);
                    }
                    for t in &c.string_tables {
                        let text: String = t
                            .cells
                            .chunks(t.columns.max(1))
                            .map(|row| row.join(",") + "
")
                            .collect();
                        let _ = std::fs::write(std::path::Path::new(&dir).join(t.name.replace(['/', '\\'], "__")), text);
                    }
                    println!("{}: {} raw files, {} string tables written", zone.display(), c.raw_files.len(), c.string_tables.len());
                }
                for (name, bytes) in c.raw_files {
                    if let Some(name) = name.strip_prefix("script:") {
                        scripts.entry(name.to_owned()).or_insert(bytes);
                    }
                }
            }
            Err(e) => {
                eprintln!("{}: {e}", zone.display());
                return Err(Failed);
            }
        }
    }
    let mut totals = Totals::default();
    let sorted: BTreeMap<_, _> = scripts.iter().collect();
    if std::env::var_os("T6GSC_NAMES").is_some() {
        for name in sorted.keys() {
            println!("name {name}");
        }
    }
    for (name, bytes) in &sorted {
        if name.starts_with(',') {
            continue;
        }
        match GscObject::parse(bytes) {
            Ok(g) => {
                let seen = g.coverage();
                if !totals.merge(seen) {
                    let (a, b, c, d) = seen;
                    println!("MISMATCH {name}: strings {a}/{b} imports {c}/{d}");
                }
            }
            Err(e) => {
                totals.parse_failures += 1;
                println!("PARSE FAIL {name}: {e}");
            }
        }
    }
    println!(
        "scripts {}: strings {}/{}, imports {}/{}, parse failures {}",
        sorted.len(),
        totals.strings.0,
        totals.strings.1,
        totals.imports.0,
        totals.imports.1,
        totals.parse_failures
    );
    let map = std::env::var("T6GSC_MAP").unwrap_or_else(|_| "zm_nuked".to_owned());
    let facts = MapScriptFacts::read(&map, |n| scripts.get(n).cloned());
    // T6GSC_DIS=<script name>: that script's opcodes (opcode, operand).
    if let Ok(want) = std::env::var("T6GSC_DIS")
        && let Some(g) = scripts.get(&want).and_then(|b| GscObject::parse(b).ok())
    {
        for i in 0..g.exports.len() {
            println!("dis fn {}", g.exports[i].name);
            for o in g.decode(i) {
                println!("dis {:#x} {:#04x} {:?}", o.at, o.op, o.arg);
            }
        }
    }
    println!("\n== {map}: {} effect table entries", facts.effects.len());
    let kinds = tally(facts.placed.iter().map(|p| p.kind.as_str()));
    println!("placed {}: {kinds:?}", facts.placed.len());
    let unresolved: Vec<&str> = facts
        .placed
        .iter()
        .filter(|p| !facts.effects.contains_key(&p.fx_key))
        .map(|p| p.fx_key.as_str())
        .collect();
    println!("placed fxids without an effect table entry: {}", unresolved.len());
    for u in unresolved.iter().take(10) {
        println!("   {u}");
    }
    for p in facts.placed.iter().take(6) {
        println!(
            "   {} {} -> {} at {:?} angles {:?} delay {} exploder {:?}",
            p.kind,
            p.fx_key,
            facts.effects.get(&p.fx_key).map_or("?", String::as_str),
            p.origin,
            p.angles,
            p.delay,
            p.exploder
        );
    }
    if std::env::var_os("T6GSC_PLACED").is_some() {
        for p in &facts.placed {
            println!(
                "placed	{}	{}	{}	{:.0}	{:.0}	{:.0}	{:.0}	{:.0}	{:.0}	{}",
                p.kind,
                p.fx_key,
                facts.effects.get(&p.fx_key).map_or("?", String::as_str),
                p.origin[0],
                p.origin[1],
                p.origin[2],
                p.angles[0],
                p.angles[1],
                p.angles[2],
                p.delay
            );
        }
    }
    println!("room tone {:?}", facts.room_tone);
    println!("sounds on effects {}:", facts.sounds_on_fx.len());
    for (fxid, alias, off) in &facts.sounds_on_fx {
        let n = facts.placed.iter().filter(|p| &p.fx_key == fxid).count();
        println!("   {fxid} -> {alias} offset {off:?} ({n} placed)");
    }
    println!("loops at points {}: {:?}", facts.loops_at.len(), facts.loops_at);
    if std::env::var_os("T6GSC_CALLS").is_some() {
        // `T6GSC_SCRIPT=<name>`: that script's calls instead.
        let chosen = std::env::var("T6GSC_SCRIPT")
            .unwrap_or_else(|_| "clientscripts/mp/zm_nuked_amb.csc".to_owned());
        for name in [chosen.as_str()] {
            if let Some(g) = scripts.get(name).and_then(|b| GscObject::parse(b).ok()) {
                for c in g.run().calls {
                    println!("call [{}] {}({:?})", c.caller, c.function, c.args);
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
    fn totals_add_up_and_flag_unread_references() {
        let mut t = Totals::default();
        assert!(t.merge((3, 3, 2, 2)));
        assert!(!t.merge((1, 2, 5, 5)));
        assert!(!t.merge((4, 4, 0, 1)));
        assert_eq!((t.strings, t.imports), ((8, 9), (7, 8)));
    }

    #[test]
    fn tally_counts_each_label() {
        let counts = tally(["fx", "snd", "fx"].into_iter());
        assert_eq!(counts.get("fx"), Some(&2));
        assert_eq!(counts.get("snd"), Some(&1));
        assert!(tally(std::iter::empty()).is_empty());
    }

    #[test]
    fn no_zone_named_means_the_default() {
        assert_eq!(zones_or_default(std::iter::empty()), vec![default_zone()]);
        assert_eq!(zones_or_default(["a.ff".to_owned()].into_iter()), vec![PathBuf::from("a.ff")]);
    }

    #[test]
    fn instructions_print_name_then_operand() {
        use gsc_t6::{Insn, op_name};
        assert_eq!(insn_text(&Insn::Plain(0)), op_name(0));
        assert_eq!(insn_text(&Insn::Int(1, -4)), format!("{} -4", op_name(1)));
        assert_eq!(insn_text(&Insn::Jump(2, 0x40)), format!("{} -> 0x40", op_name(2)));
        assert_eq!(insn_text(&Insn::Str(3, "hi".into())), format!("{} \"hi\"", op_name(3)));
    }
}
