//! bo2mp lane C: Black Ops II's multiplayer front end with no map
//! (`bo2mp frontend t6mp`), run by `lui_hud` from BO2's own scripts: the
//! multiplayer answers (`hks_t6::mp`) and his stats file go in before CoD's
//! base loads, then `T6.Main`; the engine opens `main` (the background)
//! and sends it `open_menu MainMenu`. START MATCH in the private match
//! lobby (ONLINE -> CUSTOM GAMES) hands the lobby's map, game type and bot
//! settings to the match (`session::Bo2mpMatchSetup`) and asks for the
//! swap to `t6:<map>`. His class choices are saved as they change.

use bevy::prelude::*;
use hks_t6::host::Host;
use hks_t6::mp::Mp;
use hks_t6::value::Value;

/// The game booted into BO2's multiplayer front end; his stats file (the
/// shape the menus read, `bo2_profile`) beside settings.cfg.
#[derive(Resource, Clone, Debug)]
pub struct Bo2mpFrontend {
    pub stats_path: std::path::PathBuf,
}

/// Where he was when he started a match, to come back to after it as BO2
/// does: the lobby menu (Custom Games: `PrivateOnlineGameLobby`) and the
/// front end's state then (session and game modes, dvars, the game type's
/// settings from SETUP GAME).
#[derive(Resource, Clone, Debug, Default)]
pub struct LobbyReturn {
    pub menu: String,
    pub session_modes: Vec<f32>,
    pub game_modes: Vec<i32>,
    pub dvars: Vec<(String, String)>,
    pub settings: Vec<(String, f32)>,
    /// Combat Training: the playlist, and his stats before the match (the
    /// After Action Report's "before", BO2's STATS_LOCATION_STABLE).
    pub playlist: Option<i32>,
    pub stats_before: Option<String>,
    /// Combat Training's map rotation, the match's map played.
    pub rotation: Option<hks_t6::mapvote::Rotation>,
    /// End Game ran BO2's `xpartydisbandafterround`: in a public online game
    /// (Combat Training) the engine marks the public party to disband at the
    /// round's end (cl.c CL_Live_DisbandPartyAfterRound,
    /// Com_SessionMode_IsPublicOnlineGame), so there is no public lobby to
    /// come back to.
    pub disbanded: bool,
}

/// The front end's own scripts, pictures and fonts, kept to come back to
/// after a match (the match's replace them).
#[derive(Resource, Clone)]
pub struct Bo2mpFrontendAssets {
    pub ui: assets::T6Ui,
    pub icons: assets::T6HudIcons,
    pub fonts: assets::T6HudFonts,
    pub front: Bo2mpFrontend,
}

/// After a match (its exitLevel swaps back to the menu screen): BO2's front
/// end again, at its main menu.
pub(crate) fn restore_after_match(
    mut commands: Commands,
    on_screen: Res<frame::ShellScreen>,
    saved: Option<Res<Bo2mpFrontendAssets>>,
    front: Option<Res<Bo2mpFrontend>>,
    mut played: Local<bool>,
) {
    let Some(saved) = saved else { return };
    match *on_screen {
        frame::ShellScreen::MainMenu if *played && front.is_none() => {
            *played = false;
            diag::info!(Ui, "front end: the match is over, menus back");
            commands.insert_resource(saved.ui.clone());
            commands.insert_resource(saved.icons.clone());
            commands.insert_resource(saved.fonts.clone());
            commands.insert_resource(saved.front.clone());
            commands.remove_resource::<session::Bo2mpMatchSetup>();
        }
        frame::ShellScreen::MainMenu => {}
        _ => *played = true,
    }
}

/// Before CoD's base loads: BO2's multiplayer answers, his saved stats.
pub(crate) fn install(host: &mut Host, front: &Bo2mpFrontend) -> Mp {
    let mp = hks_t6::mp::install(host);
    match bo2_profile::read_stats_text(&front.stats_path) {
        Ok(text) => {
            mp.borrow_mut().load_text(&text);
            diag::info!(Ui, "front end: stats read from {}", front.stats_path.display());
        }
        Err(_) => diag::info!(Ui, "front end: stats file {} not there yet", front.stats_path.display()),
    }
    mp
}

/// After CoD's base: the front end's values (no map running, his PC, an
/// offline session until ONLINE), the engine's roots (the scripts add to
/// them while they load), then `T6.Main`.
pub(crate) fn load(host: &mut Host) {
    hks_t6::mp::after_base(host);
    let offline = host.field("CoD", "SESSIONMODE_OFFLINE").as_num();
    {
        let mut v = host.values.borrow_mut();
        for (k, x) in [
            ("r_fontResolution", "720"),
            ("ui_gametype", "tdm"),
            ("ui_gameType", "tdm"),
            ("g_gametype", "tdm"),
            ("ui_mapname", "mp_la"),
            ("sv_running", "0"),
        ] {
            v.dvars.insert(k.to_owned(), x.to_owned());
        }
        v.session_modes = offline.into_iter().collect();
    }
    // The starting game type's settings (Team Deathmatch).
    hks_t6::mp::set_gametype_defaults(&host.values, "tdm");
    host.root(16.0 / 9.0);
    host.require("T6.Main");
}

/// Open the front end as the engine does: `main`, then `MainMenu`.
pub(crate) fn open(host: &mut Host, mp: Option<&Mp>, back: Option<&LobbyReturn>) {
    host.open_menu("main");
    // Back from a match: the lobby he started it from, the front end as it
    // was (BO2's engine reopens the party's lobby the same way).
    if let (Some(back), Some(mp)) = (back, mp) {
        {
            let mut v = host.values.borrow_mut();
            v.session_modes.clone_from(&back.session_modes);
            v.dvars.extend(back.dvars.iter().cloned());
            v.settings.extend(back.settings.iter().cloned());
        }
        {
            let mut m = mp.borrow_mut();
            m.game_modes.extend(back.game_modes.iter().copied());
            m.playlist = back.playlist;
            m.rotation.clone_from(&back.rotation);
            // Combat Training's lobby counts down to the next match.
            if back.playlist.is_some() && !back.disbanded {
                m.public_start_in = Some(hks_t6::playlists::NEXT_MATCH_MS);
            }
            if let Some(text) = &back.stats_before {
                m.set_stable_text(text);
            }
        }
        // The public party disbanded (End Game): BO2's own way out of that
        // lobby, `CoD.PublicGameLobby.ShutdownLobby` (it picks LeaveLobby1-3:
        // stop the party, back to a private one), then the menu its goBack
        // reaches, the one `PublicGameLobby.New` sets as previous for
        // multiplayer (publicgamelobby.lua).
        if back.disbanded && back.menu == "PublicGameLobby" {
            // The lobby's script loads when its menu first opens; it has not yet.
            host.require("T6.Menus.PublicGameLobby");
            let shutdown = host.field("CoD", "PublicGameLobby");
            let f = host.vm.lookup_index(&shutdown, &Value::str("ShutdownLobby")).unwrap_or(Value::Nil);
            let found = matches!(f, Value::Func(_));
            let event = hks_t6::value::LuaTable::new_ref();
            event.borrow_mut().store(Value::str("controller"), Value::Num(0.0));
            let ran = found && host.call(f, vec![Value::Nil, Value::Table(event)]).is_some();
            diag::info!(Ui, "front end: public lobby broke up (End Game; ShutdownLobby found {found}, ran {ran}), going to `PlayerMatchPartyLobby`");
            host.root_event("open_menu", &[("menuName", Value::str("PlayerMatchPartyLobby"))]);
            return;
        }
        diag::info!(Ui, "front end: going to menu `{}`", back.menu);
        host.root_event("open_menu", &[("menuName", Value::str(&back.menu))]);
        return;
    }
    host.root_event("open_menu", &[("menuName", Value::str("MainMenu"))]);
}

/// A Custom Game's START MATCH countdown (ms).
const PRIVATE_COUNTDOWN_MS: f64 = 5000.0;

/// The match is loading: BO2's own loading screen (`ui_mp/t6/hud/loading.lua`,
/// LUI.createMenu.Loading) over the front end, as the engine opens it: the
/// front end's menus go, the map and game type go into the dvars it reads
/// (`ui_mapname`, `ui_gametype`; `ls_mapname`, `ls_maplocation`,
/// `ls_gametype`: their names as mp/mapsTable.csv and
/// mp/gametypesTable.csv give them), then `Loading` opens and is told
/// `start_loading` (it fades in the map's picture `loadscreen_<map>`, its
/// name, place and game type, and a Did You Know tip).
pub(crate) fn open_loading(host: &mut Host, setup: &session::Bo2mpMatchSetup) {
    {
        let mut v = host.values.borrow_mut();
        let text = |k: &str| v.localize.get(&k.to_ascii_uppercase()).cloned().unwrap_or_default();
        let maps = hks_t6::mp::table_rows(&v, "mp/mapstable.csv");
        let map_key = maps.iter().find(|r| r.first().is_some_and(|c| *c == setup.map)).and_then(|r| r.get(3)).cloned().unwrap_or_default();
        let types = hks_t6::mp::table_rows(&v, "mp/gametypestable.csv");
        let type_key = types
            .iter()
            .find(|r| r.first().is_some_and(|c| c == "0") && r.get(1).is_some_and(|c| *c == setup.gametype))
            .and_then(|r| r.get(2))
            .cloned()
            .unwrap_or_default();
        let name = [format!("{map_key}_CAPS"), map_key.clone()].iter().map(|k| text(k)).find(|t| !t.is_empty()).unwrap_or_default();
        let place = [format!("{map_key}_LOC"), format!("{map_key}_LOCATION"), format!("{map_key}_LOC_CAPS")]
            .iter()
            .map(|k| text(k))
            .find(|t| !t.is_empty())
            .unwrap_or_default();
        let gametype = text(&type_key);
        diag::info!(Ui, "loading screen: `{}` {name:?} at {place:?}, {gametype:?} (keys {map_key} {type_key})", setup.map);
        if let Ok(probe) = std::env::var("BO2MP_LOC_GREP") {
            let probe = probe.to_ascii_uppercase();
            let mut hits: Vec<_> = v.localize.iter().filter(|(k, _)| k.contains(&probe)).collect();
            hits.sort();
            for (k, t) in hits.into_iter().take(60) {
                diag::info!(Ui, "loading screen: text {k} is {t:?}");
            }
        }
        for (k, x) in [
            ("ui_mapname", setup.map.clone()),
            ("mapname", setup.map.clone()),
            ("ui_gametype", setup.gametype.clone()),
            ("g_gametype", setup.gametype.clone()),
            ("ls_mapname", name),
            ("ls_maplocation", place),
            ("ls_gametype", gametype),
        ] {
            v.dvars.insert(k.to_owned(), x);
        }
    }
    // (T6.Main does not load it: the in-game HUD's scripts do.)
    let t = std::time::Instant::now();
    host.require("T6.HUD.Loading");
    let made = host.field("LUI", "createMenu");
    let has = matches!(host.vm.lookup_index(&made, &Value::str("Loading")), Ok(Value::Nil)).then_some("no").unwrap_or("yes");
    diag::info!(Ui, "loading screen: T6.HUD.Loading took {:.0} ms (LUI.createMenu.Loading: {has})", t.elapsed().as_secs_f64() * 1000.0);
    host.open_menu("Loading");
    diag::info!(Ui, "loading screen: Loading menu up after {:.0} ms", t.elapsed().as_secs_f64() * 1000.0);
    // (The spinner is not started here: the script's own timer raises
    // `start_spinner` after `SpinnerDelayTime`, as in BO2 - a loading screen
    // under 19 s shows no spinner.)
    host.root_event("start_loading", &[]);
    // The engine writes its connect status under the tip while a map loads
    // (BO2's own: "Awaiting challenge...5").
    let status = host.values.borrow().localize.get("EXE_AWAITINGCHALLENGE").cloned().unwrap_or_default().replace("&&1", "1");
    if !status.is_empty() {
        host.set_menu_label_text("Menu.Loading", "statusLabel", &status);
    }
    diag::info!(Ui, "loading screen: spinner going after {:.0} ms", t.elapsed().as_secs_f64() * 1000.0);
}

/// Whether an element is a 3D widget the 2D layer cannot draw (the globe,
/// the holotable grids turned into the screen).
pub(crate) fn has_depth_axis(d: &hks_t6::host::Drawn) -> bool {
    d.element_kind == "globe" || d.x_rot.abs() > 0.01 || d.y_rot.abs() > 0.01
}

/// Combat Training's rotation: Custom Games' Change Map list (BO2's
/// mp/mapsTable.csv, `Engine.GetMaps`) less the maps whose zone is not in
/// his install, the clock rolling its draws.
fn new_rotation(host: &Host, identity: Option<&frame::StartupPersona>) -> hks_t6::mapvote::Rotation {
    let installed: std::collections::HashSet<String> = identity
        .map(|id| asset_transport::multiplayer_map_names(&asset_transport::GameInstallDir(id.titles_install_dir.clone())))
        .unwrap_or_default()
        .into_iter()
        .collect();
    let maps = {
        let v = host.values.borrow();
        hks_t6::mapvote::offered(hks_t6::mp::table_rows(&v, "mp/mapstable.csv"), |m| installed.contains(&format!("t6:{m}")))
    };
    let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
    diag::info!(Ui, "bo2mp front end: Combat Training rotation {maps:?} ({} zones installed)", installed.len());
    hks_t6::mapvote::Rotation::new(maps, seed)
}

/// The public lobby's map vote closes as BO2's start timer starts (ms
/// before the match).
const VOTE_CLOSES_BEFORE_START_MS: f64 = hks_t6::playlists::START_TIMER_MS;

/// The public lobby's countdown started: the next match's map, from the
/// rotation, and the playlist's game type go to `ui_mapname` /
/// `ui_gametype`, and BO2's map vote opens on it.
fn public_lobby_opened(host: &mut Host, mp: &Mp, identity: Option<&frame::StartupPersona>) {
    let Some(p) = mp.borrow().playlist.and_then(hks_t6::playlists::by_index) else { return };
    if mp.borrow().rotation.is_none() {
        let r = new_rotation(host, identity);
        mp.borrow_mut().rotation = Some(r);
    }
    let map = mp.borrow().rotation.as_ref().and_then(|r| r.next()).unwrap_or(hks_t6::playlists::MAP).to_owned();
    let previous_allowed = host.values.borrow().dvars.get("party_isPreviousMapVoted").is_none_or(|x| x.trim() != "1");
    diag::info!(Ui, "bo2mp front end: public lobby, next map `{map}` ({}), previous votable {previous_allowed}", p.gametype);
    {
        let mut v = host.values.borrow_mut();
        v.dvars.insert("ui_mapname".to_owned(), map);
        v.dvars.insert("ui_gametype".to_owned(), p.gametype.to_owned());
        v.dvars.insert("ui_gameType".to_owned(), p.gametype.to_owned());
        // The lobby's limit: the match's player slots (BO2 sets
        // party_maxplayers to the session's slots once a public lobby is
        // joined).
        v.dvars.insert("party_maxplayers".to_owned(), p.seat_capacity().to_string());
    }
    {
        let mut m = mp.borrow_mut();
        m.vote = m.rotation.as_ref().and_then(|r| hks_t6::mapvote::Vote::open(r, previous_allowed));
    }
    send_map_vote(host, mp);
}

/// BO2's map voter (the public lobby's map): an open vote's maps and votes
/// (`map_vote_set_votes_and_maps`), or the map it closed on, already in
/// `ui_mapname` (`map_vote_map_chosen`).
fn send_map_vote(host: &mut Host, mp: &Mp) {
    let Some(vote) = mp.borrow().vote.clone() else { return };
    if vote.chosen.is_some() {
        host.root_event("map_vote_map_chosen", &[]);
        return;
    }
    // The game type goes as the name the voter shows under the map, in
    // capitals ("TEAM DEATHMATCH"), as BO2's lobby shows it.
    let gametype = {
        let v = host.values.borrow();
        let g = v.dvars.get("ui_gametype").map_or_else(String::new, Clone::clone);
        hks_t6::playlists::gametype_name(&v, &g).to_uppercase()
    };
    let [next, previous, random] = vote.counts();
    let mut fields = vec![
        ("previousMapName", Value::str(&vote.previous)),
        ("previousGametype", Value::str(&gametype)),
        ("nextMapName", Value::str(&vote.next)),
        ("nextGametype", Value::str(&gametype)),
        ("previousMapVoteCount", Value::Num(previous as f32)),
        ("nextMapVoteCount", Value::Num(next as f32)),
        ("randomMapVoteCount", Value::Num(random as f32)),
    ];
    if let Some(c) = vote.mine {
        fields.push(("selfVote", Value::Num(c as i32 as f32)));
    }
    host.root_event("map_vote_set_votes_and_maps", &fields);
}

/// His vote (`xpartyveto n`: 1 MINT_COUNTER, 2 PREVIOUS, 3 RANDOM).
pub(crate) fn map_vote(host: &mut Host, mp: &Mp, n: i32) {
    let cast = {
        let mut m = mp.borrow_mut();
        let c = hks_t6::mapvote::Choice::from_number(n);
        c.zip(m.vote.as_mut()).is_some_and(|(c, v)| v.cast(c))
    };
    diag::info!(Ui, "front end: vote for map {n} ({})", if cast { "counted" } else { "not open" });
    if cast {
        send_map_vote(host, mp);
    }
}

/// The vote closes: its map goes to `ui_mapname` for START, PREVIOUS
/// winning greys PREVIOUS out in the next lobby.
fn close_map_vote(host: &mut Host, mp: &Mp) {
    let closed = {
        let mut m = mp.borrow_mut();
        let m = &mut *m;
        match (m.vote.as_mut().filter(|v| v.chosen.is_none()), m.rotation.as_mut()) {
            (Some(v), Some(r)) => Some(v.close(r)),
            _ => None,
        }
    };
    let Some((choice, map)) = closed else { return };
    diag::info!(Ui, "front end: voting over, {choice:?} `{map}`");
    {
        let mut v = host.values.borrow_mut();
        v.dvars.insert("ui_mapname".to_owned(), map);
        let previous = if choice == hks_t6::mapvote::Choice::Previous { "1" } else { "0" };
        v.dvars.insert("party_isPreviousMapVoted".to_owned(), previous.to_owned());
    }
    send_map_vote(host, mp);
}

/// Each frame: save his stats when they changed; START MATCH starts the
/// match the lobby set up.
/// True when it handed him to a match this frame (the caller stops the menu
/// music then: BO2's front-end song ends as the map loads, and the match's
/// own `PlayMusic("")` comes late or, in a public lobby, not at all).
pub(crate) fn tick(
    host: &mut Host,
    mp: &Mp,
    front: &Bo2mpFrontend,
    identity: Option<&frame::StartupPersona>,
    commands: &mut Commands,
    swap: Option<&mut session::SessionHandoverOrder>,
) -> bool {
    for line in std::mem::take(&mut mp.borrow_mut().stat_log) {
        diag::info!(Ui, "front end: stats line {line}");
    }
    if mp.borrow_mut().take_dirty() {
        let done: bo2_profile::SaveDone = Box::new(|file, result| match result {
            Ok(()) => diag::info!(Ui, "front end: stats written to {}", file.display()),
            Err(e) => diag::warn!(Ui, "front end: stats could not be written to {}: {e}", file.display()),
        });
        let all = bo2_profile::SaveJob::Whole(mp.borrow().profile.stats.clone());
        bo2_profile::save_in_background(&front.stats_path, all, Some(done));
    }
    // ZOMBIES when the Zombies menus could not start: our Nuketown Zombies,
    // swapped in as START MATCH swaps a map.
    if mp.borrow().zombies_requested {
        mp.borrow_mut().zombies_requested = false;
        let zone = "t6:zm_nuked".to_owned();
        diag::info!(Ui, "front end: zombies picked");
        commands.remove_resource::<Bo2mpFrontend>();
        if let Some(swap) = swap {
            match swap.ask_for_region(zone.clone()) {
                Ok(id) => diag::info!(Ui, "front end: zombies map `{zone}` asked for (swap #{id})"),
                Err(e) => diag::warn!(Ui, "front end: zombies map `{zone}` refused: {e}"),
            }
        } else {
            diag::warn!(Ui, "front end: zombies map `{zone}` has no session to load in");
        }
        return true;
    }
    // Combat Training: the public lobby's countdown (from the front end's
    // first frame back from a match), run out.
    // (LUI's clock reads 0 until the new front end's first frame.)
    let wait = if host.clock_millis() > 0.0 { mp.borrow_mut().public_start_in.take() } else { None };
    if let Some(wait) = wait {
        mp.borrow_mut().public_start_ms = Some(host.clock_millis() + wait);
        public_lobby_opened(host, mp, identity);
    }
    let closing = mp.borrow().public_start_ms.is_some_and(|at| host.clock_millis() >= at - VOTE_CLOSES_BEFORE_START_MS);
    if closing {
        close_map_vote(host, mp);
    }
    let training = {
        let m = mp.borrow();
        m.public_start_ms.filter(|at| host.clock_millis() >= *at).and(m.playlist).and_then(hks_t6::playlists::by_index)
    };
    if training.is_none() && !mp.borrow().start_requested {
        mp.borrow_mut().private_start_ms = None;
        return false;
    }
    // A Custom Game counts down "Game starting in N" before it starts.
    if training.is_none() {
        let now = host.clock_millis();
        let at = *mp.borrow_mut().private_start_ms.get_or_insert(now + PRIVATE_COUNTDOWN_MS);
        if now < at {
            return false;
        }
        mp.borrow_mut().private_start_ms = None;
    }
    mp.borrow_mut().start_requested = false;
    mp.borrow_mut().public_start_ms = None;
    let mut setup = {
        let v = host.values.borrow();
        let dvar = |k: &str, d: &str| v.dvars.get(k).filter(|x| !x.is_empty()).cloned().unwrap_or_else(|| d.to_owned());
        let setting = |k: &str| v.settings.get(k).copied().unwrap_or(0.0).max(0.0) as u32;
        // SETUP BOTS keeps the bots in dvars (BO2's gameoptions.lua:
        // addDvarLeftRightSelector "bot_friends" / "bot_enemies" /
        // "bot_difficulty"), their defaults from BO2's default_private.cfg.
        let count = |k: &str| {
            v.dvars.get(k).and_then(|x| x.trim().parse::<f32>().ok()).map_or_else(|| setting(k), |n| n.max(0.0) as u32)
        };
        session::Bo2mpMatchSetup {
            map: dvar("ui_mapname", "mp_la"),
            gametype: dvar("ui_gametype", "tdm"),
            bot_friends: count("bot_friends"),
            bot_enemies: count("bot_enemies"),
            bot_difficulty: count("bot_difficulty"),
        }
    };
    // A Combat Training playlist: the lobby's map (one the rotation
    // offers), its game type and teams (`hks_t6::playlists`, ours), BO2's
    // bot difficulty setting.
    if let Some(p) = training {
        let mut m = mp.borrow_mut();
        let r = m.rotation.get_or_insert_with(|| new_rotation(host, identity));
        if !r.maps().contains(&setup.map) {
            setup.map = r.next().unwrap_or(hks_t6::playlists::MAP).to_owned();
        }
        r.play(&setup.map);
        m.vote = None;
        setup.gametype = p.gametype.to_owned();
        setup.bot_friends = p.friends;
        setup.bot_enemies = p.enemies;
        setup.bot_difficulty = p.difficulty;
    }
    let zone = setup.zone();
    diag::info!(Ui, "front end: match starting {setup:?} ranked {}", training.is_some());
    // The match's server reads and writes his stats file (his classes, XP);
    // XP counts in Combat Training, not in Custom Games.
    sim::set_local_stats_path(front.stats_path.clone());
    sim::set_local_match_ranked(training.is_some());
    // Where to come back to (Custom Games' lobby, or Combat Training's
    // public lobby, as it is now).
    {
        let v = host.values.borrow();
        commands.insert_resource(LobbyReturn {
            menu: if training.is_some() { "PublicGameLobby" } else { "PrivateOnlineGameLobby" }.to_owned(),
            session_modes: v.session_modes.clone(),
            game_modes: mp.borrow().game_modes.iter().copied().collect(),
            dvars: v.dvars.iter().map(|(k, x)| (k.clone(), x.clone())).collect(),
            settings: v.settings.iter().map(|(k, x)| (k.clone(), *x)).collect(),
            playlist: training.map(|p| p.index),
            stats_before: training.map(|_| mp.borrow().to_text()),
            rotation: mp.borrow().rotation.clone(),
            disbanded: false,
        });
    }
    commands.insert_resource(setup);
    // The match's HUD is the game's own, not the front end.
    commands.remove_resource::<Bo2mpFrontend>();
    if let Some(swap) = swap {
        match swap.ask_for_region(zone.clone()) {
            Ok(id) => diag::info!(Ui, "front end: map `{zone}` asked for (swap #{id})"),
            Err(e) => diag::warn!(Ui, "front end: map `{zone}` refused: {e}"),
        }
    } else {
        diag::warn!(Ui, "front end: map `{zone}` has no session to load in");
    }
    true
}

/// bo2mp lane C: a multiplayer match's screen values from his client dvars
/// (sim `natives_mp::publish_screen`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct MatchValues {
    /// "<allies>,<axis>".
    pub scores: String,
    /// Whole seconds left ("" = no clock).
    pub timeleft: String,
    pub team: String,
    pub gametype: String,
    /// "<n>:<menu>" ("<n>:" = his menus closed).
    pub menu: String,
    /// "<n>|<event>|<arg>..." by `;`.
    pub lui: String,
    /// "<n>|<attacker>|<victim>|<weapon>|<mod>" by `;`.
    pub feed: String,
    /// Everyone's team ("<client>:<team>,..."), the scoreboard's columns,
    /// and every player's row from the snapshot
    /// ("<client>|<name>|<score>|<kills>|<deaths>" by `;`).
    pub teams: String,
    pub cols: String,
    pub rows: String,
    /// The game type's settings ("name=value,...").
    pub settings: String,
    /// The minimap ("<material>,<ul x>,<ul y>,<lr x>,<lr y>,<north yaw>,
    /// <range>").
    pub minimap: String,
    /// His radar ("<uav> <satellite> <jammed> <free-for-all>", `bo2mp_radar`).
    pub radar: String,
    /// A spot being picked on the map (lane D's `bo2mp_locsel`:
    /// "<selector material> <radius>", "" when not).
    pub locsel: String,
    /// bo2mp scope lane: his killcam ("<kind>|<killer>": kind 1 killcam,
    /// 2 final killcam, 4 the round's end killcam; "" none): BIT_IN_KILLCAM,
    /// BIT_FINAL_KILLCAM / BIT_ROUND_END_KILLCAM and the killer's name card.
    pub killcam: String,
    /// bo2mp scope lane: his script took the HUD away (`setclientuivisibilityflag
    /// hud_visible 0`, as at the match's end) while a killcam still plays:
    /// BIT_HUD_VISIBLE goes, the killcam's own widget stays.
    pub hud_hidden: bool,
    /// bo2mp scope lane: he looks through a scope (a gun's overlay is up):
    /// BO2's BIT_IS_SCOPED (its HUD hides the minimap and voice dock).
    /// Not a dvar: the HUD sets it from his player state.
    pub scoped: bool,
    /// bo2mp vehicle screens: the scorestreak vehicle he rides ("<vehicle
    /// type> <seat>", sim `ride.rs`'s `bo2mp_vehicle`; "" on foot):
    /// BIT_IN_VEHICLE and BO2's killstreak HUD (`hud_update_killstreak_hud`).
    pub vehicle: String,
    /// bo2mp vehicle screens: the server's vision for him (`bo2mp_vision`,
    /// `setvisionsetforplayer` while `useservervisionset` is on; "" the
    /// map's own): his frame's grade.
    pub vision: String,
    /// bo2mp vehicle screens: his vehicle screen's infrared is on
    /// (`bo2mp_infrared` "1", `setinfraredvision`): BO2's HUD lights FLIR,
    /// else OPT.
    pub infrared: String,
    /// bo2mp: his Sensor Grenades' marks (`bo2mp_sensor`, sim
    /// `sensor_grenade.rs`): "<tracked ids> <id:sweeps>", each list
    /// comma-separated or "-".
    pub sensor: String,
    /// The objectives (flags, bomb sites, zones; see
    /// `hks_t6::mp::parse_objectives`).
    pub objectives: String,
    /// The lives each player starts with (BO2's `scr_numLives`, set by the
    /// server's `setclientnumlives`; One in the Chamber's HUD counts them).
    pub num_lives: String,
}

/// His minimap this frame: the map's picture and the world corners it
/// covers (BO2's `setminimap`), north, him and his team (position, yaw).
#[derive(Clone, Debug)]
pub(crate) struct MinimapView {
    pub pic: String,
    pub ul: [f32; 2],
    pub lr: [f32; 2],
    pub compass_heading: f32,
    /// The world distance across the minimap, edge to edge (the map's
    /// `compassmaxrange`; measured against real BO2 on Aftermath: the HUD
    /// box spans 2048 units, 512 map pixels = 7840 units, 1.1 box px per
    /// map pixel).
    pub range: f32,
    pub me: ([f32; 2], f32),
    pub friends: Vec<([f32; 2], f32)>,
    /// Enemies the map shows: spotted by his team's radar, or firing
    /// (position, yaw, firing, how bright: a radar ping fades out).
    pub enemies: Vec<([f32; 2], f32, bool, f32)>,
    /// BO2's radar line while a UAV is up: its offset along the map's east
    /// (see `line_at`).
    pub line: Option<f32>,
    /// An enemy Counter-UAV jams his map: BO2 draws the map with
    /// `compass_map_<map>_scrambled` instead (see `Scramble`), at this
    /// time (seconds), and him, his team and the marks are hidden.
    pub jam: Option<f32>,
}

impl MinimapView {
    pub(crate) fn of(m: &MatchValues, me: ([f32; 2], f32), friends: Vec<([f32; 2], f32)>, enemies: Vec<([f32; 2], f32, bool, f32)>) -> Option<MinimapView> {
        let f: Vec<&str> = m.minimap.split(',').collect();
        let (pic, a, b, c, d, n, r) = match f[..] {
            [pic, a, b, c, d, n, r] => (pic, a, b, c, d, n, r),
            [pic, a, b, c, d, n] => (pic, a, b, c, d, n, "2048"),
            _ => return None,
        };
        let num = |x: &str| x.trim().parse::<f32>().unwrap_or(0.0);
        Some(MinimapView {
            pic: pic.to_owned(),
            ul: [num(a), num(b)],
            lr: [num(c), num(d)],
            compass_heading: num(n),
            range: num(r).max(100.0),
            me,
            friends,
            enemies,
            line: None,
            jam: None,
        })
    }

    /// BO2's radar line `progress` (0..1) of the way across the map: the
    /// east offset of a line running north-south. It starts and ends a
    /// margin off the map (half the line's width past the minimap's
    /// corner, `compassRadarLineThickness` 0.4 of `compassMaxRange`, or
    /// `cg_hudMapRadarLineThickness` 0.15 of the map, the larger).
    pub(crate) fn line_at(&self, progress: f32) -> f32 {
        let (_, east) = self.axes();
        let world = self.span_size()[0];
        let margin = (self.range * std::f32::consts::SQRT_2 + 0.4 * self.range).max(0.15 * world) * 0.5;
        (margin * 2.0 + world) * progress + east[0] * self.ul[0] + east[1] * self.ul[1] - margin
    }

    /// Which side of a radar line a world point is on.
    pub(crate) fn behind_line(&self, line: f32, p: [f32; 2]) -> bool {
        let (_, east) = self.axes();
        p[0] * east[0] + p[1] * east[1] - line < 0.0
    }

    /// The picture's change per world unit east and per world unit south.
    fn uv_steps(&self) -> (f32, f32) {
        let (north, east) = self.axes();
        let span = [self.lr[0] - self.ul[0], self.lr[1] - self.ul[1]];
        let se = span[0] * east[0] + span[1] * east[1];
        let sn = span[0] * north[0] + span[1] * north[1];
        let safe = |x: f32| if x.abs() < 1.0 { 1.0 } else { x };
        (1.0 / safe(se), -1.0 / safe(sn))
    }

    /// BO2's HUD minimap as an `n` x `n` RGBA8 picture: the map around him,
    /// `range` world units across, turned so he faces up (bilinear
    /// samples of the map's picture; off the picture is see-through), with
    /// the radar line over it (`compass_radarline`, `lw` x `lh`: across the
    /// line its picture spans 0.4 of `range`, along it it repeats). While
    /// jammed (and `scramble` holds BO2's two pictures), the map is drawn
    /// scrambled instead (see `Scramble`).
    pub(crate) fn render(
        &self,
        src: &[u8],
        sw: usize,
        sh: usize,
        n: usize,
        radarline: Option<(&[u8], usize, usize)>,
        scramble: Option<&Scramble<'_>>,
    ) -> Vec<u8> {
        let mut out = vec![0u8; n * n * 4];
        if sw < 2 || sh < 2 || src.len() < sw * sh * 4 {
            return out;
        }
        let scrambled = self.jam.zip(scramble.filter(|s| s.ok())).map(|(t, s)| (s, ScrambleFrame::at(t)));
        let [u0, v0] = self.uv(self.me.0);
        let (du, dv) = self.uv_steps();
        let h = self.heading(self.me.1);
        let (c, s) = (h.cos(), h.sin());
        let world_per_px = self.range / n as f32;
        let texel = |x: usize, y: usize, k: usize| f32::from(src[(y * sw + x) * 4 + k]);
        let (_, east) = self.axes();
        let me_east = self.me.0[0] * east[0] + self.me.0[1] * east[1];
        let line = self.line.zip(radarline.filter(|(d, lw, lh)| *lw > 0 && *lh > 0 && d.len() >= lw * lh * 4));
        for j in 0..n {
            for i in 0..n {
                // Right and down from the middle (world units), turned back
                // to the map's east and south.
                let sx = (i as f32 + 0.5 - n as f32 * 0.5) * world_per_px;
                let sy = (j as f32 + 0.5 - n as f32 * 0.5) * world_per_px;
                let (ex, so) = (sx * c - sy * s, sx * s + sy * c);
                let o = (j * n + i) * 4;
                if let Some((at, (ld, lw, lh))) = line {
                    let ls = ((me_east + ex - at) / self.range) / 0.4 + 0.5;
                    if (0.0..1.0).contains(&ls) {
                        let lt = (-so / (0.4 * self.range)).rem_euclid(1.0);
                        let (x, y) = (((ls * lw as f32) as usize).min(lw - 1), ((lt * lh as f32) as usize).min(lh - 1));
                        out[o..o + 4].copy_from_slice(&ld[(y * lw + x) * 4..][..4]);
                    }
                }
                let (u, v) = (u0 + ex * du, v0 + so * dv);
                if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
                    continue;
                }
                let map = match &scrambled {
                    Some((s, f)) => s.colour(f, (src, sw, sh), u, v),
                    None => {
                        let (fx, fy) = (u * sw as f32 - 0.5, v * sh as f32 - 0.5);
                        let (x0, y0) = (fx.floor().clamp(0.0, (sw - 1) as f32) as usize, fy.floor().clamp(0.0, (sh - 1) as f32) as usize);
                        let (x1, y1) = ((x0 + 1).min(sw - 1), (y0 + 1).min(sh - 1));
                        let (tx, ty) = ((fx - x0 as f32).clamp(0.0, 1.0), (fy - y0 as f32).clamp(0.0, 1.0));
                        let mut map = [0.0f32; 4];
                        for (k, m) in map.iter_mut().enumerate() {
                            let top = texel(x0, y0, k) * (1.0 - tx) + texel(x1, y0, k) * tx;
                            let bottom = texel(x0, y1, k) * (1.0 - tx) + texel(x1, y1, k) * tx;
                            *m = top * (1.0 - ty) + bottom * ty;
                        }
                        map
                    }
                };
                // The line (laid down above) over the map.
                let (la, ma) = (f32::from(out[o + 3]) / 255.0, map[3] / 255.0);
                let a = la + ma * (1.0 - la);
                for k in 0..3 {
                    let mixed = f32::from(out[o + k]) * la + map[k] * ma * (1.0 - la);
                    out[o + k] = if a > 0.0 { (mixed / a).round().min(255.0) as u8 } else { 0 };
                }
                out[o + 3] = (a * 255.0).round() as u8;
            }
        }
        out
    }

    /// The whole map's square (FULL) while jammed: the map's picture
    /// (`sw` x `sh`) scrambled, `n` x `n`.
    pub(crate) fn render_scrambled(&self, src: &[u8], sw: usize, sh: usize, n: usize, scramble: &Scramble<'_>) -> Vec<u8> {
        let mut out = vec![0u8; n * n * 4];
        let Some(t) = self.jam.filter(|_| scramble.ok() && sw > 1 && sh > 1 && src.len() >= sw * sh * 4) else {
            return out;
        };
        let f = ScrambleFrame::at(t);
        for j in 0..n {
            for i in 0..n {
                let (u, v) = ((i as f32 + 0.5) / n as f32, (j as f32 + 0.5) / n as f32);
                let c = scramble.colour(&f, (src, sw, sh), u, v);
                let o = (j * n + i) * 4;
                for k in 0..4 {
                    out[o + k] = c[k].round().clamp(0.0, 255.0) as u8;
                }
            }
        }
        out
    }

    /// The map's north and east in the world.
    fn axes(&self) -> ([f32; 2], [f32; 2]) {
        let r = self.compass_heading.to_radians();
        ([r.cos(), r.sin()], [r.sin(), -r.cos()])
    }

    /// A world point on the picture (0..1 across, 0..1 down).
    pub(crate) fn uv(&self, p: [f32; 2]) -> [f32; 2] {
        let (north, east) = self.axes();
        let dot = |a: [f32; 2], b: [f32; 2]| a[0] * b[0] + a[1] * b[1];
        let d = [p[0] - self.ul[0], p[1] - self.ul[1]];
        let span = [self.lr[0] - self.ul[0], self.lr[1] - self.ul[1]];
        let u = dot(d, east) / dot(span, east).abs().max(1.0) * dot(span, east).signum();
        let v = -dot(d, north) / dot(span, north).abs().max(1.0) * (-dot(span, north)).signum();
        [u, v]
    }

    /// How far clockwise from the map's north a yaw faces (radians).
    pub(crate) fn heading(&self, yaw: f32) -> f32 {
        let (north, east) = self.axes();
        let f = [yaw.to_radians().cos(), yaw.to_radians().sin()];
        (f[0] * east[0] + f[1] * east[1]).atan2(f[0] * north[0] + f[1] * north[1])
    }

    /// The picture's size in world units (across, down).
    pub(crate) fn span_size(&self) -> [f32; 2] {
        let (se, ss) = self.uv_steps();
        [(1.0 / se).abs(), (1.0 / ss).abs()]
    }

    /// A world point around him on the minimap: right and down from its
    /// centre in world units, the map turned so he faces up.
    pub(crate) fn screen_offset(&self, p: [f32; 2]) -> [f32; 2] {
        let (north, east) = self.axes();
        let d = [p[0] - self.me.0[0], p[1] - self.me.0[1]];
        let (x, y) = (d[0] * east[0] + d[1] * east[1], -(d[0] * north[0] + d[1] * north[1]));
        let phi = -self.heading(self.me.1);
        [x * phi.cos() - y * phi.sin(), x * phi.sin() + y * phi.cos()]
    }
}

/// BO2's jammed minimap: an enemy Counter-UAV makes the engine draw the
/// map with `compass_map_<map>_scrambled` (technique
/// `sw4_2d_compass_map_static`), ported here line for line from its
/// shaders. The map breaks into big blocks (their size set by
/// `sam_block_noise`, re-rolled 15 times a second), its red, green and blue
/// pulled apart sideways by a wobble read from `tv_lookup` (scrolling once
/// a second), and the whole picture jumps every half second. Its strength
/// constant (`scriptVector0`) is taken as (1, 1): fully jammed, which
/// matches the real footage.
pub(crate) struct Scramble<'a> {
    /// `sam_block_noise` (512 x 256), repeating.
    pub noise: (&'a [u8], usize, usize),
    /// `tv_lookup` (256 x 256), repeating.
    pub tv: (&'a [u8], usize, usize),
}

/// The scramble's `scriptVector0`: x the jump's size, y the strength.
const SCRAMBLE_SV0: [f32; 2] = [1.0, 1.0];

/// The shaders' hash: frac(dot(v, K)).
fn scramble_hash(v: [f32; 4]) -> f32 {
    const K: [f32; 4] = [81.239_487, 17.348_024, 37.349_838, 59.394_84];
    let d = v[0] * K[0] + v[1] * K[1] + v[2] * K[2] + v[3] * K[3];
    d - d.floor()
}

/// One frame's values of the scramble (the same for every pixel).
pub(crate) struct ScrambleFrame {
    /// The noise's offset (the vertex shader's o0.w, o0.z).
    off: [f32; 2],
    /// The jump (x 0.01 of the map).
    jump: [f32; 2],
    /// The column of `tv_lookup` read this frame.
    tu: f32,
}

impl ScrambleFrame {
    pub(crate) fn at(t: f32) -> ScrambleFrame {
        // Vertex shader: a new noise offset 15 times a second.
        let s = (t * 15.0).floor();
        let y = s * 0.066_667 + 0.727_263;
        let z = s * 0.066_667 + 0.689_235;
        let x = scramble_hash([y; 4]);
        let w = scramble_hash([x, y, y, y]);
        let mut r2 = [x, w, 0.0, y];
        r2[2] = scramble_hash([x, w, y, y]);
        r2[3] = scramble_hash(r2);
        r2[0] = scramble_hash(r2);
        r2[1] = scramble_hash(r2);
        r2[2] = scramble_hash(r2);
        let oz = scramble_hash(r2) * 200.0 - 100.0;
        let mut r1 = [0.0, 0.0, 0.0, z];
        r1[0] = scramble_hash([z; 4]);
        r1[1] = scramble_hash([r1[0], z, z, z]);
        r1[2] = scramble_hash([r1[0], r1[1], z, z]);
        r1[3] = scramble_hash(r1);
        r1[0] = scramble_hash(r1);
        r1[1] = scramble_hash(r1);
        r1[2] = scramble_hash(r1);
        let ow = scramble_hash(r1) * 200.0 - 100.0;
        // Pixel shader: the jump, new every half second.
        let w = (2.0 * t).floor() * 0.5;
        let x = scramble_hash([w; 4]);
        let y = scramble_hash([x, w, w, w]);
        let z = scramble_hash([x, y, w, w]);
        let w2 = scramble_hash([x, y, z, w]);
        let j0 = scramble_hash([x, y, z, w2]);
        let j1 = scramble_hash([j0, y, z, w2]);
        let jump = [((j0 * 2.0 - 1.0) * 2.0 - 1.0) * SCRAMBLE_SV0[0], ((j1 * 2.0 - 1.0) * 2.0 - 1.0) * SCRAMBLE_SV0[0]];
        let tt = t + 0.5;
        ScrambleFrame { off: [ow, oz], jump, tu: tt - tt.floor() }
    }
}

impl Scramble<'_> {
    pub(crate) fn ok(&self) -> bool {
        let fits = |(d, w, h): (&[u8], usize, usize)| w > 1 && h > 1 && d.len() >= w * h * 4;
        fits(self.noise) && fits(self.tv)
    }

    /// The scrambled map's colour (0..255) at map point (`pu`, `pv`).
    pub(crate) fn colour(&self, f: &ScrambleFrame, map: (&[u8], usize, usize), pu: f32, pv: f32) -> [f32; 4] {
        let sv = SCRAMBLE_SV0;
        let (nd, nw, nh) = self.noise;
        let n = (sample_bilinear(nd, nw, nh, (pu + f.off[0]) * 0.07, (pv + f.off[1]) * 0.07, true, 0) / 255.0).clamp(0.0, 1.0);
        let a = (sv[1] * 2.0).clamp(0.0, 1.0);
        let m = n * 344.0 - 704.0;
        let res = (a * m + 722.0).floor() * 2.0;
        let wgt = ((a * m + 36.0) * 0.027_778).max(0.0);
        let k = (sv[1] * 0.7474).clamp(0.0, 1.0) * 0.15;
        let fac = [sv[1] * 1.5 - 0.5, sv[1] * 0.25, sv[1] * -0.75 + 0.5];
        let (td, tw, th) = self.tv;
        let (md, mw, mh) = map;
        let chroma = |cu: f32, cv: f32| {
            let d = (sample_bilinear(td, tw, th, f.tu, cv * sv[1] * 0.25, true, 0) / 255.0 * 2.0 - 1.0) * k;
            let (ju, jv) = (cu + f.jump[0] * 0.01, cv + f.jump[1] * 0.01);
            let r = sample_bilinear(md, mw, mh, d * fac[0] + ju, jv, false, 0);
            let g = sample_bilinear(md, mw, mh, d * fac[1] + ju, jv, false, 1);
            let b = sample_bilinear(md, mw, mh, d * fac[2] + ju, jv, false, 2);
            let alpha = (sample_bilinear(md, mw, mh, d * fac[0] + ju, jv, false, 3)
                + sample_bilinear(md, mw, mh, d * fac[1] + ju, jv, false, 3)
                + sample_bilinear(md, mw, mh, d * fac[2] + ju, jv, false, 3))
                / 3.0;
            [r, g, b, alpha]
        };
        let blocky = chroma((res * pu).floor() / res + 0.5 / res, (res * pv).floor() / res + 0.5 / res);
        if wgt <= 0.0 {
            return blocky;
        }
        let smooth = chroma(pu, pv);
        std::array::from_fn(|i| blocky[i] + wgt * (smooth[i] - blocky[i]))
    }
}

/// A bilinear sample (0..255) of one channel of an RGBA8 picture, either
/// repeating (`wrap`) or held at its edges.
fn sample_bilinear(d: &[u8], w: usize, h: usize, s: f32, t: f32, wrap: bool, k: usize) -> f32 {
    let (fx, fy) = (s * w as f32 - 0.5, t * h as f32 - 0.5);
    let (x0, y0) = (fx.floor(), fy.floor());
    let (tx, ty) = (fx - x0, fy - y0);
    let at = |v: f32, m: usize| if wrap { (v as i64).rem_euclid(m as i64) as usize } else { (v as i64).clamp(0, m as i64 - 1) as usize };
    let (xa, xb, ya, yb) = (at(x0, w), at(x0 + 1.0, w), at(y0, h), at(y0 + 1.0, h));
    let px = |x: usize, y: usize| f32::from(d[(y * w + x) * 4 + k]);
    let top = px(xa, ya) * (1.0 - tx) + px(xb, ya) * tx;
    let bottom = px(xa, yb) * (1.0 - tx) + px(xb, yb) * tx;
    top * (1.0 - ty) + bottom * ty
}

impl MatchValues {
    pub(crate) fn read(dvars: &sim::ScriptDvars<'_>) -> Option<MatchValues> {
        let s = |k: &str| dvars.string(k).unwrap_or_default().to_owned();
        dvars.string("bo2mp_scores")?;
        Some(MatchValues {
            scores: s("bo2mp_scores"),
            timeleft: s("bo2mp_timeleft"),
            team: s("bo2mp_team"),
            gametype: s("bo2mp_gametype"),
            menu: s("bo2mp_menu"),
            lui: s("bo2mp_lui"),
            feed: s("bo2mp_feed"),
            teams: s("bo2mp_teams"),
            cols: s("bo2mp_cols"),
            rows: String::new(),
            settings: s("bo2mp_settings"),
            minimap: s("bo2mp_minimap"),
            radar: s("bo2mp_radar"),
            locsel: s("bo2mp_locsel"),
            killcam: s("bo2mp_killcam"),
            hud_hidden: dvars.string("bo2zm_hud_hidden") == Some("1"),
            scoped: false,
            vehicle: s("bo2mp_vehicle"),
            vision: s("bo2mp_vision"),
            infrared: s("bo2mp_infrared"),
            sensor: s("bo2mp_sensor"),
            objectives: s("bo2mp_objectives"),
            num_lives: s("scr_numLives"),
        })
    }

    /// Every player's scoreboard values from the snapshot.
    pub(crate) fn rows_of(meta: &sim::SnapshotMeta) -> String {
        meta.clients
            .iter()
            .map(|(c, m)| {
                let name = String::from_utf8_lossy(&m.name).trim_end_matches(char::from(0)).replace(['|', ';'], " ");
                format!("{}|{name}|{}|{}|{}", c.0, m.score, m.kills, m.deaths)
            })
            .collect::<Vec<_>>()
            .join(";")
    }
}

/// What a match's HUD has already been sent (each numbered item once).
#[derive(Default)]
pub(crate) struct MatchSeen {
    shown_menu: String,
    lui: u32,
    feed: u32,
    /// The kill feed's lines (newest last) and when each came (LUI time);
    /// each row's elements in the obituary window (killer, icon, killed).
    feed_lines: std::collections::VecDeque<(FeedLine, f64)>,
    feed_rows: Vec<[Value; 3]>,
}

/// One kill feed line: the killer (none: a suicide or a fall), the
/// weapon's kill icon (picture, width per height) or else its name, the
/// one killed.
#[derive(Clone, Debug, Default)]
pub(crate) struct FeedLine {
    killer: String,
    icon: Option<(String, f32)>,
    fallen: String,
    /// Whether the killer and the one killed are on his team (`None`: no
    /// team known, the name keeps the font's white).
    attacker_mine: Option<bool>,
    victim_mine: Option<bool>,
}

/// How many kill feed lines show at once, how long each stays (ms), and its
/// fades: four obituary lines, 5 s each, a 0.25 s scroll and fade in and a
/// 0.5 s fade out (the engine family's message defaults). BO2's own game is
/// packed and one real frame shows one line, so BO2's own counts are
/// unconfirmed (INDEX.md, KNOWN GAPS).
const FEED_LINES: usize = 4;
const FEED_MS: f64 = 5000.0;
const FEED_FADE_IN_MS: f64 = 250.0;
const FEED_FADE_OUT_MS: f64 = 500.0;

/// `obj:name(args...)`.
fn call_method(host: &mut Host, obj: &Value, name: &str, args: Vec<Value>) -> Option<Vec<Value>> {
    let f = host.vm.lookup_index(obj, &Value::str(name)).ok()?;
    let mut all = vec![obj.clone()];
    all.extend(args);
    host.call(f, all)
}

/// A kill feed line as the engine writes one: the killer, the weapon's
/// kill icon, the one killed (a suicide or a fall: the icon and him).
fn feed_line(_host: &Host, mp: &Mp, item: &str, my_team: &str) -> Option<FeedLine> {
    let mut f = item.split('|').skip(1);
    let (killer, fallen, weapon) = (f.next()?, f.next()?, f.next().unwrap_or(""));
    let mod_ = f.next().unwrap_or("");
    let own = f.next() == Some("self");
    // (The sides the sim saw: the killer's, the one killed's.) The engine's
    // team colour rule: a name is MyTeam's colour on his side, EnemyTeam's on the other, and
    // white unless both his side and the name's are allies or axis (a
    // free-for-all, a spectator, no side).
    let side = |t: &str| matches!(t, "allies" | "axis");
    let mine = |team: Option<&str>| team.filter(|t| side(t) && side(my_team)).map(|t| t == my_team);
    let (attacker_mine, victim_mine) = (mine(f.next()), mine(f.next()));
    let base = weapon.split('+').next().unwrap_or("").trim_end_matches("_mp");
    let m = mp.borrow();
    // The engine's obituary rule: a gun with a kill icon draws
    // it; a gun without one draws the suicide picture; no gun at all draws
    // the picture of how he died (`killicondied` unless the means says
    // otherwise). The gun's name is never written.
    let none = base.is_empty() || base == "none";
    let named = |n: &str| m.kill_icons.get(n).cloned();
    let icon = if none {
        match mod_ {
            "MOD_MELEE" => named("hud_obit_knife"),
            "MOD_HEAD_SHOT" => named("killiconheadshot"),
            "MOD_CRUSH" => named("hud_obit_death_crush"),
            "MOD_FALLING" => named("hud_obit_death_falling"),
            "MOD_SUICIDE" | "MOD_TRIGGER_HURT" => named("hud_obit_death_suicide"),
            "MOD_IMPACT" => named("hud_obit_death_grenade_round"),
            _ => None,
        }
        .or_else(|| named("killicondied"))
    } else {
        named(base).or_else(|| named("hud_obit_death_suicide"))
    };
    Some(FeedLine {
        killer: if own { String::new() } else { killer.to_owned() },
        icon,
        fallen: fallen.to_owned(),
        attacker_mine,
        victim_mine,
    })
}

/// The kill feed in the engine's obituary window (window 0), newest at the
/// bottom, each line fading out after a few seconds: the killer's name, the
/// weapon's kill icon, the name of the one killed, left to right.
fn draw_feed(host: &mut Host, mp: &Mp, seen: &mut MatchSeen) {
    let now = host.clock_millis();
    seen.feed_lines.retain(|(_, at)| now - at < FEED_MS);
    let Some(window) = mp.borrow().windows.iter().find(|(_, i)| *i == 0).map(|(w, _)| w.clone()) else {
        return;
    };
    let fonts = host.field("CoD", "fonts");
    let font = host.vm.lookup_index(&fonts, &Value::str("ExtraSmall")).unwrap_or(Value::Nil);
    let sizes = host.field("CoD", "textSize");
    let h = host.vm.lookup_index(&sizes, &Value::str("ExtraSmall")).ok().and_then(|v| v.as_num()).unwrap_or(16.0);
    if seen.feed_rows.is_empty() {
        let ui_text = host.field("LUI", "UIText");
        let ui_image = host.field("LUI", "UIImage");
        let (Ok(new_text), Ok(new_image)) = (host.vm.lookup_index(&ui_text, &Value::str("new")), host.vm.lookup_index(&ui_image, &Value::str("new"))) else {
            return;
        };
        for _ in 0..FEED_LINES {
            let make = |host: &mut Host, new: &Value| host.call(new.clone(), vec![]).and_then(|r| r.into_iter().next());
            let (Some(a), Some(i), Some(v)) = (make(host, &new_text), make(host, &new_image), make(host, &new_text)) else { return };
            for t in [&a, &v] {
                call_method(host, t, "setFont", vec![font.clone()]);
                call_method(host, t, "setText", vec![Value::str("")]);
            }
            for e in [&a, &i, &v] {
                call_method(host, e, "setAlpha", vec![Value::Num(0.0)]);
                call_method(host, &window, "addElement", vec![e.clone()]);
            }
            seen.feed_rows.push([a, i, v]);
        }
    }
    let measure = host.vm.global("GetTextDimensions");
    let width = |host: &mut Host, words: &str| -> f32 {
        if words.is_empty() {
            return 0.0;
        }
        host.call(measure.clone(), vec![Value::str(words), font.clone(), Value::Num(h)])
            .and_then(|r| r.get(2).and_then(Value::as_num))
            .unwrap_or(h * 0.5 * words.len() as f32)
    };
    // BO2's name colours: the scripts' `CoD.teamColorFriendly` / `teamColorEnemy` (the g_TeamColor_MyTeam / EnemyTeam dvars).
    let team_rgb = |host: &mut Host, mine: bool| -> Option<[f32; 3]> {
        let t = host.field("CoD", if mine { "teamColorFriendly" } else { "teamColorEnemy" });
        let mut c = [0.0; 3];
        for (i, k) in ["r", "g", "b"].iter().enumerate() {
            c[i] = host.vm.lookup_index(&t, &Value::str(k)).ok().and_then(|v| v.as_num())?;
        }
        Some(c)
    };
    let shown: Vec<(FeedLine, f64)> = seen.feed_lines.iter().rev().take(FEED_LINES).rev().cloned().collect();
    let pad = FEED_LINES - shown.len();
    // The engine's death message: the icon sits between ordinary space
    // glyphs.
    let gap = {
        let w = width(host, "i i") - width(host, "ii");
        if w > 0.5 { w } else { h * 0.3 }
    };
    for (i, [a, icon, v]) in seen.feed_rows.clone().iter().enumerate() {
        let bottom = -(((FEED_LINES - 1 - i) as f32) * h);
        let place = |host: &mut Host, e: &Value, x0: f32, x1: f32, top: f32| {
            call_method(host, e, "setLeftRight", vec![Value::Boolean(true), Value::Boolean(false), Value::Num(x0), Value::Num(x1)]);
            call_method(host, e, "setTopBottom", vec![Value::Boolean(false), Value::Boolean(true), Value::Num(top), Value::Num(bottom)]);
        };
        let Some((line, at)) = i.checked_sub(pad).and_then(|k| shown.get(k)) else {
            for e in [a, icon, v] {
                call_method(host, e, "setAlpha", vec![Value::Num(0.0)]);
            }
            continue;
        };
        let age = now - at;
        let alpha = (age / FEED_FADE_IN_MS).min((FEED_MS - age) / FEED_FADE_OUT_MS).clamp(0.0, 1.0) as f32;
        let mut x = 0.0;
        // The killer.
        let w = width(host, &line.killer);
        call_method(host, a, "setText", vec![Value::str(&line.killer)]);
        let c = line.attacker_mine.and_then(|m| team_rgb(host, m)).unwrap_or([1.0; 3]);
        call_method(host, a, "setRGB", vec![Value::Num(c[0]), Value::Num(c[1]), Value::Num(c[2])]);
        place(host, a, x, x + w, bottom - h);
        call_method(host, a, "setAlpha", vec![Value::Num(if w > 0.0 { alpha } else { 0.0 })]);
        if w > 0.0 {
            x += w + gap;
        }
        // The weapon's kill icon (the engine's sizes: a 2:1 picture is 2.8 x
        // 1.4 font heights, a 4:1 one 2.8 x 0.7, a square one 1.4 x 1.4).
        match &line.icon {
            Some((picture, wide)) => {
                let (iw, ih) = if *wide >= 4.0 { (h * 2.0, h * 0.5) } else { (h * wide, h) };
                let register = host.vm.global("RegisterMaterial");
                if let Some(mat) = host.call(register, vec![Value::str(picture)]).and_then(|r| r.into_iter().next()) {
                    call_method(host, icon, "setImage", vec![mat]);
                }
                let low = bottom - (h - ih) * 0.5;
                call_method(host, icon, "setLeftRight", vec![Value::Boolean(true), Value::Boolean(false), Value::Num(x), Value::Num(x + iw)]);
                call_method(host, icon, "setTopBottom", vec![Value::Boolean(false), Value::Boolean(true), Value::Num(low - ih), Value::Num(low)]);
                call_method(host, icon, "setAlpha", vec![Value::Num(alpha)]);
                x += iw + gap;
            }
            None => {
                call_method(host, icon, "setAlpha", vec![Value::Num(0.0)]);
            }
        }
        let fallen = line.fallen.clone();
        // The one killed.
        let w = width(host, &fallen);
        call_method(host, v, "setText", vec![Value::str(&fallen)]);
        let c = line.victim_mine.and_then(|m| team_rgb(host, m)).unwrap_or([1.0; 3]);
        call_method(host, v, "setRGB", vec![Value::Num(c[0]), Value::Num(c[1]), Value::Num(c[2])]);
        place(host, v, x, x + w, bottom - h);
        call_method(host, v, "setAlpha", vec![Value::Num(alpha)]);
    }
}

/// His stats file for a match's class menu (beside settings.cfg).
pub(crate) fn stats_path(identity: Option<&frame::StartupPersona>) -> std::path::PathBuf {
    identity.map_or_else(|| std::path::PathBuf::from(diag::ARTIFACTS_DIR), |i| i.artifacts.clone()).join("bo2mp_stats.txt")
}

/// A match's HUD before CoD's base: the multiplayer answers and his stats
/// (the class menu shows his classes).
pub(crate) fn match_install(host: &mut Host, stats: &std::path::Path) -> Mp {
    let mp = hks_t6::mp::install(host);
    if let Ok(text) = bo2_profile::read_stats_text(stats) {
        mp.borrow_mut().load_text(&text);
    }
    mp
}

/// After CoD's base: a local online match (BO2's CUSTOM GAMES, or Combat
/// Training when ranked; its classes are the set the server gives), in game, his HUD
/// shown; then BO2's own HUD scripts (`T6.HUD`).
pub(crate) fn match_load(host: &mut Host, mp: &Mp, gametype: &str, map: &str) {
    hks_t6::mp::after_base(host);
    // A new match: the After Action Report's medals start over.
    hks_t6::aar::new_match();
    // The visibility bits a live match has on: the HUD, the kill feed,
    // the minimap, objective icons, popups (medals, score).
    let bits: Vec<i32> = [
        "BIT_HUD_VISIBLE",
        "BIT_HUD_OBITUARIES",
        "BIT_COMPASS_VISIBLE",
        "BIT_HUD_SHOWOBJICONS",
        "BIT_ENABLE_POPUPS",
        "BIT_POPUPS_VISIBLE",
    ]
    .iter()
    .filter_map(|b| host.field("CoD", b).as_num().map(|n| n as i32))
    .collect();
    let modes: Vec<f32> = ["SESSIONMODE_ONLINE", "SESSIONMODE_PRIVATE"]
        .iter()
        .filter_map(|m| host.field("CoD", m).as_num())
        .collect();
    // The class set the menus read is the server's: a ranked match (Combat
    // Training) gives the public set, any other the custom match set.
    let private = host.field("CoD", "GAMEMODE_PRIVATE_MATCH").as_num().map_or(1, |n| n as i32);
    if sim::ranked() {
        mp.borrow_mut().game_modes.remove(&private);
    } else {
        mp.borrow_mut().game_modes.insert(private);
    }
    {
        let mut v = host.values.borrow_mut();
        for (k, x) in [
            ("r_fontResolution", "720"),
            ("ui_gametype", gametype),
            ("ui_gameType", gametype),
            ("g_gametype", gametype),
            ("ui_mapname", map),
            ("mapname", map),
            ("sv_running", "1"),
            ("cl_ingame", "1"),
        ] {
            v.dvars.insert(k.to_owned(), x.to_owned());
        }
        v.session_modes = modes;
        v.bits.extend(bits);
    }
    host.require("T6.HUD");
}

/// The game type's settings ("name=value,...") for the HUD's
/// `GetGametypeSetting` (and the score limit dvar).
fn apply_settings(host: &mut Host, mp: &Mp, settings: &str) {
    mp.borrow_mut().settings = settings
        .split(',')
        .filter_map(|kv| kv.split_once('='))
        .filter_map(|(k, v)| Some((k.trim().to_ascii_lowercase(), v.trim().parse::<f32>().ok()?)))
        .collect();
    if let Some(limit) = mp.borrow().settings.get("scorelimit") {
        host.values.borrow_mut().dvars.insert("ui_scorelimit".to_owned(), format!("{limit}"));
    }
}

/// Before BO2's HUD is built: what it builds from (the scoreboard's column
/// headers and one block per team: the game type's columns and settings).
pub(crate) fn match_prime(host: &mut Host, mp: &Mp, m: &MatchValues) {
    host.values.borrow_mut().columns = m.cols.split(',').filter(|c| !c.is_empty()).map(str::to_owned).collect();
    apply_settings(host, mp, &m.settings);
}

/// Open BO2's HUD as the engine does.
pub(crate) fn match_open(host: &mut Host) {
    host.open_menu("HUD");
    host.root_event("first_snapshot", &[]);
    host.root_event("hud_update_refresh", &[]);
}

/// bo2mp vehicle screens: BO2's killstreak HUD event for the vehicle he
/// rides (airvehiclehud.lua UpdateKillstreakHUD: `infrared` lights FLIR,
/// else OPT).
fn killstreak_hud(host: &mut Host, new: &MatchValues) {
    let kind = new.vehicle.split_whitespace().next();
    host.root_event(
        "hud_update_killstreak_hud",
        &[
            ("controller", Value::Num(0.0)),
            ("chopperGunner", Value::Boolean(kind == Some("heli_player_gunner_mp"))),
            ("reaper", Value::Boolean(kind == Some("remote_mortar_mp"))),
            ("predator", Value::Boolean(kind == Some("remote_missile_mp"))),
            ("infrared", Value::Boolean(new.infrared == "1")),
        ],
    );
}

/// The match's events for what changed: the score bar, the clock, the menus
/// the server opened, the LUI notifies (score popups, medals).
/// bo2mp: the objectives' events, as the engine sends them when his
/// objectives change: BO2's GametypeBase makes a waypoint for each named one
/// its gametype draws; one gone or renamed closes (the refresh), a new one
/// opens (`new_objective`), a changed one updates (`objective_update_<name>`).
fn objective_events(host: &mut Host, mp: &Mp, row: &str) {
    let new = hks_t6::mp::parse_objectives(row);
    let old = std::mem::take(&mut mp.borrow_mut().objectives);
    let kept: std::collections::BTreeMap<i32, hks_t6::mp::ObjectiveRow> = old
        .iter()
        .filter(|(i, o)| new.get(i).is_some_and(|n| n.name == o.name))
        .map(|(i, o)| (*i, o.clone()))
        .collect();
    if kept.len() != old.len() {
        mp.borrow_mut().objectives = kept;
        host.root_event("hud_update_refresh", &[]);
    }
    mp.borrow_mut().objectives = new.clone();
    for (i, o) in new.iter().filter(|(_, o)| !o.name.is_empty()) {
        let id = [("objId", Value::Num(*i as f32))];
        match old.get(i) {
            Some(p) if p.name == o.name => {
                if p != o {
                    host.root_event(&format!("objective_update_{}", o.name), &id);
                }
            }
            _ => host.root_event("new_objective", &id),
        }
    }
}

pub(crate) fn match_events(host: &mut Host, seen: &mut MatchSeen, old: Option<&MatchValues>, new: &MatchValues, mp: &Mp) {
    hks_t6::mp::update_timers(mp);
    let changed = |f: fn(&MatchValues) -> &str| old.is_none_or(|o| f(o) != f(new));
    if changed(|v| &v.objectives) {
        objective_events(host, mp, &new.objectives);
    }
    let team_num = |host: &mut Host, t: &str| match t {
        "allies" => host.field("CoD", "TEAM_ALLIES").as_num().unwrap_or(1.0),
        "axis" => host.field("CoD", "TEAM_AXIS").as_num().unwrap_or(2.0),
        _ => host.field("CoD", "TEAM_FREE").as_num().unwrap_or(0.0),
    };
    if changed(|v| &v.team) || changed(|v| &v.scores) || changed(|v| &v.gametype) {
        diag::info!(Ui, "match hud: side {:?} scores {:?} mode {:?}", new.team, new.scores, new.gametype);
    }
    if changed(|v| &v.team) {
        let t = team_num(host, &new.team);
        host.values.borrow_mut().team = t as i32;
        host.root_event("hud_update_team_change", &[("team", Value::Num(t))]);
    }
    if changed(|v| &v.scores) || changed(|v| &v.team) {
        let (side_a, side_b) = new.scores.split_once(',').unwrap_or(("0", "0"));
        let (side_a, side_b) = (side_a.trim().parse::<f32>().unwrap_or(0.0), side_b.trim().parse::<f32>().unwrap_or(0.0));
        let (yours, enemy) = if new.team == "axis" { (side_b, side_a) } else { (side_a, side_b) };
        let t = team_num(host, &new.team);
        host.root_event(
            "hud_update_scores",
            &[("team", Value::Num(t)), ("yourScore", Value::Num(yours)), ("enemyScore", Value::Num(enemy))],
        );
    }
    if changed(|v| &v.num_lives) {
        host.values.borrow_mut().dvars.insert("scr_numLives".to_owned(), new.num_lives.clone());
    }
    if changed(|v| &v.rows) || changed(|v| &v.teams) || changed(|v| &v.cols) || changed(|v| &v.scores) {
        let cols: Vec<String> = new.cols.split(',').filter(|c| !c.is_empty()).map(str::to_owned).collect();
        let team_of = |c: &str| {
            new.teams
                .split(',')
                .find_map(|e| e.split_once(':').filter(|(k, _)| *k == c).map(|(_, t)| t.to_owned()))
        };
        let mut board = Vec::new();
        for row in new.rows.split(';') {
            let f: Vec<&str> = row.split('|').collect();
            let [c, who, score, kills, deaths] = f[..] else { continue };
            let Some(team) = team_of(c) else { continue };
            let num = |x: &str| x.trim().parse::<f32>().unwrap_or(0.0);
            let values: Vec<f32> = cols
                .iter()
                .map(|col| match col.as_str() {
                    "score" => num(score),
                    "kills" => num(kills),
                    "deaths" => num(deaths),
                    "kdratio" => num(kills) / num(deaths).max(1.0),
                    _ => 0.0,
                })
                .collect();
            let team = team_num(host, &team) as i32;
            board.push(hks_t6::mp::BoardRow { client: c.parse().unwrap_or(0), team, name: who.to_owned(), cols: values });
        }
        board.sort_by(|a, b| b.cols.first().unwrap_or(&0.0).total_cmp(a.cols.first().unwrap_or(&0.0)));
        // The board's sides (GetTeamPositions): BO2's scoreboard script asks
        // each side for its rows. In Free-for-All and the Party Games everyone
        // is on the free team, so that is the only side (Allies and Axis
        // would hold no one and the board came up empty).
        let free = team_num(host, "free") as i32;
        let scores = if !board.is_empty() && board.iter().all(|r| r.team == free) {
            vec![(free, 0.0)]
        } else {
            let (side_a, side_b) = new.scores.split_once(',').unwrap_or(("0", "0"));
            vec![
                (team_num(host, "allies") as i32, side_a.trim().parse::<f32>().unwrap_or(0.0)),
                (team_num(host, "axis") as i32, side_b.trim().parse::<f32>().unwrap_or(0.0)),
            ]
        };
        // (The After Action Report's scoreboard is the match's last.)
        hks_t6::aar::set_last_match(hks_t6::aar::LastMatch {
            gametype: new.gametype.clone(),
            columns: cols.clone(),
            rows: board.clone(),
        });
        {
            let mut m = mp.borrow_mut();
            m.board = board;
            m.team_scores = scores;
        }
        host.values.borrow_mut().columns = cols;
        host.root_event("update_scoreboard", &[]);
    }
    // Picking a spot on the map (Lightning Strike, the choppers): BO2's HUD
    // opens its whole map (its handler for BIT_SELECTING_LOCATION), the
    // engine draws the selector on it.
    if changed(|v| &v.locsel) {
        let bit = host.field("CoD", "BIT_SELECTING_LOCATION").as_num().map(|n| n as i32);
        if let Some(bit) = bit {
            let on = !new.locsel.trim().is_empty();
            {
                let mut v = host.values.borrow_mut();
                if on {
                    v.bits.insert(bit);
                } else {
                    v.bits.remove(&bit);
                }
            }
            host.root_event(&format!("hud_update_bit_{bit}"), &[("controller", Value::Num(0.0))]);
        }
    }
    // The HUD the script took away (the match's end) goes with its bit: a
    // killcam's widget stays (it is not in the HUD's container).
    if old.is_none_or(|o| o.hud_hidden != new.hud_hidden)
        && let Some(bit) = host.field("CoD", "BIT_HUD_VISIBLE").as_num().map(|n| n as i32)
    {
        {
            let mut v = host.values.borrow_mut();
            if new.hud_hidden {
                v.bits.remove(&bit);
            } else {
                v.bits.insert(bit);
            }
        }
        host.root_event(&format!("hud_update_bit_{bit}"), &[("controller", Value::Num(0.0))]);
    }
    // The killcam (BO2's engine sets these bits while one plays; its HUD's
    // Killcam widget draws the title, the bands and the killer's card).
    if changed(|v| &v.killcam) {
        let (kind, killer) = new
            .killcam
            .split_once('|')
            .map_or((0, None), |(k, c)| (k.parse::<u8>().unwrap_or(0), c.parse::<i32>().ok().filter(|c| *c >= 0)));
        mp.borrow_mut().callout = killer;
        diag::info!(Ui, "match hud: killcam now {:?}", new.killcam);
        // Kind 3 is the card at his death, before the killcam: BO2's engine
        // sends `player_obituary_callout` with the killer's name card and
        // "Killed By" (notificationpopups.lua PlayerObituaryCallout); the
        // HUD stays, and the card goes after 2.5 s or as the killcam starts.
        if kind == 3
            && let Some(killer) = killer
        {
            let data = host.field("Engine", "GetCalloutPlayerData");
            let card = host.call(data, vec![Value::Num(0.0), Value::Num(killer as f32)]).and_then(|r| r.into_iter().next());
            if let Some(Value::Table(card)) = card {
                let killed_by = host.values.borrow().localize.get("CGAME_KILLEDBY").cloned().unwrap_or_else(|| "Killed By".to_owned());
                card.borrow_mut().set_str("killString", Value::str(&killed_by));
                host.root_event_table("player_obituary_callout", card);
                diag::info!(Ui, "match hud: killed-by card up for {killer}");
            } else {
                diag::warn!(Ui, "match hud: {killer} has no callout data, so no killed-by card");
            }
        }
        // The final one's bit first: the widget reads it as it opens.
        // He is dead and watching the killer: the ammo, score and minimap
        // go (their own bits, set after the killcam's). Only a bit that
        // changes is sent: the Killed By card closes on any update of
        // BIT_IN_KILLCAM.
        let want = [
            ("BIT_FINAL_KILLCAM", kind == 2),
            ("BIT_ROUND_END_KILLCAM", kind == 4),
            ("BIT_IN_KILLCAM", matches!(kind, 1 | 2 | 4)),
            ("BIT_SPECTATING_CLIENT", matches!(kind, 1 | 2 | 4)),
            ("BIT_PLAYER_DEAD", matches!(kind, 1 | 2 | 4)),
        ];
        for (name, on) in want {
            let Some(bit) = host.field("CoD", name).as_num().map(|n| n as i32) else { continue };
            let changed = {
                let mut v = host.values.borrow_mut();
                if on { v.bits.insert(bit) } else { v.bits.remove(&bit) }
            };
            if changed {
                host.root_event(&format!("hud_update_bit_{bit}"), &[("controller", Value::Num(0.0))]);
            }
        }
    }
    if old.is_none_or(|o| o.scoped != new.scoped)
        && let Some(bit) = host.field("CoD", "BIT_IS_SCOPED").as_num().map(|n| n as i32)
    {
        {
            let mut v = host.values.borrow_mut();
            if new.scoped {
                v.bits.insert(bit);
            } else {
                v.bits.remove(&bit);
            }
        }
        host.root_event(&format!("hud_update_bit_{bit}"), &[("controller", Value::Num(0.0))]);
    }
    // bo2mp vehicle screens: riding a scorestreak vehicle. BO2's HUD hides
    // its ammo, score, game type and scorestreak parts on BIT_IN_VEHICLE
    // (ui_mp/t6/hud/ammoarea.lua, scorearea.lua, scorebottomleft.lua,
    // gametypebase.lua, rewardselection.lua), and its own screen for the
    // VTOL Warship's gunner (heli_player_gunner_mp, _helicopter_gunner.gsc)
    // is CoD.ChopperGunnerHUD, which hud.lua opens on
    // `hud_update_killstreak_hud` with chopperGunner set and closes on it
    // unset (airvehiclehud.lua UpdateKillstreakHUD).
    if changed(|v| &v.vehicle) {
        let riding = !new.vehicle.trim().is_empty();
        let gunner = new.vehicle.split_whitespace().next() == Some("heli_player_gunner_mp");
        // The Hellstorm (remote_missile_mp, _remotemissile.gsc) is not a
        // vehicle seat: BO2's HUD hides its ammo and score parts on
        // BIT_IN_GUIDED_MISSILE and opens PredatorHUD (predatorhud.lua) on
        // `hud_update_killstreak_hud` with predator set.
        let missile = new.vehicle.split_whitespace().next() == Some("remote_missile_mp");
        // The Lodestar (remote_mortar_mp, _remotemortar.gsc: his view on its
        // drone) is ReaperHUD (reaperhud.lua), which hud.lua opens on
        // `hud_update_killstreak_hud` with reaper set; BIT_IN_VEHICLE hides
        // the same parts BIT_IN_REMOTE_KILLSTREAK_STATIC does.
        let reaper = new.vehicle.split_whitespace().next() == Some("remote_mortar_mp");
        diag::info!(
            Ui,
            "match hud: riding {:?} (chopper gunner view {gunner}, hellstorm view {missile}, \
             lodestar view {reaper})",
            new.vehicle
        );
        // His keys for the vehicle's own commands: BO2 binds each (up, down,
        // change seat, attack, attack 2) on the keys he has for the command
        // its button stands for (CG_UpdateVehicleBindings), the prompts'
        // `[{+vehiclemoveup}]` and `[{+weapnext_inventory}]`.
        {
            let mut v = host.values.borrow_mut();
            v.vehicle_binds.clear();
            let mut words = new.vehicle.split_whitespace().skip(2);
            for command in ["+vehiclemoveup", "+vehiclemovedown", "+switchseat", "+vehicleattack", "+vehicleattacksecond"] {
                if let Some(from) = words.next().filter(|w| *w != "-") {
                    v.vehicle_binds.insert(command.to_owned(), from.to_owned());
                }
            }
        }
        for (bit_name, on) in [("BIT_IN_VEHICLE", riding && !missile), ("BIT_IN_GUIDED_MISSILE", missile)] {
            let Some(bit) = host.field("CoD", bit_name).as_num().map(|n| n as i32) else {
                continue;
            };
            {
                let mut v = host.values.borrow_mut();
                if on {
                    v.bits.insert(bit);
                } else {
                    v.bits.remove(&bit);
                }
            }
            host.root_event(&format!("hud_update_bit_{bit}"), &[("controller", Value::Num(0.0))]);
        }
        killstreak_hud(host, new);
        // The drones' own screens: hud.lua's `hud_update_vehicle` closes the
        // last vehicle screen and opens `LUI.createMenu[vehicleType]`, which
        // BO2 has for the Dragonfire (qrdrone_mp, hud/qrdrone.lua) and the
        // AGR (ai_tank_drone_mp, hud/aitank.lua); on foot there is no type.
        // Sent after the killstreak event, which AirVehicleHUD screens also
        // hear.
        let kind = new.vehicle.split_whitespace().next().filter(|_| !missile && !reaper).unwrap_or("");
        let mut fields = vec![("controller", Value::Num(0.0))];
        if !kind.is_empty() {
            fields.push(("vehicleType", Value::str(kind)));
        }
        host.root_event("hud_update_vehicle", &fields);
    }
    // His screen's infrared switched (the Lodestar's and the VTOL Warship's
    // change-view key): hud.lua opens a screen only when it has none, so the
    // event again just relights the boxes.
    if !changed(|v| &v.vehicle) && changed(|v| &v.infrared) {
        killstreak_hud(host, new);
    }
    if changed(|v| &v.vision) {
        let found = asset_world::set_t6_player_vision(&new.vision);
        diag::info!(Ui, "match hud: host vision {:?} (found: {found})", new.vision);
    }
    if changed(|v| &v.settings) {
        apply_settings(host, mp, &new.settings);
        host.root_event("hud_update_refresh", &[]);
    }
    if changed(|v| &v.timeleft) {
        let ms = new.timeleft.trim().parse::<f32>().map_or(0.0, |s| s * 1000.0);
        mp.borrow_mut().game_end_ms = (!new.timeleft.trim().is_empty()).then(|| host.clock_millis() + f64::from(ms));
        host.root_event("hud_update_game_timer", &[("timeLeft", Value::Num(ms))]);
    }
    if new.menu != seen.shown_menu {
        seen.shown_menu = new.menu.clone();
        match new.menu.split_once(':') {
            Some((_, "")) => host.root_event("close_all_ingame_menus", &[]),
            Some((_, menu)) => host.root_event("open_ingame_menu", &[("menuName", Value::str(menu))]),
            None => {}
        }
    }
    // Each LUI notify once: `{name = <event>, data = {args}}`.
    for item in new.lui.split(';') {
        let mut parts = item.split('|');
        let Some(n) = parts.next().and_then(|n| n.parse::<u32>().ok()) else { continue };
        if n <= seen.lui {
            continue;
        }
        seen.lui = n;
        let Some(name) = parts.next() else { continue };
        let data = hks_t6::value::LuaTable::new_ref();
        for (i, a) in parts.enumerate() {
            let v = a.trim().parse::<f32>().map_or_else(|_| Value::str(a), Value::Num);
            data.borrow_mut().store(Value::Num((i + 1) as f32), v);
        }
        diag::info!(Ui, "match hud: notify {item}");
        // (A medal shown is one the After Action Report lists.)
        if name == "medal_received" {
            if let Some(row) = data.borrow().read_key(&Value::Num(1.0)).as_num() {
                hks_t6::aar::record_medal(row as i32);
            }
        }
        host.root_event(name, &[("data", Value::Table(data))]);
    }
    for item in new.feed.split(';') {
        let Some(n) = item.split('|').next().and_then(|n| n.parse::<u32>().ok()) else { continue };
        if n <= seen.feed {
            continue;
        }
        seen.feed = n;
        if let Some(line) = feed_line(host, mp, item, &new.team) {
            let now = host.clock_millis();
            seen.feed_lines.push_back((line, now));
        }
    }
    draw_feed(host, mp, seen);
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::world::CommandQueue;

    fn ticked(mp: &Mp) -> bool {
        let mut host = Host::new(Vec::new(), Box::new(|_, _, _| 0.0));
        let front = Bo2mpFrontend { stats_path: std::env::temp_dir().join("bo2mp_tick_test_stats") };
        let world = World::new();
        let mut queue = CommandQueue::default();
        let mut commands = Commands::new(&mut queue, &world);
        tick(&mut host, mp, &front, None, &mut commands, None)
    }

    #[test]
    fn the_menu_music_stops_only_when_a_match_is_handed_over() {
        let mp: Mp = Default::default();
        assert!(!ticked(&mp), "nothing asked: the menus keep their song");
        mp.borrow_mut().start_requested = true;
        assert!(!ticked(&mp), "a Custom Game still counting down");
        let mp: Mp = Default::default();
        mp.borrow_mut().zombies_requested = true;
        assert!(ticked(&mp), "ZOMBIES swaps the map in");
    }
}
