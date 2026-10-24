//! Builds the lines of the status page from what the launch found.

use bevy::prelude::*;
use frame::{LaunchIdentity, LaunchReport};

use crate::classes::store::SessionClassStore;
use crate::gap_hud::GapHud;

/// Known holes in the renderer, listed so the page never implies they work.
const KNOWN_GAPS: [&str; 6] = [
    "map zone walk ends after ClipMap (often impactfx); common_mp walks weapon/xanim (next: menulist)",
    "equip hands out the primary from the sorted weapon catalog with CmdScale scaling; the first-person bind pose sits under the eye once gunXModel resolves",
    "sky cubemap and alpha-test/blend/multiply from techset+stateBits are live; specular, probes and spot lights are not",
    "DPVS batches go by material identity; MaterialInfo.drawSurf keys are not used yet",
    "first-person hands and gun are eye-posed (bind baseMat, rigid verts); XAnim sampling and full blend skinning are missing",
    "static-model LOD0 placements draw through DPVS; authored prop materials and the light grid are missing",
];

fn path_line(label: &str, found: &Result<std::path::PathBuf, String>) -> String {
    match found {
        Ok(path) => format!("found {label}: {}", path.display()),
        Err(error) => format!("{label}: {error}"),
    }
}

pub fn publish_gap_hud(
    identity: Option<Res<LaunchIdentity>>,
    report: Option<Res<LaunchReport>>,
    class_store: Res<SessionClassStore>,
    mut hud: ResMut<GapHud>,
) {
    let (Some(identity), Some(report)) = (identity, report) else {
        return;
    };

    let mode = sim::host_game_mode_kind();
    let equipped = class_store
        .equipped_slot()
        .map_or("(choose-class gate)", |slot| slot.name.as_str());

    let mut body = vec![
        format!("role: {}", identity.role_label),
        format!("BO2MP_GAMES: {}", identity.games_root.display()),
        format!("artifacts: {}", identity.artifacts.display()),
        format!("zone request: {}", report.zone),
        format!(
            "match: {} (`{}`) score_limit={} time_limit_ms={}",
            mode.display_name(),
            mode.token(),
            sim::FFA.score_limit,
            sim::FFA.time_limit_ms
        ),
        format!("equipped class: {equipped}"),
        path_line("common_mp.ff", &report.common_mp),
        path_line(&format!("{}.ff", report.zone), &report.zone_ff),
        String::new(),
        "world:".to_owned(),
    ];
    body.extend(report.world_report.iter().map(|row| format!("- {row}")));
    body.push(String::new());
    body.push("gaps (not faked):".to_owned());
    body.extend(KNOWN_GAPS.iter().map(|gap| format!("- {gap}")));
    body.push(format!("- {}", report.sim_gap));
    if let Some(metrics) = &report.prediction_metrics {
        body.push(format!("- prediction: {metrics}"));
    }
    body.push(String::new());
    body.push("WASD move, arrows look, escape / close window to quit".to_owned());

    let title = format!("bo2mp — {}", identity.zone);
    let sources_changed = report.is_changed() || identity.is_changed() || class_store.is_changed();
    if sources_changed || hud.title != title || hud.body != body {
        hud.title = title;
        hud.body = body;
    }
}
