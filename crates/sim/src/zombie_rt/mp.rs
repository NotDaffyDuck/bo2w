//! bo2mp: Black Ops II multiplayer on the same runtime as Zombies.
//!
//! A multiplayer map (`mp_*`) runs its game type from `maps/mp/gametypes/`
//! (Zombies': `maps/mp/gametypes_zm/`). Its scripts call one script no PC
//! zone carries, `maps/mp/gametypes/_globallogic` (the match: start, clock,
//! limits, end, next round); the engine provides it as GSC source
//! ([`GLOBALLOGIC`], OUR CODE written from what the game's scripts call and
//! wait for), compiled with `gsc_t6::compile` and linked beside the zones'.

use bevy_ecs::prelude::World;
use gsc_t6::ScriptObject;

use super::Zm;

/// Our `maps/mp/gametypes/_globallogic`.
const GLOBALLOGIC: &str = include_str!("scripts/mp_globallogic.gsc");

/// A multiplayer map, by its zone name.
pub(super) fn is_mp(map: &str) -> bool {
    map.starts_with("mp_")
}

/// The scripts the engine provides for a multiplayer map, compiled.
/// BO2MP_DEBUG_GSC=<file>: a script of ours (`bo2mp/debug`) whose `main`
/// runs on the level once the match has started (a test aid: print what
/// the game's scripts hold, `println(...)` lands in the log).
/// Then the mods folder's scripts ([`mods`]).
pub(super) fn engine_scripts() -> Result<Vec<ScriptObject>, String> {
    let mut out = vec![gsc_t6::compile(
        "maps/mp/gametypes/_globallogic",
        GLOBALLOGIC,
    )?];
    if let Ok(path) = std::env::var("BO2MP_DEBUG_GSC") {
        let src = std::fs::read_to_string(&path).map_err(|e| format!("{path}: {e}"))?;
        out.push(gsc_t6::compile("bo2mp/debug", &src)?);
    }
    out.extend(mods());
    Ok(out)
}

/// Where a mod script's path starts: the scripts the engine runs itself.
pub(super) const MOD_SCRIPTS: &str = "scripts/mp/";

/// GSC mods, laid out as Plutonium's `storage/t6` is so a public mod drops
/// straight in: `mods/` beside the game (BO2MP_MODS=<dir> for another).
/// - `mods/scripts/mp/*.gsc`: each runs its `main` then its `init` on the
///   level once the match has started (and again after a round restart).
/// - `mods/maps/mp/...gsc`: replaces BO2's script of that path (the old
///   menus' `maps/mp/gametypes/_clientids.gsc`, whose `init` the match runs).
///
/// A file is GSC source or a PC-compiled script. One that fails to read or
/// compile is named in the log and left out; the match runs without it.
/// `name.gsc.off` (any other ending) is skipped.
fn mods() -> Vec<ScriptObject> {
    let root = match std::env::var_os("BO2MP_MODS") {
        Some(dir) => std::path::PathBuf::from(dir),
        None => match std::env::current_exe() {
            Ok(exe) => exe.with_file_name("mods"),
            Err(_) => return Vec::new(),
        },
    };
    let mut files = Vec::new();
    for sub in ["scripts/mp", "maps/mp"] {
        gsc_files(&root.join(sub), &mut files);
    }
    files.sort();
    let mut out = Vec::new();
    for path in files {
        let Ok(rel) = path.strip_prefix(&root) else { continue };
        let ident = script_ident(rel);
        let result = std::fs::read(&path)
            .map_err(|err| err.to_string())
            .and_then(|bytes| mod_object(&ident, &bytes));
        match result {
            Ok(object) => {
                diag::info!(Sim, "bo2mp mods: loaded {ident} ({} functions)", object.functions.len());
                out.push(object);
            }
            Err(e) => diag::warn!(Sim, "bo2mp mods: {ident} left out: {e}"),
        }
    }
    out
}

/// A mod script's name: its path under the mods folder without the extension,
/// its folders joined by `/`, in lower case.
fn script_ident(rel: &std::path::Path) -> String {
    let parts: Vec<String> = rel.with_extension("").iter().map(|p| p.to_string_lossy().to_ascii_lowercase()).collect();
    parts.join("/")
}

fn gsc_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            gsc_files(&p, out);
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("gsc")) {
            out.push(p);
        }
    }
}

/// One mod file: a PC-compiled script as it is, source compiled under its
/// path. A compiled script keeps the name it was compiled with.
fn mod_object(name: &str, bytes: &[u8]) -> Result<ScriptObject, String> {
    if bytes.first() == Some(&0x80) {
        return ScriptObject::parse(bytes);
    }
    let src = String::from_utf8_lossy(bytes);
    let src = src.strip_prefix('\u{feff}').unwrap_or(&src);
    gsc_t6::compile(name, src)
}

/// Where the game type's scripts live.
pub(super) fn gametypes(world: &World) -> &'static str {
    if world.get_resource::<Zm>().is_some_and(|z| z.mp) {
        "maps/mp/gametypes"
    } else {
        "maps/mp/gametypes_zm"
    }
}

/// The engine's callbacks script (`codecallback_*`).
pub(super) fn callbacks(world: &World) -> String {
    format!("{}/_callbacksetup", gametypes(world))
}

/// Where Black Ops II's engine takes `CodeCallback_DestructibleEvent` from
/// (gscr.c: `maps/mp/_destructible`, zombies `maps/mp/zombies/_zm`); it is
/// not in `_callbacksetup`.
pub(super) fn destructible_callbacks(world: &World) -> &'static str {
    if world.get_resource::<Zm>().is_some_and(|z| z.mp) {
        "maps/mp/_destructible"
    } else {
        "maps/mp/zombies/_zm"
    }
}

#[cfg(test)]
mod tests {
    use super::GLOBALLOGIC;

    /// Search and Destroy's bots get BO2's _bot_sd only behind the test dvar; normal games keep BO2's.
    #[test]
    fn sd_bots_are_wired_only_for_test_runs() {
        let gate = "sdtestbots = level.gametype == \"sd\" && getdvarint( \"bo2mp_test_sd_bots\" ) == 1;";
        assert!(GLOBALLOGIC.contains(gate));
        let wired = GLOBALLOGIC.lines().filter(|l| l.contains("_bot_sd::bot_sd_think")).count();
        assert_eq!(wired, 1);
        let before = GLOBALLOGIC.split("_bot_sd::bot_sd_think").next().unwrap_or_default();
        let guard: Vec<&str> = before.lines().rev().take(2).map(str::trim).collect();
        assert_eq!(guard, ["level.bot_gametype = maps/mp/bots/", "if ( sdtestbots )"]);
    }
}
