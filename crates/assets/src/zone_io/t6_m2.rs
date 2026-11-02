//! bo2zm M2: a Black Ops II map's guns, arms, animations, effects and
//! sound, from every zone the map loads.
//!
//! A Zombies map loads shared zones before its own (`code_post_gfx_zm`,
//! `common_zm`, `patch_zm`, ...) and its patch after. Each is walked once
//! and captured; later zones win a name they share with an earlier one
//! (the patch's weapons replace the map's). Comma-named assets are
//! references to another zone's and are skipped.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use anim_bank::XAnimBuild;
use asset_game::{EffectRoster, SelfContainedImpactFxTable, WeaponBuild, ArmoryRoster};
use asset_material::SurfaceShaderRoster;
use asset_model::{FpvMeshBuild, WorldWeaponBuild};
use asset_t6::{PackSet, ZoneCapture};

/// The arms a Zombies map gives its players, by map (from the map's own
/// script: `zm_nuked.gsc` sets `c_zom_suit_viewhands` for the CIA player and
/// `c_zom_hazmat_viewhands_light` for CDC).
pub const T6_PLAYER_ARMS: &str = "c_zom_suit_viewhands";
const T6_OTHER_ARMS: [&str; 1] = ["c_zom_hazmat_viewhands_light"];

/// Zones a Zombies map loads before its own, in order.
const T6_ZM_PRE_ZONES: [&str; 4] = ["code_post_gfx_zm", "common_zm", "patch_zm", "patch_ui_zm"];

/// The zones before a Zombies map. The Zombies front end's backdrop map also
/// loads `ui_zm` (before its patch, which wins a name): the menus' own
/// pictures are there (`lui_bkg_zm`, its rocks and sun, the globe, the
/// title), and without them only the menu's black base piece is drawn.
fn zm_pre_zones(front_end: bool) -> Vec<&'static str> {
    let mut zones = T6_ZM_PRE_ZONES.to_vec();
    if front_end {
        zones.insert(3, "ui_zm");
    }
    zones
}

/// The map packs' load zones a Zombies map reads after its pre zones.
/// Nuketown is the first Zombies DLC map: its load zone carries the intro's
/// sound bank. The front end's backdrop map reads all five: each holds its
/// maps' zoomed map art (`menu_<map>_map`, its `_blur`) and start-location
/// pictures (`menu_zm_map_<map>_blit_<loc>`) that the map picker shows
/// (SelectMapZombie, GameMapZombie); with Nuketown's alone, picking any other
/// map showed only the fog.
fn zm_load_zones(stem: &str, front_end: bool) -> &'static [&'static str] {
    const PACKS: [&str; 5] = ["dlczm0_load_zm", "dlc1_load_zm", "dlc2_load_zm", "dlc3_load_zm", "dlc4_load_zm"];
    if front_end {
        &PACKS
    } else if stem == "zm_nuked" {
        &["dlczm0_load_zm"]
    } else {
        &[]
    }
}

/// A picture the Zombies front end draws although no script constant names
/// it: every picture in its menu zones (the menus join names from tables and
/// strings: `menu_` .. map .. `_map`, the maps table's signposts), and
/// TranZit's loading pictures in code_post_gfx_zm (BO2's loading script joins
/// their names).
fn front_end_picture(zone: &str, name: &str) -> bool {
    real(name)
        && (matches!(zone, "ui_zm" | "patch_ui_zm")
            || zone.ends_with("_load_zm")
            || (zone == "code_post_gfx_zm" && name.starts_with("loadscreen_")))
}

/// bo2mp: zones a multiplayer map loads before its own, in order (later
/// zones win a name: `common_patch_mp`'s guns replace `common_mp`'s, the
/// patch's scripts and tables everything before it). Then the map's two
/// factions (`faction_<name>_mp`, from `mp/mapstable.csv`).
const T6_MP_PRE_ZONES: [&str; 6] =
    ["code_post_gfx_mp", "common_mp", "common_patch_mp", "patch_mp", "ui_mp", "patch_ui_mp"];

/// bo2mp: the English text and voice zones a multiplayer map reads
/// (`zone/english/en_<zone>.ff`), the factions' and map's added after.
const T6_MP_LANG_ZONES: [&str; 5] = ["code_post_gfx_mp", "common_mp", "patch_mp", "ui_mp", "patch_ui_mp"];

/// bo2mp: Black Ops II's priority for a zone (DB_GetZonePriority of the
/// flags it loads with): code_post_gfx 52, common 54, its patch 55, a map
/// and its factions 57, a map's own patch 58, an `so_` zone 59, ui 61,
/// patch_ui 62, patch 65. A copy of a named asset loaded later replaces
/// the one held only when its zone's priority is the same or higher
/// (DB_OverrideAsset), so patch_mp's fixed map scripts outrank the map
/// zone's own, though the map loads last.
fn zone_priority(zone: &str) -> u8 {
    const SHARED_ZONES: [(&str, u8); 6] = [
        ("code_post_gfx", 52),
        ("common", 54),
        ("common_patch", 55),
        ("ui", 61),
        ("patch_ui", 62),
        ("patch", 65),
    ];
    let mode_free = ["_mp", "_zm"]
        .iter()
        .find_map(|mode| zone.strip_suffix(mode))
        .unwrap_or(zone);
    if let Some(&(_, rank)) = SHARED_ZONES.iter().find(|(stem, _)| *stem == mode_free) {
        return rank;
    }
    if zone.ends_with("_patch") {
        58
    } else if zone.starts_with("so_") {
        59
    } else {
        57
    }
}

/// bo2mp: a Black Ops II map is multiplayer (`mp_*`) or Zombies (`zm_*`).
pub(crate) fn is_mp_map(map: &str) -> bool {
    map.starts_with("mp_")
}

pub(crate) struct T6Combat {
    pub weapons: WeaponBuild,
    pub fpv: FpvMeshBuild,
    pub level_placed_weapons: WorldWeaponBuild,
    pub animation_refs: XAnimBuild,
    pub fx: EffectRoster,
    /// The models effects draw (shell casings, debris).
    pub effect_meshes: asset_game::EffectMeshRoster,
    /// The tracers every zone defines (each weapon names its own).
    pub tracers: asset_game::TracerRoster,
    pub impact_effect: Option<SelfContainedImpactFxTable>,
    pub sound: Result<asset_audio::AudioRoster, String>,
    /// The default ambient room's tone, from the map's ambience script.
    pub room_tone: Option<String>,
    /// bo2zm M3: every zone's compiled scripts, string tables and entity
    /// strings, in load order.
    pub scripts: crate::T6ScriptSet,
    /// bo2zm M3: the Zombies HUD's icons.
    pub hud_icons: crate::T6HudIcons,
    /// bo2zm M3: the Zombies HUD's fonts.
    pub hud_fonts: crate::T6HudFonts,
    /// bo2zm M4: Black Ops II's UI scripts and what they read.
    pub ui: crate::T6Ui,
    /// bo2mp: Black Ops II's bullet penetration depths
    /// (`info/bullet_penetration_mp`, later zones winning).
    pub pen_table: Option<weapon_t6::penetration::BulletPierceDepthChart>,
    /// bo2mp: Black Ops II's global hit-location damage table
    /// (`info/mp_lochit_dmgtable`, later zones winning).
    pub lochit_table: Option<[f32; weapon_t6::hitloc::HITLOC_COUNT]>,
    /// bo2zm: every zone's vision files (`vision/<name>.vision`), later
    /// zones replacing earlier ones: the visions a script turns on
    /// (Mob's Afterlife, `zm_afterlife` in its mode zone).
    pub visions: Vec<(String, Vec<u8>)>,
    /// bo2zm: the full-screen filters the scripts turn on (Mob's Afterlife).
    pub screen_filters: crate::T6ScreenFilters,
    /// bo2mp gun brain: every weapon's raw variant + def bytes and loader columns, in the
    /// `weapon_t6` facts dump format (one line per weapon, by name).
    pub gun_facts: String,
    pub load_log: Vec<String>,
}

/// The Zombies HUD's icons: the perks a Nuketown machine sells and the
/// power-ups that last a while (their materials' colour maps).
const HUD_ICONS: [&str; 60] = [
    "specialty_juggernaut_zombies",
    "specialty_quickrevive_zombies",
    "specialty_fastreload_zombies",
    "specialty_doubletap_zombies",
    "specialty_instakill_zombies",
    "specialty_doublepoints_zombies",
    "specialty_firesale_zombies",
    // The round's chalk marks, rounds 1 to 5 (hudroundstatuszombie.lua).
    "hud_chalk_1",
    "hud_chalk_2",
    "hud_chalk_3",
    "hud_chalk_4",
    "hud_chalk_5",
    // The grenades and mines he carries, one icon each (offhandicons.lua):
    // frag, Semtex, monkey, Claymore.
    "hud_us_grenade",
    "hud_icon_sticky_grenade",
    "hud_cymbal_monkey",
    "hud_icon_claymore",
    // The red screen-edge overlay the health script fades in when he is hit
    // (_zm_playerhealth's healthoverlay).
    "overlay_low_health",
    // The guns' hip-fire crosshair: its four lines and the centre pieces
    // some guns name (weapon defs' reticleSide / reticleCenter).
    "reticle_side_small",
    "reticle_center_cross",
    "reticle_flechette",
    // bo2mp: the rest Black Ops II's MP guns name (every weapon def in
    // common_mp, read with t6wsnd): the Death Machine's round sides and
    // its killstreak ring, the launchers', the throwing ones'.
    "reticle_side_round",
    "hud_ks_minigun",
    "reticle_center_cross_mg42",
    "reticle_m203",
    "c4_reticle",
    "hatchet_reticle",
    "knife_ballistic_reticle",
    "tactical_gren_reticle",
    // The orange YOU pin the killcam draws over the player who was killed.
    "headiconyouinkillcam",
    // bo2mp: the Target Finder's screen picture, drawn into the UI3D
    // texture behind its red diamonds (hud_t6 rangefinder's BACKGROUND_MATERIAL).
    "mtl_t6_attach_optic_rangefinder_ui3d_background",
    // Mob's Afterlife hands' centre mark (its weapon's reticleCenter, in
    // zm_prison.ff).
    "afterlife_reticle",
    // The scoreboard's team badges ("faction_" .. GetFactionForTeam): the
    // CDC and CIA (Nuketown's teamset).
    "faction_cdc",
    "faction_cia",
    // Nuketown's desert map behind its start locations: GameMapZombie
    // builds the name ("menu_" .. GetUIMapName() .. "_map", and "_blur"),
    // so no script names it whole.
    "menu_zm_nuked_map",
    "menu_zm_nuked_map_blur",
    // BO2's Xbox button pictures (code_post_gfx_zm): the pad's prompts in
    // its menus and hints (frame::BO2_XBOX_GLYPHS).
    "xenonbutton_a",
    "xenonbutton_b",
    "xenonbutton_x",
    "xenonbutton_y",
    "xenonbutton_lb",
    "xenonbutton_rb",
    "xenonbutton_lt",
    "xenonbutton_rt",
    "xenonbutton_back",
    "xenonbutton_start",
    "xenonbutton_ls",
    "xenonbutton_rs",
    "xenonbutton_dpad_all",
    "xenonbutton_dpad_ud",
    "xenonbutton_dpad_rl",
    "xenonbutton_dpad_up",
    "xenonbutton_dpad_down",
    "xenonbutton_dpad_left",
    "xenonbutton_dpad_right",
    // BO2 on PC: its menu tabs' arrows (frame::bo2_cycle_glyph).
    "ui_arrow_left",
    "ui_arrow_right",
    // The lobby rows' speaker (the engine's VoipImage, code_post_gfx_zm):
    // muted, and talking (its left half is the quiet one).
    "voice_off",
    "voice_on",
    // The build progress bar (_hud_util createbar: its back and its fill;
    // Buried's buildables, any table map).
    "progress_bar_bg",
    "progress_bar_fill",
];

/// The HUD's fonts by the Lua scripts' names and their files, as
/// `ui/t6/codbase.lua` registers them for English.
const HUD_FONTS: [(&str, &str); 7] = [
    ("Default", "fonts/720/normalFont"),
    ("Condensed", "fonts/720/smallFont"),
    ("Big", "fonts/720/bigFont"),
    ("Morris", "fonts/720/extraBigFont"),
    ("ExtraSmall", "fonts/720/extraSmallFont"),
    ("Italic", "fonts/720/italicFont"),
    ("SmallItalic", "fonts/720/smallItalicFont"),
];

pub(super) fn hud_fonts(
    captures: &[ZoneCapture],
    packs: Option<&PackSet>,
    notes: &mut Vec<String>,
) -> crate::T6HudFonts {
    let mut out = crate::T6HudFonts::default();
    let mut sheet_material = None;
    for (name, file) in HUD_FONTS {
        let Some(font) = captures
            .iter()
            .rev()
            .flat_map(|c| c.fonts.iter())
            .find(|f| f.name.eq_ignore_ascii_case(file))
        else {
            notes.push(format!("t6 hud font {name}: {file} missing"));
            continue;
        };
        sheet_material.get_or_insert_with(|| font.material.clone());
        out.fonts.push(crate::T6HudFont {
            name: name.to_owned(),
            raster_rows: font.raster_rows as f32,
            glyphs: font
                .glyphs
                .iter()
                .map(|g| {
                    (
                        g.letter,
                        [
                            i16::from(g.x0),
                            i16::from(g.y0),
                            i16::from(g.step_along_x),
                            i16::from(g.texel_span),
                            i16::from(g.raster_rows),
                        ],
                        g.st,
                    )
                })
                .collect(),
        });
    }
    // Every one of them draws from the same sheet (`gamefonts_pc`).
    if let (Some(material), Some(packs)) = (sheet_material, packs) {
        let image = captures.iter().find_map(|c| {
            let m = c.materials.iter().find(|m| m.name == material)?;
            let t = m.textures.first()?;
            c.images.get(t.image?.index)
        });
        match image.map(|i| super::t6_materials::decode(packs, i)) {
            Some(Ok(img)) => out.sheet = Some(std::sync::Arc::new(img)),
            Some(Err(e)) => notes.push(format!("t6 hud font sheet {material}: {e}")),
            None => notes.push(format!("t6 hud font sheet {material}: no image")),
        }
    }
    notes.push(format!(
        "t6 hud fonts: {} ({})",
        out.fonts.len(),
        out.fonts
            .iter()
            .map(|f| format!("{} {}px {} glyphs", f.name, f.raster_rows, f.glyphs.len()))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    out
}

/// bo2zm M4: the zones Black Ops II's Zombies front end reads (main menu,
/// lobby, globe and map picker), in load order; each map pack's load zone
/// adds its map pictures and loading screen (Nuketown, then the four DLC
/// packs: every Zombies map is on the globe).
const T6_ZM_FRONTEND_ZONES: [&str; 10] = [
    "code_post_gfx_zm",
    "common_zm",
    "patch_zm",
    "ui_zm",
    "patch_ui_zm",
    "dlczm0_load_zm",
    "dlc1_load_zm",
    "dlc2_load_zm",
    "dlc3_load_zm",
    "dlc4_load_zm",
];

/// bo2zm M4: Black Ops II's Zombies front end without a map: its UI
/// scripts, the pictures they name, the fonts, English strings and string
/// tables. Kept for the whole run: the menu comes back after each game.
#[derive(Clone, bevy::prelude::Resource)]
pub struct T6Frontend {
    pub ui: crate::T6Ui,
    pub icons: crate::T6HudIcons,
    pub fonts: crate::T6HudFonts,
    /// Its sounds: the menus' clicks and music (`zmb_code_post_gfx`).
    pub sound: Option<std::sync::Arc<asset_audio::AudioRoster>>,
    pub load_log: Vec<String>,
}

/// bo2zm M4: read the front end from the zones in `zone_dir`
/// (`zone/all`; the English zones are in `zone/english` beside it).
pub fn load_t6_frontend(zone_dir: &Path) -> T6Frontend {
    let clock = std::time::Instant::now();
    let mut load_log = Vec::new();
    let capture_all = |dir: &Path, name_prefix: &str, load_log: &mut Vec<String>| -> Vec<ZoneCapture> {
        T6_ZM_FRONTEND_ZONES
            .iter()
            .filter_map(|z| {
                let path = dir.join(format!("{name_prefix}{z}.ff"));
                if !path.is_file() {
                    return None;
                }
                match asset_t6::capture_zone(&path) {
                    Ok(c) => Some(c),
                    Err(e) => {
                        load_log.push(format!("t6 front end zone gap: {e}"));
                        None
                    }
                }
            })
            .collect()
    };
    let caps = capture_all(zone_dir, "", &mut load_log);
    // A Black Ops map pack in an extra folder (`BO2MP_T6_EXTRA`): its load
    // zone carries those maps' globe rows, words and pictures. Only those
    // are taken; its own menu scripts are not (BO2's globe stays BO2's).
    let pack_load = asset_transport::t6_extra::extra_file("mod_load.ff").and_then(|path| {
        asset_t6::capture_zone(&path)
            .map_err(|e| load_log.push(format!("t6 front end map pack gap: {e}")))
            .ok()
    });
    let lang_dir = zone_dir.parent().map(|d| d.join("english"));
    let lang_caps = lang_dir
        .as_deref()
        .map(|d| capture_all(d, "en_", &mut load_log))
        .unwrap_or_default();
    let captures: Vec<&ZoneCapture> = caps.iter().collect();
    load_log.push(format!(
        "t6 front end zones: {}; English: {}",
        captures
            .iter()
            .map(|c| c.zone.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        lang_caps
            .iter()
            .map(|c| c.zone.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    ));
    let packs = match PackSet::open_dir(zone_dir) {
        Ok(p) => Some(p),
        Err(e) => {
            load_log.push(format!("t6 front end image packs: {e}"));
            None
        }
    };
    let mut strings: HashMap<String, String> = HashMap::new();
    for c in &lang_caps {
        for (k, v) in &c.localize {
            strings.insert(k.clone(), v.clone());
        }
    }
    let mut tbl_list = Vec::new();
    for c in &captures {
        for t in &c.string_tables {
            if real(&t.name) {
                tbl_list.push((t.name.clone(), t.columns, t.rows, t.cells.clone()));
            }
        }
    }
    if let Some(pack) = &pack_load {
        // Its words where BO2 has none (the maps' names and starts).
        for (k, v) in &pack.localize {
            strings.entry(k.clone()).or_insert_with(|| v.clone());
        }
        let added = add_pack_maps(&mut tbl_list, pack);
        load_log.push(format!("t6 front end map pack maps: {}", added.join(" ")));
    }
    let read_ms = clock.elapsed().as_millis();
    let scripts = ui_scripts(&captures);
    let mut names = ui_strings(&scripts);
    for c in &captures {
        names.extend(
            c.materials
                .iter()
                .filter(|m| front_end_picture(&c.zone, &m.name))
                .map(|m| m.name.clone()),
        );
    }
    // And the pictures string tables name (rank icons, map signposts).
    for (_, _, _, cells) in &tbl_list {
        names.extend(
            cells
                .iter()
                .filter(|c| !c.is_empty() && c.len() < 64)
                .cloned(),
        );
    }
    // The pack's pictures (signposts, map pictures, loading screens).
    let mut icon_caps = captures.clone();
    if let Some(pack) = &pack_load {
        names.extend(
            pack.materials
                .iter()
                .filter(|m| real(&m.name))
                .map(|m| m.name.clone()),
        );
        icon_caps.push(pack);
    }
    let mut icons = hud_icons(&icon_caps, packs.as_ref(), &names, &mut load_log);
    let icons_ms = clock.elapsed().as_millis();
    let lang_packs = lang_dir.as_deref().and_then(|d| PackSet::open_dir(d).ok());
    // Pictures with English words on them (the globe's green "NEW" tag)
    // are in the English zones; their pixels in either folder's packs.
    let lang_refs: Vec<&ZoneCapture> = lang_caps.iter().collect();
    let mut found_lang = Vec::new();
    let mut lang_report = Vec::new();
    for p in [packs.as_ref(), lang_packs.as_ref()].into_iter().flatten() {
        let have: std::collections::BTreeSet<String> =
            icons.0.iter().map(|(n, _)| n.clone()).collect();
        let lang_names: std::collections::BTreeSet<String> =
            names.iter().filter(|n| !have.contains(*n)).cloned().collect();
        lang_report.clear();
        let more = hud_icons(&lang_refs, Some(p), &lang_names, &mut lang_report);
        for (n, img) in more.0 {
            if !have.contains(&n) {
                found_lang.push(n.clone());
                icons.0.push((n, img));
            }
        }
    }
    load_log.push(format!("t6 front end English pictures: {}", found_lang.join(", ")));
    load_log.extend(lang_report.into_iter().map(|l| format!("English: {l}")));
    let fonts = hud_fonts(
        &lang_caps,
        lang_packs.as_ref().or(packs.as_ref()),
        &mut load_log,
    );
    // The menus' sounds, from the same banks a map reads them from.
    let sound_dir = zone_dir
        .parent()
        .and_then(Path::parent)
        .map(|install| install.join("sound"));
    let sound = sound_dir.filter(|d| d.is_dir()).map(|dir| {
        let bases = bank_bases(&captures, "");
        let base_refs: Vec<&str> = bases.iter().map(String::as_str).collect();
        let (index, bank_report) = asset_audio::T6BankIndex::open(&dir, &base_refs);
        load_log.extend(bank_report);
        let banks: Vec<&asset_t6::SndBankRef> = captures
            .iter()
            .copied()
            .chain(lang_caps.iter())
            .flat_map(|c| c.sound_banks.iter())
            .collect();
        let globals = captures.iter().find_map(|c| c.snd_globals.as_ref());
        let (snd_catalog, census) = asset_audio::build_t6_sound_catalog(
            &banks,
            globals,
            &index,
        );
        load_log.push(format!(
            "t6 front end sound: {} aliases, {} with a clip",
            census.alias_count, census.with_clip
        ));
        std::sync::Arc::new(snd_catalog)
    });
    load_log.push(format!(
        "t6 front end: {} scripts, {} strings, {} string tables; zones read in {read_ms} ms, pictures by {icons_ms} ms, all in {} ms",
        scripts.len(),
        strings.len(),
        tbl_list.len(),
        clock.elapsed().as_millis()
    ));
    T6Frontend {
        ui: crate::T6Ui {
            scripts,
            strings: std::sync::Arc::new(strings.into_iter().collect()),
            tables: std::sync::Arc::new(tbl_list),
            configs: std::sync::Arc::new(ui_configs(&captures)),
            game_settings: std::sync::Arc::new(zm_game_settings(&captures)),
        },
        icons,
        fonts,
        sound,
        load_log: load_log,
    }
}

/// bo2zm: the maps a map pack's load zone adds to the globe: its maps table
/// rows for maps BO2's table lacks, and those maps' start locations and
/// modes from its gametypes table (renumbered after BO2's own). Returns the
/// maps added.
fn add_pack_maps(
    tables: &mut [(String, usize, usize, Vec<String>)],
    pack: &ZoneCapture,
) -> Vec<String> {
    let pack_rows = |name: &str| -> Vec<Vec<String>> {
        pack.string_tables
            .iter()
            .find(|t| t.name == name)
            .map(|t| t.cells.chunks(t.columns.max(1)).map(<[String]>::to_vec).collect())
            .unwrap_or_default()
    };
    fn cell(r: &[String], i: usize) -> &str {
        r.get(i).map_or("", String::as_str)
    }
    let mut added = Vec::new();
    // Append rows (fitted to the table's columns) to a table.
    let append = |table: &mut (String, usize, usize, Vec<String>), row_cells: Vec<Vec<String>>| {
        let cols = table.1.max(1);
        for mut r in row_cells {
            r.resize(cols, String::new());
            table.3.extend(r);
            table.2 += 1;
        }
    };
    let Some(maps) = tables.iter().position(|t| t.0 == "zm/mapstable.csv") else {
        return added;
    };
    let have: BTreeSet<String> = tables[maps]
        .3
        .chunks(tables[maps].1.max(1))
        .map(|r| cell(r, 0).to_owned())
        .collect();
    let new_maps: Vec<Vec<String>> = pack_rows("zm/mapstable.csv")
        .into_iter()
        .filter(|r| cell(r, 0).starts_with("zm_") && !have.contains(cell(r, 0)))
        .collect();
    added.extend(new_maps.iter().map(|r| cell(r, 0).to_owned()));
    append(&mut tables[maps], new_maps);
    if let Some(gt) = tables.iter().position(|t| t.0 == "zm/gametypestable.csv") {
        let cols = tables[gt].1.max(1);
        let next = |cat: &str| {
            tables[gt]
                .3
                .chunks(cols)
                .filter(|r| cell(r, 0) == cat)
                .filter_map(|r| cell(r, 1).parse::<i64>().ok())
                .max()
                .map_or(0, |m| m + 1)
        };
        let (mut loc_id, mut mode_id) = (next("5"), next("6"));
        let rows: Vec<Vec<String>> = pack_rows("zm/gametypestable.csv")
            .into_iter()
            .filter(|r| matches!(cell(r, 0), "5" | "6") && added.iter().any(|m| m == cell(r, 2)))
            .map(|mut r| {
                let id = if r[0] == "5" { &mut loc_id } else { &mut mode_id };
                r[1] = id.to_string();
                *id += 1;
                r
            })
            .collect();
        append(&mut tables[gt], rows);
    }
    added
}

/// bo2zm M4: Zombies' game settings defaults, from the zones' rawfiles
/// (`gametype_setting <name> <value>` in the default settings file, then
/// the standard mode's).
pub(super) fn zm_game_settings(captures: &[&ZoneCapture]) -> Vec<(String, f32)> {
    let mut out: Vec<(String, f32)> = Vec::new();
    for file in ["gamesettings_default.cfg", "gamesettings_zstandard.cfg"] {
        let Some(bytes) = captures
            .iter()
            .rev()
            .flat_map(|c| c.raw_files.iter())
            .find(|(n, _)| {
                n.to_ascii_lowercase().ends_with(file) && n.to_ascii_lowercase().contains("zm")
            })
            .map(|(_, b)| b.clone())
        else {
            continue;
        };
        for line in String::from_utf8_lossy(&bytes).lines() {
            let mut w = line.split_whitespace();
            if w.next() != Some("gametype_setting") {
                continue;
            }
            let (Some(name), Some(value)) =
                (w.next(), w.next().and_then(|v| v.parse::<f32>().ok()))
            else {
                continue;
            };
            out.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
            out.push((name.to_owned(), value));
        }
    }
    out
}

/// bo2zm M4: every UI script (`.lua` rawfile) the zones hold, in load
/// order.
/// bo2mp: the zones' config files (`*.cfg`) as text, by name (a later
/// zone's wins).
pub(super) fn ui_configs(captures: &[&ZoneCapture]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for (name, bytes) in captures.iter().flat_map(|c| c.raw_files.iter()) {
        if !name.to_ascii_lowercase().ends_with(".cfg") {
            continue;
        }
        let text = String::from_utf8_lossy(bytes).trim_end_matches('\0').to_owned();
        out.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
        out.push((name.clone(), text));
    }
    out
}

pub(super) fn ui_scripts(captures: &[&ZoneCapture]) -> Vec<(String, std::sync::Arc<[u8]>)> {
    captures
        .iter()
        .flat_map(|c| c.raw_files.iter())
        .filter(|(name, _)| name.to_ascii_lowercase().ends_with(".lua"))
        .map(|(name, bytes)| (name.clone(), std::sync::Arc::from(bytes.as_slice())))
        .collect()
}

/// bo2zm M4: the material names the UI scripts may draw: every string
/// constant in them (`RegisterMaterial("hud_chalk_1")`).
pub(super) fn ui_strings(scripts: &[(String, std::sync::Arc<[u8]>)]) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    for (_, bytes) in scripts {
        let Ok(chunk) = hks_t6::parse(bytes) else {
            continue;
        };
        let mut stack = vec![&chunk.main];
        while let Some(p) = stack.pop() {
            for k in &p.consts {
                if let hks_t6::Const::String(s) = k
                    && s.len() < 64
                {
                    out.insert(String::from_utf8_lossy(s).into_owned());
                }
            }
            stack.extend(p.protos.iter().map(|c| &**c));
        }
    }
    out
}

/// bo2zm: every `generic_filter_*` material the zones hold (patch zones
/// first: their copy wins), its pictures decoded and its constants by name.
fn screen_filters(
    captures: &[&ZoneCapture],
    packs: Option<&PackSet>,
    load_log: &mut Vec<String>,
) -> crate::T6ScreenFilters {
    let mut out: Vec<crate::T6ScreenFilter> = Vec::new();
    let Some(packs) = packs else {
        return crate::T6ScreenFilters(out);
    };
    let patched = captures.iter().filter(|c| c.zone.starts_with("patch_"));
    let rest = captures.iter().filter(|c| !c.zone.starts_with("patch_"));
    for c in patched.chain(rest) {
        for m in &c.materials {
            if !m.name.starts_with("generic_filter_") || out.iter().any(|f| f.material == m.name) {
                continue;
            }
            let mut images = Vec::new();
            for t in &m.textures {
                let Some(image) = t.image.and_then(|k| c.images.get(k.index)) else {
                    continue;
                };
                match super::t6_materials::decode(packs, image) {
                    Ok(img) => images.push((image.name.clone(), std::sync::Arc::new(img))),
                    Err(e) => load_log.push(format!("t6 screen filter {}: {}: {e}", m.name, image.name)),
                }
            }
            out.push(crate::T6ScreenFilter {
                material: m.name.clone(),
                tex_list: images,
                consts: m.constants.iter().map(|(_, n, v)| (n.clone(), *v)).collect(),
            });
        }
    }
    load_log.push(format!(
        "t6 screen filters: {}",
        out.iter()
            .map(|f| format!("{} ({} pictures)", f.material, f.tex_list.len()))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    crate::T6ScreenFilters(out)
}

pub(super) fn hud_icons(
    captures: &[&ZoneCapture],
    packs: Option<&PackSet>,
    ui_names: &std::collections::BTreeSet<String>,
    load_log: &mut Vec<String>,
) -> crate::T6HudIcons {
    let mut icons = Vec::new();
    let Some(packs) = packs else {
        return crate::T6HudIcons(icons);
    };
    // The materials the UI scripts name, beside the hand-picked ones.
    let mut wanted: Vec<String> = HUD_ICONS.iter().map(|s| (*s).to_owned()).collect();
    let mut seen: std::collections::BTreeSet<String> = wanted.iter().cloned().collect();
    for c in captures {
        for m in &c.materials {
            // Origins, Mob and Buried build their ammo corner's names at run
            // time (`hud_zm_num_tomb_` + digit), so no script names them.
            let map_hud = m.name.starts_with("hud_zm_");
            if (ui_names.contains(&m.name) || map_hud) && seen.insert(m.name.clone()) {
                wanted.push(m.name.clone());
            }
        }
    }
    // Every gun's own crosshair pieces (its weapon def's reticle materials):
    // the Ballistic Knife's `knife_ballistic_reticle` was never decoded, so
    // it drew no crosshair at all (his 10-14 "nothing in the middle").
    for c in captures {
        for w in &c.weapons {
            for (_, name) in &w.materials {
                if name.contains("reticle") && seen.insert(name.clone()) {
                    wanted.push(name.clone());
                }
            }
        }
    }
    // bo2mp: the objective markers' pictures and the carried flag's: BO2's
    // game scripts name them, built from parts ("waypoint_capture" + "_a"),
    // so no UI script holds the whole name.
    let ui_wanted = wanted.len();
    for c in captures {
        for m in &c.materials {
            if (m.name.starts_with("waypoint_") || m.name.starts_with("hudicon_")) && seen.insert(m.name.clone()) {
                wanted.push(m.name.clone());
            }
        }
    }
    load_log.push(format!("t6 hud icons the game scripts name: {}", wanted.len() - ui_wanted));
    let mut pictureless = 0usize;
    let mut failed_names: Vec<String> = Vec::new();
    let mut additive = 0usize;
    for name in &wanted {
        let name = name.as_str();
        let mut add = false;
        let mut add_map = None;
        // A patch zone's copy replaces the one it patches (`patch_ui_zm`'s
        // Theater frame is silver with light cracks, `ui_zm`'s dull).
        let patched = captures.iter().filter(|c| c.zone.starts_with("patch_"));
        let rest = captures.iter().filter(|c| !c.zone.starts_with("patch_"));
        let found = patched.chain(rest).find_map(|c| {
            let m = c.materials.iter().find(|m| m.name == name)?;
            add = super::t6_materials::is_additive(m);
            add_map = super::t6_materials::add_map(m, c);
            // BO2MP_ICON_STATES=<substring>: print the matching pictures' draw states (debugging aid).
            if let Ok(sub) = std::env::var("BO2MP_ICON_STATES")
                && name.contains(sub.as_str())
            {
                for t in 0..36 {
                    if let Some(d) = m.draw_state(t) {
                        eprintln!("icon-state {name} tech {t}: src {} dst {} op {} atest {:?} rgb {} a {}", d.src_blend, d.dst_blend, d.blend_op, d.alpha_test, d.color_write_rgb, d.color_write_alpha);
                    }
                }
            }
            let t = super::t6_materials::colour_texture(m, c)?;
            c.images.get(t.image?.index)
        });
        let Some(image) = found else {
            if HUD_ICONS.contains(&name) {
                load_log.push(format!("t6 hud icon {name}: no material"));
            }
            failed_names.push(format!("{name} (no colour map)"));
            pictureless += 1;
            continue;
        };
        // A `,name` image is a reference to one another zone defines (the
        // Create-a-Class perk pictures `perk_*_256` in ui_mp): its pixels
        // are found in the packs by the name alone.
        let decoded = super::t6_materials::decode(packs, image).or_else(|e| {
            if !image.name.starts_with(',') {
                return Err(e);
            }
            let (i, entry) = packs.locate_name(&image.name).ok_or(e)?;
            let bytes = packs.packs[i].read(entry)?;
            let iwi = ipak_t6::parse_iwi(&bytes).map_err(|e| e.to_string())?;
            super::t6_materials::gpu_image(&iwi)
        });
        match decoded {
            Ok(img) => {
                // BO2MP_ICON_DUMP=<dir>:<substring> writes the matching
                // pictures there as PNGs, as stored (a debugging aid).
                if let Ok(spec) = std::env::var("BO2MP_ICON_DUMP")
                    && let Some((dir, sub)) = spec.rsplit_once('|')
                    && name.contains(sub)
                    && let Some(plain) = super::t6_materials::to_rgba8(&img)
                    && let Some(data) = plain.data.as_ref()
                    && let Ok(file) = std::fs::File::create(format!("{dir}/{name}{}.png", if add { "_ADD" } else { "" }))
                {
                    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), plain.width(), plain.height());
                    enc.set_color(png::ColorType::Rgba);
                    enc.set_depth(png::BitDepth::Eight);
                    if let Ok(mut w) = enc.write_header() {
                        let _ = w.write_image_data(data);
                    }
                }
                // bo2mp: the HUD adds these onto the scene (true additive
                // blending), so keep the pictures as stored under
                // "additive:<name>" next to the see-through copy.
                // bo2mp vehicle screens: a picture with an AddMap (BO2's
                // `sw4_2d_color_add` pixel shader: the colour map's alpha
                // times the element's colour, plus the AddMap's colour, all
                // times its alpha) is kept packed (AddMap colour, colour
                // map alpha) under "additive:" and marked "coloradd:".
                let packed = add_map
                    .and_then(|i| super::t6_materials::decode(packs, i).ok())
                    .and_then(|added| super::t6_materials::pack_color_add(&img, &added));
                let img = match packed {
                    Some((packed, flat)) => {
                        icons.push((format!("coloradd:{name}"), std::sync::Arc::new(packed.clone())));
                        if add {
                            icons.push((format!("additive:{name}"), std::sync::Arc::new(packed)));
                        }
                        flat
                    }
                    None => {
                        if add && let Some(orig) = super::t6_materials::to_rgba8(&img) {
                            icons.push((format!("additive:{name}"), std::sync::Arc::new(orig)));
                        }
                        img
                    }
                };
                // Additive pictures (menu brackets, glows) get their
                // brightness as alpha: the 2D layer only alpha-blends.
                let img = match add
                    .then(|| super::t6_materials::additive_to_alpha(&img))
                    .flatten()
                {
                    Some(a) => {
                        additive += 1;
                        a
                    }
                    // bo2mp: the minimap's picture as plain pixels (the
                    // HUD turns it about him on the CPU).
                    None if name.starts_with("compass_map_") => super::t6_materials::to_rgba8(&img).unwrap_or(img),
                    None => img,
                };
                // The lobby's quiet speaker: BO2 draws `voice_on` without
                // its sound waves while nobody talks.
                if name == "voice_on" {
                    if let Some(quiet) = super::t6_materials::left_part(&img, 0.5) {
                        icons.push(("voice_quiet".to_owned(), std::sync::Arc::new(quiet)));
                    }
                }
                // bo2mp: the default emblem on a player's card (BO2 draws the
                // rank's emblem layer, `em_rank_pvt_full`, in the rank's
                // green): the PFC chevron as a white silhouette the
                // card tints.
                if name == "rank_pfc"
                    && let Some(mut sil) = super::t6_materials::to_rgba8(&img)
                    && let Some(data) = sil.data.as_mut()
                {
                    for px in data.chunks_exact_mut(4) {
                        px[0] = 255;
                        px[1] = 255;
                        px[2] = 255;
                    }
                    icons.push(("em_rank_pvt_full".to_owned(), std::sync::Arc::new(sil)));
                }
                icons.push((name.to_owned(), std::sync::Arc::new(img)));
            }
            Err(e) => {
                if HUD_ICONS.contains(&name) {
                    load_log.push(format!("t6 hud icon {name}: {e}"));
                }
                failed_names.push(format!("{name} ({e})"));
                pictureless += 1;
            }
        }
    }
    // bo2mp: pictures the scripts name that no material defines; BO2 finds
    // their pixels in the packs by the name alone (the green "NEW" badge on
    // a Create-a-Class / Barracks tile).
    // bo2mp: the jammed minimap's two pictures (`sam_block_noise`,
    // `tv_lookup`: the textures of each map's `compass_map_<map>_scrambled`,
    // which the HUD draws on the CPU), as plain pixels.
    let jam = ["sam_block_noise", "tv_lookup"];
    for name in ["menu_mp_lobby_new_small", jam[0], jam[1]] {
        if icons.iter().any(|(k, _)| k == name) || !(ui_names.contains(name) || jam.contains(&name)) {
            continue;
        }
        let Some((i, entry)) = packs.locate_name(name) else { continue };
        let Ok(bytes) = packs.packs[i].read(entry) else { continue };
        let Ok(iwi) = ipak_t6::parse_iwi(&bytes) else { continue };
        if let Ok(img) = super::t6_materials::gpu_image(&iwi) {
            let img = if jam.contains(&name) { super::t6_materials::to_rgba8(&img).unwrap_or(img) } else { img };
            icons.push((name.to_owned(), std::sync::Arc::new(img)));
        }
    }
    // bo2mp: each weapon's kill icon (BO2's kill feed draws it between the
    // two names), as "killicon:<weapon>:<ratio>" (WeaponDef killIcon and
    // killIconRatio: 0 square, 1 twice as wide, 2 four times).
    let mut kill_icons = 0usize;
    for c in captures {
        for w in &c.weapons {
            let Some((_, material)) = w.materials.iter().find(|(off, _)| *off == 1632) else { continue };
            let base = w.name.split('+').next().unwrap_or(&w.name).trim_end_matches("_mp").to_owned();
            let prefix = format!("killicon:{base}:");
            if base.is_empty() || icons.iter().any(|(k, _)| k.starts_with(&prefix)) {
                continue;
            }
            let ratio = w.def.get(1636..1640).map_or(0, |b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]));
            let mut add = false;
            let found = captures.iter().find_map(|c| {
                let m = c.materials.iter().find(|m| m.name == *material)?;
                add = super::t6_materials::is_additive(m);
                let t = super::t6_materials::colour_texture(m, c)?;
                c.images.get(t.image?.index)
            });
            let Some(image) = found else { continue };
            if let Ok(img) = super::t6_materials::decode(packs, image) {
                let img = add.then(|| super::t6_materials::additive_to_alpha(&img)).flatten().unwrap_or(img);
                icons.push((format!("{prefix}{ratio}"), std::sync::Arc::new(img)));
                kill_icons += 1;
            }
        }
    }
    // The pictures the engine draws when no gun's icon applies
    // (`hud_obit_death_suicide`, `_falling`,
    // `_crush`, `_grenade_round`, `hud_obit_knife`, `killicondied`,
    // `killiconheadshot`), each as a square icon under its own name.
    let mut obit_names: Vec<String> = Vec::new();
    for c in captures {
        for m in &c.materials {
            if !(m.name.starts_with("hud_obit_") || m.name.starts_with("killicon")) || obit_names.contains(&m.name) {
                continue;
            }
            obit_names.push(m.name.clone());
            let key = format!("killicon:{}:0", m.name);
            if icons.iter().any(|(k, _)| *k == key) {
                continue;
            }
            let add = super::t6_materials::is_additive(m);
            let Some(t) = super::t6_materials::colour_texture(m, c) else { continue };
            let Some(image) = t.image.and_then(|i| c.images.get(i.index)) else { continue };
            if let Ok(img) = super::t6_materials::decode(packs, image) {
                let img = add.then(|| super::t6_materials::additive_to_alpha(&img)).flatten().unwrap_or(img);
                icons.push((key, std::sync::Arc::new(img)));
                kill_icons += 1;
            }
        }
    }
    load_log.push(format!("t6 kill icons: {kill_icons}"));
    load_log.push(format!("t6 obituary icon materials: {}", obit_names.join(" ")));
    // bo2mp: each gun's scope picture (its variant's overlayMaterial, e.g.
    // `scope_overlay_svu`), as plain pixels keyed "overlay:<material>": the
    // HUD draws it over the zoomed view and samples its black frame for the
    // bars beside it.
    let mut overlays = 0usize;
    for c in captures {
        let unique_overlays = c
            .attachment_uniques
            .iter()
            .flat_map(|u| [&u.hud_layer_surface, &u.overlay_material_low]);
        for material in c
            .weapons
            .iter()
            .flat_map(|w| [&w.hud_layer_surface, &w.overlay_material_low])
            .chain(unique_overlays)
        {
            let key = format!("overlay:{material}");
            if material.is_empty() || icons.iter().any(|(k, _)| *k == key) {
                continue;
            }
            let found = captures.iter().find_map(|c| {
                let m = c.materials.iter().find(|m| m.name == *material)?;
                let t = super::t6_materials::colour_texture(m, c)?;
                c.images.get(t.image?.index)
            });
            let Some(image) = found else {
                load_log.push(format!("t6 scope overlay {material}: no picture"));
                continue;
            };
            match super::t6_materials::decode(packs, image) {
                Ok(img) => {
                    let img = super::t6_materials::to_rgba8(&img).unwrap_or(img);
                    icons.push((key, std::sync::Arc::new(img)));
                    overlays += 1;
                }
                Err(e) => load_log.push(format!("t6 scope overlay {material}: {e}")),
            }
        }
    }
    load_log.push(format!("t6 scope overlays: {overlays}"));
    // bo2mp: BO2's hit blood (`overlay_low_health_splat`, technique
    // sw4_2d_blood) is two pictures the HUD mixes: the blood's colour
    // (iw2_blood_color) and the reveal map (iw2_blood_reveal: how far in the
    // blood shows); both as plain pixels, by their image names.
    if let Some((m, c)) = captures
        .iter()
        .find_map(|c| c.materials.iter().find(|m| m.name == "overlay_low_health_splat").map(|m| (m, c)))
    {
        for t in &m.textures {
            let Some(image) = t.image.and_then(|k| c.images.get(k.index)) else { continue };
            if let Ok(img) = super::t6_materials::decode(packs, image)
                && let Some(img) = super::t6_materials::to_rgba8(&img)
            {
                icons.push((image.name.clone(), std::sync::Arc::new(img)));
            }
        }
    }
    if !failed_names.is_empty() {
        load_log.push(format!(
            "t6 hud icons without a picture: {}",
            failed_names.join(", ")
        ));
    }
    load_log.push(format!(
        "t6 hud icons: {} of {} ({} hand-picked, the rest the UI scripts name; {pictureless} without a picture; {additive} additive made see-through)",
        icons.len(),
        wanted.len(),
        HUD_ICONS.len()
    ));
    crate::T6HudIcons(icons)
}

/// Nuketown's weapons: every base gun the box and walls give, the starting
/// pistol, knives and equipment.
const NUKETOWN_WEAPONS: [&str; 40] = [
    "m1911_zm",
    "beretta93r_zm",
    "fiveseven_zm",
    "fivesevendw_zm",
    "kard_zm",
    "python_zm",
    "judge_zm",
    "ak74u_zm",
    "mp5k_zm",
    "qcw05_zm",
    "870mcs_zm",
    "rottweil72_zm",
    "saiga12_zm",
    "srm1216_zm",
    "m14_zm",
    "m16_zm",
    "fnfal_zm",
    "galil_zm",
    "hk416_zm",
    "saritch_zm",
    "tar21_zm",
    "type95_zm",
    "xm8_zm",
    "hamr_zm",
    "lsat_zm",
    "rpd_zm",
    "barretm82_zm",
    "dsr50_zm",
    "ray_gun_zm",
    "raygun_mark2_zm",
    "knife_ballistic_zm",
    "m32_zm",
    "usrpg_zm",
    "knife_zm",
    "bowie_knife_zm",
    "tazer_knuckles_zm",
    "frag_grenade_zm",
    "sticky_grenade_zm",
    "claymore_zm",
    "cymbal_monkey_zm",
];

/// The M2 census: every Nuketown weapon complete (first-person and world
/// models, launcher and melee models, magazine and scope attachments,
/// animations), and nothing the weapons,
/// their animations, the impact table or the map scripts name missing from
/// the effects or the sound bank. A sound BO2's own banks lack (an
/// animation's cue Nuketown never shipped) is silent in BO2 too: counted
/// apart, as is an alias BO2 ships without audio.
#[allow(clippy::too_many_arguments)]
fn census(
    captures: &[&ZoneCapture],
    name_list: &[String],
    weapon_refs: &HashMap<String, &asset_t6::WeaponRef>,
    models_missing: &[String],
    animation_refs: &XAnimBuild,
    fx: &EffectRoster,
    impact: Option<&SelfContainedImpactFxTable>,
    map_fx: &asset_audio::ScriptedMapFx,
    room_tone: Option<&str>,
    sound: Option<&asset_audio::AudioRoster>,
) -> Vec<String> {
    use fastfile_t6::layout::WeaponDef;
    let ns = asset_core::AssetCategoryScope::T6;
    let mut missing_weapons: Vec<String> = Vec::new();
    let mut missing_fx: Vec<String> = Vec::new();
    let mut missing_sounds: Vec<String> = Vec::new();
    let mut absent_in_bo2: Vec<String> = Vec::new();
    let mut silent: usize = 0;
    let (mut fx_n, mut snd_n) = (0usize, 0usize);
    let anim_notifies: HashMap<String, &asset_t6::XAnimRef> = captures
        .iter()
        .flat_map(|c| c.animation_refs.iter())
        .filter(|a| !a.name.starts_with(','))
        .map(|a| (a.name.to_ascii_lowercase(), a))
        .collect();
    let aliases: HashMap<String, &asset_audio::SnapshotSoundEntry> = sound
        .map(|cat| {
            cat.sounds
                .iter()
                .map(|s| (s.name.to_ascii_lowercase(), s))
                .collect()
        })
        .unwrap_or_default();
    let mut check_fx = |name: &str, what: &str, missing: &mut Vec<String>| {
        if name.is_empty() {
            return;
        }
        fx_n += 1;
        if fx.get_in(ns, name.trim_start_matches(',')).is_none() {
            missing.push(format!("{what}: {name}"));
        }
    };
    // A sound named by the data: in the bank with audio (or shipped silent),
    // else absent from BO2's own banks (silent in BO2 too). Missing means
    // in the bank but no audio found: a failure here.
    let mut check_sound = |name: &str, _from_anim: bool, what: &str| {
        if name.is_empty() {
            return;
        }
        snd_n += 1;
        match aliases.get(&name.to_ascii_lowercase()) {
            None => absent_in_bo2.push(name.to_owned()),
            Some(sound) => {
                let audible = sound.aliases.iter().any(|row| row.streamed.is_some());
                let shipped_silent = sound.aliases.iter().all(|row| {
                    row.streamed.is_none() && row.clip_file.as_deref().unwrap_or("").is_empty()
                });
                if !audible {
                    if shipped_silent {
                        silent += 1;
                    } else {
                        missing_sounds.push(format!("{what}: {name} (no audio)"));
                    }
                }
            }
        }
    };
    for name in name_list.iter().map(String::as_str) {
        let Some(w) = weapon_refs.get(name) else {
            missing_weapons.push(format!("{name}: not in the zones"));
            continue;
        };
        let mut missing_list = Vec::new();
        for model in w
            .gun_models
            .first()
            .into_iter()
            .chain(w.world_models.first())
            .map(String::as_str)
            .chain(
                [WeaponDef::rocketModel, WeaponDef::projectileModel, WeaponDef::additionalMeleeModel]
                    .map(|f| w.def_model(f)),
            )
            .chain(w.attach_view_models.iter().map(|a| a.1.as_str()))
            .filter(|m| !m.is_empty() && !m.starts_with(','))
        {
            if models_missing.iter().any(|m| m == model) {
                missing_list.push(format!("model {model}"));
            }
        }
        for anim in w.animation_refs.iter().filter(|a| !a.is_empty()) {
            let key = anim.trim_start_matches(',').to_ascii_lowercase();
            if animation_refs.get(ns, &key).is_none() {
                missing_list.push(format!("anim {anim}"));
            }
            if let Some(a) = anim_notifies.get(&key) {
                for (note, _) in &a.notifies {
                    if let Some(alias) = note.strip_prefix("sndnt#") {
                        check_sound(alias, true, name);
                    }
                }
            }
        }
        if !missing_list.is_empty() {
            missing_weapons.push(format!("{name}: {}", missing_list.join(", ")));
        }
        for field in [
            WeaponDef::viewFlashEffect,
            WeaponDef::worldFlashEffect,
            WeaponDef::viewShellEjectEffect,
            WeaponDef::worldShellEjectEffect,
            WeaponDef::viewLastShotEjectEffect,
            WeaponDef::worldLastShotEjectEffect,
            WeaponDef::projExplosionEffect,
            WeaponDef::projTrailEffect,
            WeaponDef::projIgnitionEffect,
        ] {
            check_fx(w.def_fx(field), name, &mut missing_fx);
        }
        for field in [
            WeaponDef::fireSound,
            WeaponDef::fireSoundPlayer,
            WeaponDef::emptyFireSound,
            WeaponDef::emptyFireSoundPlayer,
            WeaponDef::reloadSound,
            WeaponDef::reloadSoundPlayer,
            WeaponDef::reloadEmptySound,
            WeaponDef::reloadEmptySoundPlayer,
            WeaponDef::raiseSound,
            WeaponDef::raiseSoundPlayer,
            WeaponDef::putawaySound,
            WeaponDef::putawaySoundPlayer,
            WeaponDef::projExplosionSound,
            WeaponDef::meleeSwipeSoundPlayer,
            WeaponDef::meleeSwipeSound,
            WeaponDef::meleeHitSound,
            WeaponDef::meleeMissSound,
            WeaponDef::pullbackSoundPlayer,
        ] {
            check_sound(w.def_string(field), false, name);
        }
        for alias in &w.bounce_sounds {
            check_sound(alias, false, name);
        }
        // A map value may name the player's alias then the others'.
        for (_, value) in &w.notetrack_sounds {
            for alias in value.split_whitespace() {
                check_sound(alias, false, name);
            }
        }
    }
    if let Some(table) = impact {
        for row in 0..table.line_total() {
            for surf in 0..40 {
                if let Some(n) = table.effect_label(row, surf, Some(0)) {
                    check_fx(n.name, "impact table", &mut missing_fx);
                }
            }
            for flesh in 0..8 {
                if let Some(n) = table.effect_label(row, 7, Some(flesh)) {
                    check_fx(n.name, "impact table", &mut missing_fx);
                }
            }
        }
    }
    for shot in &map_fx.oneshots {
        check_fx(&shot.effect_id, "map", &mut missing_fx);
    }
    for l in &map_fx.loop_sounds {
        check_sound(&l.cue_alias, false, "map loop");
    }
    for r in &map_fx.random_sounds {
        check_sound(&r.cue_alias, false, "map");
    }
    if let Some(tone) = room_tone {
        check_sound(tone, false, "room tone");
    }
    for gait in [
        "sprint",
        "run",
        "walk",
        "prone",
        "crouch_run",
        "crouch_walk",
    ] {
        for surf in [
            "default", "asphalt", "concrete", "dirt", "metal", "wood", "grass", "gravel",
        ] {
            check_sound(&format!("fly_step_{gait}_plr_{surf}"), false, "footsteps");
        }
    }
    for surf in [
        "default", "asphalt", "concrete", "dirt", "metal", "wood", "grass", "gravel",
    ] {
        check_sound(&format!("fly_land_plr_{surf}"), false, "landings");
    }
    missing_fx.sort();
    missing_fx.dedup();
    missing_sounds.sort();
    missing_sounds.dedup();
    absent_in_bo2.sort();
    absent_in_bo2.dedup();
    vec![
        format!(
            "t6 census: {} missing weapons of {} (models, first person, animations){}",
            missing_weapons.len(),
            name_list.len(),
            if missing_weapons.is_empty() {
                String::new()
            } else {
                format!(": {missing_weapons:?}")
            }
        ),
        format!(
            "t6 census: {} missing FX of {fx_n} named (weapons, impacts, map){}",
            missing_fx.len(),
            if missing_fx.is_empty() {
                String::new()
            } else {
                format!(": {:?}", &missing_fx[..missing_fx.len().min(12)])
            }
        ),
        format!(
            "t6 census: {} missing sounds of {snd_n} named (weapons, animations, footsteps, landings, map){}; {} names BO2's own banks lack (silent in BO2 too) {:?}; {silent} shipped silent",
            missing_sounds.len(),
            if missing_sounds.is_empty() {
                String::new()
            } else {
                format!(": {:?}", &missing_sounds[..missing_sounds.len().min(12)])
            },
            absent_in_bo2.len(),
            &absent_in_bo2[..absent_in_bo2.len().min(8)],
        ),
    ]
}

/// The map's compiled client scripts, read for its placed effects and
/// ambience (later zones' copies win: the patch zone fixes the map's).
fn map_scripts(
    captures: &[&ZoneCapture],
    map: &str,
    load_log: &mut Vec<String>,
) -> (asset_audio::ScriptedMapFx, Option<String>) {
    let facts = asset_t6::MapScriptFacts::read(map, |name| {
        let want = format!("script:{name}");
        captures.iter().rev().find_map(|c| {
            c.raw_files
                .iter()
                .find(|(n, _)| *n == want)
                .map(|(_, bytes)| bytes.clone())
        })
    });
    let shown: Vec<&asset_t6::PlacedFx> = facts
        .placed
        .iter()
        .filter(|p| p.exploder.is_none() && p.kind != "exploder")
        .collect();
    // BO2MP_T6_PLACEDLOG=1: each shown placed effect (test aid).
    if std::env::var_os("BO2MP_T6_PLACEDLOG").is_some() {
        for p in &shown {
            load_log.push(format!(
                "t6 placed fx {} = {} {} at {:?} angles {:?} delay {}",
                p.fx_key,
                facts.effects.get(&p.fx_key).map_or("?", String::as_str),
                p.kind,
                p.origin,
                p.angles,
                p.delay
            ));
        }
    }
    let oneshots: Vec<asset_audio::SpawnSingleShotEffect> = shown
        .iter()
        .map(|p| asset_audio::SpawnSingleShotEffect {
            effect_id: facts
                .effects
                .get(&p.fx_key)
                .cloned()
                .unwrap_or_else(|| p.fx_key.clone()),
            position_in_inches: p.origin,
            rotation_degrees: p.angles,
            // A loop effect's delay is its repeat time, not a pre-roll.
            start_after: if p.kind == "loopfx" { 0.0 } else { p.delay },
        })
        .collect();
    let mut loop_sounds: Vec<asset_audio::SpawnEffectAudioLoop> = Vec::new();
    for (fxid, alias, off) in &facts.sounds_on_fx {
        for p in shown.iter().filter(|p| &p.fx_key == fxid) {
            loop_sounds.push(asset_audio::SpawnEffectAudioLoop {
                cue_alias: alias.clone(),
                position_in_inches: [0, 1, 2].map(|a| p.origin[a] + off[a]),
                line_end_inches: None,
            });
        }
    }
    for (alias, origin) in &facts.loops_at {
        loop_sounds.push(asset_audio::SpawnEffectAudioLoop {
            cue_alias: alias.clone(),
            position_in_inches: *origin,
            line_end_inches: None,
        });
    }
    let mut random_sounds: Vec<asset_audio::RandomPointSound> = facts
        .random_sounds
        .iter()
        .map(|(alias, wait, places)| asset_audio::RandomPointSound {
            cue_alias: alias.clone(),
            wait_secs: *wait,
            places_inches: places.clone(),
        })
        .collect();
    let (structs_random, structs_loop) =
        map_sound_structs(captures, &mut random_sounds, &mut loop_sounds);
    let unresolved = shown
        .iter()
        .filter(|p| !facts.effects.contains_key(&p.fx_key))
        .count();
    load_log.push(format!(
        "t6 map sound structs: {structs_random} now-and-then (wind gusts, creaks), {structs_loop} loops (power lines, wind lines, arcs)"
    ));
    load_log.push(format!(
        "t6 map scripts: {} placed effects ({} shown at load, {} exploders wait for the game, {unresolved} without an effect table entry); {} loop sounds ({} on effects, {} at points); {} now-and-then sounds; room tone {}",
        facts.placed.len(),
        oneshots.len(),
        facts.placed.len() - shown.len(),
        loop_sounds.len(),
        loop_sounds.len() - facts.loops_at.len(),
        facts.loops_at.len(),
        random_sounds.len(),
        facts.room_tone.as_deref().unwrap_or("none"),
    ));
    load_log.push(format!("t6 map rooms (room, echo, dry, wet): {:?}", facts.room_echoes));
    (
        asset_audio::ScriptedMapFx {
            oneshots,
            loop_sounds,
            random_sounds,
            rooms: facts.room_echoes.clone(),
        },
        facts.room_tone,
    )
}

/// bo2mp: each destructible definition's model, by definition name.
fn destructible_models(captures: &[&ZoneCapture]) -> HashMap<String, String> {
    let mut defs = HashMap::new();
    for c in captures {
        for d in &c.destructibles {
            if real(&d.name) && real(&d.model) {
                defs.insert(d.name.to_ascii_lowercase(), d.model.clone());
            }
        }
    }
    defs
}

/// bo2mp: each destructible definition the server breaks (pieces, stages,
/// what each break plays and tells the scripts), later zones replacing
/// earlier; client-only ones are left to the clients.
fn destructible_defs(captures: &[&ZoneCapture]) -> Vec<std::sync::Arc<xmodel_runtime::BreakableModelDef>> {
    let text = |s: &str| (!s.is_empty()).then(|| s.to_owned());
    let mut defs: Vec<std::sync::Arc<xmodel_runtime::BreakableModelDef>> = Vec::new();
    for c in captures {
        for d in &c.destructibles {
            if !real(&d.name) || !real(&d.model) || d.pieces.is_empty() || d.local_machine_only {
                continue;
            }
            let pieces = d
                .pieces
                .iter()
                .map(|p| xmodel_runtime::BreakableChunk {
                    stages: p.stages.clone().map(|st| xmodel_runtime::BreakableStage {
                        spawn_presets: st.spawn_models.clone().map(|m| debris_preset(captures, &m)),
                        reveal_joint: text(&st.reveal_joint),
                        shatter_hitpoints: st.shatter_hitpoints,
                        longest_duration: st.longest_duration,
                        flags: st.flags,
                        shatter_visual: text(&st.shatter_visual),
                        shatter_audio: text(&st.shatter_audio),
                        shatter_signal: text(&st.shatter_signal),
                        repeating_audio: text(&st.repeating_audio),
                        uses_physics_preset: st.physics_template,
                        debris_models: st.spawn_models.clone().map(|m| text(&m)),
                    }),
                    owner_fragment: p.owner_fragment,
                    inherited_harm_ratio: p.inherited_harm_ratio,
                    round_damage_multiplier: p.round_damage_multiplier,
                    blast_damage_multiplier: p.blast_damage_multiplier,
                    melee_damage_scale: p.melee_damage_scale,
                    impact_damage_scale: p.impact_damage_scale,
                    entity_damage_transfer: p.entity_damage_transfer,
                    health: p.health,
                    conceal_joints: p.conceal_joints,
                })
                .collect();
            defs.retain(|x| !x.name.eq_ignore_ascii_case(&d.name));
            defs.push(std::sync::Arc::new(xmodel_runtime::BreakableModelDef {
                name: d.name.clone(),
                model: d.model.clone(),
                pieces,
                local_machine_only: false,
            }));
        }
    }
    defs
}

/// codmp: Black Ops II's loose and breakable props (the clipmap's
/// `dynEntDefList`: bottles, cones, crates, TVs, lamps), which the engine
/// keeps outside the entity string. Each becomes a `script_model` with a
/// `destructibledef` of its own (`dyn_<i>`): a clutter prop (kind 1) comes
/// loose and flies off with its physics preset the first time it is hit; a
/// destruct prop (kind 2) takes its health, then plays its destroy effect
/// and turns into its destroyed model (`codmp_dyn_destroyed`), or is gone.
fn dyn_ent_props(
    c: &ZoneCapture,
    captures: &[&ZoneCapture],
) -> (String, Vec<std::sync::Arc<xmodel_runtime::BreakableModelDef>>) {
    use xmodel_runtime::{DebrisPreset, BreakableModelDef, BreakableChunk, BreakableStage};
    let mut text = String::new();
    let mut defs = Vec::new();
    let Some(clip) = &c.clip else {
        return (text, defs);
    };
    // A name with a leading comma points at another zone's preset (the
    // pucks' ",bottle_plastic" is `common_mp`'s); the map's own copy of it
    // is an empty stand-in.
    let preset = |name: &str, model: &str| -> Option<DebrisPreset> {
        let name = name.trim_start_matches(',');
        captures
            .iter()
            .rev()
            .flat_map(|z| z.phys_presets.iter())
            .find(|p| !name.is_empty() && p.name.trim_start_matches(',') == name && p.mass > 0.0)
            .map(|p| DebrisPreset {
                mass: p.mass,
                bounce: p.bounce,
                friction: p.friction,
                bullet_push: p.bullet_push,
                blast_push: p.blast_push,
                gravity_scale: p.gravity_scale,
            })
            .or_else(|| debris_preset(captures, model))
    };
    for (i, d) in clip.debris_pieces.iter().enumerate() {
        let model = d.model.trim_start_matches(',');
        if !real(model) || !(d.kind == 1 || d.kind == 2) {
            continue;
        }
        let destroyed = d.destroyed_model.trim_start_matches(',');
        let name = format!("dyn_{i}");
        let clutter = d.kind == 1;
        let health = if clutter { 1 } else { d.health.clamp(1, i32::from(i16::MAX)) };
        // (A stage with no part to show counts as already broken, so the
        // whole prop names its origin, as the guest props do.)
        let mut whole = BreakableStage {
            reveal_joint: Some("tag_origin".to_owned()),
            shatter_hitpoints: 1.0 / health as f32,
            shatter_visual: (!d.destroy_effect.is_empty()).then(|| d.destroy_effect.clone()),
            ..Default::default()
        };
        // bo2mp: a clutter prop is one object: hit, it flies off as its
        // broken model (BO2 DynEntCl_GetCurrentXModel shows destroyedModel
        // once it is destroyed, and a clutter one is at its first hit), and
        // its place is empty. It threw its whole model before, while the
        // broken one stayed on the wall: two fire extinguishers.
        let thrown = if clutter && real(destroyed) { destroyed } else { model };
        if clutter {
            whole.uses_physics_preset = true;
            whole.debris_models[0] = Some(thrown.to_owned());
            whole.spawn_presets[0] = preset(&d.physics_template, model);
        }
        defs.push(std::sync::Arc::new(BreakableModelDef {
            name: name.clone(),
            model: model.to_owned(),
            pieces: vec![BreakableChunk {
                stages: [
                    whole,
                    BreakableStage {
                        shatter_hitpoints: -1.0,
                        ..Default::default()
                    },
                    BreakableStage::default(),
                    BreakableStage::default(),
                    BreakableStage::default(),
                ],
                owner_fragment: u8::MAX,
                inherited_harm_ratio: 0.0,
                round_damage_multiplier: 1.0,
                blast_damage_multiplier: 1.0,
                melee_damage_scale: 1.0,
                impact_damage_scale: 1.0,
                entity_damage_transfer: 0.0,
                health,
                conceal_joints: [0; 5],
            }],
            local_machine_only: false,
        }));
        let angles = quat_angles(d.rotation);
        let [x, y, z] = d.origin;
        text.push_str(&format!(
            "{{\n\"classname\" \"script_model\"\n\"model\" \"{model}\"\n\"origin\" \"{x} {y} {z}\"\n\"angles\" \"{} {} {}\"\n\"destructibledef\" \"{name}\"\n",
            angles[0], angles[1], angles[2]
        ));
        if real(destroyed) && !clutter {
            text.push_str(&format!("\"codmp_dyn_destroyed\" \"{destroyed}\"\n"));
        }
        text.push_str("}\n");
    }
    (text, defs)
}

/// Pitch, yaw and roll (degrees) of a placement's quaternion (x, y, z, w).
fn quat_angles([x, y, z, w]: [f32; 4]) -> [f32; 3] {
    let fwd = [
        1.0 - 2.0 * (y * y + z * z),
        2.0 * (x * y + w * z),
        2.0 * (x * z - w * y),
    ];
    let left_z = 2.0 * (y * z + w * x);
    let up_z = 1.0 - 2.0 * (x * x + y * y);
    let pitch = (-fwd[2]).atan2(fwd[0].hypot(fwd[1])).to_degrees();
    let yaw = fwd[1].atan2(fwd[0]).to_degrees();
    let roll = left_z.atan2(up_z).to_degrees();
    [pitch, yaw, roll]
}

/// bo2mp: how a destructible's spawn model flies once it breaks off: its
/// model's `physPreset`, the latest zone's copy of the model winning.
fn debris_preset(captures: &[&ZoneCapture], mdl: &str) -> Option<xmodel_runtime::DebrisPreset> {
    if mdl.is_empty() {
        return None;
    }
    captures.iter().rev().find_map(|c| {
        let p = c
            .xmodels
            .iter()
            .filter(|x| x.name.trim_start_matches(',') == mdl)
            .find_map(|x| x.physics_template)
            .and_then(|k| c.phys_presets.get(k.index))?;
        (p.mass > 0.0).then_some(xmodel_runtime::DebrisPreset {
            mass: p.mass,
            bounce: p.bounce,
            friction: p.friction,
            bullet_push: p.bullet_push,
            blast_push: p.blast_push,
            gravity_scale: p.gravity_scale,
        })
    })
}

/// bo2mp: a map's entity text with each destructible built from its
/// definition. An entity with a `destructibledef` carries the editor's
/// stand-in in `model` (`veh_t6_police_car`, no such model in any zone);
/// the engine builds it from the definition's model, so that is what the
/// entity is given here.
fn destructible_entities(words: &str, defs: &HashMap<String, String>) -> String {
    if defs.is_empty() || !words.contains("\"destructibledef\"") {
        return words.into();
    }
    let mut out = String::with_capacity(words.len());
    let mut block: Vec<&str> = Vec::new();
    let flush = |block: &mut Vec<&str>, out: &mut String| {
        let model = block
            .iter()
            .find_map(|l| {
                let mut q = l.split('"').skip(1).step_by(2);
                (q.next()?.eq_ignore_ascii_case("destructibledef")).then(|| q.next())?
            })
            .and_then(|d| defs.get(&d.to_ascii_lowercase()));
        for l in block.drain(..) {
            let is_model = l.split('"').nth(1).is_some_and(|k| k.eq_ignore_ascii_case("model"));
            match model {
                Some(m) if is_model => {
                    out.push_str(&format!("\"model\" \"{m}\""));
                }
                _ => out.push_str(l),
            }
            out.push_str("\n");
        }
    };
    for row in words.lines() {
        block.push(row);
        if row.trim() == "}" {
            flush(&mut block, &mut out);
        }
    }
    flush(&mut block, &mut out);
    out
}

/// The map's sound structs, which every map's client script
/// (clientscripts/mp/_audio.csc) starts: a "random" one plays its sound at
/// its place every `randomfloatrange(script_wait_min, script_wait_max)` s
/// (1 to 3 by default), a "looper" loops it there, a "line_emitter" loops
/// it along the line to its target. Returns (random, loops) added.
fn map_sound_structs(
    captures: &[&ZoneCapture],
    random: &mut Vec<asset_audio::RandomPointSound>,
    loops: &mut Vec<asset_audio::SpawnEffectAudioLoop>,
) -> (usize, usize) {
    let Some(ents) = captures.iter().rev().find(|c| !c.map_ents.is_empty()) else {
        return (0, 0);
    };
    let ents: Vec<_> = ents
        .map_ents
        .iter()
        .flat_map(|e| asset_t6::parse_entities(e))
        .collect();
    let (mut nr, mut nl) = (0, 0);
    for e in &ents {
        if e.classname() != "script_struct" {
            continue;
        }
        let (Some(alias), Some(origin)) = (e.get("script_sound"), e.vec3("origin")) else {
            continue;
        };
        let alias = alias.to_owned();
        match e.get("script_label") {
            Some("random") => {
                let f = |k: &str, d: f32| e.get(k).and_then(|v| v.trim().parse().ok()).unwrap_or(d);
                random.push(asset_audio::RandomPointSound {
                    cue_alias: alias,
                    wait_secs: [f("script_wait_min", 1.0), f("script_wait_max", 3.0)],
                    places_inches: vec![origin],
                });
                nr += 1;
            }
            Some("looper") => {
                loops.push(asset_audio::SpawnEffectAudioLoop {
                    cue_alias: alias,
                    position_in_inches: origin,
                    line_end_inches: None,
                });
                nl += 1;
            }
            Some("line_emitter") => {
                let end = e.get("target").and_then(|t| {
                    ents.iter()
                        .find(|o| o.get("targetname") == Some(t))
                        .and_then(|o| o.vec3("origin"))
                });
                loops.push(asset_audio::SpawnEffectAudioLoop {
                    cue_alias: alias,
                    position_in_inches: origin,
                    line_end_inches: end,
                });
                nl += 1;
            }
            _ => {}
        }
    }
    (nr, nl)
}

/// What a bullet shows on a model's bone boxes: its paint's surface (a
/// mannequin's is plastic), the one most of its surfaces use; 0 (blood) when
/// none has one.
fn model_surface_flags(capture: &ZoneCapture, mdl: &asset_t6::XModelRef) -> u32 {
    let mut counts: BTreeMap<u32, usize> = BTreeMap::new();
    for srf in &mdl.lod0 {
        let flags = srf
            .material
            .and_then(|k| capture.material(k))
            .map_or(0, |m| m.surf_bits);
        if flags != 0 {
            *counts.entry(flags).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .max_by_key(|&(_, n)| n)
        .map_or(0, |(flags, _)| flags)
}

/// bo2zm M3: every XModel the zones carry whose name a server script
/// (as a string constant), a map entity (`"model"`) or a destructible's
/// broken-off piece names.
fn script_model_names(
    captures: &[&ZoneCapture],
    destructibles: &HashMap<String, String>,
) -> BTreeSet<String> {
    let models: BTreeSet<&str> = captures
        .iter()
        .flat_map(|c| c.xmodels.iter().map(|m| m.name.as_str()))
        .filter(|n| real(n))
        .collect();
    let mut named = BTreeSet::new();
    let mut take = |s: &str| {
        if models.contains(s) {
            named.insert(s.to_owned());
        }
    };
    for c in captures {
        for (name, bytes) in &c.raw_files {
            if !name.starts_with("script:") || !name.ends_with(".gsc") {
                continue;
            }
            for token in bytes.split(|&b| b == 0) {
                if (3..96).contains(&token.len()) && token.iter().all(|b| b.is_ascii_graphic()) {
                    take(&String::from_utf8_lossy(token));
                }
            }
        }
        // Every gun's world model (the box's floating gun, bought wall guns).
        for w in &c.weapons {
            for m in &w.world_models {
                take(m);
            }
            // And the parts it wears there (magazine, scope, the RPG's
            // rocket): BO2 hangs them on the gun (`attachWorldModel`).
            for (_, m, ..) in &w.attach_world_models {
                take(m);
            }
        }
        // The pieces a breakable prop throws (a mannequin's head and arms).
        for d in &c.destructibles {
            for p in &d.pieces {
                for st in &p.stages {
                    for m in st.spawn_models.iter().filter(|m| !m.is_empty()) {
                        take(m);
                    }
                }
            }
        }
        for d in &c.destructibles {
            for st in d.pieces.iter().flat_map(|p| &p.stages) {
                for m in &st.spawn_models {
                    take(m);
                }
            }
        }
        // codmp: the clipmap's loose and breakable props (`dyn_ent_props`).
        for d in c.clip.iter().flat_map(|clip| &clip.debris_pieces) {
            take(d.model.trim_start_matches(','));
            take(d.destroyed_model.trim_start_matches(','));
        }
        for ents in &c.map_ents {
            for e in asset_t6::parse_entities(&destructible_entities(ents, destructibles)) {
                if let Some(m) = e.get("model").filter(|m| !m.starts_with('*')) {
                    take(m);
                }
                // A zbarrier's pieces (the magic box) name their models.
                for i in 1..=8 {
                    if let Some(m) = e.get(&format!("zbarrierboardmodel{i}")) {
                        take(m);
                    }
                }
            }
        }
    }
    named
}

/// The zones around the map zone, in load order, that exist beside it.
fn zone_paths(level_file_path: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
    // bo2zm: a map from an extra folder (`BO2MP_T6_EXTRA`) still loads the
    // install's common zones.
    let dir = asset_transport::t6_extra::install_zone_dir(level_file_path);
    let dir = dir.as_path();
    let stem = stem_text(level_file_path);
    let shared: &[&str] = if is_mp_map(&stem) {
        &T6_MP_PRE_ZONES
    } else {
        &T6_ZM_PRE_ZONES
    };
    let zm_pre;
    let shared: &[&str] = if is_mp_map(&stem) {
        shared
    } else {
        zm_pre = zm_pre_zones(frame::bo2_frontend().is_some());
        &zm_pre
    };
    let mut pre: Vec<PathBuf> = shared
        .iter()
        .map(|z| dir.join(format!("{z}.ff")))
        .collect();
    if !is_mp_map(&stem) {
        let front_end = frame::bo2_frontend().is_some();
        pre.extend(zm_load_zones(&stem, front_end).iter().map(|z| dir.join(format!("{z}.ff"))));
    }
    // A map from an extra folder (Zombies Declassified) loads its mod's
    // zones (weapons, string tables, menus) before the map, as its game does.
    if level_file_path.parent() != Some(dir) {
        pre.extend(
            ["mod.ff", "mod_load.ff", "mod_patch.ff"]
                .iter()
                .filter_map(|z| asset_transport::t6_extra::extra_file(z)),
        );
    }
    // A multiplayer map has no patch zone (its fixes are in patch_mp).
    let post = if is_mp_map(&stem) {
        Vec::new()
    } else {
        // The mode zone (`so_zclassic_zm_buried.ff`: the map's zombie types,
        // their characters and animstatedefs, the mode's extra entities), then
        // the patch. Nuketown plays zstandard and has none. A survival start
        // (Tranzit's Town: zstandard) takes the zsurvival zone, Grief and
        // Turned zencounter (Borough's zombie hands and melee are only there).
        vec![
            asset_transport::t6_extra::file_in(dir, &format!("{}.ff", mode_zone(&stem))),
            asset_transport::t6_extra::file_in(dir, &format!("{stem}_patch.ff")),
        ]
    };
    let exists = |v: Vec<PathBuf>| v.into_iter().filter(|p| p.is_file()).collect();
    (exists(pre), exists(post))
}

/// bo2mp: the Zombies zones whose wonder weapons the mod menu gives in a
/// multiplayer match (the Jet Gun and the shield on TranZit, the
/// Sliquifier on Die Rise, the Paralyzer and Time Bomb on Buried, the
/// Blundergat and the tomahawks on Mob, the staffs on Origins, the Ray
/// Gun Mark II on Nuketown; the Ray Gun, the Ballistic Knife and every
/// Pack-a-Punched gun across them).
/// `BO2MP_ZM_WEAPONS=0` leaves them out.
const T6_MP_ZM_WEAPON_ZONES: [&str; 14] = [
    "common_zm",
    // The Pack-a-Punch camo table (`mp/weaponoptions_zm.csv`).
    "patch_zm",
    "zm_nuked",
    "zm_nuked_patch",
    "zm_transit",
    "zm_transit_patch",
    "zm_highrise",
    "zm_highrise_patch",
    "zm_buried",
    "zm_buried_patch",
    "zm_prison",
    "zm_prison_patch",
    "zm_tomb",
    "zm_tomb_patch",
];

/// bo2mp: the English zones holding the Zombies guns' names.
const T6_MP_ZM_LANG_ZONES: [&str; 8] = [
    "code_post_gfx_zm",
    "patch_zm",
    "zm_nuked",
    // TranZit's own text is in its game mode's zone (en_zm_transit is empty).
    // The Monkey Bomb, EMP and G-Strike names have no text anywhere in BO2.
    "so_zclassic_zm_transit",
    "zm_highrise",
    "zm_buried",
    "zm_prison",
    "zm_tomb",
];

fn zm_weapon_zone_paths(level_file_path: &Path) -> Vec<PathBuf> {
    let stem = level_file_path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    if !is_mp_map(&stem) || std::env::var("BO2MP_ZM_WEAPONS").is_ok_and(|v| v == "0") {
        return Vec::new();
    }
    let dir = level_file_path.parent().unwrap_or(Path::new("."));
    T6_MP_ZM_WEAPON_ZONES
        .iter()
        .map(|z| dir.join(format!("{z}.ff")))
        .filter(|p| p.is_file())
        .collect()
}

/// bo2mp: zones captured side by side, at most `ZONE_WORKERS` at once,
/// each result in its path's place. A zone's capture is its decryption,
/// inflation and walk, all on the CPU: one after another, the menu's 14
/// Zombies zones alone held a Carrier load up ~24 s.
const ZONE_WORKERS: usize = 6;

fn capture_many(paths: &[PathBuf]) -> Vec<Result<ZoneCapture, String>> {
    capture_many_with(paths, |_, r| r)
}

/// `capture_many`, each capture handed to `keep` (with its path's place) on
/// its worker as soon as it is read, so a zone cut down there never waits
/// whole beside the others.
fn capture_many_with<T: Send>(
    paths: &[PathBuf],
    keep: impl Fn(usize, Result<ZoneCapture, String>) -> T + Sync,
) -> Vec<T> {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let next = AtomicUsize::new(0);
    let slots: Vec<Mutex<Option<T>>> = paths.iter().map(|_| Mutex::new(None)).collect();
    std::thread::scope(|s| {
        for _ in 0..ZONE_WORKERS.min(paths.len()) {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(path) = paths.get(i) else { break };
                    let r = keep(i, asset_t6::capture_zone(path));
                    *slots[i].lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(r);
                }
            });
        }
    });
    slots
        .into_iter()
        .enumerate()
        .map(|(i, m)| {
            m.into_inner()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .unwrap_or_else(|| keep(i, Err("zone not captured".to_owned())))
        })
        .collect()
}

/// bo2mp: the weapons `getretrievableweapons` lists, sorted: each weapon
/// `(name, bRetrievable)` that the match's own zones define (`match_weapons`).
/// A path's file name without its extension, as text (empty when it has none).
fn stem_text(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

fn retrievable_weapons<'a>(
    weapons: impl IntoIterator<Item = (&'a str, bool)>,
    match_weapons: &BTreeSet<&str>,
) -> Vec<String> {
    let mut pickups: Vec<String> = weapons
        .into_iter()
        .filter(|&(n, retrievable)| retrievable && match_weapons.contains(n))
        .map(|(n, _)| n.to_ascii_lowercase())
        .collect();
    pickups.sort_unstable();
    pickups
}

/// bo2mp: a Zombies zone kept for its weapons only: their models, anims,
/// materials, effects, tracers, camos and sounds. Its map, scripts, tables,
/// text, vehicles and surface tables are dropped, so nothing of Zombies
/// runs or replaces the match's own. These zones load first, so the
/// match's own assets win any shared name.
fn weapons_only(mut c: ZoneCapture) -> ZoneCapture {
    c.world = None;
    c.clip = None;
    c.key_lights.clear();
    c.raw_files.clear();
    c.string_tables.retain(|t| t.name == "mp/weaponoptions_zm.csv");
    // Only the Zombies guns' names (the match's own text loads after).
    c.localize
        .retain(|(key, _)| key.starts_with("ZMWEAPON_") || key.starts_with("ZOMBIE_"));
    c.path_nodes.clear();
    c.path_tables = Default::default();
    c.map_ents.clear();
    c.vehicles.clear();
    c.destructibles.clear();
    c.fonts.clear();
    c.impact_tables.clear();
    c.footstep_tables.clear();
    c.footstep_fx_tables.clear();
    c.snd_globals = None;
    c
}

/// bo2mp: a multiplayer map's two factions' zones, from `mp/mapstable.csv`
/// in the zones read so far: the map's row names the allies' and axis'
/// team sets (columns 1 and 2) and, on the snow, wet and sand maps, the
/// look in its last column (16): Downhill = `faction_seals_snow_mp` +
/// `faction_pmc_snow_mp`. A team set without that look loads its own zone.
const MAPSTABLE_TEAMS: [usize; 2] = [1, 2];
const MAPSTABLE_LOOK: usize = 16;

fn mp_faction_zones(
    captures: &[ZoneCapture],
    map: &str,
    dir: &Path,
    load_log: &mut Vec<String>,
) -> Vec<PathBuf> {
    let Some(t) = captures
        .iter()
        .rev()
        .flat_map(|c| c.string_tables.iter())
        .find(|t| t.name.eq_ignore_ascii_case("mp/mapstable.csv"))
    else {
        load_log.push("t6 mp factions: no mp/mapstable.csv".to_owned());
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = Vec::new();
    for r in 0..t.rows {
        let row = &t.cells[r * t.columns..((r + 1) * t.columns).min(t.cells.len())];
        if !row.first().is_some_and(|c| c.eq_ignore_ascii_case(map)) {
            continue;
        }
        let look = row
            .get(MAPSTABLE_LOOK)
            .filter(|c| !c.is_empty())
            .map(|c| format!("_{}", c.to_ascii_lowercase()))
            .unwrap_or_default();
        for team in MAPSTABLE_TEAMS.iter().filter_map(|&i| row.get(i)) {
            let team = team.to_ascii_lowercase();
            if team.is_empty() || team.len() >= 32 {
                continue;
            }
            let path = [
                format!("faction_{team}{look}_mp.ff"),
                format!("faction_{team}_mp.ff"),
            ]
            .into_iter()
            .map(|name| dir.join(name))
            .find(|p| p.is_file());
            if let Some(path) = path.filter(|p| !out.contains(p)) {
                out.push(path);
            }
        }
    }
    load_log.push(format!(
        "t6 mp factions for {map}: {}",
        out.iter()
            .filter_map(|p| p.file_stem())
            .map(|s| s.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(", ")
    ));
    out
}

/// bo2mp: a material drawn by BO2's sight reticle shader
/// (`sw4_3d_reticle_dynamic`: the reflex's dot, the EOTech's ring, the
/// ACOG's chevron), whose picture is projected to infinity along the sight;
/// (its capture, index) where it is defined.
fn reticle_material(
    captures: &[&ZoneCapture],
    homes: &HashMap<String, (usize, usize)>,
    ci: usize,
    key: asset_t6::AssetIdentity,
) -> Option<(usize, usize)> {
    let m = captures[ci].materials.get(key.index)?;
    let (hc, hm) = if m.name.starts_with(',') {
        *homes.get(m.name.trim_start_matches(','))?
    } else {
        (ci, key.index)
    };
    let def = captures[hc].materials.get(hm)?;
    let set = def
        .method_set
        .and_then(|k| captures[hc].technique_sets.get(k.index))?;
    set.name.contains("reticle_dynamic").then_some((hc, hm))
}

/// bo2mp: a sight model's reticle as Black Ops II's shader draws it (fxc
/// disassembly of `sw4_3d_reticle_dynamic`): its picture's red channel is
/// the dot or ring in `Reticle_Color`, its green the glow around it raised
/// to `Glow_Falloff` in `Glow_Color`, both times `Emissive_Amount`, added on
/// (the picture is that colour, for the HUD to add onto the scene). Returns the picture and `Color_Map_Scale`
/// (the picture spans 1 / scale in tangent of the view angle).
fn sight_reticle(
    captures: &[&ZoneCapture],
    homes: &HashMap<String, (usize, usize)>,
    packs: &PackSet,
    model_name: &str,
    top: bool,
) -> Option<(f32, bevy::prelude::Image)> {
    let (ci, m) = captures
        .iter()
        .enumerate()
        .rev()
        .find_map(|(ci, c)| c.xmodels.iter().find(|m| m.name == model_name).map(|m| (ci, m)))?;
    // The Hybrid's two reticles: the scope's (`_down`) and the little dot
    // above it (`_up`), one for each of its two modes (`top`: the dot's).
    let found: Vec<(usize, usize)> = m
        .lod0
        .iter()
        .filter_map(|s| s.material)
        .chain(m.materials.iter().flatten().copied())
        .filter_map(|k| reticle_material(captures, homes, ci, k))
        .collect();
    let (hc, hm) = found
        .iter()
        .copied()
        .find(|&(hc, hm)| captures[hc].materials[hm].name.ends_with("_up") == top)
        .or_else(|| found.first().copied())?;
    let c = captures[hc];
    let mat = &c.materials[hm];
    let constant = |prefix: &str| {
        mat.constants
            .iter()
            .find(|(_, name, _)| name.starts_with(prefix))
            .map(|(_, _, v)| *v)
    };
    let scale = constant("Color_Map_Sc")?[0];
    let core = constant("Reticle_Colo").unwrap_or([1.0; 4]);
    let glow = constant("Glow_Color").unwrap_or([1.0, 0.0, 0.0, 1.0]);
    let falloff = constant("Glow_Falloff").map_or(1.0, |v| v[0]);
    let emissive = constant("Emissive_Amo").map_or(1.0, |v| v[0]);
    let noise = constant("Color_Map_No").map_or(0.0, |v| v[0]);
    // Its picture (slot 0xb607c0fe; the noise map beside it is another
    // zone's and empty here).
    let image = mat
        .textures
        .iter()
        .filter_map(|t| Some((t.label_digest, c.images.get(t.image?.index)?)))
        .filter(|(_, img)| img.width > 0)
        .max_by_key(|(hash, img)| (*hash == 0xb607_c0fe, u32::from(img.width) * u32::from(img.height)))
        .map(|(_, img)| img)?;
    let img = super::t6_materials::to_rgba8(&super::t6_materials::decode(packs, image).ok()?)?;
    let mut rgba = img.data.clone()?;
    // A colour map is read as sRGB (the engine's own rule: `use_srgb_reads`
    // for every colour map), so the shader's picture values are the linear
    // ones.
    let linear = |byte: u8| {
        let v = f32::from(byte) / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    for px in rgba.chunks_exact_mut(4) {
        let r = linear(px[0]);
        let g = linear(px[1]).powf(falloff);
        // The shader's noise map (another zone's, not held here) shimmers
        // the glow; its mean, one half, stands in for it. The shader's
        // output is HDR (`Emissive_Amount` runs to 9), which the game
        // squeezes before it shows: 1 - e^(-c * EXPOSURE) (a fitted soft
        // knee, no data behind it). The material adds that onto the scene
        // (blend ONE / ONE or ONE / 1 - alpha with a zero alpha), so the
        // HUD draws it additively: the picture is the colour added.
        const NOISE_MEAN: f32 = 0.5;
        const EXPOSURE: f32 = 0.35;
        for i in 0..3 {
            let c = (r * core[i] * (1.0 - noise * NOISE_MEAN) + g * glow[i] * NOISE_MEAN) * emissive;
            px[i] = ((1.0 - (-c * EXPOSURE).exp()) * 255.0).round() as u8;
        }
        px[3] = 255;
    }
    use bevy::render::render_resource::{TextureDimension, TextureFormat};
    let usage = bevy::asset::RenderAssetUsages::default();
    let mut lit = bevy::prelude::Image::new(img.texture_descriptor.size, TextureDimension::D2, rgba, TextureFormat::Rgba8Unorm, usage);
    lit.sampler = img.sampler.clone();
    Some((scale, lit))
}

/// The mode zone's name for a map (`so_zclassic_zm_transit`).
fn mode_zone(stem: &str) -> String {
    let mode = match frame::bo2_zm_start(stem).map(|(_, mode)| mode).as_deref() {
        Some("zstandard") => "zsurvival",
        Some("zgrief" | "zcleansed" | "zmeat") => "zencounter",
        _ if stem == "zm_nuked" => "zstandard",
        _ => "zclassic",
    };
    format!("so_{mode}_{stem}")
}

/// Guest weapons and the BO2 weapon each rides on: (name, donor). Half-Life
/// 2's Gravity Gun and its Pack-a-Punch, the Super Gravity Gun, and
/// Minecraft's bow, the box's enchanted bow and the enchanted stick, all
/// on the Ray Gun (whose trigger the guest weapon takes), and the name the
/// HUD shows (no string table has it: the HUD's Localize gives an unknown
/// key back as it is). On a plain map they come from the Special Weapons
/// tab only (`sim::t6::mp_specials`).
const GUEST_WEAPONS: &[(&str, &str, &str)] = &[
    ("physcannon_zm", "ray_gun_zm", "Gravity Gun"),
    ("physcannon_upgraded_zm", "ray_gun_upgraded_zm", "Super Gravity Gun"),
    ("mcbow_zm", "ray_gun_zm", "Bow"),
    ("mcbow_enchanted_zm", "ray_gun_zm", "Enchanted Bow"),
    ("mcstick_zm", "ray_gun_zm", "Enchanted Stick"),
    ("portalgun_zm", "ray_gun_zm", "Portal Gun"),
    ("portalgun_upgraded_zm", "ray_gun_upgraded_zm", "Super Portal Gun"),
];

fn real(name: &str) -> bool {
    !name.is_empty() && !name.starts_with(',')
}

/// bo2zm: another map's special weapons, loaded onto every other map so the
/// Special Weapons tab gives them anywhere (his 10-14 "Can't I test the
/// thunder gun or hacker on any map?"). Each: the map whose zones hold them
/// (its map zone, then its patch), the weapons, the scripts that run them
/// and the `level._effect` rows the map's fx script sets for them.
/// `BO2MP_T6_NO_DONORS=1` loads none.
struct DonorZone {
    map: &'static str,
    weapons: &'static [&'static str],
    scripts: &'static [&'static str],
    effects: &'static [(&'static str, &'static str)],
}

const DONOR_ZONES: &[DonorZone] = &[DonorZone {
    // Mob of the Dead: the Hell's Retriever and Redeemer, the Blundergat
    // and Acidgat, the spoon and the Golden Spork (`zm_prison.gsc`).
    map: "zm_prison",
    weapons: &[
        "bouncing_tomahawk_zm",
        "upgraded_tomahawk_zm",
        "zombie_tomahawk_flourish",
        "blundergat_zm",
        "blundergat_upgraded_zm",
        "blundersplat_zm",
        "blundersplat_upgraded_zm",
        "blundersplat_bullet_zm",
        "blundersplat_explosive_dart_zm",
        "spoon_zm_alcatraz",
        "spork_zm_alcatraz",
    ],
    scripts: &["_zm_weap_tomahawk", "_zm_weap_blundersplat"],
    // `zm_prison_fx.gsc`.
    effects: &[
        ("acid_spray", "maps/zombie_alcatraz/fx_alcatraz_acid_spray"),
        ("acid_death", "maps/zombie_alcatraz/fx_alcatraz_acid_death"),
        ("tomahawk_trail", "weapon/tomahawk/fx_tomahawk_trail"),
        ("tomahawk_trail_ug", "weapon/tomahawk/fx_tomahawk_trail_ug"),
        ("tomahawk_impact", "weapon/tomahawk/fx_tomahawk_impact"),
        ("tomahawk_impact_ug", "weapon/tomahawk/fx_tomahawk_impact_ug"),
        ("tomahawk_charge_up", "weapon/tomahawk/fx_tomahawk_charge"),
        ("tomahawk_charge_up_ug", "weapon/tomahawk/fx_tomahawk_charge_ug"),
        ("tomahawk_pickup", "maps/zombie_alcatraz/fx_alcatraz_tomahawk_pickup"),
        ("tomahawk_pickup_upgrade", "maps/zombie_alcatraz/fx_alcatraz_tomahawk_pickup_ug"),
        ("tomahawk_charged_trail", "weapon/tomahawk/fx_tomahawk_trail_charged"),
        ("tomahawk_fire_dot", "maps/zombie_alcatraz/fx_alcatraz_zmb_fire_torso"),
        ("tomahawk_hellhole", "maps/zombie_alcatraz/fx_alcatraz_brutus_footstomp"),
        // _zm_weap_blundersplat::init sets its own (the Acidgat's dart blink).
        ("dart_light", "weapon/crossbow/fx_trail_crossbow_blink_grn_os"),
    ],
}, DonorZone {
    // Kino der Toten (the Black Ops pack, `BO2MP_T6_EXTRA`): the Thunder Gun.
    // Its script is BO2's own (`_zm_weap_thundergun`, every map has it);
    // its init sets its effects.
    map: "zm_theater",
    weapons: &["thundergun_zm", "thundergun_upgraded_zm"],
    scripts: &[],
    effects: &[
        ("thundergun_viewmodel_power_cell1", "weapon/thunder_gun/fx_thundergun_power_cell_view1"),
        ("thundergun_viewmodel_power_cell2", "weapon/thunder_gun/fx_thundergun_power_cell_view2"),
        ("thundergun_viewmodel_power_cell3", "weapon/thunder_gun/fx_thundergun_power_cell_view3"),
        ("thundergun_viewmodel_steam", "weapon/thunder_gun/fx_thundergun_steam_view"),
        ("thundergun_knockdown_ground", "weapon/thunder_gun/fx_thundergun_knockback_ground"),
        ("thundergun_smoke_cloud", "weapon/thunder_gun/fx_thundergun_smoke_cloud"),
    ],
}, DonorZone {
    // Moon: the Hacker (its script and the hackables' are BO2's own too),
    // the Gersh Device, the QED and the Wave Gun. The Black Ops scripts'
    // inits load their own effects (kept as the scripts name them).
    map: "zm_moon",
    weapons: &[
        "equip_hacker_zm",
        "equip_hacker_zm_hack",
        "zombie_black_hole_bomb",
        "zombie_quantum_bomb",
        "microwavegundw_zm",
        "microwavegundw_upgraded_zm",
    ],
    scripts: &["_zm_weap_black_hole_bomb", "_zm_weap_quantum_bomb", "_zm_weap_microwavegun"],
    effects: &[],
}, DonorZone {
    // Der Riese: the Wunderwaffe DG-2.
    map: "zm_factory",
    weapons: &["tesla_gun_zm", "tesla_gun_upgraded_zm"],
    scripts: &["_zm_weap_tesla"],
    effects: &[],
}, DonorZone {
    // "Five": Winter's Howl.
    map: "zm_pentagon",
    weapons: &["freezegun_zm", "freezegun_upgraded_zm"],
    scripts: &["_zm_weap_freezegun"],
    effects: &[],
}, DonorZone {
    // Shangri-La: the Baby Gun.
    map: "zm_temple",
    weapons: &["shrink_ray_zm", "shrink_ray_upgraded_zm"],
    scripts: &["_zm_weap_shrink_ray"],
    effects: &[],
}, DonorZone {
    // Ascension: the Matryoshka Dolls.
    map: "zm_cosmodrome",
    weapons: &["zombie_nesting_dolls", "zombie_nesting_doll_single"],
    scripts: &["_zm_weap_nesting_dolls"],
    effects: &[],
}, DonorZone {
    // Die Rise: the Sliquifier (`_zm_weap_slipgun`, its init sets these).
    map: "zm_highrise",
    weapons: &["slipgun_zm", "slipgun_upgraded_zm", "slip_bolt_zm", "slip_bolt_upgraded_zm"],
    scripts: &["_zm_weap_slipgun"],
    effects: &[
        ("slipgun_explode", "weapon/liquifier/fx_liquifier_goo_explo"),
        ("slipgun_splatter", "maps/zombie/fx_zmb_goo_splat"),
        ("slipgun_simmer", "weapon/liquifier/fx_liquifier_goo_sizzle"),
        ("slipgun_viewmodel_eject", "weapon/liquifier/fx_liquifier_clip_eject"),
        ("slipgun_viewmodel_reload", "weapon/liquifier/fx_liquifier_reload_steam"),
    ],
}, DonorZone {
    // Buried: the Paralyzer (`_zm_weap_slowgun`, its init sets these), the
    // Ray Gun Mark II, the Time Bomb and the placed Trample Steam,
    // subwoofer and head chopper (each init sets its rows).
    map: "zm_buried",
    weapons: &[
        "slowgun_zm",
        "slowgun_upgraded_zm",
        "raygun_mark2_zm",
        "raygun_mark2_upgraded_zm",
        "time_bomb_zm",
        "time_bomb_detonator_zm",
        "equip_springpad_zm",
        "equip_springpad_zm_turret",
        "equip_subwoofer_zm",
        "equip_subwoofer_zm_turret",
        "equip_headchopper_zm",
        "equip_headchopper_zm_turret",
    ],
    scripts: &[
        "_zm_weap_slowgun",
        "_zm_weap_time_bomb",
        "_zm_equip_springpad",
        "_zm_equip_subwoofer",
        "_zm_equip_headchopper",
    ],
    effects: &[
        ("time_bomb_set", "weapon/time_bomb/fx_time_bomb_detonate"),
        ("time_bomb_respawns_enemy", "maps/zombie_buried/fx_buried_time_bomb_spawn"),
        ("time_bomb_kills_enemy", "maps/zombie_buried/fx_buried_time_bomb_death"),
        ("springpade_on", "maps/zombie_highrise/fx_highrise_trmpl_steam_os"),
        ("subwoofer_audio_wave", "maps/zombie_buried/fx_buried_subwoofer_blast"),
        ("subwoofer_knockdown_ground", "weapon/thunder_gun/fx_thundergun_knockback_ground"),
        ("subwoofer_disappear", "maps/zombie/fx_zmb_tranzit_turbine_explo"),
        ("headchoppere_on", "maps/zombie_buried/fx_buried_headchopper_os"),
        ("zombie_slowgun_explosion", "weapon/paralyzer/fx_paralyzer_body_disintegrate"),
        ("zombie_slowgun_explosion_ug", "weapon/paralyzer/fx_paralyzer_body_disintegrate_ug"),
        ("zombie_slowgun_sizzle", "weapon/paralyzer/fx_paralyzer_hit_dmg"),
        ("zombie_slowgun_sizzle_ug", "weapon/paralyzer/fx_paralyzer_hit_dmg_ug"),
        ("player_slowgun_sizzle", "weapon/paralyzer/fx_paralyzer_hit_noharm"),
        ("player_slowgun_sizzle_ug", "weapon/paralyzer/fx_paralyzer_hit_noharm"),
        ("player_slowgun_sizzle_1st", "weapon/paralyzer/fx_paralyzer_hit_noharm_view"),
    ],
}, DonorZone {
    // TranZit: the Jet Gun (`_zm_weap_jetgun::init` sets the first four;
    // zm_transit's lava burn is what an overheated one explodes with) and the
    // EMP grenade (its equipment-destroy effect is common_zm's), and the
    // placed turbine, turret and electric trap (each init sets its rows),
    // and the riot shield (`_zm_riotshield::init` names its models).
    map: "zm_transit",
    weapons: &[
        "jetgun_zm",
        "emp_grenade_zm",
        "equip_turbine_zm",
        "equip_turbine_zm_turret",
        "equip_turret_zm",
        "equip_turret_zm_turret",
        "zombie_bullet_crouch_zm",
        "equip_electrictrap_zm",
        "equip_electrictrap_zm_turret",
        "riotshield_zm",
    ],
    scripts: &[
        "_zm_weap_jetgun",
        "_zm_weap_emp_bomb",
        "_zm_equip_turbine",
        "_zm_equip_turret",
        "_zm_equip_electrictrap",
        "_zm_weap_riotshield",
        "_zm_riotshield",
    ],
    effects: &[
        ("jetgun_smoke_cloud", "weapon/thunder_gun/fx_thundergun_smoke_cloud"),
        ("jetgun_overheat", "weapon/jet_gun/fx_jetgun_overheat"),
        ("jetgun_vortex", "weapon/jet_gun/fx_jetgun_on"),
        ("jetgun_meat_grinder", "weapon/jet_gun/fx_jetgun_kill"),
        ("lava_burning", "env/fire/fx_fire_lava_player_torso"),
        ("turbine_on", "maps/zombie/fx_zmb_tranzit_wind_turbine_on"),
        ("turbine_med", "maps/zombie/fx_zmb_tranzit_wind_turbine_med"),
        ("turbine_low", "maps/zombie/fx_zmb_tranzit_wind_turbine_low"),
        ("turbine_aoe", "maps/zombie/fx_zmb_tranzit_wind_turbine_aoe"),
        ("turbine_explo", "maps/zombie/fx_zmb_tranzit_turbine_explo"),
        ("etrap_on", "maps/zombie/fx_zmb_tranzit_electric_trap_on"),
        ("etrap_explo", "maps/zombie/fx_zmb_tranzit_electrap_explo"),
    ],
}, DonorZone {
    // Origins: the four elemental staffs and the revive staff (zm_tomb.gsc's
    // rows; each staff script's init sets the effects below it). The staff
    // scripts include zm_tomb_utility (their charge watcher, gibs, traces).
    // The G-Strike (`_zm_weap_beacon::init` sets its effects) and the One
    // Inch Punch, plain and elemental (one_inch_precache sets its two).
    map: "zm_tomb",
    weapons: &[
        "staff_air_zm",
        "staff_air_upgraded_zm",
        "staff_air_upgraded2_zm",
        "staff_air_upgraded3_zm",
        "staff_air_melee_zm",
        "staff_fire_zm",
        "staff_fire_upgraded_zm",
        "staff_fire_upgraded2_zm",
        "staff_fire_upgraded3_zm",
        "staff_fire_melee_zm",
        "staff_lightning_zm",
        "staff_lightning_upgraded_zm",
        "staff_lightning_upgraded2_zm",
        "staff_lightning_upgraded3_zm",
        "staff_lightning_melee_zm",
        "staff_water_zm",
        "staff_water_zm_cheap",
        "staff_water_upgraded_zm",
        "staff_water_upgraded2_zm",
        "staff_water_upgraded3_zm",
        "staff_water_melee_zm",
        "staff_water_dart_zm",
        "staff_water_fake_dart_zm",
        "staff_revive_zm",
        "beacon_zm",
        "one_inch_punch_zm",
        "one_inch_punch_upgraded_zm",
        "one_inch_punch_fire_zm",
        "one_inch_punch_air_zm",
        "one_inch_punch_ice_zm",
        "one_inch_punch_lightning_zm",
        "zombie_one_inch_punch_flourish",
        "zombie_one_inch_punch_upgrade_flourish",
    ],
    scripts: &[
        "_zm_weap_staff_air",
        "_zm_weap_staff_fire",
        "_zm_weap_staff_lightning",
        "_zm_weap_staff_water",
        "_zm_weap_staff_revive",
        "zm_tomb_utility",
        "_zm_weap_beacon",
        "_zm_weap_one_inch_punch",
    ],
    effects: &[
        ("staff_charge", "weapon/zmb_staff/fx_zmb_staff_charge"),
        ("staff_soul", "weapon/zmb_staff/fx_zmb_staff_charge_souls"),
        ("fire_muzzle", "weapon/zmb_staff/fx_zmb_staff_fire_muz_flash_1p"),
        ("whirlwind", "weapon/zmb_staff/fx_zmb_staff_air_ug_impact_miss"),
        ("lightning_miss", "weapon/zmb_staff/fx_zmb_staff_elec_ug_impact_miss"),
        ("lightning_arc", "weapon/zmb_staff/fx_zmb_staff_elec_trail_bolt_cheap"),
        ("lightning_impact", "weapon/zmb_staff/fx_zmb_staff_elec_ug_impact_hit_torso"),
        ("tesla_shock_eyes", "maps/zombie/fx_zombie_tesla_shock_eyes"),
        ("staff_water_blizzard", "weapon/zmb_staff/fx_zmb_staff_ice_ug_impact_hit"),
        ("staff_water_ice_shard", "weapon/zmb_staff/fx_zmb_staff_ice_trail_bolt"),
        ("staff_water_shatter", "weapon/zmb_staff/fx_zmb_staff_ice_exp"),
        ("beacon_glow", "maps/zombie_tomb/fx_tomb_beacon_glow"),
        ("beacon_shell_explosion", "maps/zombie_tomb/fx_tomb_beacon_exp"),
        ("beacon_shell_trail", "maps/zombie_tomb/fx_tomb_beacon_trail"),
        ("beacon_launch_fx", "maps/zombie_tomb/fx_tomb_beacon_launch"),
        ("oneinch_impact", "maps/zombie_tomb/fx_tomb_perk_one_inch_punch"),
        ("punch_knockdown_ground", "weapon/thunder_gun/fx_thundergun_knockback_ground"),
    ],
}];

/// The animation states a donor map's weapons put its zombies in, which other maps'
/// animstatedefs lack (each a state name prefix): the Jet Gun's pull (`set_zombie_run_cycle
/// "jetgun_walk"` and the rest), the Acidgat's stun.
/// Donors whose zombie animstatedef (and its animations) is in another zone than the
/// map's own: TranZit's is in its classic mode's zone.
const DONOR_ASD_ZONES: &[(&str, &str)] = &[("zm_transit", "so_zclassic_zm_transit")];

const DONOR_ANIM_STATES: &[(&str, &[&str])] = &[
    // The Jet Gun's pull and deaths; an EMP'd zombie's stand, its wander and waking.
    ("zm_transit", &["zm_move_jetgun", "zm_jetgun", "zm_inert"]),
    ("zm_prison", &["zm_blundersplat_stun"]),
    // The Gersh Device's pull and deaths; the Wave Gun's; the Wunderwaffe's;
    // Winter's Howl's.
    (
        "zm_moon",
        &[
            "zm_bhb",
            "zm_black_hole_attracted_death",
            "zm_move_blackhole",
            "zm_death_sizzle",
            "zm_death_zap",
        ],
    ),
    ("zm_factory", &["zm_death_tesla"]),
    ("zm_pentagon", &["zm_death_freeze"]),
    // The staffs' deaths, stuns and the Wind Staff's whirlwind pull.
    (
        "zm_tomb",
        &[
            "zm_death_freeze",
            "zm_death_tesla",
            "zm_death_fire",
            "zm_electric_stun",
            "zm_afterlife_stun",
            "zm_move_whirlwind",
            // The One Inch Punch's knockdown and getting up.
            "zm_punch_fall",
            "zm_punch_getup",
        ],
    ),
];

/// The states of animstatedef `text` named with one of `prefixes`, added to `out` as asd
/// text; their animations go in `anims`.
fn asd_states(words: &str, prefixes: &[&str], out: &mut String, anims: &mut Vec<String>) {
    let (mut keep, mut inside) = (false, false);
    for raw in words.lines() {
        let line = raw.split("//").next().unwrap_or("").trim();
        match line {
            "" => continue,
            "{" => inside = true,
            "}" => inside = false,
            _ if !inside => {
                let name = line.split(':').next().unwrap_or("").trim().to_ascii_lowercase();
                keep = prefixes.iter().any(|p| name.starts_with(p));
            }
            _ if keep => anims.extend(line.split_whitespace().last().map(str::to_owned)),
            _ => {}
        }
        if keep {
            out.extend([line, "\n"]);
        }
    }
}

/// Whether a load takes the donors' weapons. Not for the map behind the
/// Zombies menus: nobody holds a gun there, and they took 5.2 s of its load
/// (load 10-29). The match Start Match loads takes them.
fn donors_wanted(no_donors: bool, menu_backdrop: bool) -> bool {
    !no_donors && !menu_backdrop
}

/// The donor maps' zones for this map, each cut down to its special
/// weapons, plus their English strings and `level._effect` rows.
fn donor_captures(
    level_file_path: &Path,
    zone_dir: &Path,
    load_log: &mut Vec<String>,
) -> (Vec<ZoneCapture>, Vec<(String, String)>, Vec<(String, String)>) {
    let mut caps = Vec::new();
    let mut strings = Vec::new();
    let mut fx_list = Vec::new();
    let no_donors = std::env::var("BO2MP_T6_NO_DONORS").is_ok_and(|v| v == "1");
    if !donors_wanted(no_donors, frame::bo2_frontend_waits_on_menu()) {
        return (caps, strings, fx_list);
    }
    let stem = stem_text(level_file_path);
    let lang_dir = zone_dir.parent().map(|d| d.join("english"));
    let jobs = donor_jobs(&stem, |part, zone| {
        // A Black Ops map's zones are in the extra folder.
        let dir = if part == DonorPart::Strings { lang_dir.as_deref()? } else { zone_dir };
        Some(asset_transport::t6_extra::file_in(dir, &format!("{zone}.ff"))).filter(|p| p.is_file())
    });
    // Read side by side (one after another they held every map's load up
    // ~5 s), each cut down on its worker.
    let clock = std::time::Instant::now();
    let paths: Vec<PathBuf> = jobs.iter().map(|job| job.path.clone()).collect();
    let mut read = capture_many_with(&paths, |i, r| {
        let job = &jobs[i];
        r.map(|mut c| {
            match job.part {
                DonorPart::Weapons => trim_donor(&mut c, job.donor),
                DonorPart::States => {
                    // Only the graft and its animations kept.
                    let states_only = DonorZone { map: job.donor.map, weapons: &[], scripts: &[], effects: &[] };
                    trim_donor(&mut c, &states_only);
                    // Nothing else of that zone: animations need no materials.
                    c.images.clear();
                    c.materials.clear();
                    c.technique_sets.clear();
                    c.localize.clear();
                }
                DonorPart::Strings => {}
            }
            c
        })
    })
    .into_iter();
    let mut queue = jobs.iter().peekable();
    while let Some(first) = queue.peek() {
        let donor = first.donor;
        let mut taken = 0usize;
        let mut words = Vec::new();
        while let Some(job) = queue.next_if(|job| std::ptr::eq(job.donor, donor)) {
            let r = read.next().unwrap_or_else(|| Err("zone not captured".to_owned()));
            match (job.part, r) {
                (DonorPart::Strings, Ok(c)) => words = c.localize,
                (DonorPart::Strings, Err(_)) => {}
                (_, Err(e)) => load_log.push(format!("t6 donor zone gap: {e}")),
                (DonorPart::Weapons, Ok(c)) => {
                    taken += c.weapons.len();
                    caps.push(c);
                }
                (DonorPart::States, Ok(c)) => {
                    load_log.push(format!(
                        "t6 donor {} states from {}: {} animations",
                        donor.map,
                        job.zone,
                        c.animation_refs.len()
                    ));
                    caps.push(c);
                }
            }
        }
        if taken == 0 {
            continue;
        }
        strings.extend(words);
        fx_list.extend(
            donor
                .effects
                .iter()
                .map(|&(k, fx)| (k.to_owned(), fx.to_owned())),
        );
        load_log.push(format!("t6 donor {}: {taken} weapons", donor.map));
    }
    load_log.push(format!("t6 donor zones: {} read in {:.1} s", paths.len(), clock.elapsed().as_secs_f32()));
    (caps, strings, fx_list)
}

/// Which part of a donor map one zone gives.
#[derive(Clone, Copy, Debug, PartialEq)]
enum DonorPart {
    /// Its special weapons (the map's zone and its patch).
    Weapons,
    /// Animation states from another of its zones (`DONOR_ASD_ZONES`).
    States,
    /// Its English strings (`en_<map>`).
    Strings,
}

/// One donor zone to read.
struct DonorJob {
    donor: &'static DonorZone,
    part: DonorPart,
    zone: String,
    path: PathBuf,
}

/// The donor zones for map `stem`, donor by donor in `DONOR_ZONES` order
/// (the map itself left out): its zone and patch, its animation-state zones,
/// then its strings. `find` gives a zone's file, `None` when it is not there.
fn donor_jobs(stem: &str, find: impl Fn(DonorPart, &str) -> Option<PathBuf>) -> Vec<DonorJob> {
    let mut jobs = Vec::new();
    for donor in DONOR_ZONES.iter().filter(|d| d.map != stem) {
        let weapons = [donor.map.to_owned(), format!("{}_patch", donor.map)].map(|z| (DonorPart::Weapons, z));
        let states = DONOR_ASD_ZONES
            .iter()
            .filter(|(m, _)| *m == donor.map)
            .map(|&(_, z)| (DonorPart::States, z.to_owned()));
        let words = (DonorPart::Strings, format!("en_{}", donor.map));
        for (part, zone) in weapons.into_iter().chain(states).chain([words]) {
            if let Some(path) = find(part, &zone) {
                jobs.push(DonorJob { donor, part, zone, path });
            }
        }
    }
    jobs
}

/// A donor zone cut down to what its special weapons need: the weapons
/// (and their alternate and left-hand weapons), their scripts, effects,
/// tracers, animations and models. Materials, images and technique sets
/// stay whole (models name their materials by index); the map's world,
/// entities, sounds and tables go.
fn trim_donor(c: &mut ZoneCapture, donor: &DonorZone) {
    use fastfile_t6::layout::WeaponDef;
    let mut weapons: BTreeSet<String> = donor.weapons.iter().map(|&w| w.to_owned()).collect();
    loop {
        let before = weapons.len();
        let mut more = Vec::new();
        for w in c.weapons.iter().filter(|w| weapons.contains(&w.name)) {
            more.extend([w.alt_weapon_name.clone(), w.def_string(WeaponDef::szDualWieldWeaponName).to_owned()]);
        }
        weapons.extend(more.into_iter().filter(|n| real(n)));
        if weapons.len() == before {
            break;
        }
    }
    c.weapons.retain(|w| weapons.contains(&w.name));
    // The donor's animation states its weapons need, as one graft def, with their animations.
    let prefixes = DONOR_ANIM_STATES
        .iter()
        .find(|(m, _)| *m == donor.map)
        .map_or(&[][..], |(_, p)| *p);
    let (mut graft, mut graft_anims) = (String::new(), Vec::new());
    for (name, bytes) in &c.raw_files {
        if !prefixes.is_empty() && name.starts_with("animstatedefs/") && name.ends_with(".asd") {
            asd_states(&String::from_utf8_lossy(bytes), prefixes, &mut graft, &mut graft_anims);
        }
    }
    c.raw_files.retain(|(name, _)| {
        name.strip_prefix("script:")
            .and_then(|n| n.strip_suffix(".gsc"))
            .and_then(|n| n.rsplit(['/', '\\']).next())
            .is_some_and(|n| donor.scripts.contains(&n))
    });
    // Every name the scripts use (models they spawn, animations they play).
    let mut tokens: BTreeSet<String> = BTreeSet::new();
    for (_, bytes) in &c.raw_files {
        for token in bytes.split(|&b| b == 0) {
            if (3..96).contains(&token.len()) && token.iter().all(|b| b.is_ascii_graphic()) {
                tokens.insert(String::from_utf8_lossy(token).into_owned());
            }
        }
    }
    // Effects: the weapons', the fx script's rows, and every effect those
    // spawn in turn.
    let mut fx: BTreeSet<String> = donor.effects.iter().map(|&(_, f)| f.to_owned()).collect();
    // And each effect the scripts name (a Black Ops init loads its own).
    fx.extend(
        c.fx.iter()
            .map(|e| e.name.trim_start_matches(','))
            .filter(|n| tokens.contains(*n))
            .map(str::to_owned),
    );
    for w in &c.weapons {
        fx.extend(w.fx.iter().map(|(_, n)| n.trim_start_matches(',').to_owned()));
    }
    loop {
        let before = fx.len();
        let mut more = Vec::new();
        for e in c.fx.iter().filter(|e| fx.contains(e.name.trim_start_matches(','))) {
            for el in &e.elems {
                more.extend([
                    &el.effect_upon_strike,
                    &el.effect_upon_demise,
                    &el.effect_spawned,
                    &el.effect_attached,
                ]);
                for v in &el.visuals {
                    if let asset_t6::FxVisualRef::Effect(n) = v {
                        more.push(n);
                    }
                }
            }
        }
        let more: Vec<String> = more
            .into_iter()
            .filter(|n| !n.is_empty())
            .map(|n| n.trim_start_matches(',').to_owned())
            .collect();
        fx.extend(more);
        if fx.len() == before {
            break;
        }
    }
    c.fx.retain(|e| real(&e.name) && fx.contains(e.name.trim_start_matches(',')));
    let tracers: BTreeSet<&str> = c
        .weapons
        .iter()
        .flat_map(|w| w.tracers.iter().map(|(_, n)| n.trim_start_matches(',')))
        .collect();
    let mut models: BTreeSet<String> = tokens.clone();
    let mut anims: BTreeSet<String> = tokens;
    anims.extend(graft_anims);
    if !graft.is_empty() {
        c.raw_files.push((format!("animstatedefs/bo2zm_graft_{}.asd", donor.map), graft.into_bytes()));
    }
    let mut camos: BTreeSet<String> = BTreeSet::new();
    for w in &c.weapons {
        models.extend(
            w.gun_models
                .iter()
                .chain(&w.world_models)
                .chain(w.models.iter().map(|(_, n)| n))
                .chain(w.attach_view_models.iter().map(|(_, n, ..)| n))
                .chain(w.attach_world_models.iter().map(|(_, n, ..)| n))
                .map(|n| n.trim_start_matches(',').to_owned()),
        );
        anims.extend(w.animation_refs.iter().map(|n| n.trim_start_matches(',').to_owned()));
        camos.insert(w.camo.trim_start_matches(',').to_owned());
    }
    for e in &c.fx {
        for el in &e.elems {
            for v in &el.visuals {
                if let asset_t6::FxVisualRef::Model(n) = v {
                    models.insert(n.trim_start_matches(',').to_owned());
                }
            }
        }
    }
    c.tracers.retain(|t| real(&t.name) && tracers.contains(t.name.as_str()));
    c.xmodels.retain(|m| real(&m.name) && models.contains(&m.name));
    c.animation_refs.retain(|a| real(&a.name) && anims.contains(&a.name));
    c.weapon_camos.retain(|w| camos.contains(&w.name));
    c.world = None;
    c.clip = None;
    c.key_lights.clear();
    c.string_tables.clear();
    c.zbarriers.clear();
    c.path_nodes.clear();
    c.map_ents.clear();
    c.destructibles.clear();
    c.impact_tables.clear();
    c.footstep_tables.clear();
    c.footstep_fx_tables.clear();
    c.sound_banks.clear();
    c.snd_globals = None;
    c.fonts.clear();
}

/// Where each material is really defined: name -> (capture, index), the
/// last zone that defines it (not a comma-named reference) winning.
fn material_homes(captures: &[&ZoneCapture]) -> HashMap<String, (usize, usize)> {
    let mut homes = HashMap::new();
    for (ci, c) in captures.iter().enumerate() {
        for (mi, m) in c.materials.iter().enumerate() {
            if real(&m.name) {
                homes.insert(m.name.clone(), (ci, mi));
            }
        }
    }
    homes
}

/// For every capture, the material indices (into that capture) the wanted
/// models and effects draw with, each taken from the zone that defines it.
/// bo2zm M3: Pack-a-Punch's look. The scripts give an upgraded gun camo
/// 39 on this map (`get_pack_a_punch_weapon_options`; 40 on Mob of the
/// Dead, 45 on Origins); `mp/weaponoptions_zm.csv`'s row for it says it
/// swaps materials (a 1 in column 3) and which of the gun's camo material
/// sets (column 4, counted from 1: the M1911's camo1 -> the zombies camo).
/// Each upgraded gun's view model gets a copy with those swaps,
/// `<model>+camo<n>`.
#[derive(Default)]
struct PapCamo {
    /// Copy -> (the gun model, swaps base material -> camo material, and
    /// the camo's texture tiling: the swap's constants 0 and 1, which BO2's
    /// 3D camo shader multiplies the texture coordinates by, 2 x 2 for the
    /// zombies camo).
    models: BTreeMap<String, (String, Vec<(String, String, [f32; 2])>)>,
    /// Upgraded weapon -> its gun model's copy.
    weapon_models: HashMap<String, String>,
}

/// A float as a half (IEEE binary16), rounded down; texture coordinates.
fn f32_to_half(x: f32) -> u16 {
    let bits = x.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xff) as i32 - 127 + 15;
    let mant = bits & 0x007f_ffff;
    if exp <= 0 {
        return sign;
    }
    if exp >= 31 {
        return sign | 0x7c00;
    }
    sign | ((exp as u16) << 10) | ((mant >> 13) as u16)
}

fn pap_camo(
    captures: &[&ZoneCapture],
    weapon_refs: &HashMap<String, &asset_t6::WeaponRef>,
    camo_index: u32,
) -> PapCamo {
    let mut out = PapCamo::default();
    let want = camo_index.to_string();
    let set = captures
        .iter()
        .rev()
        .flat_map(|c| c.string_tables.iter())
        .find(|t| t.name == "mp/weaponoptions_zm.csv")
        .and_then(|t| {
            (0..t.rows).find_map(|r| {
                let cell = |k: usize| t.cells.get(r * t.columns + k).map_or("", String::as_str);
                (cell(0) == want && cell(1) == "camo" && cell(3) == "1")
                    .then(|| cell(4).parse::<usize>().ok())
                    .flatten()
            })
        })
        .filter(|set| *set > 0);
    let Some(set) = set else {
        return out;
    };
    let camos: HashMap<&str, &asset_t6::WeaponCamoRef> = captures
        .iter()
        .flat_map(|c| c.weapon_camos.iter())
        .map(|c| (c.name.as_str(), c))
        .collect();
    for (name, w) in weapon_refs {
        if !name.contains("_upgraded") {
            continue;
        }
        let Some(list) = camos
            .get(w.camo.as_str())
            .and_then(|camo| camo.material_sets.get(set - 1))
        else {
            continue;
        };
        let swaps: Vec<(String, String, [f32; 2])> = list
            .iter()
            .flat_map(|m| {
                let tile = [m.consts[0], m.consts[1]].map(|t| if t > 0.0 { t } else { 1.0 });
                m.base
                    .iter()
                    .cloned()
                    .zip(m.camo.iter().cloned())
                    .map(move |(b, c)| (b, c, tile))
            })
            .filter(|(base, camo, _)| real(base) && real(camo))
            .collect();
        let Some(gun) = w.gun_models.first().filter(|n| real(n)) else {
            continue;
        };
        if swaps.is_empty() {
            continue;
        }
        let copy = format!("{gun}+camo{camo_index}");
        out.models
            .entry(copy.clone())
            .or_insert((gun.clone(), swaps));
        out.weapon_models.insert(name.clone(), copy);
    }
    out
}

fn wanted_materials(
    captures: &[&ZoneCapture],
    model_names: &BTreeSet<String>,
    homes: &HashMap<String, (usize, usize)>,
) -> Vec<BTreeSet<usize>> {
    let mut wanted: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); captures.len()];
    let mut want = |name: &str| {
        if let Some(&(ci, mi)) = homes.get(name.trim_start_matches(',')) {
            wanted[ci].insert(mi);
        }
    };
    // Models: from the last capture that defines each.
    for name in model_names {
        if let Some((ci, model)) = captures
            .iter()
            .enumerate()
            .rev()
            .find_map(|(ci, c)| c.xmodels.iter().find(|m| &m.name == name).map(|m| (ci, m)))
        {
            for srf in &model.lod0 {
                if let Some(k) = srf.material
                    && let Some(m) = captures[ci].materials.get(k.index)
                {
                    want(&m.name);
                }
            }
        }
    }
    // Effects and tracers: their material names.
    for c in captures {
        for e in c.fx.iter().filter(|e| real(&e.name)) {
            for el in &e.elems {
                for v in &el.visuals {
                    match v {
                        asset_t6::FxVisualRef::Material(n) => want(n),
                        asset_t6::FxVisualRef::Mark([a, b]) => {
                            want(a);
                            want(b);
                        }
                        _ => {}
                    }
                }
            }
        }
        for t in c.tracers.iter().filter(|t| real(&t.name)) {
            want(&t.material);
        }
    }
    wanted
}

/// The sound bank files a map's banks read: each bank asset's own files,
/// the map's location banks (`zmb_<location>*`), and every file those
/// name as a dependency.
pub(super) fn bank_bases(captures: &[&ZoneCapture], map: &str) -> Vec<String> {
    let mut bases: Vec<String> = Vec::new();
    let mut push = |b: String| {
        if !b.is_empty() && !bases.contains(&b) {
            bases.push(b);
        }
    };
    if let Some(location) = map.strip_prefix("zm_") {
        push(format!("zmb_{location}"));
        push(format!("zmb_{location}_real"));
        push(format!("zmb_{location}_real_intro"));
    }
    for c in captures {
        for b in &c.sound_banks {
            let base = b
                .name
                .rsplit_once('.')
                .map_or(b.name.as_str(), |(base, _)| base);
            push(base.to_owned());
        }
    }
    let deps: [&str; 3] = if is_mp_map(map) {
        ["mpl_common", "mpl_code_post_gfx", "cmn_root"]
    } else {
        ["zmb_common", "zmb_code_post_gfx", "cmn_root"]
    };
    for dep in deps {
        push(dep.to_owned());
    }
    bases
}

pub(crate) fn load_t6_combat(
    level_file_path: &Path,
    map_capture: &ZoneCapture,
    mat_set: &mut SurfaceShaderRoster,
    packs: Option<&PackSet>,
) -> T6Combat {
    let mut load_log = Vec::new();
    let (pre, post) = zone_paths(level_file_path);
    let zone_dir = asset_transport::t6_extra::install_zone_dir(level_file_path);
    let clock = std::time::Instant::now();
    let kept = |r: Result<ZoneCapture, String>, load_log: &mut Vec<String>| match r {
        Ok(c) => Some(c),
        Err(e) => {
            load_log.push(format!("t6 zone gap: {e}"));
            None
        }
    };
    let capture = |path: &PathBuf, load_log: &mut Vec<String>| kept(asset_t6::capture_zone(path), load_log);
    // The menu's Zombies zones and the match's own shared zones, side by
    // side (in their load order).
    let zm_paths = zm_weapon_zone_paths(level_file_path);
    let mut both = capture_many(&zm_paths.iter().chain(&pre).cloned().collect::<Vec<_>>());
    let pre_results = both.split_off(zm_paths.len());
    let zm_caps: Vec<ZoneCapture> = both
        .into_iter()
        .filter_map(|r| kept(r, &mut load_log))
        .map(weapons_only)
        .collect();
    let mut pre_caps: Vec<ZoneCapture> =
        pre_results.into_iter().filter_map(|r| kept(r, &mut load_log)).collect();
    let map_stem = stem_text(level_file_path);
    // bo2mp: a multiplayer map's factions (bodies, arms, team voices).
    if is_mp_map(&map_stem) {
        let dir = level_file_path.parent().unwrap_or(Path::new("."));
        for path in mp_faction_zones(&pre_caps, &map_stem, dir, &mut load_log) {
            if let Some(c) = capture(&path, &mut load_log) {
                pre_caps.push(c);
            }
        }
    }
    let post_caps: Vec<ZoneCapture> = post
        .iter()
        .filter_map(|p| capture(p, &mut load_log))
        .collect();
    load_log.push(format!(
        "t6 combat zones captured: {} ms ({} Zombies weapon zones)",
        clock.elapsed().as_millis(),
        zm_caps.len()
    ));
    // Other maps' special weapons, first so the map's own zones win a name.
    let (donor_caps, donor_strings, donor_effects) = donor_captures(level_file_path, &zone_dir, &mut load_log);
    let donor_n = donor_caps.len();
    let captures: Vec<&ZoneCapture> = donor_caps
        .iter()
        .chain(zm_caps.iter())
        .chain(pre_caps.iter())
        .chain(std::iter::once(map_capture))
        .chain(post_caps.iter())
        .collect();
    load_log.push(format!(
        "t6 combat zones: {}",
        captures
            .iter()
            .map(|c| c.zone.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    ));

    // bo2mp: the bullet penetration depths every gun's penetrate type
    // reads (`BULLET_PEN_TABLE`, keyed by Black Ops II's 32 surfaces).
    let mut pen_table = None;
    for (name, bytes) in captures.iter().flat_map(|c| c.raw_files.iter()) {
        if name.replace('\\', "/") != "info/bullet_penetration_mp" {
            continue;
        }
        match asset_game::parse_pen_table_t6(bytes) {
            Ok(table) => pen_table = Some(table),
            Err(e) => load_log.push(format!("t6 bullet penetration table: {e}")),
        }
    }
    match pen_table {
        Some(table) => load_log.push(format!(
            "t6 bullet penetration table: flesh small {} medium {} large {}, wood large {}",
            table.depth(1, 7),
            table.depth(2, 7),
            table.depth(3, 7),
            table.depth(3, 21)
        )),
        None => load_log.push("t6 bullet penetration table: no info/bullet_penetration_mp".to_owned()),
    }
    let mut lochit_table = None;
    for (name, bytes) in captures.iter().flat_map(|c| c.raw_files.iter()) {
        let name = name.replace('\\', "/");
        if let Some(table) = asset_game::capture_lochit_table(&name, bytes) {
            lochit_table = Some(table);
        }
    }
    load_log.push(match lochit_table {
        Some(table) => format!("t6 hit location table: {table:?}"),
        None => "t6 hit location table: no info/mp_lochit_dmgtable".to_owned(),
    });

    // bo2mp vehicle screens: the match zones' named visions
    // (`vision/<name>.vision`, later zones winning a name) for the scripts'
    // `visionsetnaked` and `setvisionsetforplayer` (the VTOL Warship's
    // `remote_mortar_enhanced`, _helicopter_gunner.gsc).
    if is_mp_map(&map_stem) {
        let visions: Vec<(String, asset_world::T6Vision)> = captures
            .iter()
            .flat_map(|c| c.raw_files.iter())
            .filter_map(|(name, bytes)| {
                let n = name.replace('\\', "/");
                let n = n.strip_prefix("vision/")?.strip_suffix(".vision")?.to_owned();
                Some((n, asset_world::parse_t6_vision(&String::from_utf8_lossy(bytes))?))
            })
            .collect();
        load_log.push(format!(
            "t6 match visions: {} ({})",
            visions.len(),
            visions.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(", ")
        ));
        asset_world::register_t6_visions(visions);
    }

    // Guest weapons: another game's weapon rides on a copy of a BO2
    // weapon under its own name, so the box, the inventory and
    // Pack-a-Punch take it; its look, sounds and what it does are the
    // guest weapon's own.
    let guests: Vec<asset_t6::WeaponRef> = GUEST_WEAPONS
        .iter()
        .filter_map(|&(name, donor, shown)| {
            let w = captures.iter().rev().find_map(|c| c.weapons.iter().find(|w| w.name == donor))?;
            let mut w = w.clone();
            w.name = name.to_owned();
            w.shown_label = shown.to_owned();
            // Its right button is the guest weapon's (the Gravity Gun's
            // pull), never a sight.
            if let Some(ads) = w.def.get_mut(fastfile_t6::layout::WeaponDef::aimDownSight) {
                *ads = 0;
            }
            Some(w)
        })
        .collect();
    // Weapons: later zones replace earlier ones of the same name.
    let mut weapon_refs: HashMap<String, &asset_t6::WeaponRef> = HashMap::new();
    let mut weapon_order: Vec<String> = Vec::new();
    for c in &captures[donor_n..] {
        for w in c.weapons.iter().filter(|w| real(&w.name)) {
            if weapon_refs.insert(w.name.clone(), w).is_none() {
                weapon_order.push(w.name.clone());
            }
        }
    }
    // Donor weapons after the map's, where the map has none of the name
    // (a later donor zone, its patch, first).
    for c in captures[..donor_n].iter().rev() {
        for w in c.weapons.iter().filter(|w| real(&w.name)) {
            if !weapon_refs.contains_key(&w.name) {
                weapon_refs.insert(w.name.clone(), w);
                weapon_order.push(w.name.clone());
            }
        }
    }
    for w in &guests {
        if weapon_refs.insert(w.name.clone(), w).is_none() {
            weapon_order.push(w.name.clone());
        }
    }
    load_log.push(format!(
        "t6 guest weapons: {}",
        guests.iter().map(|w| w.name.as_str()).collect::<Vec<_>>().join(", ")
    ));
    // bo2mp: each multiplayer gun's sight attachments (`au_<gun>_<sight>`
    // with its `<sight>` attachment), later zones winning a name.
    let mut attachments: HashMap<&str, &asset_t6::AttachmentRef> = HashMap::new();
    let mut uniques: HashMap<&str, &asset_t6::AttachmentUniqueRef> = HashMap::new();
    for c in &captures {
        for a in c.attachments.iter().filter(|a| real(&a.name)) {
            attachments.insert(a.name.as_str(), a);
        }
        for u in c.attachment_uniques.iter().filter(|u| real(&u.name)) {
            uniques.insert(u.name.as_str(), u);
        }
    }
    let mut sights: Vec<(String, &asset_t6::AttachmentRef, &asset_t6::AttachmentUniqueRef)> = Vec::new();
    // bo2mp: and its other attachments (Extended Clip, FMJ, Quickdraw,
    // Suppressor...), each `<gun>+<attachment>` with what it changes on the
    // gun. (One attachment's entry per name: a class with a sight and
    // others still resolves to the sight's; see `sim::t6::weapon`.)
    let mut others: Vec<(String, &asset_t6::AttachmentRef, &asset_t6::AttachmentUniqueRef)> = Vec::new();
    let mut attachment_names: Vec<&str> = attachments.keys().copied().collect();
    attachment_names.sort_unstable();
    for name in &weapon_order {
        let Some(base) = name.strip_suffix("_mp") else { continue };
        for sight in asset_game::T6_SIGHT_ATTACHMENTS {
            if let (Some(a), Some(u)) = (
                attachments.get(sight),
                uniques.get(format!("au_{base}_{sight}").as_str()),
            ) {
                sights.push((name.clone(), a, u));
            }
        }
        for &att in attachment_names.iter().filter(|a| !asset_game::T6_SIGHT_ATTACHMENTS.contains(a)) {
            if let (Some(a), Some(u)) = (attachments.get(att), uniques.get(format!("au_{base}_{att}").as_str())) {
                others.push((name.clone(), a, u));
            }
        }
    }
    let pap = pap_camo(&captures, &weapon_refs, 39);
    load_log.push(format!(
        "t6 Pack-a-Punch camo: {} gun models, {} upgraded weapons",
        pap.models.len(),
        pap.weapon_models.len()
    ));
    let mut model_names: BTreeSet<String> = BTreeSet::new();
    for w in weapon_refs.values() {
        if let Some(gun) = w.gun_models.first().filter(|n| real(n)) {
            model_names.insert(gun.clone());
        }
        let hands = w.def_model(fastfile_t6::layout::WeaponDef::handXModel);
        if real(hands) {
            model_names.insert(hands.to_owned());
        }
        // The magazine and (snipers) the scope, attached to the gun.
        for (_, model, ..) in &w.attach_view_models {
            if real(model) {
                model_names.insert(model.clone());
            }
        }
        // What a launcher shows loaded and fires: the RPG's rocket, the
        // ballistic knife's blade, the knife a gun's melee swings.
        for field in [
            fastfile_t6::layout::WeaponDef::rocketModel,
            fastfile_t6::layout::WeaponDef::projectileModel,
            fastfile_t6::layout::WeaponDef::additionalMeleeModel,
        ] {
            let model = w.def_model(field);
            if real(model) {
                model_names.insert(model.to_owned());
            }
        }
    }
    // bo2mp: the attachments' models (a sight, a suppressor, a grip).
    for (_, _, u) in sights.iter().chain(&others) {
        for model in [&u.view_model, &u.view_model_additional, &u.view_model_ads] {
            if real(model) {
                model_names.insert(model.clone());
            }
        }
    }
    // A multiplayer map's arms come with its factions' class scripts
    // (`mpbody/class_*`), named below with the script models.
    if !is_mp_map(&map_stem) {
        model_names.insert(T6_PLAYER_ARMS.to_owned());
        for arms in T6_OTHER_ARMS {
            model_names.insert(arms.to_owned());
        }
    }
    // Every other map's player arms (`c_zom_reporter_viewhands` in
    // Buried's mode zone): whatever viewhands its zones carry.
    for c in &captures[donor_n..] {
        for m in c.xmodels.iter() {
            if real(&m.name) && m.name.contains("_viewhands") {
                model_names.insert(m.name.clone());
            }
        }
    }
    // bo2zm M3: models the map's own scripts and entities name (perk
    // machines, the box, power-ups, zombie bodies and heads...), drawn as
    // script models and actors from the same catalog.
    let breakable_set = destructible_models(&captures);
    let script_models = script_model_names(&captures, &breakable_set);
    load_log.push(format!(
        "t6 script models: {} named by the map's scripts and entities",
        script_models.len()
    ));
    model_names.extend(script_models);

    // Third-person gun models too (a weapon cannot be given without one).
    let mut world_names: BTreeSet<String> = BTreeSet::new();
    for w in weapon_refs.values() {
        if let Some(world) = w.world_models.first().filter(|n| real(n)) {
            world_names.insert(world.clone());
        }
    }
    // bo2mp: and each attachment's world model (on the gun in other
    // players' hands and on the floor).
    for (_, _, u) in sights.iter().chain(others.iter()) {
        if real(&u.third_person_mesh) {
            world_names.insert(u.third_person_mesh.clone());
        }
    }
    // Effect models: shell casings, debris.
    let mut fx_model_names: BTreeSet<String> = BTreeSet::new();
    for c in &captures {
        for e in c.fx.iter().filter(|e| real(&e.name)) {
            for el in &e.elems {
                for v in &el.visuals {
                    if let asset_t6::FxVisualRef::Model(n) = v
                        && real(n)
                    {
                        fx_model_names.insert(n.clone());
                    }
                }
            }
        }
    }
    let all_models: BTreeSet<String> = model_names
        .union(&world_names)
        .cloned()
        .chain(fx_model_names.iter().cloned())
        .collect();

    // Materials the guns, arms and effects draw with, linked per zone.
    let homes = material_homes(&captures);
    let mut wanted = wanted_materials(&captures, &all_models, &homes);
    // The map's brush models with surfaces (doors), drawn as models.
    let map_ci = donor_n + pre_caps.len();
    let solid_models = map_capture
        .world
        .as_ref()
        .map(brush_model_refs)
        .unwrap_or_default();
    for srf in solid_models.iter().flat_map(|m| m.lod0.iter()) {
        if let Some(k) = srf.material {
            wanted[map_ci].insert(k.index);
        }
    }
    for (_, swaps) in pap.models.values() {
        for (_, camo, _) in swaps {
            if let Some(&(ci, mi)) = homes.get(camo.trim_start_matches(',')) {
                wanted[ci].insert(mi);
            }
        }
    }
    // Images by name, from the zone that holds their pixels.
    let mut image_homes: HashMap<String, asset_t6::ImageRef> = HashMap::new();
    for c in &captures {
        for img in &c.images {
            if real(&img.name) {
                image_homes.insert(img.name.clone(), img.clone());
            }
        }
    }
    // bo2mp: a world colour map named `,<image>` is a picture an earlier
    // zone holds (Carrier's parked jets and drone); the world, linked before
    // those zones were read, drew it flat. Give it its pixels now. (Shine
    // maps too: Aftermath's puddles' `,global_normal_flat_16x16`.)
    if let Some(packs) = packs {
        let (mut given, mut left) = (Vec::new(), Vec::new());
        for img in mat_set.images.iter_mut() {
            let Some(name) = img
                .name
                .as_str()
                .strip_suffix("#flat")
                .and_then(|n| n.strip_prefix(','))
            else {
                continue;
            };
            match image_homes
                .get(name)
                .map(|real| (real, super::t6_materials::decode(packs, real)))
            {
                Some((real, Ok(gpu))) => {
                    img.decoded = Some(std::sync::Arc::new(gpu));
                    img.texture_kind = real.map_kind;
                    img.width = real.width;
                    img.height = real.height;
                    img.depth = real.depth;
                    img.tier_total = real.tier_total;
                    given.push(name.to_owned());
                }
                _ => left.push(name.to_owned()),
            }
        }
        load_log.push(format!(
            "t6 world colour maps from earlier zones: {} given pixels {given:?}, {} not held anywhere {left:?}",
            given.len(),
            left.len()
        ));
    }
    let mut local: Vec<HashMap<usize, usize>> = Vec::with_capacity(captures.len());
    for (c, want) in captures.iter().zip(&wanted) {
        let linked = super::t6_materials::link_materials_with(
            mat_set,
            c,
            packs,
            want.iter().copied(),
            &c.zone,
            Some(&image_homes),
        );
        for line in linked.load_log {
            load_log.push(format!("t6 combat ({}): {line}", c.zone));
        }
        local.push(linked.local);
    }

    // A model surface's material, in the catalog: the capture's own row
    // when it linked it, else the row of the zone that defines it.
    let catalog_material = |ci: usize, key: asset_t6::AssetIdentity| -> Option<usize> {
        let defined_elsewhere = || {
            let bare = captures[ci].materials.get(key.index)?.name.trim_start_matches(',');
            let &(home_capture, home_row) = homes.get(bare)?;
            local[home_capture].get(&home_row).copied()
        };
        local[ci].get(&key.index).copied().or_else(defined_elsewhere)
    };
    let find_model = |name: &str| {
        captures
            .iter()
            .enumerate()
            .rev()
            .find_map(|(ci, c)| c.xmodels.iter().find(|m| m.name == name).map(|m| (ci, m)))
    };

    // First-person meshes: every gun and the players' arms.
    let mut fpv = FpvMeshBuild::default();
    fpv.file_new_under(asset_core::AssetCategoryScope::T6);
    let mut fpv_missing = Vec::new();
    for name in &model_names {
        let Some((ci, model)) = find_model(name) else {
            fpv_missing.push(name.clone());
            continue;
        };
        // bo2mp: a sight's reticle surface is drawn by the HUD at
        // infinity (its shader's projection), not as a picture on the glass.
        let resolve = |k: asset_t6::AssetIdentity| {
            if reticle_material(&captures, &homes, ci, k).is_some() {
                return None;
            }
            catalog_material(ci, k)
        };
        match asset_model::capture_model_skel_t6(model, resolve) {
            Some(mut bone_rig) => {
                bone_rig.face_bits = model_surface_flags(captures[ci], model);
                fpv.put_into_namespace(asset_core::AssetCategoryScope::T6, bone_rig, Some(mat_set))
            }
            None => fpv_missing.push(format!("{name} (unreadable)")),
        }
    }
    for solid in &solid_models {
        match asset_model::capture_model_skel_t6(solid, |k| catalog_material(map_ci, k)) {
            Some(bone_rig) => fpv.put_into_namespace(asset_core::AssetCategoryScope::T6, bone_rig, Some(mat_set)),
            None => fpv_missing.push(format!("{} (brush model)", solid.name)),
        }
    }
    if !solid_models.is_empty() {
        load_log.push(format!(
            "t6 brush models drawn: {}",
            solid_models
                .iter()
                .map(|m| format!(
                    "{} ({} surfaces [{}], {:.0} {:.0} {:.0} .. {:.0} {:.0} {:.0})",
                    m.name,
                    m.lod0.len(),
                    m.materials
                        .iter()
                        .flatten()
                        .filter_map(|k| map_capture.materials.get(k.index))
                        .map(|mat| mat.name.trim_start_matches(','))
                        .collect::<Vec<_>>()
                        .join(" "),
                    m.lo_corner[0],
                    m.lo_corner[1],
                    m.lo_corner[2],
                    m.hi_corner[0],
                    m.hi_corner[1],
                    m.hi_corner[2]
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    // The upgraded guns' copies, their camo surfaces swapped.
    for (copy, (gun, swaps)) in &pap.models {
        let Some((ci, model)) = find_model(gun) else {
            continue;
        };
        // Each surface's camo tiling, in the order the surfaces are read.
        let tiles: std::cell::RefCell<Vec<[f32; 2]>> = std::cell::RefCell::new(Vec::new());
        let swapped = |k: asset_t6::AssetIdentity| -> Option<usize> {
            let name = captures[ci]
                .materials
                .get(k.index)
                .map_or("", |m| m.name.trim_start_matches(','));
            match swaps
                .iter()
                .find(|(base, _, _)| base.trim_start_matches(',') == name)
            {
                Some((_, camo, tile)) => {
                    tiles.borrow_mut().push(*tile);
                    let &(hc, hm) = homes.get(camo.trim_start_matches(','))?;
                    local[hc].get(&hm).copied()
                }
                None => {
                    tiles.borrow_mut().push([1.0, 1.0]);
                    catalog_material(ci, k)
                }
            }
        };
        if let Some(mut bone_rig) = asset_model::capture_model_skel_t6(model, swapped) {
            bone_rig.name = copy.clone();
            // The camo surfaces' texture coordinates times the camo's
            // tiling (BO2's camo shader scales them; ours samples as given).
            let tiles = tiles.into_inner();
            if tiles.len() == bone_rig.facet_vertex_ranges.len() {
                for (&(start, count), tile) in bone_rig.facet_vertex_ranges.iter().zip(&tiles) {
                    if *tile == [1.0, 1.0] {
                        continue;
                    }
                    for uv in bone_rig.texcoord_set.iter_mut().skip(start).take(count) {
                        uv[0] *= tile[0];
                        uv[1] *= tile[1];
                    }
                    // The packed rows the gun draws from: two halves, v low
                    // and u high (T6's u-low word turned a half round).
                    for row in bone_rig.compact_vertices.iter_mut().skip(start).take(count) {
                        let tex = u32::from_le_bytes([row[20], row[21], row[22], row[23]]);
                        let v = asset_t6::short_float_to_full(tex as u16) * tile[1];
                        let u = asset_t6::short_float_to_full((tex >> 16) as u16) * tile[0];
                        let tex = u32::from(f32_to_half(v)) | (u32::from(f32_to_half(u)) << 16);
                        row[20..24].copy_from_slice(&tex.to_le_bytes());
                    }
                }
            }
            fpv.put_into_namespace(asset_core::AssetCategoryScope::T6, bone_rig, Some(mat_set));
        }
    }
    // Third-person gun models.
    let mut level_placed_weapons = WorldWeaponBuild::default();
    level_placed_weapons.file_new_under(asset_core::AssetCategoryScope::T6);
    let mut world_missing = Vec::new();
    for name in &world_names {
        let Some((ci, model)) = find_model(name) else {
            world_missing.push(name.clone());
            continue;
        };
        match asset_model::capture_model_skel_t6(model, |k| catalog_material(ci, k)) {
            Some(bone_rig) => {
                level_placed_weapons.put_into_namespace(asset_core::AssetCategoryScope::T6, bone_rig, Some(mat_set))
            }
            None => world_missing.push(format!("{name} (unreadable)")),
        }
    }
    load_log.push(format!(
        "t6 world gun models: {} built, {} missing {:?}",
        level_placed_weapons.len(),
        world_missing.len(),
        world_missing.iter().take(8).collect::<Vec<_>>()
    ));
    load_log.push(format!(
        "t6 first-person meshes: {} built, {} missing {:?}",
        fpv.len(),
        fpv_missing.len(),
        fpv_missing.iter().take(8).collect::<Vec<_>>()
    ));

    // Animations: every one, later zones win.
    let mut animation_refs = XAnimBuild::default();
    animation_refs.file_under(asset_core::AssetCategoryScope::T6);
    let mut anim_n = 0usize;
    for c in &captures {
        for a in &c.animation_refs {
            anim_n += usize::from(animation_refs.insert_t6(a));
        }
    }
    load_log.push(format!(
        "t6 animations: {anim_n} captured, {} kept, {} gaps",
        animation_refs.len(),
        animation_refs.missed_frames
    ));

    // The weapon catalog.
    let mut catalog = ArmoryRoster::default();
    catalog.set_walk_scope(asset_core::AssetCategoryScope::T6);
    for name in &weapon_order {
        let w = weapon_refs[name];
        // A dual-wield weapon's left-hand weapon (its gun and dw_left_*
        // animations join the right one's viewmodel).
        let left = weapon_refs
            .get(w.def_string(fastfile_t6::layout::WeaponDef::szDualWieldWeaponName))
            .copied();
        catalog.capture_t6(w, left, &pap.weapon_models);
    }
    // bo2mp gun brain: the same weapons' raw facts.
    let mut gun_facts = String::new();
    // A weapon's animation slots and whether it has a gun model.
    let anims_and_gun = |w: &asset_t6::WeaponRef| {
        let slots = w
            .animation_refs
            .iter()
            .take(128)
            .enumerate()
            .filter(|(_, a)| real(a))
            .fold(0u128, |m, (i, _)| m | 1 << i);
        (slots, w.gun_models.first().is_some_and(|m| real(m)))
    };
    for name in &weapon_order {
        let w = weapon_refs[name];
        if w.def.is_empty() {
            continue;
        }
        let (slots, has_gun) = anims_and_gun(w);
        let dual = w.def_string(fastfile_t6::layout::WeaponDef::szDualWieldWeaponName);
        gun_facts.push_str(&weapon_t6::facts::dump_line(
            name,
            &w.variant,
            &w.def,
            slots,
            has_gun,
            if real(&w.alt_weapon_name) { &w.alt_weapon_name } else { "" },
            if real(dual) { dual } else { "" },
        ));
        gun_facts.push('\n');
    }
    // BO2 links an attachment's alt-mode gun at load (a launcher, the Hybrid's other optic): the
    // zones hold no `<gun>+<attachment>` weapons, so each such pair gets the gun's facts and that link.
    for (name, a, u) in sights.iter().chain(&others) {
        let (Some(w), true) = (weapon_refs.get(name.as_str()), weapon_refs.contains_key(u.alt_weapon_name.as_str())) else {
            continue;
        };
        if w.def.is_empty() {
            continue;
        }
        let (slots, has_gun) = anims_and_gun(w);
        gun_facts.push_str(&weapon_t6::facts::dump_line(
            &format!("{name}+{}", a.name),
            &w.variant,
            &w.def,
            slots,
            has_gun,
            &u.alt_weapon_name,
            "",
        ));
        gun_facts.push('\n');
    }
    // The attachments' own bytes, last, so the guns' line numbers stay their indexes: the gun
    // brain folds them into each `<gun>+<attachment>` row (spread, sway, idle, perks, laser).
    let mut att_names: Vec<&&str> = attachments.keys().collect();
    att_names.sort();
    for name in att_names {
        gun_facts.push_str(&weapon_t6::facts::attachment_line(name, &attachments[*name].bytes));
        gun_facts.push('\n');
    }
    // And each gun's one-attachment uniques (`au_<gun>_<att>`, its base `au_<gun>_none`): the
    // first raise, alt raise and drop, and shell-by-shell counts an attachment overrides.
    let mut unique_names: Vec<&&str> =
        uniques.keys().filter(|n| n.starts_with("au_") && !n.contains('+')).collect();
    unique_names.sort();
    for name in unique_names {
        gun_facts.push_str(&weapon_t6::facts::unique_line(name, &uniques[*name].bytes));
        gun_facts.push('\n');
    }
    // bo2mp: every gun with each of its sight attachments.
    let mut sight_n = 0usize;
    let mut reticle_pictures: HashMap<(String, bool), (f32, std::sync::Arc<bevy::prelude::Image>)> =
        HashMap::default();
    let mut sight_reticles: Vec<(String, std::sync::Arc<bevy::prelude::Image>)> = Vec::new();
    for (name, a, u) in &sights {
        let optic = if real(&u.view_model) { u.view_model.as_str() } else { "" };
        let hybrid = a.name == "dualoptic" && !u.alt_weapon_name.is_empty();
        // The picture of one of the sight model's reticles (`top`: the
        // little dot above a Hybrid's scope, which its first mode aims
        // through; the other mode, and every other sight, the first below).
        let mut picture = |top: bool| match (packs, optic) {
            (_, "") | (None, _) => None,
            (Some(packs), optic) => match reticle_pictures.get(&(optic.to_owned(), top)) {
                Some(found) => Some(found.clone()),
                None => sight_reticle(&captures, &homes, packs, optic, top).map(|(scale, img)| {
                    let found = (scale, std::sync::Arc::new(img));
                    reticle_pictures.insert((optic.to_owned(), top), found.clone());
                    found
                }),
            },
        };
        let reticle = picture(hybrid);
        let anim_ms = |name: &str| {
            captures.iter().flat_map(|c| c.animation_refs.iter()).find(|a| a.name.eq_ignore_ascii_case(name)).and_then(|a| {
                (a.frame_rate_hz > 0.0).then(|| (f64::from(a.frame_total) / f64::from(a.frame_rate_hz) * 1000.0).round() as i32)
            })
        };
        let scale = reticle.as_ref().map_or(0.0, |(scale, _)| *scale);
        if catalog.capture_t6_attachment(weapon_refs[name], a, u, scale, None, None, &anim_ms) {
            // Hybrid Optic: its other mode is a weapon of its own, with the
            // other reticle.
            if hybrid && let Some(alt) = weapon_refs.get(u.alt_weapon_name.as_str()) {
                let other = picture(false);
                let other_scale = other.as_ref().map_or(0.0, |(scale, _)| *scale);
                if catalog.capture_t6_attachment(alt, a, u, other_scale, Some(&format!("{name}+{}", a.name)), None, &anim_ms) {
                    if let Some((_, img)) = other {
                        sight_reticles.push((format!("reticle:{}+{}", u.alt_weapon_name, a.name), img));
                    }
                }
            }
            sight_n += 1;
            if let Some((_, img)) = reticle {
                sight_reticles.push((format!("reticle:{name}+{}", a.name), img));
            }
        }
    }
    load_log.push(format!(
        "t6 sight reticles: {} sight models with a reticle ({:?})",
        reticle_pictures.len(),
        reticle_pictures.iter().map(|((m, top), (s, _))| format!("{m}{} {s}", if *top { " (top)" } else { "" })).collect::<Vec<_>>()
    ));
    load_log.push(format!(
        "t6 sight attachments: {sight_n} guns with a sight ({} attachments, {} uniques read)",
        attachments.len(),
        uniques.len()
    ));
    // bo2mp: every gun with each of its other attachments (no reticle).
    let no_anim = |_: &str| None;
    let mut other_n = 0usize;
    for (name, a, u) in &others {
        // Dual Wield's left-hand gun.
        let left = weapon_refs.get(u.dual_wield_weapon_name.as_str()).copied();
        if !catalog.capture_t6_attachment(weapon_refs[name], a, u, 0.0, None, left, &no_anim) {
            continue;
        }
        other_n += 1;
        // An underbarrel's launcher (`gl_<gun>_mp`), switched to from it.
        // Select Fire's mode names the gun's ammo and clip: one magazine.
        if let Some(alt) = weapon_refs.get(u.alt_weapon_name.as_str()) {
            let gun = weapon_refs[name];
            if catalog.capture_t6_attachment(alt, a, u, 0.0, Some(&format!("{name}+{}", a.name)), None, &no_anim)
                && alt.ammo_name == gun.ammo_name
                && alt.collision_label == gun.collision_label
            {
                catalog.mark_t6_shared_magazine(&format!("{}+{}", u.alt_weapon_name, a.name));
            }
        }
    }
    load_log.push(format!("t6 other attachments: {other_n} guns with an attachment (stats from the attachment)"));
    // bo2mp: every gun with two of its attachments (`<gun>+<a>+<b>`, the
    // two in name order): a BO2 class gun takes on every attachment it
    // carries, which gives `an94_mp+fmj+reflex` the sight's model, zoom and
    // ADS and FMJ's penetration. Two on the same attachment point (two
    // sights, two muzzles) never go together. Built from the game data
    // alone, so every machine builds the same weapons (the content digest
    // stays in step). A third attachment resolves to the pair with the
    // sight (`sim::t6::weapon`). Code fix, needs local proof.
    let mut per_gun: HashMap<&str, Vec<(&asset_t6::AttachmentRef, &asset_t6::AttachmentUniqueRef)>> =
        HashMap::default();
    for (name, a, u) in sights.iter().chain(others.iter()) {
        per_gun.entry(name.as_str()).or_default().push((*a, *u));
    }
    let reticle_of: HashMap<String, std::sync::Arc<bevy::prelude::Image>> = sight_reticles.iter().cloned().collect();
    let point = |a: &asset_t6::AttachmentRef| a.int_from_bytes(fastfile_t6::layout::WeaponAttachment::attachmentPoint);
    let mut pair_n = 0usize;
    for name in &weapon_order {
        let Some(list) = per_gun.get(name.as_str()) else { continue };
        for (i, x) in list.iter().enumerate() {
            for y in &list[i + 1..] {
                let (px, py) = (point(x.0), point(y.0));
                if x.0.name == y.0.name || (px > 0 && px == py) {
                    continue;
                }
                let sight = |a: &asset_t6::AttachmentRef| asset_game::T6_SIGHT_ATTACHMENTS.contains(&a.name.as_str());
                let (first, second) = if sight(y.0) || (!sight(x.0) && y.0.name < x.0.name) { (*y, *x) } else { (*x, *y) };
                let left = weapon_refs.get(second.1.dual_wield_weapon_name.as_str()).copied();
                let second_alt = weapon_refs.get(second.1.alt_weapon_name.as_str()).copied();
                if !catalog.capture_t6_attachment_pair(weapon_refs[name], first, second, second_alt, left) {
                    continue;
                }
                pair_n += 1;
                // The sight's reticle on the pair (and on the Hybrid's
                // other mode's pair).
                let combo = asset_game::t6_attachment_combo_name(name, &[first.0.name.as_str(), second.0.name.as_str()]);
                if let Some(img) = reticle_of.get(&format!("reticle:{name}+{}", first.0.name)) {
                    sight_reticles.push((format!("reticle:{combo}"), img.clone()));
                }
                let alt = first.1.alt_weapon_name.as_str();
                if let (false, Some(img)) = (alt.is_empty(), reticle_of.get(&format!("reticle:{alt}+{}", first.0.name))) {
                    let atts = combo.split_once('+').map_or("", |(_, a)| a);
                    sight_reticles.push((format!("reticle:{alt}+{atts}"), img.clone()));
                }
            }
        }
    }
    load_log.push(format!("t6 attachment pairs: {pair_n} guns with two attachments (both attachments' models and stats)"));
    // BG_GetWeaponHitLocationMultiplier: an attachment's own hit table (the sniper Suppressors).
    let hit_n = catalog
        .apply_t6_unique_hit_tables(|n| uniques.get(n).and_then(|u| u.location_damage.as_deref()));
    load_log.push(format!(
        "t6 attachment hit tables: {hit_n} guns whose attachments change a hit's damage"
    ));
    // bo2zm fix list 2: a rolling grenade rests and rolls on its body: the
    // nearest face of its projectile model's bounds to the model's origin
    // (the frag: a ball of about 1.4 units around it, the fuse above).
    let mut rolling = Vec::new();
    for name in &weapon_order {
        let w = weapon_refs[name];
        if w.def_i32(fastfile_t6::layout::WeaponDef::isRollingGrenade) == 0 {
            continue;
        }
        let proj_model = w.def_model(fastfile_t6::layout::WeaponDef::projectileModel);
        if let Some((_, m)) = find_model(proj_model) {
            let radius = (0..3)
                .flat_map(|a| [m.lo_corner[a].abs(), m.hi_corner[a].abs()])
                .fold(f32::INFINITY, f32::min);
            if radius.is_finite() && radius > 0.1 {
                catalog.set_t6_rolling_radius(name, radius);
                rolling.push(format!("{name} {radius:.2}"));
            }
        }
    }
    load_log.push(format!("t6 rolling grenades: {}", rolling.join(", ")));
    let captured = catalog.len();
    let mut weapons = catalog.into_build();
    weapons.set_scope_tag(asset_core::AssetCategoryScope::T6);
    let listed = weapons.len();
    load_log.push(format!("t6 weapons: {captured} captured, {listed} in the catalog"));

    // Effects: every one, later zones win.
    let mut fx = EffectRoster::default();
    let mut fx_n = 0usize;
    for c in &captures {
        for e in &c.fx {
            fx_n += usize::from(fx.capture_t6(e));
        }
    }
    load_log.push(format!("t6 effects: {fx_n} captured, {} kept", fx.len()));
    // bo2zm fix list 3: a gun whose first-person flash is the flash other
    // players see (it names the same effect for both) gets a first-person
    // copy at `T6_FIRST_PERSON_FLASH_SCALE` (our choice; see there). Bullet
    // guns only: the launchers' flashes are launch smoke or already small.
    // A flash made for the first person keeps its own size even without
    // parts drawn with the gun (the Ray Gun's).
    let mut first_person = Vec::new();
    let mut seen_set = BTreeSet::new();
    for name in &weapon_order {
        let w = weapon_refs[name];
        let view = w.def_fx(fastfile_t6::layout::WeaponDef::viewFlashEffect);
        let world = w.def_fx(fastfile_t6::layout::WeaponDef::worldFlashEffect);
        let bullet = w.def_i32(fastfile_t6::layout::WeaponDef::weapType) == 0;
        if !bullet || view.is_empty() || view != world || !seen_set.insert(view) {
            continue;
        }
        if let Some((before, after)) = fx.add_t6_first_person_flash(view) {
            first_person.push(format!("{view} {before:.1}->{after:.1}"));
        }
    }
    load_log.push(format!(
        "t6 first-person flashes made from far-view ones (x{}): {}",
        asset_game::T6_FIRST_PERSON_FLASH_SCALE,
        first_person.join(", ")
    ));
    let mut effect_meshes = asset_game::EffectMeshRoster::default();
    let mut fx_model_missing = Vec::new();
    for name in &fx_model_names {
        match find_model(name)
            .and_then(|(ci, m)| asset_model::capture_model_skel_t6(m, |k| catalog_material(ci, k)))
        {
            Some(bone_rig) => effect_meshes.insert_skel(bone_rig, mat_set),
            None => fx_model_missing.push(name.clone()),
        }
    }
    load_log.push(format!(
        "t6 effect models: {} built, {} missing {:?}",
        effect_meshes.len(),
        fx_model_missing.len(),
        &fx_model_missing[..fx_model_missing.len().min(8)]
    ));
    // Tracers: every one, later zones win.
    let mut tracers = asset_game::TracerRoster::default();
    let mut tracer_n = 0usize;
    for c in &captures {
        for t in &c.tracers {
            tracer_n += usize::from(tracers.capture_t6(t));
        }
    }
    load_log.push(format!(
        "t6 tracers: {tracer_n} captured, {} kept",
        tracers.len()
    ));
    let impact_effect = captures
        .iter()
        .rev()
        .find_map(|c| c.impact_tables.first())
        .map(SelfContainedImpactFxTable::from_t6);
    load_log.push(format!(
        "t6 impact table: {}",
        impact_effect.as_ref().map_or_else(
            || "none".to_owned(),
            |t| format!("{} ({} rows)", t.name, t.line_total())
        )
    ));

    // Sound: every bank's aliases, the clips in the bank files beside the
    // zones (`<install>/sound`).
    let map = stem_text(level_file_path);
    let sound_dir = Some(zone_dir.as_path())
        .and_then(Path::parent)
        .and_then(Path::parent)
        .map(|install| install.join("sound"));
    let (map_fx, room_tone) = map_scripts(&captures, &map, &mut load_log);
    // The language zones beside zone/all (zone/english/en_*.ff): the
    // English strings (hints, perk and power-up names) and the voices'
    // aliases (the announcer's, the players'). bo2zm: and the mode zone's
    // (`en_so_zclassic_zm_transit.ff`: TranZit's player and bus driver
    // lines; its `en_zm_transit.ff` is empty, so the driver never spoke).
    let lang_dir = Some(zone_dir.as_path())
        .and_then(Path::parent)
        .map(|d| d.join("english"));
    let mode = mode_zone(&map);
    let lang_zones: Vec<String> = if is_mp_map(&map) {
        // bo2mp: the shared zones', then the factions' (team voices), then
        // the map's.
        T6_MP_LANG_ZONES
            .iter()
            .map(|z| (*z).to_owned())
            .chain(
                captures
                    .iter()
                    .filter(|c| c.zone.starts_with("faction_"))
                    .map(|c| c.zone.clone()),
            )
            .chain(std::iter::once(map.clone()))
            .collect()
    } else {
        [
            "code_post_gfx_zm",
            "common_zm",
            "patch_zm",
            "ui_zm",
            "patch_ui_zm",
            map.as_str(),
            mode.as_str(),
        ]
        .iter()
        .map(|z| (*z).to_owned())
        .collect()
    };
    let lang_caps: Vec<ZoneCapture> = lang_zones
        .iter()
        .filter_map(|z| {
            let path = asset_transport::t6_extra::file_in(lang_dir.as_ref()?, &format!("en_{z}.ff"));
            if !path.is_file() {
                return None;
            }
            match asset_t6::capture_zone(&path) {
                Ok(c) => Some(c),
                Err(e) => {
                    load_log.push(format!("t6 language zone gap: {e}"));
                    None
                }
            }
        })
        .collect();
    // bo2mp: each alias's length in ms (its longest variant's clip), for
    // the scripts' soundgetplaybacktime (the announcer waits one line out).
    let mut sound_lengths: Vec<(String, u32)> = Vec::new();
    let sound = match sound_dir {
        Some(dir) if dir.is_dir() => {
            let bases = bank_bases(&captures, &map);
            let base_refs: Vec<&str> = bases.iter().map(String::as_str).collect();
            let (index, bank_report) = asset_audio::T6BankIndex::open(&dir, &base_refs);
            load_log.extend(bank_report);
            let banks: Vec<&asset_t6::SndBankRef> = captures
                .iter()
                .copied()
                .chain(lang_caps.iter())
                .flat_map(|c| c.sound_banks.iter())
                .collect();
            let globals = captures.iter().find_map(|c| c.snd_globals.as_ref());
            let (mut sound_cat, census) = asset_audio::build_t6_sound_catalog(&banks, globals, &index);
            load_log.push(format!(
                "t6 sound: {} aliases, {} variants ({} with a clip, {} without); {} falloff curves",
                census.alias_count,
                census.variants,
                census.with_clip,
                census.without_clip,
                globals.map_or(0, |g| g.curves.len())
            ));
            // bo2mp: the rumbles (rumble/<name> and its .rmb graphs) ride
            // the sound bank, as the legacy do. Empty stand-ins are skipped; a
            // later zone wins a shared name.
            let mut rumbles = 0;
            for (name, bytes) in captures.iter().flat_map(|c| c.raw_files.iter()) {
                let name = name.to_ascii_lowercase().replace('\\', "/");
                if !name.starts_with("rumble/") || bytes.is_empty() {
                    continue;
                }
                // One trailing NUL terminator is dropped.
                let body = bytes.strip_suffix(&[0u8]).unwrap_or(&bytes[..]).to_vec();
                rumbles += 1;
                sound_cat.rawfiles.insert((asset_core::AssetCategoryScope::T6, name), body);
            }
            load_log.push(format!("t6 sound: {rumbles} rumble files"));
            for s in &sound_cat.sounds {
                let ms = s
                    .aliases
                    .iter()
                    .filter_map(|a| a.streamed.as_ref())
                    .filter_map(|(_, id)| u32::from_str_radix(id, 16).ok())
                    .filter_map(|id| index.clip_by_id(id))
                    .map(|(_, e)| (u64::from(e.frame_count) * 1000 / u64::from(e.rate().max(1))) as u32)
                    .max();
                if let Some(ms) = ms {
                    sound_lengths.push((s.name.to_ascii_lowercase(), ms));
                }
            }
            sound_cat.scripted_map_fx = Some(map_fx);
            let mut echoes: Vec<String> = sound_cat
                .radverbs
                .iter()
                .map(|(name, v)| format!("{name} {v:?}"))
                .collect();
            echoes.sort();
            load_log.push(format!("t6 sound: {} room echoes: {}", echoes.len(), echoes.join("; ")));
            Ok(sound_cat)
        }
        _ => Err("Black Ops II sound folder not found beside the zones".to_owned()),
    };
    let models_missing: Vec<String> = fpv_missing
        .iter()
        .chain(world_missing.iter())
        .cloned()
        .collect();
    // The guns checked: Nuketown Zombies' list, or (bo2mp) every
    // multiplayer gun the zones carry.
    let census_names: Vec<String> = if is_mp_map(&map) {
        let mut v: Vec<String> = weapon_order
            .iter()
            .filter(|n| n.ends_with("_mp"))
            .cloned()
            .collect();
        v.sort();
        v
    } else {
        NUKETOWN_WEAPONS.iter().map(|n| (*n).to_owned()).collect()
    };
    let census_lines = census(
        &captures,
        &census_names,
        &weapon_refs,
        &models_missing,
        &animation_refs,
        &fx,
        impact_effect.as_ref(),
        sound
            .as_ref()
            .ok()
            .and_then(|c| c.scripted_map_fx.as_ref())
            .unwrap_or(&asset_audio::ScriptedMapFx::default()),
        room_tone.as_deref(),
        sound.as_ref().ok(),
    );
    for line in &census_lines {
        diag::info!(World, "{line}");
    }
    load_log.extend(census_lines);
    let mut scripts = crate::T6ScriptSet::default();
    let (mut playeranim, mut playeranim_types) = (None, None);
    // The match's zones in Black Ops II's override order (lowest priority
    // first, then load order), so each later copy of a name winning keeps
    // the copy the game keeps; the menu's Zombies zones stay first.
    let mut by_priority = captures.clone();
    by_priority[zm_caps.len()..].sort_by_key(|c| zone_priority(&c.zone));
    let mut script_at: HashMap<String, usize> = HashMap::new();
    for c in &by_priority {
        for (name, bytes) in &c.raw_files {
            if let Some(n) = name.strip_prefix("script:")
                && !n.starts_with(',')
                && n.ends_with(".gsc")
            {
                // One copy of each script, where its first copy stood.
                match script_at.get(&n.to_ascii_lowercase()) {
                    Some(&i) => scripts.objects[i] = bytes.clone(),
                    None => {
                        script_at.insert(n.to_ascii_lowercase(), scripts.objects.len());
                        scripts.objects.push(bytes.clone());
                    }
                }
            }
        }
    }
    for c in &captures {
        for t in &c.string_tables {
            if real(&t.name) {
                scripts
                    .tables
                    .push((t.name.clone(), t.columns, t.rows, t.cells.clone()));
            }
        }
        for v in &c.vehicles {
            scripts.vehicles.retain(|n| n.name != v.name);
            scripts.vehicles.push(v.clone());
        }
        scripts.entities.extend(
            c.map_ents
                .iter()
                .map(|t| destructible_entities(t, &breakable_set)),
        );
        // codmp: the clipmap's loose and breakable props, as script models
        // that break by a definition of their own.
        let (text, defs) = dyn_ent_props(c, &captures);
        if !defs.is_empty() {
            load_log.push(format!("codmp t6 dynents: {} props as breakables", defs.len()));
            scripts.entities.push(text);
            for d in defs {
                scripts.destructibles.retain(|x| x.name != d.name);
                scripts.destructibles.push(d);
            }
        }
        // bo2mp: every destructible the map's entities can name.
        for d in destructible_defs(&[*c]) {
            scripts.destructibles.retain(|x| x.name != d.name);
            scripts.destructibles.push(d);
        }
        for z in &c.zbarriers {
            scripts.zbarriers.retain(|o| !o.name.eq_ignore_ascii_case(&z.name));
            scripts.zbarriers.push(z.clone());
        }
        if !c.path_nodes.is_empty() {
            scripts.path_nodes = c.path_nodes.clone();
            scripts.path_tables = c.path_tables.clone();
        }
        for (name, bytes) in &c.raw_files {
            let folded = name.to_ascii_lowercase();
            if let Some(shock) = folded.strip_prefix("shock/").and_then(|n| n.strip_suffix(".shock")) {
                scripts.shocks.retain(|(n, _)| n != shock);
                scripts
                    .shocks
                    .push((shock.to_owned(), String::from_utf8_lossy(bytes).into_owned()));
            }
            if name.starts_with("animstatedefs/") && name.ends_with(".asd") {
                let text = String::from_utf8_lossy(bytes).into_owned();
                scripts.animstatedefs.retain(|(n, _)| n != name);
                scripts.animstatedefs.push((name.clone(), text));
            }
            // bo2mp: the shellshocks (`shock/flashbang.shock`, ...).
            let folded = name.to_ascii_lowercase().replace('\\', "/");
            if let Some(shock) = folded
                .strip_prefix("shock/")
                .and_then(|n| n.strip_suffix(".shock"))
            {
                let text = String::from_utf8_lossy(bytes).into_owned();
                scripts.shocks.retain(|(n, _)| n != shock);
                scripts.shocks.push((shock.to_owned(), text));
            }
            if name.contains("/gamesettings_") && name.ends_with(".cfg") {
                let text = String::from_utf8_lossy(bytes).into_owned();
                scripts.gamesettings.retain(|(n, _)| n != name);
                scripts.gamesettings.push((name.clone(), text));
            }
            // bo2mp: how a dead body goes limp.
            if folded == "ragdoll.cfg" {
                scripts.ragdoll = Some(String::from_utf8_lossy(bytes).into_owned());
            }
            // bo2mp: how players look to others (third-person animations).
            if name == "mp/playeranim.script" {
                playeranim = Some(String::from_utf8_lossy(bytes).into_owned());
            }
            if name == "mp/playeranimtypes.txt" {
                playeranim_types = Some(String::from_utf8_lossy(bytes).into_owned());
            }
        }
    }
    scripts.playeranim = playeranim.zip(playeranim_types);
    // bo2zm: the donors' grafted states go into every animstatedef of this map (first, so
    // the map's own states win a name).
    let (grafts, own): (Vec<_>, Vec<_>) = std::mem::take(&mut scripts.animstatedefs)
        .into_iter()
        .partition(|(n, _)| n.starts_with("animstatedefs/bo2zm_graft_"));
    let graft = grafts.into_iter().map(|(_, t)| t).collect::<Vec<_>>().join("\n");
    scripts.animstatedefs = own
        .into_iter()
        .map(|(n, t)| if graft.is_empty() { (n, t) } else { (n, format!("{graft}\n{t}")) })
        .collect();
    // Every animation's timing, root motion and notetracks; the zone the
    // game keeps a name from wins.
    let mut anim_facts: HashMap<String, crate::T6AnimFacts> = HashMap::new();
    for c in &by_priority {
        for a in c.animation_refs.iter().filter(|a| real(&a.name)) {
            anim_facts.insert(
                a.name.clone(),
                crate::T6AnimFacts {
                    name: a.name.clone(),
                    frame_total: a.frame_total,
                    fps: a.frame_rate_hz,
                    looping: a.looping,
                    delta_trans: a.delta_trans.clone(),
                    delta_quat: a.delta_quat.clone(),
                    notifies: a.notifies.clone(),
                },
            );
        }
    }
    scripts.anims = anim_facts.into_values().collect();
    let mut strings: HashMap<String, String> = HashMap::new();
    // bo2mp: the Zombies guns' names ("Ray Gun Mark II") are in Zombies'
    // own language zones; only their text is read, first, so the match's
    // own strings win any shared key.
    let zm_lang_caps: Vec<ZoneCapture> = if zm_weapon_zone_paths(level_file_path).is_empty() {
        Vec::new()
    } else {
        let paths: Vec<PathBuf> = T6_MP_ZM_LANG_ZONES
            .iter()
            .filter_map(|z| Some(lang_dir.as_ref()?.join(format!("en_{z}.ff"))))
            .filter(|p| p.is_file())
            .collect();
        capture_many(&paths).into_iter().filter_map(Result::ok).collect()
    };
    for c in zm_lang_caps.iter().chain(&lang_caps) {
        for (k, v) in &c.localize {
            strings.insert(k.clone(), v.clone());
        }
    }
    // Strings a mod zone carries (Zombies Declassified's mod_load), where
    // no language zone has the key.
    for c in &captures {
        for (k, v) in &c.localize {
            strings.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }
    // The donor maps' (the Hell's Retriever's name).
    for (k, v) in &donor_strings {
        strings.entry(k.clone()).or_insert_with(|| v.clone());
    }
    scripts.donor_effects = donor_effects;
    // bo2mp: the map's name as players know it (`mp/mapstable.csv` column
    // 3: `MPUI_LA` = "AFTERMATH").
    if is_mp_map(&map) {
        let key = captures
            .iter()
            .rev()
            .flat_map(|c| c.string_tables.iter())
            .find(|t| t.name.eq_ignore_ascii_case("mp/mapstable.csv"))
            .and_then(|t| {
                t.cells
                    .chunks(t.columns.max(1))
                    .find(|r| r.first().is_some_and(|c| c.eq_ignore_ascii_case(&map)))
                    .and_then(|r| r.get(3).cloned())
            });
        if let Some(key) = key {
            let shown = strings
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(&key))
                .map_or("-", |(_, v)| v.as_str());
            load_log.push(format!("t6 mp map name: {key} = {shown}"));
        }
    }
    scripts.strings = strings.into_iter().collect();
    scripts.sound_lengths = sound_lengths;
    // bo2mp: weapons a player can pick back up once placed or thrown (their
    // weapon file's bRetrievable: the hatchet, claymore, Bouncing Betty...),
    // as the engine's getretrievableweapons lists them. Black Ops II lists
    // the weapons the level itself parsed, so a multiplayer match never
    // holds a Zombies one: the menu's Zombies guns (`zm_caps`) stay out.
    // (`claymore_zm` and `knife_ballistic_zm` share a watcher name with
    // their `_mp` guns, and _weaponobjects' by-weapon lookup then finds
    // none: 12 script errors a match.)
    let match_weapons: BTreeSet<&str> = captures
        .iter()
        .skip(zm_caps.len())
        .flat_map(|c| c.weapons.iter().map(|w| w.name.as_str()))
        .collect();
    let retrievable = retrievable_weapons(
        weapon_refs
            .iter()
            .map(|(n, w)| (n.as_str(), w.def_u8(fastfile_t6::layout::WeaponDef::bRetrievable) != 0)),
        &match_weapons,
    );
    load_log.push(format!("t6 retrievable weapons: {}", retrievable.join(", ")));
    // bo2mp: each weapon's own melee damage (iMeleeDamage), to check the
    // knife against BO2 (a melee hit kills in one).
    let mut melee: Vec<String> = weapon_refs
        .iter()
        .filter(|(n, _)| ["knife_mp", "knife_held_mp", "mp7_mp", "an94_mp", "dsr50_mp", "fiveseven_mp", "riotshield_mp", "knife_ballistic_mp"].contains(&n.as_str()))
        .map(|(n, w)| format!("{n}={}", w.def_i32(fastfile_t6::layout::WeaponDef::iMeleeDamage)))
        .collect();
    melee.sort();
    load_log.push(format!("t6 melee damage: {}", melee.join(" ")));
    scripts.retrievable_weapons = retrievable;
    // Every sound alias the banks hold (the scripts' soundexists).
    if let Ok(cat) = &sound {
        scripts.sound_aliases = cat
            .sounds
            .iter()
            .map(|s| s.name.to_ascii_lowercase())
            .collect();
    }
    load_log.push(format!(
        "t6 scripts: {} server script objects, {} string tables, {} entity strings, {} path nodes, {} animations, {} animstatedefs, {} English strings, {} barrier types",
        scripts.objects.len(),
        scripts.tables.len(),
        scripts.entities.len(),
        scripts.path_nodes.len(),
        scripts.anims.len(),
        scripts.animstatedefs.len(),
        scripts.strings.len(),
        scripts.zbarriers.len()
    ));
    let ui_script_list = ui_scripts(&captures);
    let mut ui_names = ui_strings(&ui_script_list);
    // The Zombies front end's backdrop map: the menus' pictures too.
    if frame::bo2_frontend().is_some() && !map.starts_with("mp_") {
        for c in &captures {
            ui_names.extend(
                c.materials
                    .iter()
                    .filter(|m| front_end_picture(&c.zone, &m.name))
                    .map(|m| m.name.clone()),
            );
        }
    }
    // bo2mp lane C: a multiplayer map's menus (the class menu) also draw
    // pictures named by table cells (weapons, perks).
    if map.starts_with("mp_") {
        ui_names.extend(super::t6_frontend::table_picture_names(&captures));
        // The minimap the engine draws: the map's picture, the pings
        // (him, his team), its overlay; the scripts' hit marker, the
        // outcome screen's team icons; his damage feedback (blood, the
        // low-health edge, the hit's direction).
        for c in &captures {
            for m in &c.materials {
                let n = m.name.as_str();
                if n.starts_with("compass_map_")
                    || n.starts_with("compassping_")
                    || n == "mp_minimap_overlay"
                    || n == "compass_radarline"
                    || n.starts_with("damage_feedback")
                    || n.starts_with("faction_")
                    || n.starts_with("overlay_low_health")
                    || n == "hit_direction"
                    || (n.starts_with("map_") && n.contains("selector"))
                    // The scorestreak earned popup's big icon (the item's
                    // picture and "_big", notificationpopups.lua) and the
                    // column's small ones (rewardselection.lua).
                    || n.starts_with("hud_ks_")
                    // The care package's floating picture (the ammo crate's
                    // has no hud_ks_ name) and its minimap marks: green his
                    // team's, red the enemy's, black one he has hacked
                    // (_supplydrop.gsc).
                    || n.starts_with("hud_ammo_refill")
                    || n.starts_with("compass_supply_drop_")
                    // Express's bullet train cars' minimap marks
                    // (mp_express.csc's train_move: compass_train_engine,
                    // compass_train_carriage).
                    || n.starts_with("compass_train_")
                    // The Hellstorm's and its bomblets' minimap marks
                    // (_remotemissile.csc), in common_mp.
                    || n == "remotemissile_target"
                    // The equipment pips beside his ammo (offhandicons.lua).
                    || matches!(
                        n,
                        "hud_us_grenade"
                            | "hud_grenadeicon"
                            | "hud_icon_sticky_grenade"
                            | "hud_hatchet"
                            | "hud_icon_claymore"
                            | "hud_icon_satchelcharge"
                            | "hud_bounce_betty"
                            | "hud_us_flashgrenade"
                            | "hud_us_stungrenade"
                            | "hud_willy_pete"
                            | "hud_empgrenade"
                            | "hud_trophy_system"
                            | "hud_tact_insert"
                            // The grenade warning around his crosshair
                            // (cgMedia: the pointer, the throw-back hand;
                            // a weapon's own indicatorIcon).
                            | "hud_grenadepointer"
                            | "hud_grenadethrowback"
                            // The "stuck" pulse of an enemy semtex or bolt
                            // stuck to him (the stuck menu, ui_mp).
                            | "hud_icon_stuck_semtex"
                            | "hud_icon_stuck_arrow"
                            | "hud_indicator_arrow"
                            // The mouse buttons' pictures his key hints
                            // draw (`^BBUTTON_MOUSE_LEFT^`, the RC-XD's
                            // rcbombhints).
                            | "button_left_mouse"
                            | "button_right_mouse"
                            | "button_middle_mouse"
                            // The dirt and dust its wheels throw on his
                            // screen (fullscreen_dirt / fullscreen_dust).
                            | "fullscreen_dirt_bottom"
                            | "fullscreen_dirt_bottom_b"
                            | "fullscreen_dirt_right"
                            | "fullscreen_dirt_left"
                    )
                {
                    ui_names.insert(n.to_owned());
                }
            }
        }
    }
    let mut hud_icons = hud_icons(&captures, packs, &ui_names, &mut load_log);
    hud_icons.0.extend(sight_reticles);
    let screen_filters = screen_filters(&captures, packs, &mut load_log);
    load_log.push(format!("t6 ui: {} scripts", ui_script_list.len()));
    let ui = crate::T6Ui {
        scripts: ui_script_list,
        strings: std::sync::Arc::new(scripts.strings.clone()),
        tables: std::sync::Arc::new(scripts.tables.clone()),
        configs: std::sync::Arc::new(ui_configs(&captures)),
        game_settings: std::sync::Arc::new(zm_game_settings(&captures)),
    };
    // The font sheet's pixels are in the language's own image packs.
    let lang_packs = Some(zone_dir.as_path())
        .and_then(Path::parent)
        .map(|d| d.join("english"))
        .and_then(|d| asset_t6::PackSet::open_dir(&d).ok());
    let hud_fonts = hud_fonts(&lang_caps, lang_packs.as_ref().or(packs), &mut load_log);
    T6Combat {
        weapons,
        fpv,
        level_placed_weapons,
        animation_refs,
        fx,
        effect_meshes,
        tracers,
        impact_effect,
        sound,
        room_tone,
        scripts,
        hud_icons,
        hud_fonts,
        ui,
        pen_table,
        lochit_table,
        visions: script_visions(&captures),
        screen_filters,
        gun_facts,
        load_log,
    }
}

fn script_visions(captures: &[&ZoneCapture]) -> Vec<(String, Vec<u8>)> {
    let mut out: Vec<(String, Vec<u8>)> = Vec::new();
    for (name, bytes) in captures.iter().flat_map(|c| c.raw_files.iter()) {
        let name = name.to_ascii_lowercase();
        if !(name.starts_with("vision/") && name.ends_with(".vision")) {
            continue;
        }
        out.retain(|(n, _)| *n != name);
        out.push((name, bytes.clone()));
    }
    out
}

/// bo2zm: a map's brush models that carry surfaces of their own (Zombies
/// Declassified's Nacht: the first room's door `*11` is a
/// `script_brushmodel` drawn from the world's surfaces, where BO2's own maps
/// use a script model), each as a one-bone model named `*N` so the entity
/// draws where the scripts move it. The world vertex's first 32 bytes are
/// the model vertex (`GfxPackedVertex`); the surfaces are authored about
/// the entity's origin, as the submodel's brushes are.
fn brush_model_refs(world: &asset_t6::WorldRef) -> Vec<asset_t6::XModelRef> {
    const T6_WORLD_VERTEX: usize = 36;
    let mut out = Vec::new();
    for (n, &(first, count, bounds)) in world.solid_models.iter().enumerate().skip(1) {
        let mut lod0 = Vec::new();
        for surf in world
            .surfaces
            .iter()
            .skip(first as usize)
            .take(count as usize)
        {
            let start = surf.base_index.max(0) as usize;
            let Some(triangle_list) = world
                .indices
                .get(start..start + usize::from(surf.tri_tally) * 3)
            else {
                continue;
            };
            let Some(&top) = triangle_list.iter().max() else {
                continue;
            };
            let vertex_points = usize::from(top) + 1;
            let at = surf.vertex_data_offset0.max(0) as usize;
            let Some(block) = world.vd0.get(at..at + vertex_points * T6_WORLD_VERTEX) else {
                continue;
            };
            lod0.push(asset_t6::XSurfaceRef {
                // The world's vertex colour (per surface: magenta, green
                // on Nacht's door) tints a model: draw it white.
                vertex_points: block
                    .chunks_exact(T6_WORLD_VERTEX)
                    .flat_map(|v| {
                        let mut m = [0u8; 32];
                        m.copy_from_slice(&v[..32]);
                        m[16..20].fill(0xff);
                        m
                    })
                    .collect(),
                vertex_tally: vertex_points as u16,
                indices: triangle_list.to_vec(),
                vert_lists: vec![(0, vertex_points as u16, 0, surf.tri_tally)],
                material: surf.material,
                ..Default::default()
            });
        }
        if lod0.is_empty() {
            continue;
        }
        out.push(asset_t6::XModelRef {
            name: format!("*{n}"),
            numsurfs: lod0.len() as u8,
            materials: lod0.iter().map(|s| s.material).collect(),
            skeleton_joint_total: 1,
            detail_tier_total: 1,
            lo_corner: bounds[0],
            hi_corner: bounds[1],
            radius: 0.0,
            lighting_origin_offset: [0.0; 3],
            lighting_origin_range: 0.0,
            lod0,
            joint_labels: vec!["tag_origin".to_owned()],
            top_joint_total: 1,
            ancestor_chain: Vec::new(),
            rotation_quaternions: Vec::new(),
            shift_offset: Vec::new(),
            foundation_surface: vec![([0.0, 0.0, 0.0, 1.0], [0.0; 3])],
            body_part_category: vec![0],
            bone_info: vec![None],
            detail_tiers: vec![(0.0, 0, 0)],
            contents: 0,
            collision_detail_tier: -1,
            collision_faces: Vec::new(),
            collmap_brushes: Vec::new(),
            physics_template: None,
            collmap: Vec::new(),
            collmap_contents: 0,
            collmap_brush_verts: Vec::new(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn donor_zones_come_donor_by_donor_without_the_map_itself() {
        let jobs = donor_jobs("zm_transit", |part, zone| {
            (zone != "zm_buried_patch").then(|| PathBuf::from(format!("{part:?}/{zone}")))
        });
        let maps: Vec<&str> = jobs.iter().map(|j| j.donor.map).collect();
        assert!(!maps.contains(&"zm_transit"), "a map is not its own donor");
        let mut runs = maps.clone();
        runs.dedup();
        let mut unique = runs.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(runs.len(), unique.len(), "each donor's zones together: {maps:?}");
        let buried: Vec<(DonorPart, &str)> =
            jobs.iter().filter(|j| j.donor.map == "zm_buried").map(|j| (j.part, j.zone.as_str())).collect();
        assert_eq!(buried, [(DonorPart::Weapons, "zm_buried"), (DonorPart::Strings, "en_zm_buried")]);
        let all = donor_jobs("zm_nuked", |_, zone| Some(PathBuf::from(zone)));
        let transit: Vec<DonorPart> = all.iter().filter(|j| j.donor.map == "zm_transit").map(|j| j.part).collect();
        assert_eq!(transit, [DonorPart::Weapons, DonorPart::Weapons, DonorPart::States, DonorPart::Strings]);
    }

    #[test]
    fn zones_read_side_by_side_keep_their_places() {
        let paths: Vec<PathBuf> = (0..9).map(|i| PathBuf::from(format!("no_such_zone_{i}.ff"))).collect();
        let got = capture_many_with(&paths, |i, r| (i, r.is_err()));
        assert_eq!(got, (0..9).map(|i| (i, true)).collect::<Vec<_>>());
    }

    #[test]
    fn retrievable_weapons_are_the_matchs_own() {
        // A multiplayer match with the menu's Zombies guns loaded: only the
        // match's own retrievable weapons, so no `_zm` weapon shares a
        // watcher name with an `_mp` one.
        let own: BTreeSet<&str> = ["claymore_mp", "knife_ballistic_mp", "hatchet_mp", "m16_mp"].into();
        let all = [
            ("claymore_zm", true),
            ("knife_ballistic_zm", true),
            ("equip_turbine_zm", true),
            ("hatchet_mp", true),
            ("claymore_mp", true),
            ("knife_ballistic_mp", true),
            ("m16_mp", false),
        ];
        assert_eq!(
            retrievable_weapons(all, &own),
            ["claymore_mp", "hatchet_mp", "knife_ballistic_mp"]
        );
        // A Zombies map has no menu zones: all its own retrievable weapons.
        let zm: BTreeSet<&str> = ["claymore_zm", "equip_turbine_zm"].into();
        assert_eq!(
            retrievable_weapons([("equip_turbine_zm", true), ("claymore_zm", true)], &zm),
            ["claymore_zm", "equip_turbine_zm"]
        );
    }

    #[test]
    fn the_zombies_front_end_loads_its_menu_pictures() {
        // In a match: no ui_zm. Behind the front end: ui_zm, before its patch.
        assert_eq!(zm_pre_zones(false), ["code_post_gfx_zm", "common_zm", "patch_zm", "patch_ui_zm"]);
        assert_eq!(
            zm_pre_zones(true),
            ["code_post_gfx_zm", "common_zm", "patch_zm", "ui_zm", "patch_ui_zm"]
        );
        // The backdrop's art and the title are kept; a map's own and
        // code_post_gfx_zm's non-loading pictures are not.
        assert!(front_end_picture("ui_zm", "lui_bkg_zm"));
        assert!(front_end_picture("ui_zm", "lui_bkg_zm_rocks_front_forward"));
        assert!(front_end_picture("ui_zm", "menu_zm_title_screen"));
        assert!(front_end_picture("patch_ui_zm", "ui_globe_zm"));
        assert!(front_end_picture("dlczm0_load_zm", "menu_zm_nuked_map"));
        assert!(front_end_picture("code_post_gfx_zm", "loadscreen_zm_nuked_zstandard_nuked"));
        assert!(!front_end_picture("code_post_gfx_zm", "hud_chalk_1"));
        assert!(!front_end_picture("zm_nuked", "lui_bkg_zm"));
    }

    #[test]
    fn the_menu_backdrop_loads_no_donor_weapons() {
        assert!(donors_wanted(false, false));
        assert!(!donors_wanted(false, true));
        assert!(!donors_wanted(true, false));
    }

    #[test]
    fn the_zombies_map_picker_loads_every_map_packs_art() {
        // Behind the front end: every pack's load zone, whatever the map.
        let all = ["dlczm0_load_zm", "dlc1_load_zm", "dlc2_load_zm", "dlc3_load_zm", "dlc4_load_zm"];
        assert_eq!(zm_load_zones("zm_nuked", true), all);
        assert_eq!(zm_load_zones("zm_transit", true), all);
        // In a match: Nuketown's own (its intro's sound bank), no other.
        assert_eq!(zm_load_zones("zm_nuked", false), ["dlczm0_load_zm"]);
        assert!(zm_load_zones("zm_transit", false).is_empty());
    }

    #[test]
    fn patch_zones_outrank_the_map() {
        // A multiplayer match's zones in load order: patch_mp's copy of a
        // map script beats the map zone's, ui_mp's loses to patch_mp's.
        let mut order: Vec<&str> = T6_MP_PRE_ZONES.to_vec();
        order.extend(["faction_seals_mp", "mp_nuketown_2020"]);
        order.sort_by_key(|z| zone_priority(z));
        let want = "code_post_gfx_mp common_mp common_patch_mp faction_seals_mp \
                    mp_nuketown_2020 ui_mp patch_ui_mp patch_mp";
        assert_eq!(order.join(" "), want);
        // Zombies: the map's own patch beats the map, patch_zm both.
        assert!(zone_priority("zm_nuked_patch") > zone_priority("zm_nuked"));
        assert!(zone_priority("patch_zm") > zone_priority("zm_nuked_patch"));
    }
}
