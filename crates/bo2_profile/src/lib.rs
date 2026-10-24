//! bo2mp: Black Ops II multiplayer's items, classes and the player's
//! stats, shared by the front end's menus (`hks_t6::mp`) and the server
//! (the loadout a player spawns with). Std only.
//!
//! Items come from his `mp/statstable.csv` (column 0 index, 2 group, 3 name
//! key, 4 reference - a perk's specialties joined by `|` -, 6 image, 7
//! description key, 8 the attachments a weapon takes, 9 order within the
//! group, 10 unlock rank (0 = level 1), 11 the default classes it is in,
//! 12 Pick-10 cost, 13 loadout slot, 16 scorestreak cost) and
//! `mp/attachmenttable.csv` (0 index, 2 kind, 3 name key, 4 reference, 6
//! image, 7 description key, 11 the attachments it goes with, 12 cost).
//!
//! His stats are one store of values by stats path, the names the scripts
//! use, lower case, array indices as numbers: `playerstatslist.rank.
//! statvalue`, `custommatchcacloadouts.customclass.0.primary`. A value never
//! written reads 0. A class slot holds an item index, a weapon's
//! `<slot>attachment<n>` the attachment's number in that weapon's own list
//! (1 first, 0 none), a grenade's `<slot>count` the count.

use std::collections::HashMap;
use std::path::Path;

/// The class slots (the menus' CoD.CACUtility.loadoutSlotNames and the
/// attachment, option and count slots beside them).
pub const SLOTS: [&str; 24] = [
    "primary",
    "primaryattachment1",
    "primaryattachment2",
    "primaryattachment3",
    "primarycamo",
    "primaryreticle",
    "secondary",
    "secondaryattachment1",
    "secondaryattachment2",
    "secondarycamo",
    "secondaryreticle",
    "primarygrenade",
    "primarygrenadecount",
    "specialgrenade",
    "specialgrenadecount",
    "specialty1",
    "specialty2",
    "specialty3",
    "specialty4",
    "specialty5",
    "specialty6",
    "bonuscard1",
    "bonuscard2",
    "bonuscard3",
];

/// BO2's default custom classes, in class order (mp/reset_classes.cfg:
/// `equipdefaultclass 0 class_custom_assault` ... and the same five again
/// for classes 5-9; the offline profile, custom match and league sets take
/// the first five).
pub const DEFAULT_CLASSES: [&str; 5] =
    ["class_custom_assault", "class_custom_smg", "class_custom_lmg", "class_custom_cqb", "class_custom_sniper"];

/// The class sets and how many classes each holds.
pub const CLASS_SETS: [(&str, usize); 4] =
    [("profile.cacloadouts", 5), ("cacloadouts", 10), ("custommatchcacloadouts", 5), ("leaguecacloadouts", 5)];

/// The set a local match uses: BO2's ONLINE -> CUSTOM GAMES private match
/// (online session + private match game mode; see `class_set`).
pub const LOCAL_MATCH_SET: &str = "custommatchcacloadouts";

/// A match's preset classes, the class menu's default classes: the
/// statstable's class name and the class's name key, in BO2's class order
/// (loadoutClass_t: the ten custom classes are 0-9, DEFAULT_CLASS_SMG 10
/// ... DEFAULT_CLASS_SNIPER 14).
pub const PRESET_CLASSES: [(&str, &str); 5] = [
    ("class_smg", "CLASS_SMG"),
    ("class_cqb", "CLASS_CQB"),
    ("class_assault", "CLASS_ASSAULT"),
    ("class_lmg", "CLASS_LMG"),
    ("class_sniper", "CLASS_SNIPER"),
];

/// The class number of the first preset class.
pub const PRESET_CLASS_FIRST: i32 = 10;

/// A class's number from its name, as BO2's
/// BG_UnlockablesGetClassIndexFromName finds it in its fifteen names (any
/// case; -1 if none): CLASS_CUSTOM1-CLASS_CUSTOM10 are 0-9, then the presets
/// (`class_smg` is 10).
pub fn class_index_from_name(name: &str) -> i32 {
    let customs = (1..=10).map(|n| format!("CLASS_CUSTOM{n}"));
    let presets = PRESET_CLASSES.iter().map(|(_, key)| (*key).to_owned());
    customs.chain(presets).position(|c| c.eq_ignore_ascii_case(name)).map_or(-1, |i| i as i32)
}

/// The class set the menus edit and a match gives, as BO2's scripts choose
/// it (CACUtility's `cacRoot`): offline, the exe profile's classes; online,
/// the custom match set in a private match, the league set in league play,
/// else the public one.
pub fn class_set(online: bool, private_match: bool, league: bool) -> &'static str {
    if !online {
        "profile.cacloadouts"
    } else if private_match {
        "custommatchcacloadouts"
    } else if league {
        "leaguecacloadouts"
    } else {
        "cacloadouts"
    }
}

/// BO2's unlock token pool (`unlocks[0]`).
pub const UNLOCK_TOKENS: &str = "unlocks.0";

/// One `mp/statstable.csv` row.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Item {
    pub index: i32,
    pub group: String,
    pub name: String,
    pub reference: String,
    pub image: String,
    pub desc: String,
    /// The attachments a weapon takes, in the order the menus number them
    /// (1 first).
    pub attachments: Vec<String>,
    /// Its place within its group (column 9).
    pub sort: i32,
    /// The rank (0 = level 1) it unlocks at.
    pub unlock_rank: i32,
    /// The default classes it is in, with each one's extra words
    /// (attachments, or a grenade count).
    pub classes: Vec<(String, Vec<String>)>,
    pub cost: i32,
    pub slot: String,
    pub momentum: i32,
    /// Bought with an unlock token once its rank is reached (column 17
    /// set); the rest (the level-4 starter set: MP7, MTAR, Five-seven,
    /// Lightweight, UAV, ...) come free with their rank.
    pub token: bool,
}

/// One `mp/attachmenttable.csv` row.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Attachment {
    pub index: i32,
    pub reference: String,
    pub name: String,
    pub image: String,
    pub desc: String,
    pub cost: i32,
    /// The attachments it goes with on one gun, by reference (column 11:
    /// BO2's `compatibleAttachments`).
    pub goes_with: Vec<String>,
}

/// A stat: a number or text.
#[derive(Clone, Debug, PartialEq)]
pub enum StatValue {
    Num(f32),
    Str(String),
}

impl StatValue {
    pub fn as_num(&self) -> Option<f32> {
        match self {
            StatValue::Num(n) => Some(*n),
            StatValue::Str(s) => s.trim().parse().ok(),
        }
    }
}

fn cell(row: &[String], i: usize) -> String {
    row.get(i).map_or_else(String::new, |c| c.trim().to_owned())
}

fn int(row: &[String], i: usize) -> i32 {
    cell(row, i).parse::<i32>().unwrap_or(0)
}

/// A number as the stats file writes it (whole numbers without a point).
fn number_text(n: f32) -> String {
    if n == n.trunc() && n.abs() < 1e9 { format!("{}", n as i64) } else { format!("{n}") }
}

/// The items, the attachments and his stats.
#[derive(Clone, Debug, Default)]
pub struct Profile {
    /// The items by index (gaps are empty items).
    pub items: Vec<Item>,
    pub attachments: Vec<Attachment>,
    /// His stats by path (see the crate notes).
    pub stats: HashMap<String, StatValue>,
    /// A stat changed since the owner last saved (`take_dirty`).
    pub dirty: bool,
    /// The class set `loadout_*` read (`LOCAL_MATCH_SET` unless set).
    pub match_set: String,
    /// mp/rankTable.csv's ranks (`set_rank_table`).
    pub ranks: Vec<RankRow>,
    /// Console lines a stats command leaves for its owner to run next, in
    /// order (`take_followups`): a Prestige Award's config file and the
    /// stats it writes after it.
    pub followup: Vec<String>,
}

/// One rank of mp/rankTable.csv: its number (0 = level 1), its XP span and
/// what reaching it unlocks: (BO2's unlock index, count); `token` is index
/// 0, the unlock tokens (`UNLOCK_TOKENS`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RankRow {
    pub id: i32,
    pub min_xp: f32,
    pub max_xp: f32,
    pub unlocks: Vec<(usize, i32)>,
}

/// BO2's unlock pools (`unlocks[0..19]`): mp/rankTable.csv names only the
/// first, `token`.
pub const UNLOCK_NAMES: [&str; 1] = ["token"];
pub const UNLOCK_COUNT: usize = 20;

impl Profile {
    /// The items from his string tables (rows of cells), the default
    /// classes filled into every class set (only slots `stats` lacks: read
    /// his file with `load_text` first), the class names from `localize`
    /// (`CLASS_SLOT1` -> "Custom 1").
    pub fn from_tables(
        statstable: &[Vec<String>],
        attachmenttable: &[Vec<String>],
        localize: &dyn Fn(&str) -> Option<String>,
        stats: HashMap<String, StatValue>,
    ) -> Profile {
        let mut stats = stats;
        // bo2mp: gun XP a match wrote before it had one path with the menus
        // (`itemstats.<i>.xp.statvalue`) counts as the gun's (`gun_xp_path`),
        // once: only where the gun has none there yet.
        let old: Vec<(String, StatValue)> = stats
            .iter()
            .filter_map(|(k, v)| {
                let item = k.strip_prefix("itemstats.")?.strip_suffix(".xp.statvalue")?.parse::<i32>().ok()?;
                Some((gun_xp_path(item), v.clone()))
            })
            .collect();
        for (path, v) in old {
            stats.entry(path).or_insert(v);
        }
        let mut p = Profile { stats, match_set: LOCAL_MATCH_SET.to_owned(), ..Profile::default() };
        for row in statstable {
            let Ok(index) = cell(row, 0).parse::<i32>() else { continue };
            if cell(row, 4).is_empty() {
                continue;
            }
            // Column 11: `class_smg steadyaim class_custom_smg` - each
            // class name, then its words.
            let mut classes: Vec<(String, Vec<String>)> = Vec::new();
            for w in cell(row, 11).split_whitespace() {
                if w.starts_with("class_") || w == "all" {
                    classes.push((w.to_owned(), Vec::new()));
                } else if let Some(last) = classes.last_mut() {
                    last.1.push(w.to_owned());
                }
            }
            let at = index.max(0) as usize;
            if p.items.len() <= at {
                p.items.resize(at + 1, Item::default());
            }
            p.items[at] = Item {
                index,
                group: cell(row, 2),
                name: cell(row, 3),
                reference: cell(row, 4),
                image: cell(row, 6),
                desc: cell(row, 7),
                attachments: cell(row, 8).split_whitespace().map(str::to_owned).collect(),
                sort: int(row, 9),
                unlock_rank: int(row, 10),
                classes,
                cost: int(row, 12),
                slot: cell(row, 13),
                momentum: int(row, 16),
                token: !cell(row, 17).is_empty(),
            };
        }
        for row in attachmenttable {
            let Ok(index) = cell(row, 0).parse::<i32>() else { continue };
            p.attachments.push(Attachment {
                index,
                reference: cell(row, 4),
                name: cell(row, 3),
                image: cell(row, 6),
                desc: cell(row, 7),
                cost: int(row, 12),
                goes_with: if cell(row, 2) == "attachment" { cell(row, 11).split_whitespace().map(str::to_owned).collect() } else { Vec::new() },
            });
        }
        p.default_loadouts(localize);
        p
    }

    /// The default classes in every class set, as mp/reset_classes*.cfg
    /// equip them (only slots not saved): the offline profile's 5
    /// (`equipdefaultclasstoprofile`), the public 10 (the five twice), the
    /// custom match and league 5. Names are BO2's "CLASS_SLOT<n>" text
    /// (`setprofilelocclass` / `setStatFromLocString` store the text).
    fn default_loadouts(&mut self, localize: &dyn Fn(&str) -> Option<String>) {
        for (set, count) in CLASS_SETS {
            for n in 0..count {
                for (slot, v) in self.default_class(DEFAULT_CLASSES[n % 5]) {
                    self.stats.entry(format!("{set}.customclass.{n}.{slot}")).or_insert(StatValue::Num(v as f32));
                }
                let loc = format!("CLASS_SLOT{}", n + 1);
                let name = localize(&loc).unwrap_or(loc);
                self.stats.entry(format!("{set}.customclassname.{n}")).or_insert(StatValue::Str(name));
            }
        }
        // A match's preset classes (the class menu's default classes,
        // `GetGametypeSettings().cacLoadouts[team]`), in the menu's order:
        // the statstable's class_smg, class_cqb, class_assault, class_lmg,
        // class_sniper and their names (CLASS_SMG = "Operative", ...).
        for team in 0..3 {
            let set = format!("gametypesettings.cacloadouts.{team}");
            for (n, (class, loc)) in PRESET_CLASSES.iter().enumerate() {
                for (slot, v) in self.default_class(class) {
                    self.stats.entry(format!("{set}.customclass.{n}.{slot}")).or_insert(StatValue::Num(v as f32));
                }
                let name = localize(loc).unwrap_or_else(|| (*loc).to_owned());
                self.stats.entry(format!("{set}.customclassname.{n}")).or_insert(StatValue::Str(name));
            }
        }
        // A new profile's stats were never reset: no "Your online stats
        // have been reset" notice (MainLobby shows it while this is 0), and
        // no "Your custom classes have been reset" (while classWarning is
        // 0). BO2's fresh stats set both flags (LiveStats_ResetStats).
        self.stats.entry("cacloadouts.resetwarningdisplayed".to_owned()).or_insert(StatValue::Num(1.0));
        self.stats.entry("cacloadouts.classwarningdisplayed".to_owned()).or_insert(StatValue::Num(1.0));
    }

    /// A default class's slots: (slot, item index or count). `class` is a
    /// statstable class name (`class_custom_smg`, `class_smg`).
    pub fn default_class(&self, class: &str) -> Vec<(String, i32)> {
        let mut out: Vec<(String, i32)> = Vec::new();
        for it in &self.items {
            let Some((_, words)) = it.classes.iter().find(|(c, _)| c == class) else { continue };
            let slot = it.slot.as_str();
            match slot {
                "primary" | "secondary" | "primarygrenade" | "specialgrenade" => {
                    out.push((slot.to_owned(), it.index));
                    if slot.ends_with("grenade") {
                        let count = words.first().and_then(|w| w.parse::<i32>().ok()).unwrap_or(1);
                        out.push((format!("{slot}count"), count));
                    } else {
                        // Its attachments, numbered as the weapon lists them.
                        for (i, w) in words.iter().enumerate().take(3) {
                            if let Some(a) = it.attachments.iter().position(|x| x == w) {
                                out.push((format!("{slot}attachment{}", i + 1), a as i32 + 1));
                            }
                        }
                    }
                }
                // A perk's slot is its specialty number (specialty1-3).
                s if s.starts_with("specialty") => out.push((s.to_owned(), it.index)),
                // The default scorestreaks ("all" classes carry them, in
                // table order): killstreak1..3 whatever the slot is named.
                s if s.starts_with("killstreak") => {
                    let n = out.iter().filter(|(k, _)| k.starts_with("killstreak")).count() + 1;
                    out.push((format!("killstreak{n}"), it.index));
                }
                // Wildcards: no default class carries one.
                _ => {}
            }
        }
        out
    }

    /// The stats path of a class slot in a set.
    pub fn class_key(set: &str, class: i32, slot: &str) -> String {
        format!("{set}.customclass.{class}.{}", slot.to_ascii_lowercase())
    }

    pub fn stat(&self, path: &str) -> Option<&StatValue> {
        self.stats.get(path)
    }

    /// A stat as a number (0 when never written).
    pub fn stat_num(&self, path: &str) -> f32 {
        self.stat(path).and_then(StatValue::as_num).unwrap_or(0.0)
    }

    pub fn set_stat(&mut self, path: String, v: StatValue) {
        self.stats.insert(path, v);
        self.dirty = true;
    }

    /// Whether a stat changed since the last call.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    /// His rank (0 = level 1).
    pub fn rank(&self) -> i32 {
        self.stat_num(RANK) as i32
    }

    pub fn item(&self, i: i32) -> Option<&Item> {
        self.items.get(usize::try_from(i).ok()?).filter(|it| !it.reference.is_empty())
    }

    pub fn item_by_ref(&self, reference: &str) -> Option<&Item> {
        self.items.iter().find(|it| it.reference == reference)
    }

    /// Locked: above his rank, unless everything is free (BO2's custom
    /// games unlock every item) or a prestige token unlocked it for good.
    pub fn locked(&self, i: i32, all_free: bool) -> bool {
        !all_free && self.item(i).is_some_and(|it| it.unlock_rank > self.rank()) && !self.permanently_unlocked(i)
    }

    /// His unlock tokens (BO2's `unlocks[0]`, the pool its Create-a-Class
    /// spends from).
    pub fn tokens(&self) -> i32 {
        self.stat_num(UNLOCK_TOKENS) as i32
    }

    /// An item's own: free with its rank, or bought with a token
    /// (`itemstats.<i>.purchased`).
    pub fn purchased(&self, i: i32, all_free: bool) -> bool {
        all_free
            || self.item(i).is_some_and(|it| !it.token || self.stat_num(&format!("itemstats.{i}.purchased")) != 0.0)
            || self.permanently_unlocked(i)
    }

    /// Spend `cost` tokens on an item his rank has reached (BO2's
    /// `Engine.PurchaseItem`); false when he can't.
    pub fn purchase(&mut self, i: i32, cost: i32) -> bool {
        if self.locked(i, false) || self.purchased(i, false) || self.tokens() < cost {
            return false;
        }
        let left = self.tokens() - cost;
        self.set_stat(UNLOCK_TOKENS.to_owned(), StatValue::Num(left as f32));
        self.set_stat(format!("itemstats.{i}.purchased"), StatValue::Num(1.0));
        true
    }

    /// A weapon's attachment by its number (0 = none, then the weapon's
    /// list). A list word the table lacks (`reflex_pistol`) is its base
    /// attachment (`reflex`).
    pub fn attachment(&self, item: i32, n: i32) -> Option<&Attachment> {
        let reference = if n <= 0 {
            "none".to_owned()
        } else {
            self.item(item)?.attachments.get(n as usize - 1)?.clone()
        };
        self.attachment_ref(&reference)
    }

    /// An attachment by its reference, or the base attachment a list word
    /// names (`reflex_pistol` is `reflex`).
    pub fn attachment_ref(&self, reference: &str) -> Option<&Attachment> {
        self.attachments
            .iter()
            .find(|a| a.reference == reference)
            .or_else(|| self.attachments.iter().find(|a| reference.starts_with(&format!("{}_", a.reference))))
    }

    /// Whether two attachments go on one gun together (BO2's
    /// BG_AreAttachmentsCompatible): none goes with anything; otherwise
    /// both must be in the table and each list the other, so none goes
    /// with itself.
    pub fn attachments_compatible(&self, a: &str, b: &str) -> bool {
        let (Some(a), Some(b)) = (self.attachment_ref(a), self.attachment_ref(b)) else { return false };
        if a.reference == "none" || b.reference == "none" {
            return true;
        }
        a.goes_with.contains(&b.reference) && b.goes_with.contains(&a.reference)
    }

    /// His stats as text, one `path<TAB>value` per line (text starts with
    /// `s:`), sorted: the local stats file.
    pub fn to_text(&self) -> String {
        let mut lines: Vec<String> = self
            .stats
            .iter()
            .map(|(k, v)| match v {
                StatValue::Num(n) => format!("{k}\t{}", number_text(*n)),
                StatValue::Str(t) => format!("{k}\ts:{t}"),
            })
            .collect();
        lines.sort();
        lines.join("\n") + "\n"
    }

    /// Stats from a file written by `to_text`.
    pub fn stats_from_text(text: &str) -> HashMap<String, StatValue> {
        let mut out = HashMap::new();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('\t') else { continue };
            let value = match v.strip_prefix("s:") {
                Some(t) => StatValue::Str(t.to_owned()),
                None => v.trim().parse::<f32>().map_or_else(|_| StatValue::Str(v.to_owned()), StatValue::Num),
            };
            out.insert(k.to_owned(), value);
        }
        out
    }

    /// His stats file (none yet = no stats).
    pub fn read_stats_file(path: &Path) -> HashMap<String, StatValue> {
        std::fs::read_to_string(path).map(|t| Profile::stats_from_text(&t)).unwrap_or_default()
    }

    /// Write his stats file (a temporary file, then renamed over it).
    pub fn save_file(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, self.to_text())?;
        std::fs::rename(&tmp, path)
    }

    // The server's answers for a player's class (`class` = the class
    // number the scripts pass, 0 the first custom class), in `match_set`.

    /// `getloadoutitem(class, slot)`: the slot's item index (0 = none).
    pub fn loadout_item(&self, class: i32, slot: &str) -> i32 {
        self.stat_num(&Profile::class_key(&self.match_set, class, slot)) as i32
    }

    /// Whether he has picked scorestreaks (Clear All counts: none picked):
    /// they sit in class 0, shared by every class, in `match_set`.
    pub fn has_streak_picks(&self) -> bool {
        (1..=3).any(|n| self.stat(&Profile::class_key(&self.match_set, 0, &format!("killstreak{n}"))).is_some())
    }

    /// `getloadoutitemref(class, slot)`: the slot's item reference ("" for
    /// none).
    pub fn loadout_item_ref(&self, class: i32, slot: &str) -> String {
        self.item(self.loadout_item(class, slot)).map(|it| it.reference.clone()).unwrap_or_default()
    }

    /// `getloadoutweapon(class, "primary" | "secondary")`: the weapon BO2
    /// gives, `<reference>_mp` with its attachments as `+<attachment>`, in
    /// the attachment table's order (alphabetical in his table); no weapon
    /// = `weapon_null_mp`. Dual Wield picked gives the gun's own two-gun
    /// weapon instead (see [`dual_wield_weapon_name`]).
    pub fn loadout_weapon(&self, class: i32, slot: &str) -> String {
        let index = self.loadout_item(class, slot);
        let Some(it) = self.item(index).filter(|it| it.reference != "weapon_null") else {
            return "weapon_null_mp".to_owned();
        };
        let mut atts: Vec<&Attachment> = (1..=3)
            .filter_map(|n| {
                let a = self.loadout_item(class, &format!("{slot}attachment{n}"));
                self.attachment(index, a).filter(|_| a > 0)
            })
            .collect();
        let picked: Vec<&str> = atts.iter().map(|a| a.reference.as_str()).collect();
        if let Some(name) = dual_wield_weapon_name(&it.reference, &picked) {
            return name;
        }
        atts.sort_by(|a, b| a.index.cmp(&b.index).then_with(|| a.reference.cmp(&b.reference)));
        atts.dedup_by(|a, b| a.reference == b.reference);
        let mut name = format!("{}_mp", it.reference);
        for a in atts {
            name.push('+');
            name.push_str(&a.reference);
        }
        name
    }

    /// `getloadoutperks(class)`: the specialties of the class's perks
    /// (specialty1-6), each perk's `specialty_*` names in table order.
    pub fn loadout_perks(&self, class: i32) -> Vec<String> {
        let mut out = Vec::new();
        for n in 1..=6 {
            let i = self.loadout_item(class, &format!("specialty{n}"));
            if let Some(it) = self.item(i).filter(|it| it.group == "specialty") {
                out.extend(it.reference.split('|').map(str::trim).filter(|s| !s.is_empty() && *s != "specialty_null").map(str::to_owned));
            }
        }
        out
    }

    /// `isbonuscardactive(card, class)`: a wildcard slot holds that card.
    /// `card` numbers the statstable's wildcards in index order (0 primary
    /// gunfighter, 1 secondary gunfighter, 2 overkill, 3-5 perk greeds,
    /// 6 danger close, 7 two tacticals), as _class.gsc's 0 / 1 / 2 checks
    /// read them.
    pub fn bonus_card_active(&self, card: i32, class: i32) -> bool {
        let Some(want) = self.items.iter().filter(|it| it.group == "bonuscard").nth(usize::try_from(card).unwrap_or(usize::MAX)) else {
            return false;
        };
        (1..=3).any(|n| self.loadout_item(class, &format!("bonuscard{n}")) == want.index)
    }
}

/// bo2mp: the weapon a class slot gives when Dual Wield is one of its
/// attachments, else `None`. BO2 then joins with `_`, not `+`: the
/// reference, `_dw`, the other attachments in slot order, then `_mp`. So
/// Five-seven + Dual Wield is `fiveseven_dw_mp`, the zone's own two-gun
/// weapon (left hand `fiveseven_lh_mp`), never `fiveseven_mp+dw` (whose
/// attachment names no left gun). Checked in BO2's server program.
pub fn dual_wield_weapon_name(reference: &str, slot_atts: &[&str]) -> Option<String> {
    if !slot_atts.contains(&"dw") {
        return None;
    }
    let mut name = format!("{reference}_dw");
    for a in slot_atts.iter().filter(|a| **a != "dw" && !a.is_empty()) {
        name.push('_');
        name.push_str(a);
    }
    name.push_str("_mp");
    Some(name)
}

/// His rank (0 = level 1), rank XP and prestige: BO2's
/// `playerstatslist.<RANK|RANKXP|PLEVEL>.statvalue` (CoDBase's
/// `UIExpression.GetStatByName(controller, "PLEVEL")`).
/// bo2mp: a gun's XP (BO2's `ItemStats.<item>.xp`, an item field beside
/// `purchased`): what its gun level reads (mp/gunlevels.csv). The match
/// adds to it (its scoreInfo GunXP events); Create-a-Class reads it.
pub fn gun_xp_path(item: i32) -> String {
    format!("itemstats.{item}.xp")
}

/// bo2mp: one gun level of a weapon, a row of mp/gunlevels.csv: its rank
/// (column 0, from 0: weapon level rank + 2), the gun XP that reaches it
/// (1), the weapon's reference (2), the attachment it unlocks (3) and its
/// reward XP (4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GunLevel {
    pub rank: i32,
    pub xp: i32,
    pub attachment: String,
    pub reward_xp: i32,
}

/// bo2mp: a weapon's gun levels (`reference`: `an94`), in the table's
/// order.
pub fn gun_levels(gunlevels: &[Vec<String>], reference: &str) -> Vec<GunLevel> {
    let cell = |r: &[String], i: usize| r.get(i).map_or("", |c| c.trim()).to_owned();
    let int = |r: &[String], i: usize| r.get(i).and_then(|c| c.trim().parse::<f32>().ok()).map_or(0, |x| x as i32);
    gunlevels
        .iter()
        .filter(|r| cell(r, 2) == reference)
        .map(|r| GunLevel { rank: int(r, 0), xp: int(r, 1), attachment: cell(r, 3), reward_xp: int(r, 4) })
        .collect()
}

/// bo2mp: how many of a weapon's gun levels its XP has reached.
pub fn gun_rank(levels: &[GunLevel], xp: f32) -> usize {
    levels.iter().filter(|l| l.xp as f32 <= xp).count()
}

pub const RANK: &str = "playerstatslist.rank.statvalue";
pub const RANK_XP: &str = "playerstatslist.rankxp.statvalue";
pub const PLEVEL: &str = "playerstatslist.plevel.statvalue";
/// How many times his stats were reset (BO2's STATRESETCOUNT).
pub const STAT_RESET_COUNT: &str = "playerstatslist.statresetcount.statvalue";

/// BO2's prestige token kinds (prestigeTokenType_e: RESPEC, RESET_STATS,
/// EXTRA_CAC), as his stats name them:
/// `prestigetokens.<prestige>.tokentype.<kind>`.
pub const PRESTIGE_TOKEN_TYPES: [&str; 3] = ["respec", "resetstats", "extracac"];

/// The public match class sets mp/reset_public_class_sets.cfg resets
/// (`classsets.cacloadouts.<n>`; `hks_t6` swaps them in and out).
pub const PUBLIC_CLASS_SETS: &str = "classsets.cacloadouts";

/// Prestige. BO2's menus decide when he may enter it (CoDBase's
/// PrestigeAvail: his prestige under mp/rankIconTable.csv's `maxprestige`,
/// his rank XP at mp/rankTable.csv's last rank's end) and run
/// mp/prestige_reset.cfg, whose stats commands land here
/// (`run_stat_command`). Each prestige gives one prestige token (the
/// Barracks' `Engine.IsPrestigeTokenSpent(controller)` asks after the
/// current prestige's one), spent on a permanent unlock (Create-a-Class's
/// `Engine.PermanentlyUnlockItem(controller, item)`): kept as
/// `prestigetokens.<prestige>.tokenspent` and `.itemunlocked`.
impl Profile {
    /// His prestige (0 = none).
    pub fn plevel(&self) -> i32 {
        self.stat_num(PLEVEL) as i32
    }

    fn token_key(level: i32, field: &str) -> String {
        format!("prestigetokens.{level}.{field}")
    }

    fn token_unspent(&self, level: i32) -> bool {
        self.stat_num(&Profile::token_key(level, "tokenspent")) == 0.0
    }

    /// Whether his current prestige's token is spent (none before his first
    /// prestige).
    pub fn prestige_token_spent(&self) -> bool {
        let p = self.plevel();
        p <= 0 || !self.token_unspent(p)
    }

    /// His prestige tokens not yet spent (one per prestige reached).
    pub fn prestige_tokens(&self) -> i32 {
        (1..=self.plevel()).filter(|l| self.token_unspent(*l)).count() as i32
    }

    /// An item one of his prestige tokens unlocked for good: unlocked and
    /// his whatever his rank, through every later prestige.
    pub fn permanently_unlocked(&self, i: i32) -> bool {
        (1..=self.plevel()).any(|l| {
            self.stat(&Profile::token_key(l, "itemunlocked")).and_then(StatValue::as_num).is_some_and(|n| n as i32 == i)
        })
    }

    /// Spend a prestige token on an item (BO2's `PermanentlyUnlockItem`):
    /// his current prestige's token first, else the latest one unspent.
    /// False when he has none or the item is his for good already.
    pub fn permanently_unlock(&mut self, i: i32) -> bool {
        if self.item(i).is_none() || self.permanently_unlocked(i) {
            return false;
        }
        let Some(level) = (1..=self.plevel()).rev().find(|l| self.token_unspent(*l)) else {
            return false;
        };
        self.set_stat(Profile::token_key(level, "tokenspent"), StatValue::Num(1.0));
        self.set_stat(Profile::token_key(level, "itemunlocked"), StatValue::Num(i as f32));
        true
    }

    /// A class back to a default class (`equipdefaultclass <n> <class>`):
    /// every slot cleared, then the default class's slots.
    pub fn reset_class(&mut self, set: &str, n: i32, class: &str) {
        let prefix = format!("{set}.customclass.{n}.");
        self.stats.retain(|k, _| !k.starts_with(&prefix));
        for (slot, v) in self.default_class(class) {
            self.stats.insert(format!("{prefix}{slot}"), StatValue::Num(v as f32));
        }
        self.dirty = true;
    }

    /// `PrestigeStatsReset` (mp/prestige_reset.cfg: "Mark all of the items,
    /// attachments and options as unpurchased"): every item bought with an
    /// unlock token is unbought. His unspent unlock tokens, weapon levels
    /// and permanent unlocks stay (the file touches none of them).
    pub fn prestige_stats_reset(&mut self) {
        self.stats.retain(|k, _| !(k.starts_with("itemstats.") && k.ends_with(".purchased")));
        self.dirty = true;
    }

    /// `prestigerequest`: the next prestige, up to `max_prestige`
    /// (mp/rankIconTable.csv `maxprestige`). His rank XP is not checked
    /// here: mp/prestige_reset.cfg zeroes it on the line before, and the
    /// menus offer Prestige only at the last rank's end.
    pub fn prestige_request(&mut self, max_prestige: i32) -> bool {
        let p = self.plevel();
        if p >= max_prestige {
            return false;
        }
        self.set_stat(PLEVEL.to_owned(), StatValue::Num((p + 1) as f32));
        true
    }

    /// mp/rankTable.csv's ranks: rows whose first cell is a rank number
    /// (column 2 its first XP, 7 the XP past it, 17 its unlocks as name and
    /// count pairs: `token 4`, `token` = one).
    pub fn set_rank_table(&mut self, ranktable: &[Vec<String>]) {
        self.ranks = ranktable
            .iter()
            .filter_map(|r| {
                let id = cell(r, 0).parse::<i32>().ok()?;
                let min_xp = cell(r, 2).parse::<f32>().ok()?;
                let max_xp = cell(r, 7).parse::<f32>().ok()?;
                let words = cell(r, 17);
                let words: Vec<&str> = words.split_whitespace().collect();
                let mut unlocks = Vec::new();
                for (i, w) in words.iter().enumerate() {
                    let Some(k) = UNLOCK_NAMES.iter().position(|n| w.eq_ignore_ascii_case(n)) else { continue };
                    let n = words.get(i + 1).and_then(|x| x.parse::<i32>().ok()).filter(|n| *n != 0).unwrap_or(1);
                    unlocks.push((k, n));
                }
                Some(RankRow { id, min_xp, max_xp, unlocks })
            })
            .collect();
    }

    /// The rank an XP total is (BO2's CL_GetRankForXp): the rank whose span
    /// holds it; past the last, the last. His rank stat without a table.
    pub fn rank_for_xp(&self, xp: f32) -> i32 {
        let Some(last) = self.ranks.last() else { return self.rank() };
        self.ranks.iter().find(|r| r.min_xp <= xp && xp < r.max_xp).map_or(if xp < 0.0 { 0 } else { last.id }, |r| r.id)
    }

    /// Spend his current prestige's token on a Prestige Award (BO2's
    /// SpendPrestigeToken): only past his first prestige, only while that
    /// prestige's token is unspent.
    pub fn spend_prestige_token(&mut self, kind: &str) -> bool {
        let p = self.plevel();
        if p <= 0 || !self.token_unspent(p) {
            return false;
        }
        self.set_stat(Profile::token_key(p, "tokenspent"), StatValue::Num(1.0));
        self.set_stat(Profile::token_key(p, &format!("tokentype.{kind}")), StatValue::Num(1.0));
        true
    }

    /// The award a prestige's token went on (BO2's GetPrestigeTokenType);
    /// None while unspent or spent on a permanent unlock.
    pub fn prestige_token_type(&self, level: i32) -> Option<&'static str> {
        if self.token_unspent(level) {
            return None;
        }
        PRESTIGE_TOKEN_TYPES
            .iter()
            .copied()
            .find(|k| self.stat_num(&Profile::token_key(level, &format!("tokentype.{k}"))) != 0.0)
    }

    /// His custom classes (BO2's GetCustomClassCountFromBuffer): five, one
    /// more for each prestige whose token went on an extra class, ten at
    /// most.
    pub fn custom_class_count(&self) -> i32 {
        let p = self.plevel();
        let extra = (1..=p).filter(|l| self.prestige_token_type(*l) == Some("extracac")).count() as i32;
        if p > 0 { (5 + extra).min(10) } else { 5 }
    }

    /// The custom classes the menus show in a class set: his count, no more
    /// than the set holds (BO2's reset files fill ten in the public set,
    /// five in the others).
    pub fn custom_class_count_in(&self, set: &str) -> i32 {
        let size = CLASS_SETS.iter().find(|(s, _)| *s == set).map_or(5, |(_, n)| *n as i32);
        self.custom_class_count().min(size)
    }

    /// `PrestigeRespec`, the Prestige Award "Refund Unlock Tokens" (BO2's
    /// PrestigeRespecCmd): spends his current prestige's token, unbuys
    /// every item bought with an unlock token, and gives him the unlocks
    /// his rank has earned (mp/rankTable.csv's, ranks 1 to the rank of his
    /// rank XP), none spent. Then his classes go back to the defaults
    /// (mp/reset_classes.cfg) and the class warning is cleared
    /// (`followup`). His permanent unlocks stay.
    pub fn prestige_respec(&mut self) -> bool {
        if !self.spend_prestige_token("respec") {
            return false;
        }
        self.prestige_stats_reset();
        let rank = self.rank_for_xp(self.stat_num(RANK_XP));
        let mut unlocks = [0i32; UNLOCK_COUNT];
        for row in self.ranks.iter().filter(|r| r.id >= 1 && r.id <= rank) {
            for &(k, n) in &row.unlocks {
                unlocks[k] += n;
            }
        }
        self.stats.retain(|k, _| !k.starts_with("unlocks."));
        for (k, n) in unlocks.iter().enumerate() {
            if k == 0 || *n != 0 {
                self.set_stat(format!("unlocks.{k}"), StatValue::Num(*n as f32));
            }
        }
        self.followup = vec!["exec mp/reset_classes.cfg".into(), "statwriteddl cacloadouts classWarningDisplayed 1".into()];
        true
    }

    /// `PrestigeStatsResetAll`, the Prestige Award "Fresh Start" (BO2's
    /// PrestigeStatsResetAllCmd): spends his current prestige's token, then
    /// wipes his stats, prestige included, back to a new profile's (BO2
    /// zeroes the whole stats buffer and runs mp/stats_init.cfg,
    /// `followup`); both reset notices are marked shown and his reset
    /// count goes up one. His options and match settings (`profile.*`,
    /// `gametypesettings.*`, not stats in BO2) stay.
    pub fn prestige_reset_all(&mut self, localize: &dyn Fn(&str) -> Option<String>) -> bool {
        let count = self.stat_num(STAT_RESET_COUNT);
        if !self.spend_prestige_token("resetstats") {
            return false;
        }
        self.stats.retain(|k, _| k.starts_with("profile.") || k.starts_with("gametypesettings."));
        self.default_loadouts(localize);
        self.dirty = true;
        self.followup = vec![
            "exec mp/stats_init.cfg".into(),
            "statwriteddl cacloadouts resetWarningDisplayed 1".into(),
            "statwriteddl cacloadouts classWarningDisplayed 1".into(),
            format!("statwriteddl playerstatslist statresetcount statvalue {}", number_text(count + 1.0)),
        ];
        true
    }

    /// The lines the last command left to run next (`followup`).
    pub fn take_followups(&mut self) -> Vec<String> {
        std::mem::take(&mut self.followup)
    }

    /// A public match class set's classes and their names (`.customclass*`).
    fn public_set_classes(n: i32) -> String {
        format!("{PUBLIC_CLASS_SETS}.{n}.customclass")
    }

    /// Carry out one of BO2's stats commands (`hks_t6::host::STAT_COMMANDS`,
    /// as mp/prestige_reset.cfg, mp/stats_init.cfg, mp/reset_classes*.cfg,
    /// mp/reset_public_class_sets.cfg, mp/playerstats_reset.cfg and the
    /// Prestige menus write them): `equipdefaultclass <n> <class>` (the
    /// public set), `equipdefaultclasstoprofile <n> <class>` (the offline
    /// profile's), `equipdefaultcustommatchclass <n> <class>`,
    /// `setStatFromLocString <set> customclassname <n> <key>`,
    /// `setprofilelocclass <n> <key>`, `statwriteddl <path words> <number>`,
    /// `statsetbyname <NAME> <number>` (playerstatslist), the public class
    /// sets' `setPublicMatchClassSetNameFromLocString <n> <key>`,
    /// `copyToPublicMatchClassSet <n>` and `zeroPublicMatchClassSet <n>`,
    /// `PrestigeStatsReset`, `prestigerequest`, and the Prestige Awards'
    /// `PrestigeRespec`, `PrestigeStatsResetAll`, `PrestigeAddCAC` (which
    /// may leave lines in `followup`). False for one not carried out (a
    /// value BO2 computes, `( dvarint( ... ) )`; an award with no token).
    pub fn run_stat_command(&mut self, line: &str, max_prestige: i32, localize: &dyn Fn(&str) -> Option<String>) -> bool {
        let words: Vec<&str> = line.split_whitespace().collect();
        let Some(verb) = words.first().map(|w| w.to_ascii_lowercase()) else { return false };
        let n = |i: usize| words.get(i).and_then(|w| w.parse::<i32>().ok());
        let text = |key: &str| localize(&key.to_ascii_uppercase()).unwrap_or_else(|| key.to_owned());
        match verb.as_str() {
            "equipdefaultclass" | "equipdefaultclasstoprofile" | "equipdefaultcustommatchclass" => {
                let (Some(class), Some(name)) = (n(1), words.get(2)) else { return false };
                let set = match verb.as_str() {
                    "equipdefaultclass" => "cacloadouts",
                    "equipdefaultclasstoprofile" => "profile.cacloadouts",
                    _ => "custommatchcacloadouts",
                };
                self.reset_class(set, class, &name.to_ascii_lowercase());
                true
            }
            "setstatfromlocstring" => {
                let (Some(set), Some(field), Some(i), Some(key)) = (words.get(1), words.get(2), n(3), words.get(4)) else {
                    return false;
                };
                let path = format!("{}.{}.{i}", set.to_ascii_lowercase(), field.to_ascii_lowercase());
                self.set_stat(path, StatValue::Str(text(key)));
                true
            }
            "setprofilelocclass" => {
                let (Some(i), Some(key)) = (n(1), words.get(2)) else { return false };
                self.set_stat(format!("profile.cacloadouts.customclassname.{i}"), StatValue::Str(text(key)));
                true
            }
            "statwriteddl" => {
                let Some((value, path)) = words[1..].split_last() else { return false };
                let Ok(value) = value.parse::<f32>() else { return false };
                if path.is_empty() || path.iter().any(|w| w.starts_with('(')) {
                    return false;
                }
                let path = path.join(".").to_ascii_lowercase();
                // The public class set he is on: where `hks_t6` keeps it.
                let path = if path == "publicmatchclasssetindex" { format!("{PUBLIC_CLASS_SETS}.current") } else { path };
                self.set_stat(path, StatValue::Num(value));
                true
            }
            "statsetbyname" => {
                let (Some(name), Some(value)) = (words.get(1), words.get(2).and_then(|w| w.parse::<f32>().ok())) else {
                    return false;
                };
                self.set_stat(format!("playerstatslist.{}.statvalue", name.to_ascii_lowercase()), StatValue::Num(value));
                true
            }
            "setpublicmatchclasssetnamefromlocstring" => {
                let (Some(i), Some(key)) = (n(1), words.get(2)) else { return false };
                self.set_stat(format!("{PUBLIC_CLASS_SETS}.{i}.name"), StatValue::Str(text(key)));
                true
            }
            // A public class set becomes a copy of the public classes, or
            // empty classes named "Custom 1" to "Custom 10"
            // (BG_UnlockablesCopyToClassSet / BG_UnlockablesZeroClassSet).
            "copytopublicmatchclassset" | "zeropublicmatchclassset" => {
                let Some(i) = n(1) else { return false };
                let to = Profile::public_set_classes(i);
                self.stats.retain(|k, _| !k.starts_with(&to));
                if verb == "copytopublicmatchclassset" {
                    let from: Vec<(String, StatValue)> = self
                        .stats
                        .iter()
                        .filter_map(|(k, v)| Some((format!("{to}{}", k.strip_prefix("cacloadouts.customclass")?), v.clone())))
                        .collect();
                    self.stats.extend(from);
                } else {
                    for c in 0..10 {
                        let name = text(&format!("CLASS_SLOT{}", c + 1));
                        self.stats.insert(format!("{to}name.{c}"), StatValue::Str(name));
                    }
                }
                self.dirty = true;
                true
            }
            "prestigestatsreset" => {
                self.prestige_stats_reset();
                true
            }
            "prestigerequest" => self.prestige_request(max_prestige),
            "prestigerespec" => self.prestige_respec(),
            "prestigestatsresetall" => self.prestige_reset_all(localize),
            "prestigeaddcac" => self.spend_prestige_token("extracac"),
            _ => false,
        }
    }
}

/// A write for the stats saver.
pub enum SaveJob {
    /// The whole file: his profile's stats (the front end's).
    Whole(HashMap<String, StatValue>),
    /// Changed stats, merged over what the file holds then (a match's).
    Merge(Vec<(String, StatValue)>),
}

/// What the saver says when a write is done.
pub type SaveDone = Box<dyn FnOnce(&Path, &std::io::Result<()>) + Send>;

struct Saver {
    tx: std::sync::Mutex<std::sync::mpsc::Sender<(std::path::PathBuf, SaveJob, Option<SaveDone>)>>,
    pending: std::sync::Arc<(std::sync::Mutex<usize>, std::sync::Condvar)>,
}

fn saver() -> Option<&'static Saver> {
    static SAVER: std::sync::OnceLock<Option<Saver>> = std::sync::OnceLock::new();
    SAVER
        .get_or_init(|| {
            let (tx, rx) = std::sync::mpsc::channel::<(std::path::PathBuf, SaveJob, Option<SaveDone>)>();
            let pending = std::sync::Arc::new((std::sync::Mutex::new(0usize), std::sync::Condvar::new()));
            let left = pending.clone();
            std::thread::Builder::new()
                .name("bo2mp stats saver".into())
                .spawn(move || {
                    while let Ok(first) = rx.recv() {
                        let mut jobs = vec![first];
                        jobs.extend(rx.try_iter());
                        let count = jobs.len();
                        // One write per file, its jobs applied in the order sent.
                        let mut files: Vec<(std::path::PathBuf, Option<HashMap<String, StatValue>>, Vec<SaveDone>)> =
                            Vec::new();
                        for (path, job, done) in jobs {
                            let at = match files.iter().position(|(f, _, _)| *f == path) {
                                Some(at) => at,
                                None => {
                                    files.push((path, None, Vec::new()));
                                    files.len() - 1
                                }
                            };
                            let (path, stats, dones) = &mut files[at];
                            match job {
                                SaveJob::Whole(all) => *stats = Some(all),
                                SaveJob::Merge(changed) => {
                                    stats.get_or_insert_with(|| Profile::read_stats_file(path)).extend(changed)
                                }
                            }
                            dones.extend(done);
                        }
                        for (path, stats, dones) in files {
                            let result = Profile { stats: stats.unwrap_or_default(), ..Profile::default() }.save_file(&path);
                            for done in dones {
                                done(&path, &result);
                            }
                        }
                        let (n, cv) = &*left;
                        if let Ok(mut n) = n.lock() {
                            *n = n.saturating_sub(count);
                        }
                        cv.notify_all();
                    }
                })
                .ok()
                .map(|_| Saver { tx: std::sync::Mutex::new(tx), pending })
        })
        .as_ref()
}

/// Write his stats file on the saver's own thread, so a slow or busy disk
/// never holds up a frame (BO2 never waits on its stats writes either).
/// Writes are kept in the order sent; those queued while one is written
/// become one write per file. Without a saver it writes here.
pub fn save_in_background(path: &Path, job: SaveJob, done: Option<SaveDone>) {
    let Some(s) = saver() else { return write_now(path, job, done) };
    if let Ok(mut n) = s.pending.0.lock() {
        *n += 1;
    }
    let unsent = match s.tx.lock() {
        Ok(tx) => tx.send((path.to_path_buf(), job, done)).err().map(|e| e.0),
        Err(_) => Some((path.to_path_buf(), job, done)),
    };
    if let Some((path, job, done)) = unsent {
        if let Ok(mut n) = s.pending.0.lock() {
            *n = n.saturating_sub(1);
        }
        write_now(&path, job, done);
    }
}

fn write_now(path: &Path, job: SaveJob, done: Option<SaveDone>) {
    let stats = match job {
        SaveJob::Whole(all) => all,
        SaveJob::Merge(changed) => {
            let mut s = Profile::read_stats_file(path);
            s.extend(changed);
            s
        }
    };
    let result = Profile { stats, ..Profile::default() }.save_file(path);
    if let Some(done) = done {
        done(path, &result);
    }
}

/// Wait (at most `limit`) until the saver has written what it was given:
/// before his stats file is read, so a read never misses a write.
pub fn wait_saved(limit: std::time::Duration) {
    let Some(s) = saver() else { return };
    let (n, cv) = &*s.pending;
    if let Ok(n) = n.lock() {
        let _ = cv.wait_timeout_while(n, limit, |n| *n > 0);
    }
}

/// His stats file's text once the saver is done with it.
pub fn read_stats_text(path: &Path) -> std::io::Result<String> {
    wait_saved(std::time::Duration::from_secs(10));
    std::fs::read_to_string(path)
}

#[cfg(test)]
mod gun_level_tests {
    use super::*;

    fn rows(r: &[&[&str]]) -> Vec<Vec<String>> {
        r.iter().map(|r| r.iter().map(|c| (*c).to_owned()).collect()).collect()
    }

    #[test]
    fn a_guns_levels_and_its_rank_from_its_xp() {
        // (Made-up rows: the real ones are BO2's mp/gunlevels.csv.)
        let t = rows(&[&["0", "100", "an94", "reflex", "50"], &["1", "300", "an94", "fmj", "60"], &["0", "100", "mp7", "holo", "50"]]);
        let l = gun_levels(&t, "an94");
        assert_eq!(l.len(), 2);
        assert_eq!(l[1], GunLevel { rank: 1, xp: 300, attachment: "fmj".into(), reward_xp: 60 });
        assert_eq!(gun_rank(&l, 0.0), 0);
        assert_eq!(gun_rank(&l, 100.0), 1);
        assert_eq!(gun_rank(&l, 299.0), 1);
        assert_eq!(gun_rank(&l, 300.0), 2);
        assert_eq!(gun_xp_path(12), "itemstats.12.xp");
    }

    #[test]
    fn gun_xp_saved_under_the_old_path_carries_over_once() {
        let stats = HashMap::from([
            ("itemstats.12.xp.statvalue".to_owned(), StatValue::Num(400.0)),
            ("itemstats.13.xp.statvalue".to_owned(), StatValue::Num(50.0)),
            ("itemstats.13.xp".to_owned(), StatValue::Num(900.0)),
        ]);
        let p = Profile::from_tables(&[], &[], &|_| None, stats);
        assert_eq!(p.stat_num("itemstats.12.xp"), 400.0);
        assert_eq!(p.stat_num("itemstats.13.xp"), 900.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(cells: &[(usize, &str)]) -> Vec<String> {
        let mut r = vec![String::new(); 18];
        for (i, c) in cells {
            r[*i] = (*c).to_owned();
        }
        r
    }

    fn sample() -> Profile {
        let stats = vec![
            row(&[(0, "31"), (2, "weapon_assault"), (4, "hk416"), (8, "reflex acog dualclip"), (11, "class_custom_assault"), (12, "1"), (13, "primary")]),
            row(&[(0, "53"), (2, "weapon_launcher"), (4, "smaw"), (11, "class_custom_assault"), (12, "1"), (13, "secondary")]),
            row(&[(0, "71"), (2, "weapon_grenade"), (4, "concussion_grenade"), (11, "class_custom_assault 2"), (12, "1"), (13, "specialgrenade")]),
            row(&[(0, "153"), (2, "specialty"), (4, "specialty_immunecounteruav|specialty_immuneemp"), (11, "class_custom_assault"), (12, "1"), (13, "specialty2")]),
            row(&[(0, "178"), (2, "bonuscard"), (4, "bonuscard_primary_gunfighter"), (13, "bonuscard1")]),
            row(&[(0, "179"), (2, "bonuscard"), (4, "bonuscard_secondary_gunfighter"), (13, "bonuscard2")]),
            row(&[(0, "180"), (2, "bonuscard"), (4, "bonuscard_overkill"), (13, "bonuscard3")]),
        ];
        let atts = vec![
            row(&[(0, "0"), (4, "none")]),
            row(&[(0, "1"), (4, "acog")]),
            row(&[(0, "2"), (4, "dualclip")]),
            row(&[(0, "21"), (4, "reflex")]),
        ];
        Profile::from_tables(&stats, &atts, &|k| (k == "CLASS_SLOT1").then(|| "Custom 1".to_owned()), HashMap::new())
    }

    #[test]
    fn default_classes_fill_every_set() {
        let p = sample();
        assert_eq!(p.loadout_item(0, "primary"), 31);
        assert_eq!(p.loadout_item(0, "specialgrenadecount"), 2);
        assert_eq!(p.stat("profile.cacloadouts.customclassname.0"), Some(&StatValue::Str("Custom 1".into())));
        assert_eq!(p.loadout_weapon(0, "primary"), "hk416_mp");
        assert_eq!(p.loadout_perks(0), vec!["specialty_immunecounteruav", "specialty_immuneemp"]);
    }

    #[test]
    fn classes_are_numbered_as_bo2_numbers_them() {
        assert_eq!(class_index_from_name("CLASS_CUSTOM1"), 0);
        assert_eq!(class_index_from_name("class_custom10"), 9);
        // The class menu passes the statstable's names.
        assert_eq!(class_index_from_name("class_smg"), 10);
        assert_eq!(class_index_from_name("CLASS_SNIPER"), 14);
        assert_eq!(class_index_from_name("custom0"), -1);
        assert_eq!(class_index_from_name("class_custom_smg"), -1);
        assert_eq!(class_index_from_name(""), -1);
    }

    #[test]
    fn weapon_names_take_attachments_in_table_order() {
        let mut p = sample();
        // reflex (list 1), acog (list 2): acog first in the table.
        p.set_stat(Profile::class_key(LOCAL_MATCH_SET, 0, "primaryattachment1"), StatValue::Num(1.0));
        p.set_stat(Profile::class_key(LOCAL_MATCH_SET, 0, "primaryattachment2"), StatValue::Num(2.0));
        assert_eq!(p.loadout_weapon(0, "primary"), "hk416_mp+acog+reflex");
        assert_eq!(p.loadout_item_ref(0, "secondary"), "smaw");
    }

    #[test]
    fn dual_wield_gives_the_two_gun_weapon() {
        assert_eq!(dual_wield_weapon_name("fiveseven", &["dw"]).as_deref(), Some("fiveseven_dw_mp"));
        assert_eq!(dual_wield_weapon_name("kard", &["extclip", "dw"]).as_deref(), Some("kard_dw_extclip_mp"));
        assert_eq!(dual_wield_weapon_name("fiveseven", &["extclip"]), None);
    }

    #[test]
    fn items_unlock_at_their_rank_and_private_matches_free_them() {
        let stats = vec![
            row(&[(0, "10"), (2, "weapon_assault"), (4, "mtar"), (10, "0")]),
            row(&[(0, "11"), (2, "weapon_assault"), (4, "type25"), (10, "4")]),
            row(&[(0, "12"), (2, "weapon_assault"), (4, "xpr"), (10, "30")]),
        ];
        let mut p = Profile::from_tables(&stats, &[], &|_| None, HashMap::new());
        // A new profile (rank 0, shown as level 1): only rank-0 items.
        assert!(!p.locked(10, false) && p.locked(11, false) && p.locked(12, false));
        // Private matches free everything.
        assert!(!p.locked(12, true));
        // Rank 4 reached (rankxp is read through the rank table; set plevel/xp
        // directly would need that table, so only the free path is asserted).
        p.set_stat("playerstatslist.rankxp.statvalue".to_owned(), StatValue::Num(0.0));
        assert!(p.locked(11, false));
    }

    /// mp/prestige_reset.cfg's stats lines, as BO2 ships them (with
    /// mp/reset_classes.cfg's first class and its name).
    #[test]
    fn prestige_reset_cfg_resets_rank_purchases_and_classes() {
        let stats = vec![
            row(&[(0, "31"), (2, "weapon_assault"), (4, "hk416"), (8, "reflex acog"), (10, "0"), (11, "class_custom_assault"), (13, "primary")]),
            row(&[(0, "12"), (2, "weapon_assault"), (4, "xpr"), (10, "30"), (13, "primary"), (17, "token")]),
        ];
        let loc = |k: &str| (k == "CLASS_SLOT1").then(|| "Custom 1".to_owned());
        let mut p = Profile::from_tables(&stats, &[], &loc, HashMap::new());
        for (k, v) in [(RANK, 54.0), (RANK_XP, 1249100.0), (UNLOCK_TOKENS, 3.0), ("itemstats.12.purchased", 1.0)] {
            p.set_stat(k.to_owned(), StatValue::Num(v));
        }
        p.set_stat(Profile::class_key("cacloadouts", 0, "primary"), StatValue::Num(12.0));
        p.set_stat(Profile::class_key("cacloadouts", 0, "primaryattachment2"), StatValue::Num(2.0));
        p.set_stat("cacloadouts.customclassname.0".to_owned(), StatValue::Str("Mine".into()));
        assert!(p.purchased(12, false) && !p.locked(12, false));
        for line in [
            "equipdefaultclass 0 class_custom_assault",
            "setStatFromLocString cacloadouts customclassname 0 CLASS_SLOT1",
            "PrestigeStatsReset",
            "statwriteddl playerstatslist rankxp statvalue 0",
            "statwriteddl playerstatslist rank statvalue 0",
            "prestigerequest",
        ] {
            assert!(p.run_stat_command(line, 11, &loc), "{line}");
        }
        assert!(!p.run_stat_command("statwriteddl cacloadouts loadoutVersion ( dvarint( classVersionNumber ) )", 11, &loc));
        assert_eq!((p.rank(), p.stat_num(RANK_XP), p.plevel()), (0, 0.0, 1));
        assert!(p.locked(12, false) && !p.purchased(12, false));
        assert_eq!(p.tokens(), 3, "unspent unlock tokens stay");
        assert_eq!(p.stat_num(&Profile::class_key("cacloadouts", 0, "primary")), 31.0);
        assert_eq!(p.stat(&Profile::class_key("cacloadouts", 0, "primaryattachment2")), None);
        assert_eq!(p.stat("cacloadouts.customclassname.0"), Some(&StatValue::Str("Custom 1".into())));
        // One prestige token, spent on the XPR: unlocked and his for good.
        assert_eq!((p.prestige_tokens(), p.prestige_token_spent()), (1, false));
        assert!(p.permanently_unlock(12));
        assert!(!p.locked(12, false) && p.purchased(12, false));
        assert_eq!((p.prestige_tokens(), p.prestige_token_spent()), (0, true));
        assert!(!p.permanently_unlock(31));
        // The next prestige keeps it; Prestige Master is the last.
        p.set_stat(RANK.to_owned(), StatValue::Num(54.0));
        assert!(p.run_stat_command("PrestigeStatsReset", 11, &loc) && p.run_stat_command("prestigerequest", 11, &loc));
        assert!(p.permanently_unlocked(12) && p.prestige_tokens() == 1);
        p.set_stat(PLEVEL.to_owned(), StatValue::Num(11.0));
        assert!(!p.run_stat_command("prestigerequest", 11, &loc));
    }

    /// mp/rankTable.csv's first rows as BO2 ships them (the header rows,
    /// ranks 0-2 with no unlocks, rank 3 "token 4", then "token").
    fn rank_rows() -> Vec<Vec<String>> {
        vec![
            row(&[(0, "a0"), (2, "c2"), (7, "h7"), (17, "p17")]),
            row(&[(0, "maxrank"), (1, "54")]),
            row(&[(0, "0"), (2, "0"), (7, "800")]),
            row(&[(0, "1"), (2, "800"), (7, "1900")]),
            row(&[(0, "2"), (2, "1900"), (7, "3300")]),
            row(&[(0, "3"), (2, "3300"), (7, "5300"), (17, "token 4")]),
            row(&[(0, "4"), (2, "5300"), (7, "7900"), (17, "token")]),
            row(&[(0, "5"), (2, "7900"), (7, "10900"), (17, "token")]),
        ]
    }

    /// A profile at prestige 1 with the XPR bought, rank 4 by his XP.
    fn prestiged() -> Profile {
        let stats = vec![
            row(&[(0, "31"), (2, "weapon_assault"), (4, "hk416"), (10, "0"), (11, "class_custom_assault"), (13, "primary")]),
            row(&[(0, "12"), (2, "weapon_assault"), (4, "xpr"), (10, "3"), (13, "primary"), (17, "token")]),
        ];
        let mut p = Profile::from_tables(&stats, &[], &|_| None, HashMap::new());
        p.set_rank_table(&rank_rows());
        for (k, v) in [(PLEVEL, 1.0), (RANK, 4.0), (RANK_XP, 6000.0), (UNLOCK_TOKENS, 1.0), ("itemstats.12.purchased", 1.0)] {
            p.set_stat(k.to_owned(), StatValue::Num(v));
        }
        p.set_stat("unlocks.3".to_owned(), StatValue::Num(2.0));
        p
    }

    #[test]
    fn rank_table_ranks_and_unlocks() {
        let p = prestiged();
        assert_eq!(p.ranks.len(), 6, "the header rows are not ranks");
        assert_eq!(p.ranks[3].unlocks, vec![(0, 4)]);
        assert_eq!(p.ranks[4].unlocks, vec![(0, 1)]);
        assert_eq!((p.rank_for_xp(0.0), p.rank_for_xp(799.0), p.rank_for_xp(800.0), p.rank_for_xp(6000.0)), (0, 0, 1, 4));
        assert_eq!(p.rank_for_xp(1e9), 5, "past the last rank: the last");
    }

    /// Refund Unlock Tokens: his token spent, every bought item unbought,
    /// the tokens his rank earned back (ranks 1-4: 4 + 1), the other
    /// unlock pools cleared, then mp/reset_classes.cfg and the class
    /// warning; once a prestige.
    #[test]
    fn prestige_respec_refunds_his_tokens() {
        let mut p = prestiged();
        assert!(p.purchased(12, false));
        assert!(p.run_stat_command("PrestigeRespec", 11, &|_| None));
        assert_eq!(p.tokens(), 5);
        assert_eq!(p.stat("unlocks.3"), None);
        assert!(!p.purchased(12, false));
        assert_eq!((p.prestige_token_spent(), p.prestige_token_type(1)), (true, Some("respec")));
        assert_eq!(p.take_followups(), vec!["exec mp/reset_classes.cfg", "statwriteddl cacloadouts classWarningDisplayed 1"]);
        assert!(!p.run_stat_command("PrestigeRespec", 11, &|_| None), "one award a prestige");
        assert!(p.take_followups().is_empty());
        // A permanent unlock stays through a refund.
        let mut p = prestiged();
        p.set_stat(PLEVEL.to_owned(), StatValue::Num(2.0));
        p.set_stat("prestigetokens.1.tokenspent".to_owned(), StatValue::Num(1.0));
        p.set_stat("prestigetokens.1.itemunlocked".to_owned(), StatValue::Num(12.0));
        assert!(p.run_stat_command("prestigerespec", 11, &|_| None));
        assert!(p.purchased(12, false) && p.prestige_token_type(2) == Some("respec"));
        assert_eq!(p.prestige_token_type(1), None, "spent on an item, not an award");
        // No prestige, no award.
        let mut p = prestiged();
        p.set_stat(PLEVEL.to_owned(), StatValue::Num(0.0));
        assert!(!p.run_stat_command("PrestigeRespec", 11, &|_| None));
        assert_eq!(p.tokens(), 1);
    }

    /// Fresh Start: everything back to a new profile's, prestige included;
    /// his options and match settings stay; mp/stats_init.cfg and the
    /// notices follow, his reset count one up.
    #[test]
    fn prestige_stats_reset_all_wipes_to_a_new_profile() {
        let loc = |k: &str| (k == "CLASS_SLOT1").then(|| "Custom 1".to_owned());
        let mut p = prestiged();
        p.set_stat(STAT_RESET_COUNT.to_owned(), StatValue::Num(2.0));
        p.set_stat("profile.cacloadouts.customclassname.0".to_owned(), StatValue::Str("Mine".into()));
        p.set_stat("gametypesettings.timelimit".to_owned(), StatValue::Num(10.0));
        p.set_stat("cacloadouts.customclassname.0".to_owned(), StatValue::Str("Mine".into()));
        p.set_stat("classsets.cacloadouts.3.name".to_owned(), StatValue::Str("Set".into()));
        assert!(p.run_stat_command("PrestigeStatsResetAll", 11, &loc));
        assert_eq!((p.plevel(), p.rank(), p.tokens(), p.stat_num(RANK_XP)), (0, 0, 0, 0.0));
        assert!(!p.purchased(12, false) && p.prestige_tokens() == 0);
        assert_eq!(p.stat("cacloadouts.customclassname.0"), Some(&StatValue::Str("Custom 1".into())));
        assert_eq!(p.stat_num(&Profile::class_key("cacloadouts", 0, "primary")), 31.0);
        assert_eq!(p.stat("classsets.cacloadouts.3.name"), None);
        assert_eq!(p.stat("profile.cacloadouts.customclassname.0"), Some(&StatValue::Str("Mine".into())));
        assert_eq!(p.stat_num("gametypesettings.timelimit"), 10.0);
        assert_eq!(
            p.take_followups(),
            vec![
                "exec mp/stats_init.cfg",
                "statwriteddl cacloadouts resetWarningDisplayed 1",
                "statwriteddl cacloadouts classWarningDisplayed 1",
                "statwriteddl playerstatslist statresetcount statvalue 3",
            ]
        );
        for line in ["statwriteddl cacloadouts resetWarningDisplayed 1", "statwriteddl playerstatslist statresetcount statvalue 3"] {
            assert!(p.run_stat_command(line, 11, &loc));
        }
        assert_eq!((p.stat_num("cacloadouts.resetwarningdisplayed"), p.stat_num(STAT_RESET_COUNT)), (1.0, 3.0));
        assert!(!p.run_stat_command("PrestigeStatsResetAll", 11, &loc), "no prestige left");
    }

    /// Extra Create-a-Class: one more class a prestige, ten at most; the
    /// menus show no more than the set holds.
    #[test]
    fn prestige_add_cac_adds_a_class_up_to_ten() {
        let mut p = prestiged();
        assert_eq!(p.custom_class_count(), 5);
        assert!(p.run_stat_command("PrestigeAddCAC", 11, &|_| None));
        assert_eq!(p.custom_class_count(), 6);
        assert_eq!((p.custom_class_count_in("cacloadouts"), p.custom_class_count_in(LOCAL_MATCH_SET)), (6, 5));
        assert!(!p.run_stat_command("PrestigeAddCAC", 11, &|_| None));
        for l in 2..=10 {
            p.set_stat(PLEVEL.to_owned(), StatValue::Num(l as f32));
            assert!(p.run_stat_command("prestigeaddcac", 11, &|_| None));
        }
        assert_eq!(p.custom_class_count(), 10);
        p.set_stat(PLEVEL.to_owned(), StatValue::Num(0.0));
        assert_eq!(p.custom_class_count(), 5, "none before his first prestige");
    }

    /// mp/reset_public_class_sets.cfg and mp/playerstats_reset.cfg lines.
    #[test]
    fn public_class_sets_reset_and_stats_by_name() {
        let loc = |k: &str| k.strip_prefix("CLASS_SLOT").map(|n| format!("Custom {n}"));
        let mut p = prestiged();
        p.set_stat("classsets.cacloadouts.current".to_owned(), StatValue::Num(3.0));
        p.set_stat("classsets.cacloadouts.0.customclass.0.primary".to_owned(), StatValue::Num(12.0));
        p.set_stat("classsets.cacloadouts.2.customclass.1.primary".to_owned(), StatValue::Num(12.0));
        for line in [
            "statwriteddl publicMatchClassSetIndex 0",
            "setPublicMatchClassSetNameFromLocString 0 CLASS_SET1",
            "copyToPublicMatchClassSet 0",
            "zeroPublicMatchClassSet 2",
            "statsetbyname RANKXP 0",
            "equipdefaultcustommatchclass 0 class_custom_assault",
        ] {
            assert!(p.run_stat_command(line, 11, &loc), "{line}");
        }
        assert_eq!(p.stat_num("classsets.cacloadouts.current"), 0.0);
        assert_eq!(p.stat("classsets.cacloadouts.0.name"), Some(&StatValue::Str("CLASS_SET1".into())));
        assert_eq!(p.stat_num("classsets.cacloadouts.0.customclass.0.primary"), 31.0, "a copy of the public classes");
        assert_eq!(p.stat("classsets.cacloadouts.0.customclassname.0"), p.stat("cacloadouts.customclassname.0"));
        assert_eq!(p.stat("classsets.cacloadouts.2.customclass.1.primary"), None, "emptied");
        assert_eq!(p.stat("classsets.cacloadouts.2.customclassname.9"), Some(&StatValue::Str("Custom 10".into())));
        assert_eq!(p.stat_num(RANK_XP), 0.0);
        assert_eq!(p.stat_num(&Profile::class_key("custommatchcacloadouts", 0, "primary")), 31.0);
    }

    #[test]
    fn bonus_cards_by_number_and_stats_round_trip() {
        let mut p = sample();
        p.set_stat(Profile::class_key(LOCAL_MATCH_SET, 1, "bonuscard2"), StatValue::Num(180.0));
        assert!(p.bonus_card_active(2, 1));
        assert!(!p.bonus_card_active(0, 1));
        let back = Profile::stats_from_text(&p.to_text());
        assert_eq!(back, p.stats);
    }

    #[test]
    fn attachments_go_together_only_when_each_lists_the_other() {
        let att = |i: &str, r: &str, with: &str| row(&[(0, i), (2, "attachment"), (4, r), (11, with)]);
        let atts = vec![
            att("0", "none", ""),
            att("1", "acog", "dualclip extbarrel"),
            att("2", "dualclip", "acog reflex"),
            att("5", "extbarrel", "dualclip"),
            att("20", "reflex", "dualclip"),
            row(&[(0, "1"), (2, "weaponoption"), (4, "camo_multicam"), (11, "acog")]),
        ];
        let p = Profile::from_tables(&[], &atts, &|_| None, HashMap::new());
        assert!(p.attachments_compatible("acog", "dualclip") && p.attachments_compatible("dualclip", "acog"));
        // A list word is its base attachment.
        assert!(p.attachments_compatible("reflex_pistol", "dualclip"));
        // Extended Barrel does not list the ACOG, so neither goes with the other.
        assert!(!p.attachments_compatible("acog", "extbarrel") && !p.attachments_compatible("extbarrel", "acog"));
        assert!(!p.attachments_compatible("acog", "acog"));
        assert!(p.attachments_compatible("none", "acog") && p.attachments_compatible("reflex", "none"));
        assert!(!p.attachments_compatible("acog", "camo_multicam") && !p.attachments_compatible("none", "nothing"));
    }
}
