//! BO2's dropped-gun rules on the T6 player state: what a thrown-down gun carries
//! (ThrowDownWeapon), what walking over one takes (Touch_Item, WeaponPickup's ammo leech) and
//! what picking one up gives (WeaponPickup_Grab). The game side spawns, moves and frees the item
//! entity; these functions only read and write the gun state and the item's two ammo slots.
//!
//! BO2 keeps a held gun's clip and reserve in the slot beside it, so a gun that is not held has
//! no ammo to read or write: every read here is 0 and every write is skipped for a gun that is
//! not held (BG_GetAmmoClip / BG_GetAmmoPool return nothing for it).
//!
//! `max` is BG_GetMaxAmmo for a weapon id (the combat table's max ammo).
//!
//! Not ported: the shared ammo caps (bg_sharedAmmoCaps), the frag-class give in Add_Ammo, the
//! weapon's camo / render options on the item, BG_PlayerCanPickUpWeaponType (the class/perk
//! restrictions on what a player may pick up) and the grenade throwback pickup.

use crate::facts::{FactsTable, inventory, offhand_class, weap_type};
use crate::player::{T6WeaponPs, wf};

/// One ammo slot on a dropped gun: slot 1 is the gun itself, slot 2 its left-hand gun (dual
/// wield) or its alternate mode (a grenade launcher ...).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ItemSlot {
    pub weapon: u32,
    /// Rounds in the magazine; -1 on a fresh item means "fill the magazine from the stock".
    pub clip: i32,
    pub stock: i32,
}

pub type ItemAmmo = [ItemSlot; 2];

/// What throwing a gun down does (ThrowDownWeapon).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThrowDown {
    /// Nothing at all (an alternate mode with no gun behind it).
    Nothing,
    /// The gun is taken and nothing lands (no ammo to carry, or not held).
    TakeOnly(u32),
    /// The gun lands carrying this ammo, then it is taken.
    Drop(u32, ItemAmmo),
}

/// What walking over a gun did (WeaponPickup's ammo leech).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Leech {
    /// Some ammo moved to the player (the AMMO_PICKUP event and the "trigger" notify).
    pub added: bool,
    /// The item goes away.
    pub take: bool,
}

fn base(facts: &FactsTable, w: u32) -> &str {
    let n = facts.name(w);
    n.split('+').next().unwrap_or(n)
}

/// Same gun under its attachments (BO2 compares the root names).
fn same_base(facts: &FactsTable, a: u32, b: u32) -> bool {
    if a == b {
        return true;
    }
    let (x, y) = (base(facts, a), base(facts, b));
    !x.is_empty() && x == y
}

fn held_list(ps: &T6WeaponPs) -> Vec<u32> {
    ps.held
        .iter()
        .map(|h| h.weapon)
        .filter(|&w| w != 0)
        .collect()
}

/// BG_GetAmmoInClip through BG_GetAmmoClip: 0 when the magazine's gun is not held.
pub fn clip(ps: &T6WeaponPs, facts: &FactsTable, w: u32) -> i32 {
    let cw = facts.ammo_clip_weapon(w);
    if ps.has_weapon(cw) {
        ps.clip_slot(cw)
    } else {
        0
    }
}

/// BG_GetAmmoNotInClip through BG_GetAmmoPool: 0 when the reserve's gun is not held.
pub fn stock(ps: &T6WeaponPs, facts: &FactsTable, w: u32) -> i32 {
    let pw = facts.ammo_pool_weapon(w);
    if ps.has_weapon(pw) {
        ps.pool_slot(pw)
    } else {
        0
    }
}

fn set_clip(ps: &mut T6WeaponPs, facts: &FactsTable, w: u32, n: i32) {
    let cw = facts.ammo_clip_weapon(w);
    if ps.has_weapon(cw) {
        ps.set_clip_slot(cw, n.max(0));
    }
}

fn set_stock(ps: &mut T6WeaponPs, facts: &FactsTable, w: u32, n: i32) {
    let pw = facts.ammo_pool_weapon(w);
    if ps.has_weapon(pw) {
        ps.set_pool_slot(pw, n.max(0));
    }
}

/// BG_GetAmmoPlayerMax(w, excluded): how much reserve the player can carry for `w`'s pool. A
/// clip-only gun's is its magazine. Otherwise the max ammo of every held gun drawing from the
/// same reserve, leaving out `excluded`, and leaving out an alternate mode whose gun is
/// `excluded` under its attachments.
pub fn player_max(
    ps: &T6WeaponPs,
    facts: &FactsTable,
    max: &dyn Fn(u32) -> i32,
    w: u32,
    excluded: u32,
) -> i32 {
    let c = facts.get(w).core;
    if c.clip_only {
        return c.clip_size;
    }
    let pool = facts.ammo_pool_weapon(w);
    let mut total = 0;
    for h in held_list(ps) {
        if h == excluded || facts.ammo_pool_weapon(h) != pool {
            continue;
        }
        let hc = facts.get(h).core;
        if hc.inventory_type == inventory::ALTMODE
            && excluded != 0
            && same_base(facts, hc.alt_weapon, excluded)
        {
            continue;
        }
        total += max(h);
    }
    total
}

/// BG_GetMaxPickupableAmmo: room left in the magazine (clip-only) or the reserve.
pub fn max_pickupable(
    ps: &T6WeaponPs,
    facts: &FactsTable,
    max: &dyn Fn(u32) -> i32,
    w: u32,
) -> i32 {
    if facts.get(w).core.clip_only {
        facts.get(w).core.clip_size - clip(ps, facts, w)
    } else {
        player_max(ps, facts, max, w, 0) - stock(ps, facts, w)
    }
}

fn pool_matches(facts: &FactsTable, h: u32, w: u32) -> bool {
    let (ph, pw) = (facts.ammo_pool_weapon(h), facts.ammo_pool_weapon(w));
    ph == pw || same_base(facts, ph, pw)
}

/// BG_PlayerHasCompatibleWeapon: a held gun draws from `w`'s reserve, or from one of the same
/// root gun.
pub fn has_compatible(ps: &T6WeaponPs, facts: &FactsTable, w: u32) -> bool {
    held_list(ps).into_iter().any(|h| pool_matches(facts, h, w))
}

/// HaveRoomForAmmo: a held gun of `w`'s reserve still has room.
pub fn has_room_for_ammo(
    ps: &T6WeaponPs,
    facts: &FactsTable,
    max: &dyn Fn(u32) -> i32,
    w: u32,
) -> bool {
    held_list(ps)
        .into_iter()
        .any(|h| pool_matches(facts, h, w) && max_pickupable(ps, facts, max, h) > 0)
}

/// WeaponPickup's compatible gun: `w` itself when held, else the first held gun of the same
/// root, else none.
pub fn compatible_weapon(ps: &T6WeaponPs, facts: &FactsTable, w: u32) -> u32 {
    if ps.has_weapon(w) {
        return w;
    }
    held_list(ps)
        .into_iter()
        .find(|&h| same_base(facts, h, w))
        .unwrap_or(0)
}

fn is_frag(facts: &FactsTable, w: u32) -> bool {
    facts.get(w).core.offhand_class == offhand_class::FRAG_GRENADE
}

/// The dropped gun's second ammo slot: its left-hand gun when dual wield, else its alternate.
pub fn slot2_weapon(facts: &FactsTable, w: u32) -> u32 {
    let c = facts.get(w).core;
    if c.dual_wield {
        c.dual_wield_weapon
    } else {
        c.alt_weapon
    }
}

/// PlayerHasAnyAmmoToTransferToWeapon: a round in the magazine, or reserve beyond what the
/// player's other guns of that reserve can carry.
pub fn has_ammo_to_transfer(
    ps: &T6WeaponPs,
    facts: &FactsTable,
    max: &dyn Fn(u32) -> i32,
    w: u32,
) -> bool {
    clip(ps, facts, w) >= 1 || stock(ps, facts, w) - player_max(ps, facts, max, w, w) > 0
}

/// TransferPlayerAmmoToWeaponEntity: each slot takes the gun's magazine and the reserve the
/// player's other guns cannot carry. The player's own rows are left as they are (the take that
/// follows clears them).
pub fn transfer(ps: &T6WeaponPs, facts: &FactsTable, max: &dyn Fn(u32) -> i32, w: u32) -> ItemAmmo {
    let mut out = [ItemSlot::default(); 2];
    for (slot, ws) in out.iter_mut().zip([w, slot2_weapon(facts, w)]) {
        if ws == 0 {
            continue;
        }
        *slot = ItemSlot {
            weapon: ws,
            clip: clip(ps, facts, ws),
            stock: (stock(ps, facts, ws) - player_max(ps, facts, max, ws, ws)).max(0),
        };
    }
    out
}

/// ThrowDownWeapon's decision for `w`: an alternate mode throws its gun; a gun with no ammo to
/// carry (or a clip-only gun with an empty magazine) is only taken.
pub fn throw_down(
    ps: &T6WeaponPs,
    facts: &FactsTable,
    max: &dyn Fn(u32) -> i32,
    w: u32,
) -> ThrowDown {
    let mut w = w;
    let c = facts.get(w).core;
    if c.inventory_type == inventory::ALTMODE {
        if c.alt_weapon == 0 {
            return ThrowDown::Nothing;
        }
        w = c.alt_weapon;
    }
    if !ps.has_weapon(w) || !has_ammo_to_transfer(ps, facts, max, w) {
        return ThrowDown::TakeOnly(w);
    }
    if facts.get(w).core.clip_only && clip(ps, facts, w) == 0 {
        return ThrowDown::TakeOnly(w);
    }
    ThrowDown::Drop(w, transfer(ps, facts, max, w))
}

/// Fill_Clip: the magazine tops up from the reserve.
fn fill_clip(clip_size: i32, clip: &mut i32, pool: &mut i32) {
    let n = (clip_size - *clip).min(*pool).max(0);
    *pool -= n;
    *clip += n;
}

/// Add_Ammo(w, count, fill): `count` rounds go into `w`'s reserve, up to what the player can
/// carry (`fill` tops up the magazine first; a clip-only gun's rounds all go to the magazine).
/// Returns the rounds that went in.
pub fn add_ammo(
    ps: &mut T6WeaponPs,
    facts: &FactsTable,
    max: &dyn Fn(u32) -> i32,
    w: u32,
    count: i32,
    fill: bool,
) -> i32 {
    if !ps.has_weapon(w) && !has_compatible(ps, facts, w) && !is_frag(facts, w) {
        return 0;
    }
    let c = facts.get(w).core;
    let stock0 = stock(ps, facts, w);
    let clip0 = clip(ps, facts, w);
    let cap = player_max(ps, facts, max, w, 0);
    let mut pool = (stock0 + count).max(0);
    let mut clip_n = clip0;
    if c.clip_only {
        fill_clip(c.clip_size, &mut clip_n, &mut pool);
        pool = 0;
    } else {
        if fill {
            fill_clip(c.clip_size, &mut clip_n, &mut pool);
        }
        pool = pool.min(cap);
    }
    clip_n = clip_n.min(c.clip_size);
    set_clip(ps, facts, w, clip_n);
    set_stock(ps, facts, w, pool);
    clip_n + pool - clip0 - stock0
}

/// WeaponPickup_LeechFromWeaponEnt: walking over a gun takes its reserve into the player's gun
/// of that kind (and its magazine too when `exact`, the player holding that very gun). The
/// item's slots keep what is left.
pub fn leech(
    ps: &mut T6WeaponPs,
    facts: &FactsTable,
    max: &dyn Fn(u32) -> i32,
    item: &mut ItemAmmo,
    exact: bool,
) -> Leech {
    let mut any_left = false;
    let mut added_any = false;
    let mut grenade = false;
    for s in item.iter_mut() {
        if s.weapon == 0 {
            continue;
        }
        let compat = compatible_weapon(ps, facts, s.weapon);
        if compat == 0 {
            continue;
        }
        grenade |= facts.get(s.weapon).core.weap_type == weap_type::GRENADE;
        let amount = s.stock + if exact { s.clip } else { 0 };
        let added = add_ammo(ps, facts, max, compat, amount, false);
        if added == 0 {
            any_left |= s.stock >= 1 || s.clip > 0;
            continue;
        }
        added_any = true;
        s.stock -= added;
        if s.stock >= 0 {
            any_left |= s.stock >= 1 || s.clip > 0;
        } else {
            s.clip += s.stock;
            s.stock = 0;
            if s.clip < 0 {
                s.clip = 0;
            } else {
                any_left |= s.clip > 0;
            }
        }
    }
    Leech {
        added: added_any,
        take: (grenade && !any_left) || (exact && added_any),
    }
}

/// WeaponEntCanBeGrabbed for one weapon: a frag, or a gun the player holds or has the reserve
/// for, with room for more; or a scavenger pack for a player with the perk.
fn ent_can_be_grabbed(
    ps: &T6WeaponPs,
    facts: &FactsTable,
    max: &dyn Fn(u32) -> i32,
    w: u32,
) -> bool {
    if w == 0 {
        return false;
    }
    (is_frag(facts, w) || ps.has_weapon(w) || has_compatible(ps, facts, w))
        && has_room_for_ammo(ps, facts, max, w)
}

/// BG_CanItemBeGrabbed's first gate: with the weapons taken away (disableweapons) nothing on
/// the ground can be touched or picked up.
pub fn weapons_disabled(ps: &T6WeaponPs) -> bool {
    ps.weap_flags & wf::DISABLE_SWITCH_TO_NONE != 0
}

/// BG_CanItemBeGrabbed on the use path (WeaponEntCanBeGrabbed with no touch): the gun, or its
/// alternate mode, is one the player does not hold. The caller has already refused the
/// player's own drop and disabled weapons.
pub fn use_can_be_grabbed(ps: &T6WeaponPs, facts: &FactsTable, w: u32) -> bool {
    let alt = facts.get(w).core.alt_weapon;
    (w != 0 && !ps.has_weapon(w)) || (alt != 0 && !ps.has_weapon(alt))
}

/// BG_CanItemBeGrabbed for a dropped gun on the touch path (the caller has already refused the
/// player's own drop and disabled weapons): the gun or its alternate mode can be leeched, or it
/// is a scavenger pack and he has the perk.
pub fn can_be_grabbed(
    ps: &T6WeaponPs,
    facts: &FactsTable,
    max: &dyn Fn(u32) -> i32,
    item: &ItemAmmo,
    scavenger: bool,
    has_perk: bool,
) -> bool {
    if scavenger {
        return has_perk;
    }
    let w = item[0].weapon;
    ent_can_be_grabbed(ps, facts, max, w)
        || ent_can_be_grabbed(ps, facts, max, facts.get(w).core.alt_weapon)
}

/// WeaponPickup_Touch's gate on a plain gun: the player holds it, has its reserve, or it is a
/// frag. Returns whether the touch is exact (he holds that very gun).
pub fn touch_gate(ps: &T6WeaponPs, facts: &FactsTable, w: u32) -> Option<bool> {
    let exact = ps.has_weapon(w);
    if !exact && !has_compatible(ps, facts, w) && !is_frag(facts, w) {
        return None;
    }
    Some(exact)
}

/// WeaponPickup's killstreak gate: while a carried killstreak is in his hands, only ammo for a
/// gun he has the reserve for is taken.
pub fn killstreak_blocks(ps: &T6WeaponPs, facts: &FactsTable, w: u32) -> bool {
    facts.get(ps.weapon).core.carried_killstreak && !has_compatible(ps, facts, w)
}

/// BG_PlayerWeaponsFull_Primaries: two primaries held (carried killstreaks do not count).
pub fn primaries_full(ps: &T6WeaponPs, facts: &FactsTable) -> bool {
    held_list(ps)
        .into_iter()
        .filter(|&h| {
            let c = facts.get(h).core;
            c.inventory_type == inventory::PRIMARY && !c.carried_killstreak
        })
        .count()
        > 1
}

/// CurrentPrimaryWeapon: the primary in his hands (an alternate mode counts as its gun).
pub fn current_primary(ps: &T6WeaponPs, facts: &FactsTable) -> u32 {
    let mut w = ps.weapon;
    let c = facts.get(w).core;
    if c.inventory_type == inventory::ALTMODE {
        w = c.alt_weapon;
    }
    if ps.has_weapon(w) && facts.get(w).core.inventory_type == inventory::PRIMARY {
        w
    } else {
        0
    }
}

/// WeaponPickup_AddWeapon's swap: with both primaries full, a held melee primary goes first,
/// else the primary in his hands. 0 when there is nothing to swap (the pickup fails).
pub fn swap_target(ps: &T6WeaponPs, facts: &FactsTable) -> u32 {
    held_list(ps)
        .into_iter()
        .find(|&h| {
            let c = facts.get(h).core;
            c.inventory_type == inventory::PRIMARY && c.weap_type == weap_type::MELEE
        })
        .unwrap_or_else(|| current_primary(ps, facts))
}

/// Whether picking up `w` needs a swap: a primary while both primaries are full.
pub fn needs_swap(ps: &T6WeaponPs, facts: &FactsTable, w: u32) -> bool {
    facts.get(w).core.inventory_type == inventory::PRIMARY && primaries_full(ps, facts)
}

/// WeaponPickup_AddWeapon's refusal: the hands hold a gun that is not in the held list.
pub fn add_weapon_refused(ps: &T6WeaponPs) -> bool {
    ps.weapon != 0 && !ps.has_weapon(ps.weapon)
}

/// WeaponPickup_AddAmmoForNewWeapon: each slot's magazine is set (a -1 magazine fills from the
/// stock instead), then its stock goes in.
pub fn add_ammo_for_new_weapon(
    ps: &mut T6WeaponPs,
    facts: &FactsTable,
    max: &dyn Fn(u32) -> i32,
    item: &ItemAmmo,
) {
    for s in item {
        if s.weapon == 0 || !ps.has_weapon(s.weapon) {
            continue;
        }
        if s.clip >= 0 {
            set_clip(ps, facts, s.weapon, s.clip);
        }
        add_ammo(ps, facts, max, s.weapon, s.stock, s.clip == -1);
    }
}

/// WeaponPickup_Grab's raise: the new gun comes up unless it is a grenade or a mine that goes
/// in an offhand slot.
pub fn grab_selects(facts: &FactsTable, w: u32) -> bool {
    let c = facts.get(w).core;
    !((c.weap_type == weap_type::GRENADE || c.weap_type == weap_type::MINE)
        && c.offhand_class != offhand_class::NONE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facts::T6WeaponFacts;

    const AN94: u32 = 1;
    const AN94_GL: u32 = 2;
    const KNIFE: u32 = 3;
    const FIVE7: u32 = 4;
    const FIVE7_LH: u32 = 5;
    const FRAG: u32 = 6;
    const AN94_FMJ: u32 = 7;
    const MP7: u32 = 8;
    const BALLISTIC: u32 = 9;

    fn row(f: impl FnOnce(&mut crate::facts::CoreFacts)) -> T6WeaponFacts {
        let mut t = T6WeaponFacts::default();
        t.core.clip_size = 30;
        f(&mut t.core);
        t
    }

    fn table() -> FactsTable {
        let mut t = FactsTable::new();
        t.set(AN94, "an94_mp", row(|c| c.alt_weapon = AN94_GL));
        t.set(
            AN94_GL,
            "gl_an94_mp",
            row(|c| {
                c.inventory_type = inventory::ALTMODE;
                c.ignore_attachments = true;
                c.alt_weapon = AN94;
                c.clip_size = 1;
            }),
        );
        t.set(
            KNIFE,
            "knife_mp",
            row(|c| c.inventory_type = inventory::MELEE),
        );
        t.set(
            FIVE7,
            "fiveseven_dw_mp",
            row(|c| {
                c.dual_wield = true;
                c.dual_wield_weapon = FIVE7_LH;
                c.clip_size = 20;
            }),
        );
        t.set(
            FIVE7_LH,
            "fiveseven_lh_mp",
            row(|c| {
                c.inventory_type = inventory::DWLEFTHAND;
                c.dual_wield = true;
                c.dual_wield_weapon = FIVE7;
                c.clip_size = 20;
            }),
        );
        t.set(
            FRAG,
            "frag_grenade_mp",
            row(|c| {
                c.inventory_type = inventory::OFFHAND;
                c.weap_type = weap_type::GRENADE;
                c.offhand_class = offhand_class::FRAG_GRENADE;
                c.clip_only = true;
                c.clip_size = 2;
            }),
        );
        t.set(AN94_FMJ, "an94_mp+fmj", row(|_| {}));
        t.set(MP7, "mp7_mp", row(|_| {}));
        t.set(
            BALLISTIC,
            "knife_ballistic_mp",
            row(|c| {
                c.weap_type = weap_type::MELEE;
                c.clip_size = 1;
            }),
        );
        t
    }

    fn max(w: u32) -> i32 {
        match w {
            AN94 | AN94_FMJ | MP7 => 120,
            AN94_GL => 1,
            FIVE7 | FIVE7_LH => 80,
            FRAG => 2,
            BALLISTIC => 2,
            _ => 0,
        }
    }

    fn holding(guns: &[(u32, i32, i32)]) -> T6WeaponPs {
        let mut ps = T6WeaponPs::default();
        for &(w, c, s) in guns {
            ps.give(w);
            ps.set_clip_slot(w, c);
            ps.set_pool_slot(w, s);
        }
        ps
    }

    #[test]
    fn a_dropped_gun_carries_its_magazine_and_reserve_and_its_launcher() {
        let facts = table();
        let ps = holding(&[(AN94, 25, 90), (AN94_GL, 1, 1), (KNIFE, 0, 0)]);
        let ammo = transfer(&ps, &facts, &max, AN94);
        assert_eq!(
            ammo[0],
            ItemSlot {
                weapon: AN94,
                clip: 25,
                stock: 90
            }
        );
        assert_eq!(
            ammo[1],
            ItemSlot {
                weapon: AN94_GL,
                clip: 1,
                stock: 1
            }
        );
        assert_eq!(
            throw_down(&ps, &facts, &max, AN94_GL),
            ThrowDown::Drop(AN94, ammo)
        );
    }

    #[test]
    fn a_dual_wield_drop_carries_the_left_magazine_and_only_the_spare_reserve() {
        let facts = table();
        // Both hands draw from the right gun's reserve: each slot keeps what the other gun's
        // max leaves over.
        let ps = holding(&[(FIVE7, 18, 100), (FIVE7_LH, 12, 0)]);
        let ammo = transfer(&ps, &facts, &max, FIVE7);
        assert_eq!(
            ammo[0],
            ItemSlot {
                weapon: FIVE7,
                clip: 18,
                stock: 20
            }
        );
        assert_eq!(
            ammo[1],
            ItemSlot {
                weapon: FIVE7_LH,
                clip: 12,
                stock: 20
            }
        );
    }

    #[test]
    fn an_empty_gun_is_only_taken() {
        let facts = table();
        let ps = holding(&[(MP7, 0, 0)]);
        assert_eq!(throw_down(&ps, &facts, &max, MP7), ThrowDown::TakeOnly(MP7));
        let ps = holding(&[(FRAG, 0, 0)]);
        assert_eq!(
            throw_down(&ps, &facts, &max, FRAG),
            ThrowDown::TakeOnly(FRAG)
        );
        // Not held at all.
        assert_eq!(throw_down(&ps, &facts, &max, MP7), ThrowDown::TakeOnly(MP7));
    }

    #[test]
    fn the_carry_cap_counts_every_gun_of_the_reserve_but_the_excluded_one() {
        let facts = table();
        let ps = holding(&[(FIVE7, 20, 0), (FIVE7_LH, 20, 0)]);
        assert_eq!(player_max(&ps, &facts, &max, FIVE7, 0), 160);
        assert_eq!(player_max(&ps, &facts, &max, FIVE7, FIVE7), 80);
        // The launcher is left out with its gun.
        let ps = holding(&[(AN94, 0, 0), (AN94_GL, 0, 0)]);
        assert_eq!(player_max(&ps, &facts, &max, AN94_GL, AN94), 0);
        assert_eq!(player_max(&ps, &facts, &max, AN94_GL, 0), 1);
        // Clip-only: the magazine.
        assert_eq!(player_max(&ps, &facts, &max, FRAG, 0), 2);
    }

    #[test]
    fn add_ammo_caps_the_reserve_and_fill_tops_the_magazine() {
        let facts = table();
        let mut ps = holding(&[(MP7, 10, 100)]);
        assert_eq!(add_ammo(&mut ps, &facts, &max, MP7, 50, false), 20);
        assert_eq!((clip(&ps, &facts, MP7), stock(&ps, &facts, MP7)), (10, 120));
        let mut ps = holding(&[(MP7, 10, 0)]);
        assert_eq!(add_ammo(&mut ps, &facts, &max, MP7, 50, true), 50);
        assert_eq!((clip(&ps, &facts, MP7), stock(&ps, &facts, MP7)), (30, 30));
        // A clip-only grenade: the rounds go to the magazine, up to its size.
        let mut ps = holding(&[(FRAG, 1, 0)]);
        assert_eq!(add_ammo(&mut ps, &facts, &max, FRAG, 3, false), 1);
        assert_eq!((clip(&ps, &facts, FRAG), stock(&ps, &facts, FRAG)), (2, 0));
        // Not held, nothing like it held: nothing.
        let mut ps = holding(&[(KNIFE, 0, 0)]);
        assert_eq!(add_ammo(&mut ps, &facts, &max, MP7, 50, false), 0);
    }

    #[test]
    fn walking_over_his_own_gun_takes_the_magazine_too_and_the_gun_goes() {
        let facts = table();
        let mut ps = holding(&[(MP7, 30, 40)]);
        let mut item = [
            ItemSlot {
                weapon: MP7,
                clip: 30,
                stock: 20,
            },
            ItemSlot::default(),
        ];
        let exact = touch_gate(&ps, &facts, MP7).unwrap();
        assert!(exact);
        let l = leech(&mut ps, &facts, &max, &mut item, exact);
        assert_eq!(
            l,
            Leech {
                added: true,
                take: true
            }
        );
        assert_eq!(stock(&ps, &facts, MP7), 90);
        assert_eq!(
            item[0],
            ItemSlot {
                weapon: MP7,
                clip: 0,
                stock: 0
            }
        );
    }

    #[test]
    fn a_compatible_gun_gives_only_its_reserve_and_stays() {
        let facts = table();
        // He holds the plain AN-94; the dropped one has FMJ.
        let mut ps = holding(&[(AN94, 30, 100)]);
        let mut item = [
            ItemSlot {
                weapon: AN94_FMJ,
                clip: 30,
                stock: 60,
            },
            ItemSlot::default(),
        ];
        let exact = touch_gate(&ps, &facts, AN94_FMJ).unwrap();
        assert!(!exact);
        let l = leech(&mut ps, &facts, &max, &mut item, exact);
        assert_eq!(
            l,
            Leech {
                added: true,
                take: false
            }
        );
        assert_eq!(stock(&ps, &facts, AN94), 120);
        assert_eq!(
            item[0],
            ItemSlot {
                weapon: AN94_FMJ,
                clip: 30,
                stock: 40
            }
        );
        // A full reserve takes nothing, and the gun can no longer be grabbed.
        assert!(!can_be_grabbed(&ps, &facts, &max, &item, false, false));
        let l = leech(&mut ps, &facts, &max, &mut item, exact);
        assert_eq!(
            l,
            Leech {
                added: false,
                take: false
            }
        );
    }

    #[test]
    fn a_grenade_emptied_by_a_touch_goes() {
        let facts = table();
        let mut ps = holding(&[(FRAG, 1, 0)]);
        let mut item = [
            ItemSlot {
                weapon: FRAG,
                clip: 0,
                stock: 1,
            },
            ItemSlot::default(),
        ];
        let l = leech(&mut ps, &facts, &max, &mut item, false);
        assert_eq!(
            l,
            Leech {
                added: true,
                take: true
            }
        );
        assert_eq!(clip(&ps, &facts, FRAG), 2);
    }

    #[test]
    fn an_unrelated_gun_is_not_touched() {
        let facts = table();
        let ps = holding(&[(AN94, 30, 0)]);
        assert_eq!(touch_gate(&ps, &facts, MP7), None);
        let item = [
            ItemSlot {
                weapon: MP7,
                clip: 30,
                stock: 60,
            },
            ItemSlot::default(),
        ];
        assert!(!can_be_grabbed(&ps, &facts, &max, &item, false, false));
        assert!(can_be_grabbed(&ps, &facts, &max, &item, true, true));
        assert!(!can_be_grabbed(&ps, &facts, &max, &item, true, false));
    }

    #[test]
    fn the_swap_takes_a_melee_primary_first_then_the_gun_in_hand() {
        let facts = table();
        let mut ps = holding(&[(AN94, 30, 0), (MP7, 30, 0), (KNIFE, 0, 0)]);
        ps.weapon = MP7;
        assert!(primaries_full(&ps, &facts));
        assert!(needs_swap(&ps, &facts, AN94_FMJ));
        assert_eq!(swap_target(&ps, &facts), MP7);
        // The launcher in hand swaps its gun.
        ps.give(AN94_GL);
        ps.weapon = AN94_GL;
        assert_eq!(swap_target(&ps, &facts), AN94);
        let mut ps = holding(&[(AN94, 30, 0), (BALLISTIC, 1, 0)]);
        ps.weapon = AN94;
        assert_eq!(swap_target(&ps, &facts), BALLISTIC);
        // Knife in hand, no melee primary: nothing to swap.
        let mut ps = holding(&[(AN94, 30, 0), (MP7, 30, 0), (KNIFE, 0, 0)]);
        ps.weapon = KNIFE;
        assert_eq!(swap_target(&ps, &facts), 0);
    }

    #[test]
    fn a_picked_up_gun_gets_the_items_ammo() {
        let facts = table();
        let mut ps = holding(&[(MP7, 0, 0)]);
        let item = [
            ItemSlot {
                weapon: MP7,
                clip: 12,
                stock: 200,
            },
            ItemSlot::default(),
        ];
        add_ammo_for_new_weapon(&mut ps, &facts, &max, &item);
        assert_eq!((clip(&ps, &facts, MP7), stock(&ps, &facts, MP7)), (12, 120));
        // A -1 magazine fills from the stock.
        let mut ps = holding(&[(MP7, 0, 0)]);
        let item = [
            ItemSlot {
                weapon: MP7,
                clip: -1,
                stock: 50,
            },
            ItemSlot::default(),
        ];
        add_ammo_for_new_weapon(&mut ps, &facts, &max, &item);
        assert_eq!((clip(&ps, &facts, MP7), stock(&ps, &facts, MP7)), (30, 20));
        assert!(grab_selects(&facts, MP7));
        assert!(!grab_selects(&facts, FRAG));
    }

    #[test]
    fn disabled_weapons_stop_every_pickup() {
        let mut ps = holding(&[(MP7, 30, 0)]);
        assert!(!weapons_disabled(&ps));
        ps.weap_flags |= wf::DISABLE_SWITCH_TO_NONE;
        assert!(weapons_disabled(&ps));
    }

    #[test]
    fn using_a_gun_needs_one_he_does_not_hold() {
        let facts = table();
        let ps = holding(&[(MP7, 30, 0)]);
        assert!(use_can_be_grabbed(&ps, &facts, AN94));
        assert!(!use_can_be_grabbed(&ps, &facts, MP7));
        // Holding the gun but not its launcher: the launcher can still be taken.
        let ps = holding(&[(AN94, 30, 0)]);
        assert!(use_can_be_grabbed(&ps, &facts, AN94));
        let ps = holding(&[(AN94, 30, 0), (AN94_GL, 1, 0)]);
        assert!(!use_can_be_grabbed(&ps, &facts, AN94));
        assert!(!use_can_be_grabbed(&ps, &facts, 0));
    }

    #[test]
    fn touching_a_dual_wield_gun_checks_the_gun_not_its_left_hand() {
        let facts = table();
        // BO2 checks the gun and its alternate mode, never the left-hand slot.
        let ps = holding(&[(MP7, 30, 0)]);
        let item = [
            ItemSlot {
                weapon: FIVE7,
                clip: 20,
                stock: 40,
            },
            ItemSlot {
                weapon: FIVE7_LH,
                clip: 20,
                stock: 0,
            },
        ];
        assert!(!can_be_grabbed(&ps, &facts, &max, &item, false, false));
        let ps = holding(&[(FIVE7, 10, 0)]);
        assert!(can_be_grabbed(&ps, &facts, &max, &item, false, false));
    }
}
