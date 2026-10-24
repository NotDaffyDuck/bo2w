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
use crate::player::{Hand, T6WeaponPs};

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
