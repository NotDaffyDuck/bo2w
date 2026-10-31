//! bo2mp lane C: the players' stats on the server, as BO2's scripts read
//! and write them (its DDL stats: `getdstat`, `setdstat`, `adddstat`,
//! `addplayerstat`, `addweaponstat`, ...), and his own classes from them.
//!
//! The local player's stats are his stats file (`bo2mp_stats.txt`, the
//! front end's: `bo2_profile`'s store by stats path), read when the match
//! first asks and, in a ranked match, written back as they change and at
//! its end (merged into the file, so a path the match never touched stays
//! as the front end left it). A bot's stats start empty and are never
//! saved.
//!
//! The file: what the front end sets (`set_local_stats_path`), else
//! `BO2MP_STATS`, else `bo2mp-artifacts/bo2mp_stats.txt` when there is one
//! (a match started straight from the command line).

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::Mutex;

use bevy_ecs::prelude::{Resource, World};
use bo2_profile::{Profile, StatValue};
use gsc_t6::{ScriptValue, Vm};

use super::{Zm, arg, entnum, text};

static LOCAL_STATS: Mutex<Option<PathBuf>> = Mutex::new(None);
static RANKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether the next match is ranked (Combat Training: XP counts, the
/// public class set), as the front end's playlist chose; `BO2MP_RANKED=1`
/// for a match started from the command line.
pub fn set_local_match_ranked(ranked: bool) {
    RANKED.store(ranked, std::sync::atomic::Ordering::Relaxed);
}

pub fn ranked() -> bool {
    RANKED.load(std::sync::atomic::Ordering::Relaxed) || std::env::var("BO2MP_RANKED").is_ok_and(|v| v == "1")
}

/// His stats file for the matches this process plays (the front end calls
/// this at START MATCH).
pub fn set_local_stats_path(path: PathBuf) {
    if let Ok(mut p) = LOCAL_STATS.lock() {
        *p = Some(path);
    }
}

fn local_path() -> Option<PathBuf> {
    if let Some(saved) = LOCAL_STATS.lock().ok().and_then(|p| p.clone()) {
        return Some(saved);
    }
    if let Ok(p) = std::env::var("BO2MP_STATS") {
        return Some(PathBuf::from(p));
    }
    let p = PathBuf::from("bo2mp-artifacts").join("bo2mp_stats.txt");
    p.exists().then_some(p)
}

/// How often changed stats are written (ms of match time).
const SAVE_EVERY_MS: i64 = 10000;

/// One player's stats.
pub(crate) struct PlayerStats {
    /// The items (his tables) and the stats; his classes read from them.
    pub profile: Profile,
    /// His stats file (the local player only).
    file: Option<PathBuf>,
    /// The paths changed since the last save.
    changed: BTreeSet<String>,
}

#[derive(Resource, Default)]
pub(crate) struct ServerStats {
    players: HashMap<u32, PlayerStats>,
    last_save: i64,
}

fn table_rows(world: &World, name: &str) -> Vec<Vec<String>> {
    let zm = world.resource::<Zm>();
    let Some(t) = zm.tables.get(name) else {
        return Vec::new();
    };
    t.entries.chunks(t.col_total.max(1)).map(<[String]>::to_vec).collect()
}

/// Whether a client is the local player (the listen server's own, not a
/// bot).
fn is_local(world: &World, client: u32) -> bool {
    client == 0 && !world.resource::<Zm>().bots.contains_key(&client)
}

/// A player's stats, read the first time they are asked for.
pub(crate) fn player(world: &mut World, client: u32) -> &mut PlayerStats {
    if !world.contains_resource::<ServerStats>() {
        world.insert_resource(ServerStats::default());
    }
    if !world.resource::<ServerStats>().players.contains_key(&client) {
        let file = is_local(world, client).then(local_path).flatten();
        if file.is_some() {
            bo2_profile::wait_saved(std::time::Duration::from_secs(10));
        }
        let stats = file.as_deref().map(Profile::read_stats_file).unwrap_or_default();
        let (items, atts) = (table_rows(world, "mp/statstable.csv"), table_rows(world, "mp/attachmenttable.csv"));
        let mut profile = Profile::out_of_charts(&items, &atts, &|_| None, stats);
        profile.match_set = class_set(world).to_owned();
        if let Some(f) = &file {
            diag::info!(
                Sim,
                "bo2mp stats: player {client} from {} ({} stats, rank {}, classes from {})",
                f.display(),
                profile.stats.len(),
                profile.rank(),
                profile.match_set
            );
        }
        world
            .resource_mut::<ServerStats>()
            .players
            .insert(client, PlayerStats { profile, file, changed: BTreeSet::new() });
    }
    world.resource_mut::<ServerStats>().into_inner().players.get_mut(&client).expect("inserted above")
}

/// The class set a match gives (BO2's `cacRoot`): a ranked match (Combat
/// Training) the public one, else the custom match set.
fn class_set(world: &World) -> &'static str {
    if world.resource::<Zm>().ranked_match { "cacloadouts" } else { bo2_profile::LOCAL_MATCH_SET }
}

/// The local player's profile, when the match has his stats (his classes).
pub(crate) fn local_profile(world: &mut World, client: u32) -> Option<&Profile> {
    if !is_local(world, client) {
        return None;
    }
    let p = player(world, client);
    p.file.is_some().then_some(&p.profile)
}

fn get(game: &mut World, peer: u32, route: &str) -> Option<StatValue> {
    player(game, peer).profile.stat(route).cloned()
}

fn set(game: &mut World, peer: u32, route: String, v: StatValue) {
    let p = player(game, peer);
    if p.profile.stat(&route) != Some(&v) {
        p.changed.insert(route.clone());
        p.profile.set_stat(route, v);
    }
}

fn add(game: &mut World, peer: u32, route: String, delta: f32) {
    let now = get(game, peer, &route).and_then(|v| v.as_num()).unwrap_or(0.0);
    set(game, peer, route, StatValue::Num(now + delta));
}

/// A stats path from the scripts' words (`"PlayerStatsList", "RANKXP",
/// "StatValue"` -> `playerstatslist.rankxp.statvalue`).
fn path_of(vm: &mut Vm<World>, words: &[ScriptValue]) -> String {
    words
        .iter()
        .map(|w| match w {
            ScriptValue::Int(n) => n.to_string(),
            ScriptValue::Float(f) => (*f as i64).to_string(),
            other => vm.to_text(other).to_ascii_lowercase(),
        })
        .collect::<Vec<_>>()
        .join(".")
}

fn to_value(vm: &mut Vm<World>, v: Option<StatValue>) -> ScriptValue {
    match v {
        Some(StatValue::Num(n)) if n == n.trunc() && n.abs() < 2.0e9 => ScriptValue::Int(n as i32),
        Some(StatValue::Num(n)) => ScriptValue::Float(n),
        Some(StatValue::Text(s)) => vm.string(&s),
        None => ScriptValue::Int(0),
    }
}

fn of_value(vm: &mut Vm<World>, v: &ScriptValue) -> StatValue {
    match v {
        ScriptValue::Int(n) => StatValue::Num(*n as f32),
        ScriptValue::Float(f) => StatValue::Num(*f),
        other => {
            let t = vm.to_text(other);
            t.trim().parse::<f32>().map_or(StatValue::Text(t), StatValue::Num)
        }
    }
}

/// A weapon's stats path start (`itemstats.<item index>.stats`): its
/// item from the statstable by reference (`mp7_mp+reflex` -> MP7).
fn weapon_item(game: &mut World, peer: u32, gun: &str) -> Option<i32> {
    let base = gun.split('+').next().unwrap_or(gun);
    let reference = base.strip_suffix("_mp").unwrap_or(base);
    player(game, peer).profile.item_by_ref(reference).map(|it| it.index)
}

/// Each tick: hand the local player's changed stats to the saver (merged
/// over what his file holds then). Only a ranked match's (Combat
/// Training): as in BO2, a custom game keeps its stats to itself (his
/// Combat Record and weapon stats count only ranked play).
pub(crate) fn save(game: &mut World, forced: bool) {
    if !game.resource::<Zm>().ranked_match {
        return;
    }
    let now = game.resource::<Zm>().clock_millis;
    let Some(mut st) = game.get_resource_mut::<ServerStats>() else { return };
    if !forced && now - st.last_save < SAVE_EVERY_MS {
        return;
    }
    st.last_save = now;
    for (peer, pt) in &mut st.players {
        let Some(file) = pt.file.clone() else { continue };
        if pt.changed.is_empty() {
            continue;
        }
        let changed: Vec<(String, StatValue)> = std::mem::take(&mut pt.changed)
            .into_iter()
            .filter_map(|path| pt.profile.stat(&path).cloned().map(|v| (path, v)))
            .collect();
        let peer = *peer;
        let done: bo2_profile::SaveDone = Box::new(move |file, result| match result {
            Ok(()) => diag::info!(Sim, "bo2mp stats: player {peer}'s saved to {}", file.display()),
            Err(e) => diag::warn!(Sim, "bo2mp stats: player {peer}'s not saved ({}): {e}", file.display()),
        });
        bo2_profile::save_in_background(&file, bo2_profile::SaveJob::Merge(changed), Some(done));
    }
}

fn receiver_for(vm: &mut Vm<World>, s: &ScriptValue) -> Option<u32> {
    entnum(vm, s)
}

// Ranks and XP: the engine's half of BO2's `_rank.gsc` (its `addrankxp`
// and `CodeCallback_RankUp`). The scripts' own half (giverankxp,
// incrankxp, updaterank, syncxpstat) runs as written.

/// A table by name, any case (`mp/rankTable.csv`).
fn table<'a>(zm: &'a Zm, name: &str) -> Option<&'a super::T6Table> {
    zm.tables.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, t)| t)
}

/// mp/rankTable.csv: each rank's id, the XP it starts at, the XP it ends
/// at, and the unlock tokens reaching it gives (column 17: `token` is one,
/// `token 4` four).
struct RankRow {
    id: i32,
    min: f32,
    max: f32,
    tokens: i32,
}

fn rank_rows(world: &World) -> Vec<RankRow> {
    let zm = world.resource::<Zm>();
    let Some(t) = table(zm, "mp/rankTable.csv") else { return Vec::new() };
    t.entries
        .chunks(t.col_total.max(1))
        .filter_map(|entry| {
            let cell = |i: usize| entry.get(i).map_or("", |c| c.trim());
            Some(RankRow {
                id: cell(0).parse().ok()?,
                min: cell(2).parse().ok()?,
                max: cell(7).parse().ok()?,
                tokens: cell(17)
                    .strip_prefix("token")
                    .map_or(0, |n| n.trim().parse().unwrap_or(1)),
            })
        })
        .collect()
}

fn rank_for_xp(rows: &[RankRow], xp: f32) -> i32 {
    rows.iter().filter(|r| r.min <= xp).map(|r| r.id).max().unwrap_or(0)
}

/// An event's row in mp/scoreInfo.csv: its XP in this game type (its
/// `<gametype> XP` column, what BO2's _rank.gsc registers with registerxp),
/// whether it also counts toward the weapon (its GunXP), and its row number
/// when it is a medal (a Medal Reference in column 2).
struct ScoreEvent {
    xp: f32,
    gun: bool,
    medal: Option<i32>,
}

fn score_event(world: &World, gametype: &str, event: &str) -> Option<ScoreEvent> {
    let zm = world.resource::<Zm>();
    let t = table(zm, "mp/scoreInfo.csv")?;
    let mut rows = t.entries.chunks(t.col_total.max(1));
    let header = rows.next()?;
    let col = header.iter().position(|h| h.trim().eq_ignore_ascii_case(&format!("{gametype} XP")))?;
    let gun = header.iter().position(|h| h.trim().eq_ignore_ascii_case("GunXP"));
    // Row numbers count the header as row 0 (the scripts' and the HUD's
    // tablelookup rows).
    let (index, row) = rows.enumerate().find(|(_, r)| r.first().is_some_and(|c| c.trim().eq_ignore_ascii_case(event)))?;
    let xp = row.get(col)?.trim().parse::<f32>().ok()?;
    let gunxp = gun.and_then(|g| row.get(g)).is_some_and(|c| c.trim().eq_ignore_ascii_case("TRUE"));
    let medal = row.get(2).is_some_and(|c| !c.trim().is_empty()).then_some(index as i32 + 1);
    Some(ScoreEvent { xp, gun: gunxp, medal })
}

/// What a stat reads as a number.
fn num(world: &mut World, client: u32, path: &str) -> f32 {
    get(world, client, path).and_then(|v| v.as_num()).unwrap_or(0.0)
}

/// The unlock tokens he has to spend in Create-a-Class (Pick 10): BO2's
/// `unlocks[]` stat, the pool its menus read (CoD.CAC.GetUnlockCountForGroup:
/// `GetDStat(controller, "unlocks", GetUnlockIndexFromGroupName(group))`,
/// one pool, index 0).
use bo2_profile::UNLOCK_TOKENS;
const RANK_XP: &str = "playerstatslist.rankxp.statvalue";

/// XP for a player in a ranked match: his rank XP (capped at the last
/// rank's end), the weapon's XP, and a new rank: its stats, its unlock
/// tokens, and BO2's `CodeCallback_RankUp(rank, prestige, tokens added)`
/// (the scripts' rank-up popup).
fn give_xp(vm: &mut Vm<World>, game: &mut World, who: &ScriptValue, peer: u32, xp: f32, weapon: Option<(&str, bool)>) {
    if xp <= 0.0 {
        return;
    }
    let rows = rank_rows(game);
    let cap = rows.iter().map(|r| r.max).fold(0.0f32, f32::max);
    let old = num(game, peer, "playerstatslist.rankxp.statvalue");
    let new = if cap > 0.0 { (old + xp).min(cap) } else { old + xp };
    set(game, peer, "playerstatslist.rankxp.statvalue".to_owned(), StatValue::Num(new));
    if let Some((wd, true)) = weapon
        && let Some(item) = weapon_item(game, peer, wd)
    {
        // The gun's XP, where Create-a-Class reads its gun level.
        let path = bo2_profile::gun_xp_path(item);
        let before = num(game, peer, &path);
        add(game, peer, path, xp);
        gun_level_up(vm, game, who, peer, item, before, before + xp);
    }
    // The scripts' own count (pers["rankxp"]) stays with the stat: their
    // syncxpstat writes it back.
    if let ScriptValue::Object(o) = who {
        let pers = vm.intern("pers");
        if let ScriptValue::Array(a) = vm.raw_field(*o, pers) {
            let key = gsc_t6::Key::Str(vm.intern("rankxp"));
            a.write().set(key, ScriptValue::Int(new as i32));
        }
    }
    rank_up(vm, game, who, peer, old, new);
}

/// His rank XP went from `old` to `new` (the engine's addrankxp, or the
/// scripts' giverankxp writing it with setdstat): a new rank gets its
/// stats, its unlock tokens, and BO2's `CodeCallback_RankUp(rank,
/// prestige, tokens added)` (the scripts' rank-up popup).
fn rank_up(vm: &mut Vm<World>, game: &mut World, who: &ScriptValue, peer: u32, old: f32, new: f32) {
    let rows = rank_rows(game);
    let (from, to) = (rank_for_xp(&rows, old), rank_for_xp(&rows, new));
    if to <= from {
        return;
    }
    let tokens: i32 = rows.iter().filter(|r| r.id > from && r.id <= to).map(|r| r.tokens).sum();
    add(game, peer, UNLOCK_TOKENS.to_owned(), tokens as f32);
    set(game, peer, "playerstatslist.rank.statvalue".to_owned(), StatValue::Num(to as f32));
    if let Some(r) = rows.iter().find(|r| r.id == to) {
        set(game, peer, "playerstatslist.minxp.statvalue".to_owned(), StatValue::Num(r.min));
        set(game, peer, "playerstatslist.maxxp.statvalue".to_owned(), StatValue::Num(r.max));
    }
    let prestige = num(game, peer, "playerstatslist.plevel.statvalue") as i32;
    diag::info!(Sim, "bo2mp rank: player {peer} rank {from} -> {to} ({new} XP, {tokens} unlock tokens)");
    vm.spawn_named(
        game,
        "maps/mp/gametypes/_rank",
        "codecallback_rankup",
        who.clone(),
        vec![ScriptValue::Int(to), ScriptValue::Int(prestige), ScriptValue::Int(tokens)],
    );
}

/// A gun's XP went from `old` to `new`: each of its gun levels reached
/// (mp/gunlevels.csv) is BO2's `CodeCallback_GunChallengeComplete(reward
/// XP, attachment index, item index, rank)`, the scripts' gun level-up
/// notice (`gun_level_complete` to his HUD). Only a player whose stats are
/// kept (his own): bots need no gun levels. (Whether the engine also adds
/// the reward XP to his rank is not in the scripts: it is not added.
/// PLACEHOLDER for the local team.)
fn gun_level_up(vm: &mut Vm<World>, game: &mut World, who: &ScriptValue, peer: u32, item: i32, old: f32, new: f32) {
    let p = player(game, peer);
    if p.file.is_none() {
        return;
    }
    let Some(reference) = p.profile.item(item).map(|it| it.reference.clone()) else { return };
    let rows = {
        let zm = game.resource::<Zm>();
        let Some(t) = table(zm, "mp/gunlevels.csv") else { return };
        t.entries.chunks(t.col_total.max(1)).map(<[String]>::to_vec).collect::<Vec<_>>()
    };
    let reached: Vec<bo2_profile::GunLevel> = bo2_profile::gun_levels(&rows, &reference)
        .into_iter()
        .filter(|l| old < l.xp as f32 && l.xp as f32 <= new)
        .collect();
    for l in reached {
        let attachment = player(game, peer)
            .profile
            .attachments
            .iter()
            .find(|a| a.reference == l.attachment)
            .map_or(0, |a| a.index);
        diag::info!(Sim, "bo2mp gun level: player {peer} {reference} rank {} ({new} XP, unlocks {})", l.rank, l.attachment);
        vm.spawn_named(
            game,
            "maps/mp/gametypes/_persistence",
            "codecallback_gunchallengecomplete",
            who.clone(),
            vec![ScriptValue::Int(l.reward_xp), ScriptValue::Int(attachment), ScriptValue::Int(item), ScriptValue::Int(l.rank)],
        );
    }
}

pub(super) fn bind(vm: &mut Vm<World>) {
    // The match's end: his stats written first, then the session takes it
    // from here (as natives_game's exitlevel).
    vm.bind("exitlevel", false, |_, world, _, _| {
        save(world, true);
        world.resource_mut::<crate::gsc_host::Runtime>().exit_level = true;
        Ok(ScriptValue::Undefined)
    });
    // A ranked match (Combat Training): XP counts.
    for name in ["gamemodeisusingxp", "gamemodeisusingstats"] {
        vm.bind(name, false, |_, world, _, _| Ok(ScriptValue::Int(i32::from(world.resource::<Zm>().ranked_match))));
    }
    // addrankxp(event, weapon, ...): a medal event with XP shows his medal in
    // any match (the engine calls BO2's _medals::codecallback_medal(row)
    // before its stats checks); the event's XP (mp/scoreInfo.csv) counts in
    // a ranked match, scaled by scr_xpscale.
    vm.bind("addrankxp", true, |vm, world, s, a| {
        let Some(c) = receiver_for(vm, s) else { return Ok(ScriptValue::Undefined) };
        let event = text(vm, a, 0);
        let gametype = vm.dvars.get("g_gametype").cloned().unwrap_or_default();
        let Some(e) = score_event(world, &gametype, &event) else { return Ok(ScriptValue::Undefined) };
        if e.xp > 0.0
            && let Some(row) = e.medal
        {
            vm.spawn_named(world, "maps/mp/_medals", "codecallback_medal", s.clone(), vec![ScriptValue::Int(row)]);
        }
        if !world.resource::<Zm>().ranked_match {
            return Ok(ScriptValue::Undefined);
        }
        let weapon = text(vm, a, 1);
        let scale = vm.dvars.get("scr_xpscale").and_then(|v| v.parse::<f32>().ok()).filter(|x| *x > 0.0).unwrap_or(1.0);
        give_xp(vm, world, s, c, (e.xp * scale).round(), Some((&weapon, e.gun)));
        Ok(ScriptValue::Undefined)
    });
    // addrankxpvalue(event, value): that much XP.
    vm.bind("addrankxpvalue", true, |vm, world, s, a| {
        if !world.resource::<Zm>().ranked_match {
            return Ok(ScriptValue::Undefined);
        }
        let Some(c) = receiver_for(vm, s) else { return Ok(ScriptValue::Undefined) };
        let xp = arg(a, 1).as_float().unwrap_or(0.0);
        give_xp(vm, world, s, c, xp, None);
        Ok(ScriptValue::Undefined)
    });
    // getdstat(path words...): the stat (0 never written).
    vm.bind("getdstat", true, |vm, world, s, a| {
        let Some(c) = receiver_for(vm, s) else { return Ok(ScriptValue::Int(0)) };
        let path = path_of(vm, a);
        let v = get(world, c, &path);
        Ok(to_value(vm, v))
    });
    // setdstat(path words..., value).
    vm.bind("setdstat", true, |vm, world, s, a| {
        if let (Some(c), Some((value, words))) = (receiver_for(vm, s), a.split_last()) {
            let path = path_of(vm, words);
            let v = of_value(vm, value);
            // The scripts' own XP (giverankxp's syncxpstat): a new rank
            // as the engine sees one.
            // (His own only: BO2's bot scripts write the bots' made-up
            // ranks the same way.)
            let xp = (path == RANK_XP && world.resource::<Zm>().ranked_match && is_local(world, c))
                .then(|| (num(world, c, RANK_XP), v.as_num().unwrap_or(0.0)));
            set(world, c, path, v);
            if let Some((old, new)) = xp {
                rank_up(vm, world, s, c, old, new);
            }
        }
        Ok(ScriptValue::Undefined)
    });
    // adddstat(path words..., amount).
    vm.bind("adddstat", true, |vm, world, s, a| {
        if let (Some(c), Some((value, words))) = (receiver_for(vm, s), a.split_last()) {
            let path = path_of(vm, words);
            let n = value.as_float().unwrap_or(0.0);
            add(world, c, path, n);
        }
        Ok(ScriptValue::Undefined)
    });
    // addplayerstat(name, amount): PlayerStatsList.<name>.StatValue.
    vm.bind("addplayerstat", true, |vm, world, s, a| {
        if let Some(c) = receiver_for(vm, s) {
            let name = text(vm, a, 0).to_ascii_lowercase();
            let n = arg(a, 1).as_float().unwrap_or(0.0);
            add(world, c, format!("playerstatslist.{name}.statvalue"), n);
        }
        Ok(ScriptValue::Undefined)
    });
    // addplayerstatwithgametype(name, amount): BO2's adds to both rows,
    // PlayerStatsList.<name> and its game type's
    // (PlayerStatsByGameType.<gametype>.<name>.StatValue).
    vm.bind("addplayerstatwithgametype", true, |vm, world, s, a| {
        if let Some(c) = receiver_for(vm, s) {
            let name = text(vm, a, 0).to_ascii_lowercase();
            let n = arg(a, 1).as_float().unwrap_or(0.0);
            let gametype = vm.dvars.get("g_gametype").cloned().unwrap_or_default().to_ascii_lowercase();
            add(world, c, format!("playerstatslist.{name}.statvalue"), n);
            add(world, c, format!("playerstatsbygametype.{gametype}.{name}.statvalue"), n);
        }
        Ok(ScriptValue::Undefined)
    });
    // updatestatratio(ratio, numerator, denominator): BO2's ratio stats
    // (kdratio, wlratio) are numerator * 1000 / denominator, a denominator
    // of 1 or less counting as 1, in both rows; none when
    // scr_disableStatTracking is set.
    vm.bind("updatestatratio", true, |vm, world, s, a| {
        let Some(c) = receiver_for(vm, s) else { return Ok(ScriptValue::Undefined) };
        let off = vm.dvars.get("scr_disablestattracking").and_then(|v| v.trim().parse::<i32>().ok()).unwrap_or(0) > 0;
        if off {
            return Ok(ScriptValue::Undefined);
        }
        let ratio = text(vm, a, 0).to_ascii_lowercase();
        let top = text(vm, a, 1).to_ascii_lowercase();
        let bottom = text(vm, a, 2).to_ascii_lowercase();
        let gametype = vm.dvars.get("g_gametype").cloned().unwrap_or_default().to_ascii_lowercase();
        for row in ["playerstatslist".to_string(), format!("playerstatsbygametype.{gametype}")] {
            let n = num(world, c, &format!("{row}.{top}.statvalue"));
            let d = num(world, c, &format!("{row}.{bottom}.statvalue")).max(1.0);
            set(world, c, format!("{row}.{ratio}.statvalue"), StatValue::Num((n * 1000.0 / d).trunc()));
        }
        Ok(ScriptValue::Undefined)
    });
    // uploadstats(player): BO2 sent his stats to its servers when he left;
    // here they are written to his stats file (a ranked match's only).
    vm.bind("uploadstats", false, |_, world, _, _| {
        save(world, true);
        Ok(ScriptValue::Undefined)
    });
    // The match's line in BO2's match history ring: nothing here reads it.
    // The nemesis's xuid (`AfterActionReportStats.nemesisXuid`, the report's
    // player card background for him): every xuid here is "0" and every
    // card the default one, so there is nothing to keep. No theater
    // recording (demo file 0) and no leagues (team 0).
    for name in ["gamehistoryfinishmatch", "setnemesisxuid"] {
        vm.bind(name, true, |_, _, _, _| Ok(ScriptValue::Undefined));
    }
    vm.bind("getdemofileid", false, |_, _, _, _| Ok(ScriptValue::Int(0)));
    vm.bind("getleagueteamid", true, |_, _, _, _| Ok(ScriptValue::Int(0)));
    // addweaponstat(weapon, stat, amount): ItemStats.<item>.stats.<stat>.
    vm.bind("addweaponstat", true, |vm, world, s, a| {
        if let Some(c) = receiver_for(vm, s) {
            let weapon = text(vm, a, 0);
            let stat = text(vm, a, 1).to_ascii_lowercase();
            let n = arg(a, 2).as_float().unwrap_or(0.0);
            if let Some(item) = weapon_item(world, c, &weapon) {
                add(world, c, format!("itemstats.{item}.stats.{stat}.statvalue"), n);
            }
        }
        Ok(ScriptValue::Undefined)
    });
    // getrankxpstat() / getcodpointsstat(): PlayerStatsList.rankxp / codpoints.
    vm.bind("getrankxpstat", true, |vm, world, s, _| {
        let Some(c) = receiver_for(vm, s) else { return Ok(ScriptValue::Int(0)) };
        let v = get(world, c, "playerstatslist.rankxp.statvalue");
        Ok(to_value(vm, v))
    });
    vm.bind("getcodpointsstat", true, |vm, world, s, _| {
        let Some(c) = receiver_for(vm, s) else { return Ok(ScriptValue::Int(0)) };
        let v = get(world, c, "playerstatslist.codpoints.statvalue");
        Ok(to_value(vm, v))
    });
}
