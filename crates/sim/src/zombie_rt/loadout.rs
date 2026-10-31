//! bo2mp: a player's class as Black Ops II's class script reads it
//! (`getloadoutweapon`, `getloadoutitem`, `getloadoutitemref`,
//! `getloadoutperks`, `isbonuscardactive`, `getweaponattachments`).
//!
//! Items come from his `mp/statstable.csv` (index, group, name, reference,
//! attachments, default classes, slot). A bot's class is what its own
//! scripts built (`botclassadditem`, `botclassaddattachment`); a player's
//! is BO2's default class for that slot (mp/reset_classes.cfg: the five
//! default custom classes, then the same five again) until his saved
//! classes are read: with his stats file (`stats`), the local player's
//! class is his own, as the front end's Create-a-Class saved it.

use std::sync::{Arc, Mutex, PoisonError};

use bevy_ecs::prelude::World;
use gsc_t6::{ScriptValue, Vm};

use super::{Zm, arg, entnum, list, text};

const TABLE: &str = "mp/statstable.csv";

/// BO2's default custom classes in class order.
const DEFAULT_CLASSES: [&str; 5] = [
    "class_custom_assault",
    "class_custom_smg",
    "class_custom_lmg",
    "class_custom_cqb",
    "class_custom_sniper",
];

#[derive(Clone, Debug, Default)]
struct Item {
    index: i32,
    /// Its group (`bonuscard`, `specialty`, a gun's class).
    family: String,
    name: String,
    reference: String,
    attachments: Vec<String>,
    /// Default classes it is in, each with its extra words (a count).
    classes: Vec<(String, Vec<String>)>,
    slot: String,
    /// The HUD picture (a scorestreak's) and its momentum cost.
    picture: String,
    cost: i32,
}

/// The table's items, read once (its size the key: the table is read
/// again with the level): the HUD asks each tick, the class scripts at
/// each spawn.
#[derive(Default)]
pub(crate) struct ItemCache(Mutex<Option<((usize, usize), Arc<Vec<Item>>)>>);

fn items(world: &World) -> Arc<Vec<Item>> {
    let zm = world.resource::<Zm>();
    let Some(t) = zm.tables.get(TABLE) else {
        return Arc::default();
    };
    let key = (t.row_total, t.entries.len());
    let mut cache = zm.items.0.lock().unwrap_or_else(PoisonError::into_inner);
    match &*cache {
        Some((k, all)) if *k == key => all.clone(),
        _ => {
            let all = Arc::new(read_items(t));
            *cache = Some((key, all.clone()));
            all
        }
    }
}

fn read_items(t: &super::T6Table) -> Vec<Item> {
    let cell = |r: usize, c: usize| t.entries.get(r * t.col_total + c).map_or("", String::as_str);
    let mut out = Vec::new();
    for r in 0..t.row_total {
        let Ok(index) = cell(r, 0).parse::<i32>() else {
            continue;
        };
        if cell(r, 4).is_empty() {
            continue;
        }
        let mut classes: Vec<(String, Vec<String>)> = Vec::new();
        for wd in cell(r, 11).split_whitespace() {
            if wd.starts_with("class_") || wd == "all" {
                classes.push((wd.to_owned(), vec![]));
            } else if let Some(last) = classes.last_mut() {
                last.1.push(wd.to_owned());
            }
        }
        let text = |c: usize| cell(r, c).to_owned();
        out.push(Item {
            index,
            family: text(2),
            name: text(3),
            reference: text(4),
            attachments: cell(r, 8).split_whitespace().map(str::to_owned).collect(),
            classes,
            slot: text(13),
            picture: text(6),
            cost: cell(r, 16).parse().unwrap_or(0),
        });
    }
    out
}

/// A player's scorestreaks as BO2's HUD column lists them: `icon:cost`
/// by `;`, the three he picked (the default ones, in table order, until
/// his own are read), the dearest first (the column lists it top down).
pub(crate) fn streak_picks(world: &World) -> String {
    let mut picks: Vec<(String, i32)> = items(world)
        .iter()
        .filter(|i| i.slot.starts_with("killstreak") && i.classes.iter().any(|(c, _)| c == "all") && i.cost > 0)
        .map(|i| (i.picture.clone(), i.cost))
        .collect();
    picks.truncate(3);
    picks.sort_by_key(|p| std::cmp::Reverse(p.1));
    picks.iter().map(|(i, c)| format!("{i}:{c}")).collect::<Vec<_>>().join(";")
}

/// A player's own scorestreaks as BO2's HUD column lists them:
/// `icon:cost:held` by `;`, dearest first, `held` 1 while his streak weapon
/// holds one (its quantity: earned and not used yet). His picks are the
/// menu references BO2's class script gives him (`player.killstreak`);
/// none read yet: the default ones (`streak_picks`).
pub(crate) fn his_streaks(vm: &mut Vm<World>, game: &mut World, peer: u32) -> Option<String> {
    let obj = game.resource::<Zm>().players.get(&peer)?.obj;
    let f = vm.intern("killstreak");
    let refs: Vec<String> = match vm.raw_field(obj, f) {
        ScriptValue::Array(a) => a.read().values_in_order().map(|v| vm.to_text(v)).collect(),
        _ => return None,
    };
    if refs.is_empty() {
        return None;
    }
    let all = items(game);
    let (types, killstreaks, weapon) = (
        vm.intern("menureferenceforkillstreak"),
        vm.intern("killstreaks"),
        vm.intern("weapon"),
    );
    let lookup = |vm: &mut Vm<World>, table: &ScriptValue, key: &str| match table {
        ScriptValue::Array(a) => {
            let k = gsc_t6::Key::Str(vm.intern(key));
            a.read().get(&k).cloned().unwrap_or_default()
        }
        _ => ScriptValue::Undefined,
    };
    let (by_menu, by_type) = (vm.raw_field(vm.level, types), vm.raw_field(vm.level, killstreaks));
    let mut out: Vec<(String, i32, bool)> = Vec::new();
    for r in refs {
        let Some(it) = all.iter().find(|i| i.reference == r) else {
            continue;
        };
        let kind = lookup(vm, &by_menu, &r);
        let kind = vm.to_text(&kind);
        let held = match lookup(vm, &by_type, &kind) {
            ScriptValue::Object(o) => {
                let name = vm.to_text(&vm.raw_field(o, weapon));
                super::weapon(game, &name).ok().filter(|w| *w != 0).is_some_and(|w| {
                    crate::client_script::ammo_clip(&super::frame(game), crate::sim_parts::ClientId(peer), w)
                        > 0
                })
            }
            _ => false,
        };
        out.push((it.picture.clone(), it.cost, held));
    }
    out.sort_by_key(|p| std::cmp::Reverse(p.1));
    Some(
        out.iter()
            .map(|(i, c, h)| format!("{i}:{c}:{}", u8::from(*h)))
            .collect::<Vec<_>>()
            .join(";"),
    )
}

/// A player's perks as BO2's perk list draws them (the spawn list, the
/// killcam's killer card): `icon|name` by `;`, perk 1 to 3.
pub(crate) fn perk_card(vm: &mut Vm<World>, game: &mut World, who: &ScriptValue, class: i32) -> String {
    let all = items(game);
    let mut picks: Vec<i32> = Vec::new();
    if let Some(p) = his(vm, game, who, "specialty1") {
        picks = (1..=3).map(|n| p.loadout_item(class, &format!("specialty{n}"))).collect();
    } else {
        for n in 1..=3 {
            let slot = format!("specialty{n}");
            picks.push(slot_item(vm, game, who, class, &slot).map_or(0, |(i, _)| i.index));
        }
    }
    let rows: Vec<String> = picks
        .into_iter()
        .filter_map(|index| all.iter().find(|i| i.index == index && !i.reference.is_empty() && i.reference != "specialty_null"))
        .map(|it| format!("{}|{}", it.picture, it.name))
        .collect();
    rows.join(";")
}

/// The default class a class index names. Past the defaults there is none:
/// BO2's GetLoadoutWeapon gives such an index "weapon_null_mp" and
/// GetLoadoutPerks nothing, which is what ends bot_choose_class's loops
/// (a wrapped index named a real class again and they never ended).
/// A preset class (10-14, CLASS_SMG to CLASS_SNIPER) is the statstable's
/// own preset (class_smg ... class_sniper).
fn default_class(class: i32) -> Option<&'static str> {
    if class >= bo2_profile::PRESET_CLASS_FIRST {
        let n = (class - bo2_profile::PRESET_CLASS_FIRST) as usize;
        return bo2_profile::PRESET_CLASSES.get(n).map(|(name, _)| *name);
    }
    DEFAULT_CLASSES.get(class.max(0) as usize).copied()
}

/// A class's items by slot: (slot, item, extra words or attachments).
fn class_items(vm: &mut Vm<World>, world: &World, who: &ScriptValue, class: i32) -> Vec<(String, Item, Vec<String>)> {
    let all = items(world);
    let n = entnum(vm, who).unwrap_or(u32::MAX);
    let zm = world.resource::<Zm>();
    if let Some(bot) = zm.bots.get(&n)
        && let Some(c) = bot.classes.get(&class)
        && !c.items.is_empty()
    {
        let mut out = Vec::new();
        for name in &c.items {
            let Some(it) = all.iter().find(|i| i.name.eq_ignore_ascii_case(name) || i.reference == *name) else {
                continue;
            };
            let atts: Vec<String> = c
                .attachments
                .iter()
                .filter(|(w, _, _)| w.eq_ignore_ascii_case(&it.name) || *w == it.reference)
                .map(|(_, a, _)| a.clone())
                .collect();
            out.push((it.slot.clone(), it.clone(), atts));
        }
        return out;
    }
    let Some(default) = default_class(class) else {
        return Vec::new();
    };
    all.iter()
        .filter_map(|it| {
            // "all": in every default class (the default scorestreaks).
            let (_, words) = it.classes.iter().find(|(c, _)| c == default || c == "all")?;
            Some((it.slot.clone(), it.clone(), words.clone()))
        })
        .collect()
}

fn slot_item(
    vm: &mut Vm<World>,
    world: &World,
    who: &ScriptValue,
    class: i32,
    slot: &str,
) -> Option<(Item, Vec<String>)> {
    // Scorestreaks are the player's, not a class's: `killstreakN` is the
    // Nth he picked - a bot's own picks (its scripts add them to class 0),
    // else the default ones, in table order.
    if let Some(n) = slot.strip_prefix("killstreak").and_then(|n| n.parse::<usize>().ok()) {
        return class_items(vm, world, who, 0)
            .into_iter()
            .filter(|(s, _, _)| s.starts_with("killstreak"))
            .nth(n.saturating_sub(1))
            .map(|(_, i, w)| (i, w));
    }
    class_items(vm, world, who, class)
        .into_iter()
        .find(|(s, _, _)| s == slot)
        .map(|(_, i, w)| (i, w))
}

/// The local player's own classes (his stats file), for a class slot his
/// classes hold (scorestreaks are not a class's: those stay as below).
fn his<'w>(vm: &mut Vm<World>, world: &'w mut World, who: &ScriptValue, slot: &str) -> Option<&'w bo2_profile::Profile> {
    let n = entnum(vm, who)?;
    super::stats::local_profile(world, n)?;
    // A preset class is read from the match's presets for his team.
    let obj = world.resource::<Zm>().players.get(&n).map(|p| p.obj);
    let team = obj.map_or(0, |o| {
        let f = vm.intern("team");
        let v = vm.raw_field(o, f);
        super::goals::team_number(&vm.to_text(&v)).map_or(0, i32::from)
    });
    super::stats::player(world, n).profile.team = team;
    let p = super::stats::local_profile(world, n)?;
    // Scorestreaks are class 0's, shared by every class; his picks count
    // only once he has made some (else the defaults, as below).
    (!slot.starts_with("killstreak") || p.has_streak_picks()).then_some(p)
}

pub(super) fn bind(vm: &mut Vm<World>) {
    macro_rules! f {
        ($name:literal, $body:expr) => {
            vm.bind($name, false, $body);
        };
    }
    macro_rules! m {
        ($name:literal, $body:expr) => {
            vm.bind($name, true, $body);
        };
    }
    // The gun in a slot: `<reference>_mp`, its class's attachments after it
    // (`+reflex+fmj`: a bot's, or a default class's for a player without his
    // own classes), as BO2 gives it; Dual Wield gives `<reference>_dw_mp`.
    m!("getloadoutweapon", |vm, world, s, a| {
        let class = arg(a, 0).as_int().unwrap_or(0);
        let slot = text(vm, a, 1);
        if let Some(p) = his(vm, world, s, &slot) {
            let w = p.loadout_weapon(class, &slot);
            return Ok(vm.string(&w));
        }
        let Some((it, extra)) = slot_item(vm, world, s, class, &slot) else {
            return Ok(vm.string("weapon_null_mp"));
        };
        let picked: Vec<&str> = extra.iter().filter(|a| it.attachments.contains(a)).map(String::as_str).collect();
        if let Some(name) = bo2_profile::dual_wield_weapon_name(&it.reference, &picked) {
            return Ok(vm.string(&name));
        }
        let mut name = format!("{}_mp", it.reference);
        for att in extra.iter().filter(|a| it.attachments.contains(a)) {
            name.push('+');
            name.push_str(att);
        }
        Ok(vm.string(&name))
    });
    m!("getloadoutitem", |vm, world, s, a| {
        let class = arg(a, 0).as_int().unwrap_or(0);
        let slot = text(vm, a, 1);
        let class = if slot.starts_with("killstreak") { 0 } else { class };
        if let Some(p) = his(vm, world, s, &slot) {
            return Ok(ScriptValue::Int(p.loadout_item(class, &slot)));
        }
        let item = slot_item(vm, world, s, class, &slot);
        // A grenade slot's count rides on its own name ("<slot>count").
        if let Some(base) = slot.strip_suffix("count") {
            let n = slot_item(vm, world, s, class, base).map_or(0, |(_, w)| {
                w.first().and_then(|w| w.parse::<i32>().ok()).unwrap_or(1)
            });
            return Ok(ScriptValue::Int(n));
        }
        Ok(ScriptValue::Int(item.map_or(0, |(i, _)| i.index)))
    });
    m!("getloadoutitemref", |vm, world, s, a| {
        let class = arg(a, 0).as_int().unwrap_or(0);
        let slot = text(vm, a, 1);
        let class = if slot.starts_with("killstreak") { 0 } else { class };
        if let Some(p) = his(vm, world, s, &slot) {
            let r = p.loadout_item_ref(class, &slot);
            return Ok(vm.string(&r));
        }
        Ok(match slot_item(vm, world, s, class, &slot) {
            Some((it, _)) => vm.string(&it.reference),
            None => vm.string(""),
        })
    });
    // The class's perks as the engine gives them (setperk names): every
    // specialty a perk item names (`specialty_movefaster|specialty_fallheight`).
    m!("getloadoutperks", |vm, world, s, a| {
        let class = arg(a, 0).as_int().unwrap_or(0);
        if let Some(p) = his(vm, world, s, "specialty1") {
            let perks = p.loadout_perks(class);
            return Ok(list(perks.iter().map(|n| vm.string(n)).collect()));
        }
        let mut out = Vec::new();
        for (slot, it, _) in class_items(vm, world, s, class) {
            if slot.starts_with("specialty") {
                for p in it.reference.split('|').filter(|p| !p.is_empty()) {
                    out.push(vm.string(p));
                }
            }
        }
        Ok(list(out))
    });
    // BO2's GScr_IsBonusCardActive: the card is BO2's own number (0 Primary
    // Gunfighter ... 7 Two Tacticals: the bonus cards in table order), past
    // 7 or a class past 14 a script error; a bot (a test client) and a
    // class past 10 have none.
    m!("isbonuscardactive", |vm, world, s, a| {
        let card = arg(a, 0).as_int().unwrap_or(-1);
        let class = arg(a, 1).as_int().unwrap_or(0);
        if card > 7 {
            return Err(format!("isbonuscardactive: bonus card {card} is past the last (7)"));
        }
        if class > 14 {
            return Err(format!("isbonuscardactive: class {class} is past the last (14)"));
        }
        let bot = entnum(vm, s).is_some_and(|n| super::is_bot(world, n));
        if bot || class > 10 {
            return Ok(ScriptValue::bool(false));
        }
        if let Some(p) = his(vm, world, s, "bonuscard1") {
            return Ok(ScriptValue::bool(p.bonus_card_active(card, class)));
        }
        let Some(want) = bonus_card_item(&items(world), card) else {
            return Ok(ScriptValue::bool(false));
        };
        Ok(ScriptValue::bool(
            class_items(vm, world, s, class)
                .iter()
                .any(|(slot, it, _)| slot.starts_with("bonuscard") && it.index == want),
        ))
    });
    // A gun's item number from its weapon name (`mp7_mp+reflex` -> MP7's).
    f!("getbaseweaponitemindex", |vm, world, _, a| {
        let w = text(vm, a, 0);
        let base = w.split('+').next().unwrap_or(&w);
        let reference = base.strip_suffix("_mp").unwrap_or(base);
        Ok(ScriptValue::Int(
            items(world)
                .iter()
                .find(|i| i.reference == reference)
                .map_or(0, |i| i.index),
        ))
    });
    f!("getreffromitemindex", |vm, world, _, a| {
        let Some(index) = arg(a, 0).as_int() else {
            return Ok(vm.string(""));
        };
        let r = items(world)
            .iter()
            .find(|i| i.index == index)
            .map(|i| i.reference.clone())
            .unwrap_or_default();
        Ok(vm.string(&r))
    });
    // A gun's attachments by its item name (`WEAPON_MK48`) or reference;
    // with attachments already picked after it (BO2's bots:
    // `getweaponattachments(weapon, a1, a2)`), the ones that can still go
    // with all of them, as mp/attachmentTable.csv lists (no second sight,
    // Dual Wield with nothing: never `kard_dw_tacknife_mp`).
    f!("getweaponattachments", |vm, world, _, a| {
        let want = text(vm, a, 0);
        let picked: Vec<String> = (1..a.len()).map(|i| text(vm, a, i)).filter(|p| !p.is_empty()).collect();
        let atts = items(world)
            .iter()
            .find(|i| i.name.eq_ignore_ascii_case(&want) || i.reference == want)
            .map(|i| i.attachments.clone())
            .unwrap_or_default();
        let v: Vec<String> = {
            let zm = world.resource::<Zm>();
            let table = zm.tables.get(ATTACHMENT_TABLE);
            atts.into_iter().filter(|att| goes_with(table, att, &picked)).collect()
        };
        Ok(list(v.iter().map(|s| vm.string(s)).collect()))
    });
}

const ATTACHMENT_TABLE: &str = "mp/attachmenttable.csv";

/// Whether an attachment can go on a gun beside the ones picked: not one of
/// them, and each lists the other in mp/attachmentTable.csv's column 11 (the
/// attachments it combines with; the Create-a-Class menu's
/// `AreAttachmentsCompatible`). Without the table, only the one sight rule.
/// The statstable spells some a gun's own way (`reflex_pistol`,
/// `extclip_sig`); the table and BO2's weapon names use the part before the
/// `_` (`evoskorpion_mp+steadyaim`).
fn goes_with(table: Option<&super::T6Table>, att: &str, picked: &[String]) -> bool {
    let base = |a: &str| a.split('_').next().unwrap_or(a).to_owned();
    let (att, picked): (String, Vec<String>) = (base(att), picked.iter().map(|p| base(p)).collect());
    if picked.contains(&att) {
        return false;
    }
    let Some(t) = table else {
        let sight = |a: &str| super::T6_SIGHTS.contains(&a);
        return !(sight(&att) && picked.iter().any(|p| sight(p)));
    };
    let combines = |a: &str| -> Option<&str> {
        (0..t.row_total)
            .find(|&r| t.entries.get(r * t.col_total + 4).is_some_and(|c| c == a))
            .and_then(|r| t.entries.get(r * t.col_total + 11))
            .map(String::as_str)
    };
    let lists = |a: &str, b: &str| combines(a).is_some_and(|l| l.split_whitespace().any(|w| w == b));
    picked.iter().all(|p| lists(p, &att) && lists(&att, p))
}

/// BO2's bonus card number (0 Primary Gunfighter ... 7 Two Tacticals) as
/// its item number: the Nth item of the `bonuscard` group, in table order.
fn bonus_card_item(items: &[Item], card: i32) -> Option<i32> {
    let card = usize::try_from(card).ok()?;
    items.iter().filter(|i| i.family == "bonuscard").nth(card).map(|i| i.index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bonus_card_is_the_nth_bonus_card_item() {
        let item = |index, group: &str| Item { index, family: group.to_owned(), ..Item::default() };
        let items = [
            item(40, "specialty"),
            item(170, "bonuscard"),
            item(171, "bonuscard"),
            item(90, "weapon_smg"),
            item(172, "bonuscard"),
        ];
        assert_eq!(bonus_card_item(&items, 0), Some(170));
        assert_eq!(bonus_card_item(&items, 2), Some(172));
        assert_eq!(bonus_card_item(&items, 3), None);
        assert_eq!(bonus_card_item(&items, -1), None);
    }

    fn table(rows: &[(&str, &str)]) -> super::super::T6Table {
        let columns = 12;
        let mut cells: Vec<String> = Vec::with_capacity(rows.len() * columns);
        for (reference, combines) in rows {
            let mut row = vec![String::new(); columns];
            row[4] = (*reference).to_owned();
            row[11] = (*combines).to_owned();
            cells.extend(row);
        }
        super::super::T6Table { col_total: columns, row_total: rows.len(), entries: cells }
    }

    #[test]
    fn a_class_index_past_the_defaults_names_no_class() {
        assert_eq!(default_class(0), Some("class_custom_assault"));
        assert_eq!(default_class(4), Some("class_custom_sniper"));
        assert_eq!(default_class(-1), Some("class_custom_assault"));
        // No wrap: bot_choose_class reads index 5 and up until one names nothing.
        assert_eq!(default_class(5), None);
        assert_eq!(default_class(9), None);
    }

    #[test]
    fn a_bot_picks_only_attachments_that_combine() {
        // BO2's rows: Dual Wield combines with nothing; Tactical Knife with
        // Extended Clip and Reflex, not a second sight's partner list.
        let t = table(&[
            ("dw", ""),
            ("tacknife", "extclip reflex"),
            ("extclip", "tacknife reflex"),
            ("reflex", "tacknife extclip"),
            ("acog", "extclip"),
        ]);
        let picked = |p: &[&str]| p.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert!(goes_with(Some(&t), "tacknife", &[]));
        assert!(!goes_with(Some(&t), "tacknife", &picked(&["dw"])));
        assert!(!goes_with(Some(&t), "dw", &picked(&["extclip"])));
        assert!(goes_with(Some(&t), "reflex", &picked(&["tacknife", "extclip"])));
        // Two sights never: neither lists the other.
        assert!(!goes_with(Some(&t), "acog", &picked(&["reflex"])));
        // One-way listing is not enough (acog lists extclip, extclip not acog).
        assert!(!goes_with(Some(&t), "acog", &picked(&["extclip"])));
        assert!(!goes_with(Some(&t), "extclip", &picked(&["extclip"])));
        // A gun's own spellings count as their base attachment.
        assert!(goes_with(Some(&t), "extclip_sig", &picked(&["reflex_pistol"])));
        assert!(!goes_with(Some(&t), "reflex_pistol", &picked(&["reflex"])));
        assert!(!goes_with(Some(&t), "acog_sniper", &picked(&["reflex_pistol"])));
        // Without the table: only the one-sight rule.
        assert!(goes_with(None, "tacknife", &picked(&["dw"])));
        assert!(!goes_with(None, "acog", &picked(&["reflex"])));
        assert!(!goes_with(None, "acog_sniper", &picked(&["reflex_pistol"])));
    }
}
