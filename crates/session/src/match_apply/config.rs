//! What a match is told before its scripts run: the dvars they start with,
//! the game type's settings, and which game type it is.

use std::collections::BTreeMap;

use gamemode_t6::GameModeKind;

/// The config file the game's own scripts expect to have been run.
pub(super) const MATCH_CONFIG: &str = "default_xboxlive.cfg";

/// The dvars a match always starts with, whatever the config says.
const MAX_CLIENTS: &str = "18";

/// The `set` and `seta` lines of a config, in order. A `//` starts a comment;
/// a line that sets a name and no value sets it to the empty string.
pub(super) fn config_sets(text: &str) -> Vec<(String, String)> {
    let mut sets = Vec::new();
    for line in text.lines() {
        let line = line.split("//").next().unwrap_or("").trim();
        let Some(rest) = line
            .strip_prefix("set ")
            .or_else(|| line.strip_prefix("seta "))
        else {
            continue;
        };
        let rest = rest.trim_start();
        let (name, value) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
        if !name.is_empty() {
            sets.push((name.to_owned(), value.trim().trim_matches('"').to_owned()));
        }
    }
    sets
}

/// Dvars a test run adds from the environment: `name value` lines, with `;`
/// standing in for a line break so they fit in one variable.
pub(super) fn dvar_overrides(text: &str) -> Vec<(String, String)> {
    config_sets(&text.replace(';', "\n"))
}

/// The dvars the game scripts start with: the config's, then the map and game
/// type, then the host's rules over the top (a rule replaces a dvar of the
/// same name, whatever its case, and otherwise adds one).
pub(super) fn script_dvars(
    config: Option<&str>,
    zone: &str,
    gametype: &str,
    rules: &[(String, String)],
) -> Vec<(String, String)> {
    let mut dvars = config.map_or_else(Vec::new, config_sets);
    dvars.push(("mapname".to_owned(), zone.to_owned()));
    dvars.push(("g_gametype".to_owned(), gametype.to_owned()));
    dvars.push(("sv_maxclients".to_owned(), MAX_CLIENTS.to_owned()));
    for (name, value) in rules {
        match dvars
            .iter_mut()
            .find(|(existing, _)| existing.eq_ignore_ascii_case(name))
        {
            Some((_, existing)) => existing.clone_from(value),
            None => dvars.push((name.clone(), value.clone())),
        }
    }
    dvars
}

/// The `gametype_setting <name> <value>` lines of the zones' settings files:
/// first of the default file, then of the game type's own, whose values win.
/// Names are lower case.
pub(super) fn gamesettings(files: &[(String, String)], gametype: &str) -> BTreeMap<String, String> {
    let mut settings = BTreeMap::new();
    let wanted = [
        "gamesettings_default.cfg".to_owned(),
        format!("gamesettings_{gametype}.cfg"),
    ];
    for suffix in &wanted {
        let matching = files
            .iter()
            .filter(|(name, _)| name.to_ascii_lowercase().ends_with(suffix.as_str()));
        for (_, text) in matching {
            for line in text.lines() {
                let code = line.split("//").next().unwrap_or("");
                let mut words = code.split_whitespace();
                if words.next() != Some("gametype_setting") {
                    continue;
                }
                if let (Some(name), Some(value)) = (words.next(), words.next()) {
                    settings.insert(
                        name.to_ascii_lowercase(),
                        value.trim_matches('"').to_owned(),
                    );
                }
            }
        }
    }
    settings
}

/// The game type to run: the host's choice, else the test variable's, else
/// free-for-all. A variable naming no game type is refused.
pub(super) fn match_kind(
    chosen: Option<GameModeKind>,
    from_env: Option<&str>,
) -> Result<GameModeKind, &'static str> {
    if let Some(kind) = chosen {
        return Ok(kind);
    }
    match from_env.map(str::trim).filter(|text| !text.is_empty()) {
        Some(text) => GameModeKind::parse_ascii_ignore_case(text)
            .ok_or("BO2MP_GAMETYPE names no known gametype (gsc.mode.out-of-scope)"),
        None => Ok(GameModeKind::FreeForAll),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(name: &str, value: &str) -> (String, String) {
        (name.to_owned(), value.to_owned())
    }

    #[test]
    fn set_and_seta_lines_are_read_in_order() {
        let text = "set a 1\nseta b \"two words\"\n";
        assert_eq!(
            config_sets(text),
            vec![pair("a", "1"), pair("b", "two words")]
        );
    }

    #[test]
    fn comments_and_other_lines_are_skipped() {
        let text = "// header\nbind x +attack\nset a 1 // trailing\n   \nsetu c 3\n";
        assert_eq!(config_sets(text), vec![pair("a", "1")]);
    }

    #[test]
    fn a_name_with_no_value_is_set_to_nothing() {
        assert_eq!(config_sets("set lonely"), vec![pair("lonely", "")]);
        assert_eq!(config_sets("set lonely   "), vec![pair("lonely", "")]);
    }

    #[test]
    fn indented_lines_still_count() {
        assert_eq!(config_sets("   set  a   1  "), vec![pair("a", "1")]);
    }

    #[test]
    fn overrides_may_be_split_with_semicolons() {
        assert_eq!(
            dvar_overrides("set a 1;set b 2"),
            vec![pair("a", "1"), pair("b", "2")]
        );
        assert!(dvar_overrides("").is_empty());
    }

    #[test]
    fn a_match_always_names_its_map_and_game_type() {
        let dvars = script_dvars(None, "t6:mp_x", "tdm", &[]);
        assert_eq!(
            dvars,
            vec![
                pair("mapname", "t6:mp_x"),
                pair("g_gametype", "tdm"),
                pair("sv_maxclients", "18"),
            ]
        );
    }

    #[test]
    fn the_config_comes_first() {
        let dvars = script_dvars(Some("set a 1"), "m", "g", &[]);
        assert_eq!(dvars[0], pair("a", "1"));
        assert_eq!(dvars.len(), 4);
    }

    #[test]
    fn a_host_rule_replaces_a_dvar_of_any_case() {
        let rules = vec![pair("SCR_Score_Limit", "50")];
        let dvars = script_dvars(Some("set scr_score_limit 10"), "m", "g", &rules);
        assert_eq!(dvars[0], pair("scr_score_limit", "50"));
        assert_eq!(dvars.len(), 4);
    }

    #[test]
    fn a_host_rule_for_a_new_dvar_is_added_last() {
        let rules = vec![pair("extra", "1")];
        let dvars = script_dvars(None, "m", "g", &rules);
        assert_eq!(dvars.last(), Some(&pair("extra", "1")));
    }

    #[test]
    fn a_rule_can_replace_a_dvar_the_match_sets_itself() {
        let rules = vec![pair("sv_maxclients", "8")];
        let dvars = script_dvars(None, "m", "g", &rules);
        assert!(dvars.contains(&pair("sv_maxclients", "8")));
        assert!(!dvars.contains(&pair("sv_maxclients", "18")));
    }

    fn files() -> Vec<(String, String)> {
        vec![
            pair(
                "maps/mp/gametypes/gamesettings_default.cfg",
                "gametype_setting Timelimit 10 // minutes\ngametype_setting scorelimit \"75\"",
            ),
            pair(
                "maps/mp/gametypes/gamesettings_tdm.cfg",
                "gametype_setting timelimit 20",
            ),
            pair(
                "maps/mp/gametypes/gamesettings_dom.cfg",
                "gametype_setting timelimit 99",
            ),
        ]
    }

    #[test]
    fn game_type_settings_win_over_the_defaults() {
        let settings = gamesettings(&files(), "tdm");
        assert_eq!(settings.get("timelimit").map(String::as_str), Some("20"));
        assert_eq!(settings.get("scorelimit").map(String::as_str), Some("75"));
        assert_eq!(settings.len(), 2);
    }

    #[test]
    fn another_game_types_settings_are_not_read() {
        let settings = gamesettings(&files(), "ffa");
        assert_eq!(settings.get("timelimit").map(String::as_str), Some("10"));
    }

    #[test]
    fn a_setting_with_no_value_is_skipped() {
        let files = vec![pair("gamesettings_default.cfg", "gametype_setting lonely")];
        assert!(gamesettings(&files, "tdm").is_empty());
    }

    #[test]
    fn the_hosts_choice_wins_over_the_variable() {
        assert_eq!(
            match_kind(Some(GameModeKind::Domination), Some("tdm")),
            Ok(GameModeKind::Domination)
        );
    }

    #[test]
    fn the_variable_names_a_game_type_in_any_case() {
        let token = GameModeKind::TeamDeathmatch.token().to_ascii_uppercase();
        assert_eq!(
            match_kind(None, Some(&format!(" {token} "))),
            Ok(GameModeKind::TeamDeathmatch)
        );
    }

    #[test]
    fn with_no_choice_the_default_is_free_for_all() {
        assert_eq!(match_kind(None, None), Ok(GameModeKind::FreeForAll));
        assert_eq!(match_kind(None, Some("  ")), Ok(GameModeKind::FreeForAll));
    }

    #[test]
    fn a_variable_naming_nothing_is_refused() {
        assert!(match_kind(None, Some("not-a-mode")).is_err());
    }
}
