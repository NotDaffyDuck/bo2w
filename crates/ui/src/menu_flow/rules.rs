//! Reading the match setup the front end has chosen, and the match rules
//! that go with it.

use frame::{HostMatchRules, UiMenuDvars};
use sim::HostChosenMode;

use crate::ScreenArenaRoster;

/// Config file the default rules are read from.
pub const MATCH_CONFIG: &str = "default_xboxlive.cfg";

/// The map and mode picked in the menu, or the reason there is no valid pick.
pub fn selected_game(
    dvars: &UiMenuDvars,
    maps: &ScreenArenaRoster,
) -> Result<(String, HostChosenMode), String> {
    let map = dvars.get("ui_mapname").ok_or("No map selected")?;
    if !maps.contains(map) {
        return Err(format!("Map `{map}` is not installed"));
    }
    let mode = dvars
        .get("ui_gametype")
        .and_then(HostChosenMode::from_short_code)
        .ok_or("Unsupported game mode")?;
    Ok((map.to_owned(), mode))
}

/// A setting that changes how the match plays, as opposed to menu state.
fn is_rule(name: &str) -> bool {
    name == "g_hardcore" || name.starts_with("scr_")
}

/// Fills in each rule found as a `set <name> <value>` line of `config`,
/// unless the menu already holds a value for it. Comments (`//`) are cut off.
pub fn seed_rules(dvars: &mut UiMenuDvars, config: &str) {
    for line in config.lines() {
        let code = line.split("//").next().unwrap_or_default().trim();
        let Some(setting) = code.strip_prefix("set ") else {
            continue;
        };
        let mut words = setting.split_whitespace();
        let Some(name) = words.next() else {
            continue;
        };
        let name = name.to_ascii_lowercase();
        if is_rule(&name) && dvars.get(&name).is_none() {
            let value = words.next().unwrap_or_default().trim_matches('"');
            dvars.set(&name, value);
        }
    }
}

/// Every rule the menu holds, in the form the host takes.
pub fn host_rules(dvars: &UiMenuDvars) -> HostMatchRules {
    let rules = dvars
        .iter()
        .filter(|(dvar, _)| is_rule(dvar))
        .map(|(dvar, setting)| (dvar.to_owned(), setting.to_owned()))
        .collect();
    HostMatchRules(rules)
}
