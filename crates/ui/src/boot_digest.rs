//! Fills the interim status page with what start-up found: paths, the match rules, the
//! world summary and the renderer's known holes.

use std::path::PathBuf;

use bevy::prelude::*;
use frame::{StartupPersona, StartupReport};

use crate::classes::store::MatchLoadoutVault;
use crate::status_page::InterimOverlay;

/// Renderer holes the page names outright, so nobody reads them as working.
const KNOWN_GAPS: [&str; 6] = [
    "map zone walk ends after ClipMap (often impactfx); common_mp walks weapon/xanim, menulist comes next",
    "equip hands out the primary from the sorted weapon catalog with CmdScale scaling; the first-person bind pose sits under the eye once gunXModel resolves",
    "sky cubemap and alpha-test/blend/multiply from techset+stateBits are live; specular, probes and spot lights are not",
    "DPVS batches go by material identity; MaterialInfo.drawSurf keys are not used yet",
    "first-person hands and gun are eye-posed (bind baseMat, rigid verts); XAnim sampling and full blend skinning are missing",
    "static-model LOD0 placements draw through DPVS; authored prop materials and the light grid are missing",
];

/// One page row for a file search: where it was found, or why not.
fn found_row(what: &str, lookup: Result<&PathBuf, &String>) -> String {
    match lookup {
        Ok(at) => format!("found {what}: {}", at.display()),
        Err(why) => format!("{what}: {why}"),
    }
}

/// Appends `items` to `rows`, each as a dash bullet.
fn bullets<T: std::fmt::Display>(rows: &mut Vec<String>, items: impl IntoIterator<Item = T>) {
    rows.extend(items.into_iter().map(|item| format!("- {item}")));
}

pub fn share_interim_overlay(
    persona: Option<Res<StartupPersona>>,
    found: Option<Res<StartupReport>>,
    vault: Res<MatchLoadoutVault>,
    mut page: ResMut<InterimOverlay>,
) {
    let Some(persona) = persona else { return };
    let Some(found) = found else { return };

    let mode = sim::hosted_ruleset();
    let class_name = match vault.armed_loadout_index() {
        Some(kit) => kit.name.as_str(),
        None => "(choose-class gate)",
    };
    let zone_file = format!("{}.ff", found.zone);

    let rules = &sim::FFA;
    let mode_line = format!(
        "{} (`{}`) score_limit={} time_limit_ms={}",
        mode.shown_label(),
        mode.token(),
        rules.win_score,
        rules.round_ms,
    );
    let labelled = [
        ("role", persona.duty_caption.to_string()),
        ("BO2MP_GAMES", persona.titles_install_dir.display().to_string()),
        ("artifacts", persona.artifacts.display().to_string()),
        ("zone request", found.zone.to_string()),
        ("match", mode_line),
        ("equipped class", class_name.to_owned()),
    ];
    let mut rows: Vec<String> = labelled.iter().map(|(tag, value)| format!("{tag}: {value}")).collect();
    rows.extend([
        found_row("common_mp.ff", found.shared_zone.as_ref()),
        found_row(&zone_file, found.zone_archive_file.as_ref()),
        String::new(),
        "world:".to_owned(),
    ]);
    bullets(&mut rows, &found.level_digest);
    rows.push(String::new());
    rows.push("gaps (not faked):".to_owned());
    bullets(&mut rows, KNOWN_GAPS);
    bullets(&mut rows, [&found.stepping_lag]);
    bullets(&mut rows, found.forecast_figures.iter().map(|figures| format!("prediction: {figures}")));
    rows.push(String::new());
    rows.push("WASD move, arrow keys look, escape / close window to quit".to_owned());

    let heading = format!("bo2mp — {}", persona.zone);
    let inputs_moved = found.is_changed() || persona.is_changed() || vault.is_changed();
    if inputs_moved || page.heading != heading || page.rows != rows {
        page.rows = rows;
        page.heading = heading;
    }
}
