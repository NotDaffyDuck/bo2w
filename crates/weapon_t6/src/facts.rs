//! Per-weapon facts the brain reads, pulled from the BO2 fastfile weapon assets.
//!
//! Each mechanic window owns one sub-struct here (its fields and its reader); window 1 owns
//! the composition. Values are read little-endian at `fastfile_t6::layout` offsets, from the
//! weapon's variant bytes and its WeaponDef bytes.

use fastfile_t6::layout::{WeaponDef as D, WeaponVariantDef as V};

/// BO2 `weapType_t`.
pub mod weap_type {
    pub const BULLET: u8 = 0;
    pub const GRENADE: u8 = 1;
    pub const PROJECTILE: u8 = 2;
    pub const BINOCULARS: u8 = 3;
    pub const GAS: u8 = 4;
    pub const BOMB: u8 = 5;
    pub const MINE: u8 = 6;
    pub const MELEE: u8 = 7;
    pub const RIOTSHIELD: u8 = 8;
}

/// BO2 `weapClass_t`.
pub mod weap_class {
    pub const RIFLE: u8 = 0;
    pub const MG: u8 = 1;
    pub const SMG: u8 = 2;
    pub const SPREAD: u8 = 3;
    pub const PISTOL: u8 = 4;
    pub const GRENADE: u8 = 5;
    pub const ROCKETLAUNCHER: u8 = 6;
    pub const TURRET: u8 = 7;
    pub const NON_PLAYER: u8 = 8;
    pub const GAS: u8 = 9;
    pub const ITEM: u8 = 10;
    pub const MELEE: u8 = 11;
    pub const KILLSTREAK_ALT_STORED_WEAPON: u8 = 12;
    pub const PISTOL_SPREAD: u8 = 13;
}

/// BO2 `weapInventoryType_t`.
pub mod inventory {
    pub const PRIMARY: u8 = 0;
    pub const OFFHAND: u8 = 1;
    pub const ITEM: u8 = 2;
    pub const ALTMODE: u8 = 3;
    pub const MELEE: u8 = 4;
    pub const DWLEFTHAND: u8 = 5;
}

/// BO2 `weapFireType_t`.
pub mod fire_type {
    pub const FULLAUTO: u8 = 0;
    pub const SINGLESHOT: u8 = 1;
    pub const BURSTFIRE2: u8 = 2;
    pub const BURSTFIRE3: u8 = 3;
    pub const BURSTFIRE4: u8 = 4;
    pub const BURSTFIRE5: u8 = 5;
    pub const STACKED: u8 = 6;
    pub const MINIGUN: u8 = 7;
    pub const CHARGESHOT: u8 = 8;
    pub const JETGUN: u8 = 9;

    /// BG_WeaponUsesBurstFire: the burst types and the charge shot.
    pub fn uses_burst(t: u8) -> bool {
        (BURSTFIRE2..=BURSTFIRE5).contains(&t) || t == CHARGESHOT
    }
}

/// BO2 `OffhandClass`.
pub mod offhand_class {
    pub const NONE: u8 = 0;
    pub const FRAG_GRENADE: u8 = 1;
    pub const SMOKE_GRENADE: u8 = 2;
    pub const FLASH_GRENADE: u8 = 3;
    pub const GEAR: u8 = 4;
    pub const SUPPLYDROP_MARKER: u8 = 5;
}

/// BO2 `OffhandSlot`.
pub mod offhand_slot {
    pub const NONE: u8 = 0;
    pub const LETHAL_GRENADE: u8 = 1;
    pub const TACTICAL_GRENADE: u8 = 2;
    pub const EQUIPMENT: u8 = 3;
    pub const SPECIFIC_USE: u8 = 4;
}

fn rd_i32(b: &[u8], at: usize) -> i32 {
    b.get(at..at + 4)
        .map(|s| i32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .unwrap_or(0)
}

fn rd_u32(b: &[u8], at: usize) -> u32 {
    rd_i32(b, at) as u32
}

fn rd_u8(b: &[u8], at: usize) -> u8 {
    b.get(at).copied().unwrap_or(0)
}

fn rd_bool(b: &[u8], at: usize) -> bool {
    rd_u8(b, at) != 0
}

/// Window 1: what every mechanic needs to know about a weapon.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CoreFacts {
    pub weap_type: u8,
    pub weap_class: u8,
    pub inventory_type: u8,
    pub fire_type: u8,
    pub offhand_class: u8,
    pub offhand_slot: u8,
    pub unlimited_ammo: bool,
    pub no_drops_or_raises: bool,
    pub can_use_in_vehicle: bool,
    pub bolt_action: bool,
    pub dual_wield: bool,
    pub alt_weapon_ads_only: bool,
    /// Variant `altWeaponIndex` (0 = none).
    pub alt_weapon: u32,
    /// Def `dualWieldWeaponIndex`: the left-hand gun (0 = none).
    pub dual_wield_weapon: u32,
    pub clip_size: i32,
    pub burst_delay_time: i32,
    /// `bClipOnly`: the clip is the whole weapon (no stock).
    pub clip_only: bool,
    pub has_detonator: bool,
    pub plantable: bool,
    pub hold_button_to_throw: bool,
    /// `bAltWeaponDisableSwitching`.
    pub alt_weapon_disable_switching: bool,
    /// `guidedMissileType_t` (0 = none, 3 = javelin, 6 = TV guided, 7 = drone ...).
    pub guided_missile_type: i32,
    /// Filled by the loader (follows pointers): bit N set = variant anim slot N (0-87) names an
    /// anim. `read` leaves it 0.
    pub anim_slots: u128,
    /// Filled by the loader: the def has a gun model (`gunXModel[0]` set).
    pub has_gun_model: bool,
}

impl CoreFacts {
    pub fn read(variant: &[u8], def: &[u8]) -> Self {
        Self {
            weap_type: rd_u8(def, D::weapType),
            weap_class: rd_u8(def, D::weapClass),
            inventory_type: rd_u8(def, D::inventoryType),
            fire_type: rd_u8(def, D::fireType),
            offhand_class: rd_u8(def, D::offhandClass),
            offhand_slot: rd_u8(def, D::offhandSlot),
            unlimited_ammo: rd_bool(def, D::unlimitedAmmo),
            no_drops_or_raises: rd_bool(def, D::bNoDropsOrRaises),
            can_use_in_vehicle: rd_bool(def, D::bCanUseInVehicle),
            bolt_action: rd_bool(def, D::bBoltAction),
            dual_wield: rd_bool(def, D::bDualWield),
            alt_weapon_ads_only: rd_bool(def, D::bAltWeaponAdsOnly),
            alt_weapon: rd_u32(variant, V::altWeaponIndex),
            dual_wield_weapon: rd_u32(def, D::dualWieldWeaponIndex),
            clip_size: rd_i32(variant, V::iClipSize),
            burst_delay_time: rd_i32(def, D::iBurstDelayTime),
            clip_only: rd_bool(def, D::bClipOnly),
            has_detonator: rd_bool(def, D::hasDetonator),
            plantable: rd_bool(def, D::plantable),
            hold_button_to_throw: rd_bool(def, D::holdButtonToThrow),
            alt_weapon_disable_switching: rd_bool(def, D::bAltWeaponDisableSwitching),
            guided_missile_type: rd_i32(def, D::guidedMissileType),
            anim_slots: 0,
            has_gun_model: false,
        }
    }
}

/// Window 2 (switching): every raise/drop time.
///
/// `first_raise`, `alt_drop` and `alt_raise` hold the value after BO2's attachment-unique
/// override. Our weapon ids carry no attachments yet, so the reader stores the base value;
/// a loader that knows the variant's attachments overwrites them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SwitchFacts {
    pub drop: i32,
    pub raise: i32,
    pub alt_drop: i32,
    pub alt_raise: i32,
    pub quick_drop: i32,
    pub quick_raise: i32,
    pub first_raise: i32,
    pub empty_raise: i32,
    pub empty_drop: i32,
    pub no_quick_drop_when_empty: bool,
    pub cancel_auto_holster_when_empty: bool,
}

impl SwitchFacts {
    pub fn read(variant: &[u8], def: &[u8]) -> Self {
        Self {
            drop: rd_i32(def, D::iDropTime),
            raise: rd_i32(def, D::iRaiseTime),
            alt_drop: rd_i32(def, D::iAltDropTime),
            alt_raise: rd_i32(variant, V::iAltRaiseTime),
            quick_drop: rd_i32(def, D::quickDropTime),
            quick_raise: rd_i32(def, D::quickRaiseTime),
            first_raise: rd_i32(def, D::iFirstRaiseTime),
            empty_raise: rd_i32(def, D::iEmptyRaiseTime),
            empty_drop: rd_i32(def, D::iEmptyDropTime),
            no_quick_drop_when_empty: rd_bool(def, D::bNoQuickDropWhenEmpty),
            cancel_auto_holster_when_empty: rd_bool(def, D::cancelAutoHolsterWhenEmpty),
        }
    }
}

/// A weapon's attachment time scales: BO2 multiplies every attachment's scale (up to 3) at each
/// read (`BG_GetFireTime`, `BG_GetADSTransTimeScales`); 1 with no attachments.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AttachmentScales {
    /// Product of `fFireTimeScale`.
    pub fire_time: f32,
    /// Product of `fAdsTransInTimeScale`.
    pub ads_in: f32,
    /// Product of `fAdsTransOutTimeScale`.
    pub ads_out: f32,
}

impl Default for AttachmentScales {
    fn default() -> Self {
        Self {
            fire_time: 1.0,
            ads_in: 1.0,
            ads_out: 1.0,
        }
    }
}

impl AttachmentScales {
    pub fn fold(atts: &[&[u8]]) -> Self {
        use fastfile_t6::layout::WeaponAttachment as A;
        let rd = |a: &[u8], at: usize| f32::from_le_bytes(a[at..at + 4].try_into().unwrap());
        let mut s = Self::default();
        for a in &atts[..atts.len().min(3)] {
            s.fire_time *= rd(a, A::fFireTimeScale);
            s.ads_in *= rd(a, A::fAdsTransInTimeScale);
            s.ads_out *= rd(a, A::fAdsTransOutTimeScale);
        }
        s
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct T6WeaponFacts {
    pub core: CoreFacts,
    pub switch: SwitchFacts,
    pub fire: crate::pm::fire::FireFacts,
    pub reload: crate::pm::reload::ReloadFacts,
    pub offhand: crate::pm::offhand::OffhandFacts,
    pub melee: crate::pm::melee::MeleeFacts,
    pub moves: crate::pm::moves::MoveFacts,
    pub ads: crate::pm::ads::AdsFacts,
    pub shield: crate::pm::shield::ShieldFacts,
    pub spread: crate::pm::spread::SpreadFacts,
    pub dual: crate::pm::dual::DualFacts,
    pub att: AttachmentScales,
}

impl T6WeaponFacts {
    pub fn read(variant: &[u8], def: &[u8]) -> Self {
        Self {
            core: CoreFacts::read(variant, def),
            switch: SwitchFacts::read(variant, def),
            fire: crate::pm::fire::FireFacts::read(variant, def),
            reload: crate::pm::reload::ReloadFacts::read(variant, def),
            offhand: crate::pm::offhand::OffhandFacts::read(variant, def),
            melee: crate::pm::melee::MeleeFacts::read(variant, def),
            moves: crate::pm::moves::MoveFacts::read(variant, def),
            ads: crate::pm::ads::AdsFacts::read(variant, def),
            shield: crate::pm::shield::ShieldFacts::read(variant, def),
            spread: crate::pm::spread::SpreadFacts::read(variant, def),
            dual: crate::pm::dual::DualFacts::read(variant, def),
            att: AttachmentScales::default(),
        }
    }
}

/// Facts by weapon index. Index 0 (no weapon) and unknown indexes read as all-zero facts.
#[derive(Clone, Debug, Default)]
pub struct FactsTable {
    rows: Vec<T6WeaponFacts>,
    names: Vec<String>,
    /// Attachment bytes by name (`#att` dump lines), folded into `<gun>+<att>` rows.
    attachments: std::collections::HashMap<String, Vec<u8>>,
}

/// BO2's default attachment (`bg_weaponAttachmentDefault`): an attachment "overrides" a value
/// when it differs from this one.
pub const DEFAULT_ATTACHMENT: &str = "defaultattachment";

impl FactsTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, weapon: u32, name: &str, facts: T6WeaponFacts) {
        let i = weapon as usize;
        if self.rows.len() <= i {
            self.rows.resize(i + 1, T6WeaponFacts::default());
            self.names.resize(i + 1, String::new());
        }
        self.rows[i] = facts;
        self.names[i] = name.to_owned();
    }

    pub fn get(&self, weapon: u32) -> T6WeaponFacts {
        // Unknown index: no weapon, all-zero facts.
        self.rows
            .get(weapon as usize)
            .copied()
            .unwrap_or(T6WeaponFacts::default())
    }

    pub fn name(&self, weapon: u32) -> &str {
        self.names
            .get(weapon as usize)
            .map(String::as_str)
            .unwrap_or("")
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn find(&self, name: &str) -> Option<u32> {
        self.names.iter().position(|n| n == name).map(|i| i as u32)
    }

    /// Parse the dump format, one weapon per line; line N (from 1) is weapon index N:
    /// `name<TAB>variant hex<TAB>def hex` then optional `key=value` columns the bytes cannot
    /// carry (pointers and load-time indexes): `slots=HEX` ([`CoreFacts::anim_slots`]),
    /// `gun=1` ([`CoreFacts::has_gun_model`]), `alt=NAME` and `dual=NAME` (the alternate
    /// weapon and the left-hand gun, resolved to this table's indexes).
    pub fn from_dump(text: &str) -> Self {
        let mut t = Self::new();
        let mut links: Vec<(u32, String, String)> = Vec::new();
        for (i, line) in text.lines().enumerate() {
            if let Some(rest) = line.strip_prefix("#att\t") {
                if let Some((name, bytes)) = rest
                    .split_once('\t')
                    .and_then(|(n, h)| Some((n, unhex(h)?)))
                {
                    t.attachments.insert(name.to_owned(), bytes);
                }
                continue;
            }
            let mut parts = line.split('\t');
            let (Some(name), Some(v), Some(d)) = (parts.next(), parts.next(), parts.next()) else {
                continue;
            };
            let (Some(v), Some(d)) = (unhex(v), unhex(d)) else {
                continue;
            };
            let index = i as u32 + 1;
            let mut f = T6WeaponFacts::read(&v, &d);
            // Indexes in the bytes are the fastfile's, not ours.
            f.core.alt_weapon = 0;
            f.core.dual_wield_weapon = 0;
            let (mut alt, mut dual) = (String::new(), String::new());
            for col in parts {
                match col.split_once('=') {
                    Some(("slots", x)) => {
                        f.core.anim_slots = u128::from_str_radix(x, 16).unwrap_or(0)
                    }
                    Some(("gun", x)) => f.core.has_gun_model = x == "1",
                    Some(("alt", x)) => alt = x.to_owned(),
                    Some(("dual", x)) => dual = x.to_owned(),
                    _ => {}
                }
            }
            if !alt.is_empty() || !dual.is_empty() {
                links.push((index, alt, dual));
            }
            t.set(index, name, f);
        }
        for (index, alt, dual) in links {
            let alt = t.find(&alt).unwrap_or(0);
            let dual = t.find(&dual).unwrap_or(0);
            let row = &mut t.rows[index as usize];
            row.core.alt_weapon = alt;
            row.core.dual_wield_weapon = dual;
        }
        t
    }

    /// The same facts under the game's weapon numbers: `rows` gives each game index and its
    /// name. A name missing here falls back to its base gun (the part before `+`, the game's
    /// attachment rows). Alternate and left-hand weapon links are renumbered by name.
    pub fn reindexed<'a>(&self, rows: impl IntoIterator<Item = (u32, &'a str)>) -> Self {
        let rows: Vec<(u32, &str)> = rows.into_iter().collect();
        let game_index = |name: &str| rows.iter().find(|(_, n)| *n == name).map(|(i, _)| *i);
        let mut t = Self::new();
        for &(index, name) in &rows {
            let Some(src) = self
                .find(name)
                .or_else(|| name.split_once('+').and_then(|(base, _)| self.find(base)))
            else {
                continue;
            };
            let mut f = self.get(src);
            // BO2 reads a gun's own alt first, then the one its attachment names (a launcher,
            // the Hybrid's other optic: the `<gun>+<attachment>` rows).
            if f.core.alt_weapon == 0
                && let Some((base, atts)) = name.split_once('+')
            {
                f.core.alt_weapon = atts
                    .split('+')
                    .filter_map(|att| self.find(&format!("{base}+{att}")))
                    .map(|i| self.get(i).core.alt_weapon)
                    .find(|&alt| alt != 0)
                    .unwrap_or(0);
            }
            // BO2's alternate and left-hand weapons keep the gun's attachments (only the base
            // index changes): `an94_mp+gl` goes to `gl_an94_mp+gl`, and back to `an94_mp+gl`.
            let atts = name.split_once('+').map(|(_, a)| a);
            let relink = |i: u32| {
                if i == 0 {
                    return 0;
                }
                let other = self.name(i);
                atts.and_then(|a| game_index(&format!("{other}+{a}")))
                    .or_else(|| game_index(other))
                    .unwrap_or(0)
            };
            f.core.alt_weapon = relink(f.core.alt_weapon);
            f.core.dual_wield_weapon = relink(f.core.dual_wield_weapon);
            self.fold_attachments(name, &mut f);
            t.set(index, name, f);
        }
        t.attachments = self.attachments.clone();
        t
    }

    /// A `<gun>+<att>[+<att>...]` row's spread, sway, idle and perk values with its attachments
    /// folded in (BO2's getters read the variant's attachments; without the default
    /// attachment asset they compare against an all-zero one, as BO2's no-fastfile loader).
    fn fold_attachments(&self, name: &str, f: &mut T6WeaponFacts) {
        let Some((_, atts)) = name.split_once('+') else {
            return;
        };
        let atts: Vec<&[u8]> = atts
            .split('+')
            .filter_map(|a| self.attachments.get(a).map(Vec::as_slice))
            .collect();
        if atts.is_empty() {
            return;
        }
        let zero = [0u8; fastfile_t6::layout::WeaponAttachment::SIZE];
        let default = self
            .attachments
            .get(DEFAULT_ATTACHMENT)
            .map_or(&zero[..], Vec::as_slice);
        f.spread.apply_attachments(&atts, default);
        f.att = AttachmentScales::fold(&atts);
        // BO2's alt-mode flag getters also take any of the gun's attachments' flags (select fire, Hybrid).
        use fastfile_t6::layout::WeaponAttachment as A;
        for a in &atts[..atts.len().min(3)] {
            f.core.alt_weapon_ads_only |= a[A::bAltWeaponAdsOnly] != 0;
            f.core.alt_weapon_disable_switching |= a[A::bAltWeaponDisableSwitching] != 0;
        }
    }
}

/// One attachment's bytes as a dump line (see [`FactsTable::from_dump`]).
pub fn attachment_line(name: &str, bytes: &[u8]) -> String {
    format!("#att\t{name}\t{}", hex(bytes))
}

/// One dump line (see [`FactsTable::from_dump`]); empty `alt`/`dual` are left out.
pub fn dump_line(
    name: &str,
    variant: &[u8],
    def: &[u8],
    anim_slots: u128,
    has_gun_model: bool,
    alt: &str,
    dual: &str,
) -> String {
    let mut line = format!(
        "{name}\t{}\t{}\tslots={anim_slots:x}",
        hex(variant),
        hex(def)
    );
    if has_gun_model {
        line.push_str("\tgun=1");
    }
    if !alt.is_empty() {
        line.push_str(&format!("\talt={alt}"));
    }
    if !dual.is_empty() {
        line.push_str(&format!("\tdual={dual}"));
    }
    line
}

pub fn hex(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len() * 2);
    for x in b {
        s.push_str(&format!("{x:02x}"));
    }
    s
}

pub fn unhex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attachment_rows_fold_their_attachments() {
        use fastfile_t6::layout::WeaponAttachment as A;
        let (v, d) = ([0u8; 600], vec![0u8; 2048]);
        let mut laser = vec![0u8; A::SIZE];
        laser[A::laserSight] = 1;
        laser[A::perks..A::perks + 8].copy_from_slice(&(1u64 << 3).to_le_bytes());
        for (at, x) in [
            (A::fFireTimeScale, 0.8f32),
            (A::fAdsTransInTimeScale, 0.5),
            (A::fAdsTransOutTimeScale, 1.0),
        ] {
            laser[at..at + 4].copy_from_slice(&x.to_le_bytes());
        }
        let dump = [
            dump_line("gun_mp", &v, &d, 0, true, "", ""),
            attachment_line("steadyaim", &laser),
            attachment_line(DEFAULT_ATTACHMENT, &[0u8; A::SIZE]),
        ]
        .join("\n");
        let t = FactsTable::from_dump(&dump).reindexed([(4, "gun_mp"), (9, "gun_mp+steadyaim")]);
        assert!(!t.get(4).spread.laser_sight);
        assert!(t.get(9).spread.laser_sight);
        assert!(t.get(9).spread.has_perk(3));
        assert_eq!(t.get(4).att, AttachmentScales::default());
        assert_eq!((t.get(9).att.fire_time, t.get(9).att.ads_in), (0.8, 0.5));
        assert_eq!(t.find("gun_mp+steadyaim"), Some(9));
    }

    #[test]
    fn reads_switch_times_at_layout_offsets() {
        let mut def = vec![0u8; 2048];
        def[D::iDropTime..D::iDropTime + 4].copy_from_slice(&450i32.to_le_bytes());
        def[D::iFirstRaiseTime..D::iFirstRaiseTime + 4].copy_from_slice(&1200i32.to_le_bytes());
        def[D::bNoDropsOrRaises] = 1;
        let f = T6WeaponFacts::read(&[0u8; 600], &def);
        assert_eq!(f.switch.drop, 450);
        assert_eq!(f.switch.first_raise, 1200);
        assert!(f.core.no_drops_or_raises);
        let dump = format!("gun_mp\t{}\t{}\n", hex(&[0u8; 600]), hex(&def));
        let t = FactsTable::from_dump(&dump);
        assert_eq!(t.get(1).switch.drop, 450);
        assert_eq!(t.find("gun_mp"), Some(1));
        assert_eq!(t.get(9), T6WeaponFacts::default());
    }

    #[test]
    fn dump_links_and_loader_columns_follow_names() {
        let v = [0u8; 600];
        let d = vec![0u8; 2048];
        let dump = [
            dump_line("gun_mp", &v, &d, 0x5, true, "gun_gl_mp", ""),
            dump_line("gun_gl_mp", &v, &d, 0, false, "gun_mp", ""),
            dump_line("pistol_dw_mp", &v, &d, 0, true, "", "pistol_lh_mp"),
            dump_line("pistol_lh_mp", &v, &d, 0, true, "", ""),
        ]
        .join("\n");
        let t = FactsTable::from_dump(&dump);
        assert_eq!(t.get(1).core.anim_slots, 0x5);
        assert!(t.get(1).core.has_gun_model);
        assert!(!t.get(2).core.has_gun_model);
        assert_eq!(t.get(1).core.alt_weapon, 2);
        assert_eq!(t.get(2).core.alt_weapon, 1);
        assert_eq!(t.get(3).core.dual_wield_weapon, 4);
        // The game numbers them differently and adds attachment rows.
        let g = t.reindexed([
            (7, "pistol_lh_mp"),
            (3, "gun_mp"),
            (4, "gun_gl_mp"),
            (5, "gun_mp+extclip"),
            (9, "pistol_dw_mp"),
        ]);
        assert_eq!(g.get(3).core.alt_weapon, 4);
        assert_eq!(g.get(4).core.alt_weapon, 3);
        assert_eq!(g.get(5).core.anim_slots, 0x5);
        assert_eq!(g.get(9).core.dual_wield_weapon, 7);
        assert_eq!(g.find("gun_mp+extclip"), Some(5));
    }

    #[test]
    fn an_attachment_row_gives_its_gun_the_alt() {
        let v = [0u8; 600];
        let d = vec![0u8; 2048];
        let dump = [
            dump_line("rifle_mp", &v, &d, 0x7, true, "", ""),
            dump_line("gl_rifle_mp", &v, &d, 0, true, "rifle_mp", ""),
            dump_line("rifle_mp+gl", &v, &d, 0x7, true, "gl_rifle_mp", ""),
        ]
        .join(
            "
",
        );
        let g = FactsTable::from_dump(&dump).reindexed([
            (2, "rifle_mp"),
            (3, "gl_rifle_mp"),
            (4, "rifle_mp+gl"),
            (5, "rifle_mp+extclip+gl"),
            (6, "rifle_mp+extclip"),
        ]);
        // The bare gun has no alt; with the launcher on, alone or with others, it is the launcher.
        assert_eq!(g.get(2).core.alt_weapon, 0);
        assert_eq!(g.get(4).core.alt_weapon, 3);
        assert_eq!(g.get(5).core.alt_weapon, 3);
        assert_eq!(g.get(5).core.anim_slots, 0x7);
        assert_eq!(g.get(6).core.alt_weapon, 0);
        assert_eq!(g.get(3).core.alt_weapon, 2);
    }

    #[test]
    fn an_alt_keeps_the_guns_attachments() {
        let v = [0u8; 600];
        let d = vec![0u8; 2048];
        let dump = [
            dump_line("rifle_mp", &v, &d, 0x7, true, "", ""),
            dump_line("gl_rifle_mp", &v, &d, 0, true, "rifle_mp", ""),
            dump_line("rifle_mp+gl", &v, &d, 0x7, true, "gl_rifle_mp", ""),
            dump_line("pistol_dw_mp", &v, &d, 0, true, "", "pistol_lh_mp"),
            dump_line("pistol_lh_mp", &v, &d, 0, true, "", ""),
        ]
        .join("\n");
        let g = FactsTable::from_dump(&dump).reindexed([
            (2, "rifle_mp"),
            (3, "gl_rifle_mp"),
            (4, "rifle_mp+gl"),
            (5, "gl_rifle_mp+gl"),
            (6, "rifle_mp+extclip+gl"),
            (7, "gl_rifle_mp+extclip+gl"),
            (8, "pistol_dw_mp+extclip"),
            (9, "pistol_lh_mp+extclip"),
            (10, "pistol_lh_mp"),
        ]);
        // The game's launcher rows carry the gun's attachments, both ways (BG_GetAltWeapon
        // swaps only the base index).
        assert_eq!(g.get(4).core.alt_weapon, 5);
        assert_eq!(g.get(5).core.alt_weapon, 4);
        assert_eq!(g.get(6).core.alt_weapon, 7);
        assert_eq!(g.get(7).core.alt_weapon, 6);
        // Without its own `+att` row the bare launcher still answers.
        assert_eq!(g.get(3).core.alt_weapon, 2);
        assert_eq!(g.get(8).core.dual_wield_weapon, 9);
    }

    #[test]
    fn an_attachment_sets_the_alt_mode_flags() {
        use fastfile_t6::layout::WeaponAttachment as A;
        let v = [0u8; 600];
        let d = vec![0u8; 2048];
        let mut sf = vec![0u8; A::SIZE];
        sf[A::bAltWeaponDisableSwitching] = 1;
        let mut optic = vec![0u8; A::SIZE];
        optic[A::bAltWeaponAdsOnly] = 1;
        optic[A::bAltWeaponDisableSwitching] = 1;
        let dump = [
            attachment_line("sf", &sf),
            attachment_line("dualoptic", &optic),
            dump_line("rifle_mp", &v, &d, 0x7, true, "", ""),
        ]
        .join("
");
        let g = FactsTable::from_dump(&dump).reindexed([
            (2, "rifle_mp"),
            (3, "rifle_mp+sf"),
            (4, "rifle_mp+dualoptic"),
        ]);
        let flags = |i| (g.get(i).core.alt_weapon_ads_only, g.get(i).core.alt_weapon_disable_switching);
        assert_eq!(flags(2), (false, false));
        assert_eq!(flags(3), (false, true));
        assert_eq!(flags(4), (true, true));
    }
}
