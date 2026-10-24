//! The per-tick context every mechanic hook gets, plus one file per mechanic window.
//!
//! Window 1 owns this file and the order the hooks run in ([`crate::tick`]). Each file below
//! belongs to its window: its facts struct, the reader for it, and its hooks. A hook starts as
//! a stub that does what BO2 does when that mechanic never triggers, so the frame runs (and
//! raise/drop/idle work) before the window fills it in. Change a hook's signature through
//! window 1; the body is the window's own.

pub mod ads; // window 10
pub mod dual; // window 8
pub mod fire; // window 3
pub mod melee; // window 6
pub mod moves; // window 9
pub mod offhand; // window 5
pub mod reload; // window 4
pub mod shield; // window 11
pub mod spread; // window 12

use crate::cmd::T6Cmd;
use crate::env::T6Env;
use crate::events::{AnimScript, Event};
use crate::facts::{FactsTable, T6WeaponFacts};
use crate::player::{Hand, T6WeaponPs, wf};
use crate::state;

/// Everything one weapon tick produced besides the player-state change, in BO2's order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FrameOut {
    pub events: Vec<Event>,
    pub scripts: Vec<AnimScript>,
    /// The server-only console lines BO2 prints ("end weapon (idle)" ...), kept for the trace.
    pub prints: Vec<&'static str>,
}

impl FrameOut {
    /// Add what a later step of the same frame produced.
    pub fn append(&mut self, later: FrameOut) {
        self.events.extend(later.events);
        self.scripts.extend(later.scripts);
        self.prints.extend(later.prints);
    }
}

/// `BG_GetViewmodelWeaponIndex`: the weapon in the arms. The offhand while one is in use
/// (PWF_USING_OFFHAND) in the offhand states, the melee weapon while a gun that is not itself
/// a melee weapon swings (MELEE_INIT, MELEE_FIRE), else the held weapon.
pub fn viewmodel_weapon(ps: &T6WeaponPs, facts: &FactsTable) -> u32 {
    let swinging = matches!(ps.weaponstate, state::MELEE_INIT | state::MELEE_FIRE);
    viewmodel_weapon_of(&ViewmodelWeaponInputs {
        using_offhand: ps.weap_flags & wf::USING_OFFHAND != 0,
        weaponstate: ps.weaponstate,
        weapon: ps.weapon,
        off_hand_weapon: ps.off_hand_weapon,
        melee_weapon: ps.melee_weapon,
        weapon_is_melee: swinging && facts.get(ps.weapon).melee.use_as_melee,
    })
}

/// What [`viewmodel_weapon_of`] reads, as plain values, for a caller without a [`T6WeaponPs`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ViewmodelWeaponInputs {
    /// PWF_USING_OFFHAND is set.
    pub using_offhand: bool,
    pub weaponstate: i32,
    pub weapon: u32,
    pub off_hand_weapon: u32,
    pub melee_weapon: u32,
    /// The held weapon is itself a melee weapon (its `use_as_melee`).
    pub weapon_is_melee: bool,
}

/// [`viewmodel_weapon`]'s rule (`BG_GetViewmodelWeaponIndex`) on plain values.
pub fn viewmodel_weapon_of(i: &ViewmodelWeaponInputs) -> u32 {
    let st = i.weaponstate;
    if i.using_offhand && (state::OFFHAND_INIT..=state::OFFHAND_END).contains(&st) {
        return i.off_hand_weapon;
    }
    if (st == state::MELEE_INIT || st == state::MELEE_FIRE) && !i.weapon_is_melee {
        return i.melee_weapon;
    }
    i.weapon
}

/// BO2's `pmove_t` + `pml_t`, cut down to what the weapon code touches.
pub struct Pm<'a> {
    pub ps: &'a mut T6WeaponPs,
    pub cmd: &'a T6Cmd,
    pub env: &'a mut T6Env,
    pub facts: &'a FactsTable,
    pub out: &'a mut FrameOut,
    /// The hand's delayed action fired this tick (WeaponTimeAdjust returned 1); set by
    /// [`crate::timing::time_adjust`].
    pub delay_fired: bool,
}

impl Pm<'_> {
    /// `bRunLeftGun`: this pass runs the left hand.
    pub fn left(&self) -> bool {
        self.ps.b_run_left_gun
    }

    /// The weapon in the hand being run: the dual-wield partner for the left hand.
    pub fn hand_weapon(&self) -> u32 {
        if self.left() {
            self.facts.get(self.ps.weapon).core.dual_wield_weapon
        } else {
            self.ps.weapon
        }
    }

    pub fn weapon_facts(&self) -> T6WeaponFacts {
        self.facts.get(self.ps.weapon)
    }

    pub fn hand_facts(&self) -> T6WeaponFacts {
        self.facts.get(self.hand_weapon())
    }

    /// `BG_HasPerk`: a player perk, or one the weapon in the arms grants
    /// (`BG_WeaponHasPerk` on [`viewmodel_weapon`]: its variant's or attachments' perks).
    pub fn has_perk(&self, perk: u32) -> bool {
        self.env.has_perk(perk) || self.facts.get(viewmodel_weapon(self.ps, self.facts)).spread.has_perk(perk)
    }

    /// The perks PM_Weapon_WeaponTimeAdjust reads: the player's plus the hand weapon's
    /// (`BG_GetPerks` on the gun this pass runs, the dual-wield partner for the left hand).
    pub fn hand_has_perk(&self, perk: u32) -> bool {
        self.env.has_perk(perk) || self.hand_facts().spread.has_perk(perk)
    }

    pub fn hand(&self) -> Hand {
        self.ps.hand(self.left())
    }

    pub fn set_hand(&mut self, h: Hand) {
        let left = self.left();
        self.ps.set_hand(left, h);
    }

    /// `PM_AddEvent`.
    pub fn add_event(&mut self, id: u16) {
        self.out.events.push(Event {
            id,
            parm: 0,
            predictable: false,
        });
    }

    /// `BG_AddPredictableEventToPlayerstate`.
    pub fn add_predictable_event(&mut self, id: u16, parm: u32) {
        self.out.events.push(Event {
            id,
            parm,
            predictable: true,
        });
    }

    /// A console line BO2 prints only on the server.
    pub fn server_print(&mut self, line: &'static str) {
        if self.env.is_server {
            self.out.prints.push(line);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewmodel_weapon_of_follows_bo2() {
        let base = ViewmodelWeaponInputs {
            weaponstate: state::READY,
            weapon: 5,
            off_hand_weapon: 9,
            melee_weapon: 7,
            ..ViewmodelWeaponInputs::default()
        };
        assert_eq!(viewmodel_weapon_of(&base), 5);
        let offhand = ViewmodelWeaponInputs { using_offhand: true, ..base };
        assert_eq!(viewmodel_weapon_of(&offhand), 5, "offhand bit outside the offhand states");
        for st in [state::OFFHAND_INIT, state::OFFHAND_END] {
            assert_eq!(viewmodel_weapon_of(&ViewmodelWeaponInputs { weaponstate: st, ..offhand }), 9);
        }
        for st in [state::MELEE_INIT, state::MELEE_FIRE] {
            let swing = ViewmodelWeaponInputs { weaponstate: st, ..base };
            assert_eq!(viewmodel_weapon_of(&swing), 7);
            assert_eq!(viewmodel_weapon_of(&ViewmodelWeaponInputs { weapon_is_melee: true, ..swing }), 5);
        }
        assert_eq!(viewmodel_weapon_of(&ViewmodelWeaponInputs { weaponstate: state::MELEE_END, ..base }), 5);
    }
}
