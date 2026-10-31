//! Which announcer and team-music aliases a match needs, and how a team's
//! generic announcer line is routed to the faction's own recording.

use std::collections::HashMap;

use asset_core::AssetCategoryScope;
use bevy::prelude::Resource;

/// Team music lines, each named under a team's voice prefix.
const TEAM_MUSIC: [&str; 5] = [
    "spawn_music",
    "victory_music",
    "defeat_music",
    "winning_music",
    "losing_music",
];

/// Marks a generic announcer line under a team voice prefix.
const TEAM_LINE: &str = "1mc_";

/// Every `game["voice"][team]` prefix the faction scripts in Black Ops II set
/// (`_teamset_*.gsc`): seals, pmc, cd, fbi, isa, pla.
const T6_ANNOUNCER_PREFIXES: [&str; 6] = [
    "vox_st6_", "vox_pmc_", "vox_cda_", "vox_fbi_", "vox_isa_", "vox_pla_",
];

/// Generic announcer line (after `TEAM_LINE`) to the line the older games' recordings use.
const T5_LINES: &[(&str, &str)] = &[
    ("acheive_bomb", "sd_bomb_taken_taken"),
    ("bomb_defused", "sd_bomb_defused"),
    ("bomb_planted", "sd_bomb_planted"),
    ("bomb_taken", "sd_bomb_drop"),
    ("boost", "generic_boost"),
    ("capture_a", "dom_capture_a"),
    ("capture_b", "dom_capture_c"),
    ("capture_c", "dom_capture_b"),
    ("capturing_a", "dom_capturing_a"),
    ("capturing_b", "dom_capturing_b"),
    ("capturing_c", "dom_capturing_c"),
    ("captureflag", "ctf_start"),
    ("demolition", "demo_start"),
    ("dest_sentrygun", "dest_sentry"),
    ("domination", "dom_start"),
    ("freeforall", "ffa_start"),
    ("headquarters", "hq_start"),
    ("hq_captured", "hq_capture"),
    ("hq_destroyed", "hq_defend"),
    ("losing_a", "dom_losing_a"),
    ("losing_b", "dom_losing_b"),
    ("losing_c", "dom_losing_c"),
    ("lost_a", "dom_lost_a"),
    ("lost_b", "dom_lost_b"),
    ("lost_c", "dom_lost_c"),
    ("obj_defend", "defend_start"),
    ("obj_destroy", "destroy_start"),
    ("positions_lock", "dom_lock_wetake"),
    ("sabotage", "sab_start"),
    ("searchdestroy", "sd_start"),
    ("secure_a", "dom_secured_a"),
    ("secure_b", "dom_secured_b"),
    ("secure_c", "dom_secured_c"),
    ("securing_a", "dom_securing_a"),
    ("securing_b", "dom_securing_b"),
    ("securing_c", "dom_securing_c"),
    ("switching", "switchingsides"),
    ("take_positions", "dom_lock_theytake"),
    ("tm_death", "tdm_start"),
];

/// Names of every Black Ops II alias in the catalog.
fn t6_alias_names(bank: &asset_audio::AudioRoster) -> impl Iterator<Item = &str> {
    (0..bank.sounds.len())
        .filter(|&index| bank.space_of_slot(index) == AssetCategoryScope::T6)
        .filter_map(|index| bank.label_at(index))
}

/// The aliases to have ready for a match: everything under the teams'
/// voice prefixes or the generic death prefix, plus each team's music lines.
pub(crate) fn team_voice_aliases(
    bank: &asset_audio::AudioRoster,
    allies: Option<&str>,
    along: Option<&str>,
) -> Vec<String> {
    let teams: Vec<&str> = [allies, along].into_iter().flatten().collect();

    let mut aliases: Vec<String> = t6_alias_names(bank)
        .filter(|name| {
            teams
                .iter()
                .copied()
                .chain(["generic_death_"])
                .any(|prefix| name.starts_with(prefix))
        })
        .map(str::to_owned)
        .collect();

    for team in teams {
        for line in TEAM_MUSIC {
            let alias = format!("{team}{line}");
            if !aliases.contains(&alias) {
                aliases.push(alias);
            }
        }
    }
    aliases
}

/// The announcer lines of the match's factions.
///
/// The game plays `game["voice"][team] + game["dialog"][line]` as a local
/// sound (e.g. `vox_st6_timesup`). The map loads only this match's two
/// factions' language zones, so any Black Ops II alias with one of the six
/// prefixes belongs to them. They are in neither the weapon nor the map sets;
/// without preparing them here they would first be decoded when the script
/// plays them and would miss the start deadline (a late line is dropped).
pub(crate) fn t6_announcer_aliases(bank: &asset_audio::AudioRoster) -> Vec<String> {
    t6_alias_names(bank)
        .filter(|name| {
            let lowered = name.to_ascii_lowercase();
            T6_ANNOUNCER_PREFIXES
                .iter()
                .any(|prefix| lowered.starts_with(prefix))
        })
        .map(str::to_owned)
        .collect()
}

/// Where each announcer alias of the match is actually played from.
#[derive(Resource, Default)]
pub(crate) struct AnnouncerRoutes(HashMap<String, (AssetCategoryScope, String)>);

/// The alias a team's `line` is played from, when it has one.
fn routed_line(
    line: &str,
    voice: Option<&str>,
    music: &asset_game::TeamMusic,
    renames: &[(&str, &str)],
) -> Option<String> {
    if let Some(announcement) = line.strip_prefix(TEAM_LINE) {
        let announcement = renames
            .iter()
            .find(|(from, _)| *from == announcement)
            .map_or(announcement, |(_, to)| to);
        return Some(format!("{}{announcement}", voice?));
    }
    match line {
        "spawn_music" => music.spawn.clone(),
        "victory_music" => music.victory.clone(),
        "defeat_music" => music.defeat.clone(),
        "winning_music" => music.winning.clone(),
        "losing_music" => music.losing.clone(),
        _ => None,
    }
}

impl AnnouncerRoutes {
    /// Where to play `alias` from; aliases without a route play as themselves.
    pub(crate) fn route<'a>(&'a self, cue_id: &'a str) -> (AssetCategoryScope, &'a str) {
        match self.0.get(cue_id) {
            Some((cue_ns, routed)) => (*cue_ns, routed),
            None => (AssetCategoryScope::T6, cue_id),
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }

    pub(crate) fn build(
        bank: &asset_audio::AudioRoster,
        namespace: AssetCategoryScope,
        team_prefixes: [Option<&str>; 2],
        native: Option<&asset_game::MapTeamSettings>,
        aliases: &[String],
    ) -> Self {
        let Some(native) = native else {
            return Self::default();
        };
        let renames: &[(&str, &str)] = match namespace {
            AssetCategoryScope::T5 => T5_LINES,
            // Black Ops II announcer lines are not renamed.
            AssetCategoryScope::T6 => &[],
        };

        let [allies, axis] = team_prefixes;
        let teams: Vec<(&str, Option<&str>, &asset_game::TeamMusic)> = [
            (allies, native.allies_voice.as_deref(), &native.allies_music),
            (axis, native.axis_voice.as_deref(), &native.axis_music),
        ]
        .into_iter()
        .filter_map(|(prefix, voice, music)| Some((prefix?, voice, music)))
        .collect();

        let routes = aliases
            .iter()
            .filter_map(|alias| {
                teams.iter().find_map(|&(prefix, voice, music)| {
                    let line = alias.strip_prefix(prefix)?;
                    let routed = routed_line(line, voice, music, renames)?;
                    bank.slot_in(namespace, &routed)?;
                    Some((alias.clone(), (namespace, routed)))
                })
            })
            .collect();
        Self(routes)
    }
}
