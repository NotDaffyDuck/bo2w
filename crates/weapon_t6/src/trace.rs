//! The frame trace and scripted input runner: drives [`tick::run`] from a small text script and
//! prints one line per tick (both hands' state, timers and anim, then what the tick raised).
//! Mechanic windows prove their work with it before it goes in game.
//!
//! Script, one command per line (`#` starts a comment):
//!
//! | command | effect |
//! |---|---|
//! | `msec N` | tick length in ms (default 50) |
//! | `server on\|off` | prints go out (server) or not (client) |
//! | `spawn NAME` | give NAME, hold it, select it, READY, every held gun ever-had (a spawned player) |
//! | `give NAME` | add NAME to the held weapons with a full clip |
//! | `take NAME` | remove NAME from the held weapons |
//! | `select NAME` | the weapon the player asks for (`none` = 0) |
//! | `clip NAME N` / `stock NAME N` | set ammo |
//! | `hold B...` / `release B...` | buttons: attack ads melee reload use sprint frag smoke prone crouch jump breath offhand_switch |
//! | (sprint) | sprint held with `stick` forward past 105 sets the sprint flag; a breaker (fire, reload, jump, crouch...) clears it (`moves::update_sprint_flag`) |
//! | `perk N` / `noperk N` | BO2 perk number on/off |
//! | `input pad\|mouse\|keyboard` | the last input device (dual wield: which trigger fires which gun) |
//! | `zombies on\|off` | Zombies session rules |
//! | `pmtype N` / `pmflags HEX` | movement values the tick reads |
//! | `stick F R` | forwardmove / rightmove (-127..127) |
//! | `water N` | waterlevel |
//! | `jumptime N` | jump time (below 0 = a landed dive sliding) |
//! | `slidenext` | a slide starts on the next tick |
//! | `dtpend` | the last dive ended now |
//! | `console 0\|1` / `vehicle 0\|1` | console rules / in a vehicle (default 0) |
//! | `lunge ENT DIST` / `lunge none` | the client's melee lunge target |
//! | `ground 0\|1` / `plantok 0\|1` / `tacok 0\|1` | world answers the game fills (default 1) |
//! | `blocked 0\|1` | riot shield spot refused (default 0) |
//! | `run N` | run N ticks, printing each |
//! | `everhad NAME 0\|1` | BO2's ever-had flag (`give` leaves it clear: the next raise is a first raise) |
//! | `offhand primary\|secondary NAME` | the lethal / tactical slot |
//! | `expect state NAME` / `expect left NAME` / `expect anim NAME` / `expect weapon NAME` / `expect leftanim NAME` | check, FAIL line if not |
//! | `expect clip NAME N` / `expect stock NAME N` | rounds in NAME's clip / reserve (a left-hand gun shows its partner's pool) |
//! | `expect event NAME` | the last tick raised event NAME (`EV_` optional) |

use crate::cmd::{T6Cmd, button};
use crate::env::T6Env;
use crate::facts::{FactsTable, T6WeaponFacts};
use crate::player::T6WeaponPs;
use crate::pm::FrameOut;
use crate::{anim, events, state, tick};

/// What a script run printed and how many `expect` lines failed.
#[derive(Clone, Debug, Default)]
pub struct TraceResult {
    pub lines: Vec<String>,
    pub failures: usize,
}

/// The player, input and world one script drives.
pub struct Runner<'f> {
    pub ps: T6WeaponPs,
    pub env: T6Env,
    pub cmd: T6Cmd,
    pub facts: &'f FactsTable,
    /// Ms since the script started.
    pub now: i32,
    out: TraceResult,
    /// The last tick's event ids, for `expect event`.
    last_events: Vec<u16>,
    /// BO2's "let go of sprint first" latch (the sprint button drives the sprint flag here).
    sprint_up_required: bool,
}

fn button_bit(name: &str) -> Option<u32> {
    Some(match name.to_ascii_lowercase().as_str() {
        "attack" | "fire" => button::ATTACK,
        "ads" => button::ADS,
        "melee" => button::MELEE,
        "reload" => button::RELOAD,
        "use" => button::USE,
        "use_reload" => button::USE_RELOAD,
        "sprint" => button::SPRINT,
        "frag" => button::FRAG,
        "smoke" => button::SMOKE,
        "prone" => button::PRONE,
        "crouch" => button::CROUCH,
        "jump" => button::JUMP,
        "breath" => button::BREATH,
        "offhand_switch" => button::OFFHAND_SWITCH,
        "toggle_ads" => button::TOGGLE_ADS,
        "change_zoom" => button::CHANGE_ZOOM,
        _ => return None,
    })
}

impl<'f> Runner<'f> {
    pub fn new(facts: &'f FactsTable) -> Self {
        Self {
            ps: T6WeaponPs::default(),
            env: T6Env {
                is_server: true,
                view_height_settled: true,
                on_ground: true,
                plant_spot_ok: true,
                tac_insert_allowed: true,
                other_flags: crate::env::of::WEAPON_USABLE,
                ..T6Env::default()
            },
            cmd: T6Cmd {
                msec: 50,
                ..T6Cmd::default()
            },
            facts,
            now: 0,
            out: TraceResult::default(),
            last_events: Vec::new(),
            sprint_up_required: false,
        }
    }

    fn weapon_id(&self, name: &str) -> Result<u32, String> {
        if name.eq_ignore_ascii_case("none") {
            return Ok(0);
        }
        self.facts
            .find(name)
            .ok_or_else(|| format!("no weapon named {name}"))
    }

    /// One tick, traced.
    pub fn step(&mut self) {
        self.now += self.cmd.msec;
        self.cmd.server_time = self.now;
        let mut out = tick::pre_weapon(&mut self.ps, &self.cmd, &mut self.env, self.facts);
        // The movement code's sprint update runs after the ADS flag, before the weapon.
        let mut pm = crate::pm::Pm {
            ps: &mut self.ps,
            cmd: &self.cmd,
            env: &mut self.env,
            facts: self.facts,
            out: &mut out,
            delay_fired: false,
        };
        crate::pm::moves::update_sprint_flag(&mut pm, &mut self.sprint_up_required, 0);
        let main = tick::run(&mut self.ps, &self.cmd, &mut self.env, self.facts);
        out.append(main);
        self.last_events = out.events.iter().map(|e| e.id).collect();
        // Next tick's previous command (BO2 oldcmd).
        self.cmd.old_buttons = self.cmd.buttons;
        self.out.lines.push(frame_line(self.facts, self.now, &self.ps, &out));
    }

    fn expect(&mut self, n: usize, words: &[&str]) -> Result<(), String> {
        let what = words.get(1).copied().ok_or("missing argument")?;
        let want = words.get(2).copied().ok_or("missing argument")?;
        if what == "event" {
            let want = want.trim_start_matches("EV_");
            let got: Vec<&str> = self
                .last_events
                .iter()
                .map(|&id| events::name(id))
                .collect();
            let pass = got.iter().any(|g| g.eq_ignore_ascii_case(want));
            self.check(n, what, pass, &got.join(","), want);
            return Ok(());
        }
        if what == "clip" || what == "stock" {
            let w = self.weapon_id(want)?;
            let n_want = words.get(3).copied().ok_or("missing argument")?;
            let got = if what == "clip" {
                self.ps.ammo_in_clip(self.facts, w)
            } else {
                self.ps.ammo_not_in_clip(self.facts, w)
            }
            .to_string();
            self.check(n, what, got == n_want, &got, n_want);
            return Ok(());
        }
        let got = match what {
            "state" => state::name(self.ps.weaponstate).to_owned(),
            "left" => state::name(self.ps.weaponstate_left).to_owned(),
            "anim" => anim::name(self.ps.weap_anim).to_owned(),
            "leftanim" => anim::name(self.ps.weap_anim_left).to_owned(),
            "weapon" if self.ps.weapon == 0 => "none".to_owned(),
            "weapon" => self.facts.name(self.ps.weapon).to_owned(),
            _ => return Err(format!("expect what? {what}")),
        };
        let pass = got.eq_ignore_ascii_case(want);
        self.check(n, what, pass, &got, want);
        Ok(())
    }

    fn check(&mut self, n: usize, what: &str, pass: bool, got: &str, want: &str) {
        if pass {
            self.out.lines.push(format!("ok   line {n}: {what} {want}"));
        } else {
            self.out.failures += 1;
            self.out
                .lines
                .push(format!("FAIL line {n}: {what} is {got}, want {want}"));
        }
    }

    /// Run one script line (`n` = its line number, for FAIL lines), already split into words.
    pub fn command(&mut self, n: usize, words: &[&str]) -> Result<(), String> {
        let arg = |i: usize| {
            words
                .get(i)
                .copied()
                .ok_or_else(|| "missing argument".to_owned())
        };
        let num = |i: usize| -> Result<i64, String> {
            let s = arg(i)?;
            let s = s.trim_start_matches("0x");
            let radix = if arg(i)?.starts_with("0x") { 16 } else { 10 };
            i64::from_str_radix(s, radix).map_err(|e| format!("{s}: {e}"))
        };
        match words[0] {
            "msec" => self.cmd.msec = num(1)? as i32,
            "server" => self.env.is_server = arg(1)? == "on",
            "spawn" => {
                let w = self.weapon_id(arg(1)?)?;
                self.give(w);
                self.ps.weapon = w;
                self.cmd.weapon = w;
                self.ps.weaponstate = state::READY;
                self.ps.weaponstate_left = state::READY;
                // A spawned player's first raise has marked every gun held.
                self.ps.set_ever_had_all(true);
            }
            "give" => {
                let w = self.weapon_id(arg(1)?)?;
                self.give(w);
            }
            "take" => {
                let w = self.weapon_id(arg(1)?)?;
                self.ps.take(w);
            }
            "select" => {
                // The client fills the alt-mode memory with the selection (CL_SetWeapon).
                let w = self.weapon_id(arg(1)?)?;
                self.cmd.weapon = w;
                self.cmd.last_weapon_alt_mode_switch =
                    crate::switch::ClientSwitch::cmd_alt_memory(&self.ps, self.facts, w);
            }
            "clip" => {
                let w = self.weapon_id(arg(1)?)?;
                self.ps.set_ammo_in_clip(self.facts, w, num(2)? as i32);
            }
            "stock" => {
                let w = self.weapon_id(arg(1)?)?;
                self.ps.set_ammo_not_in_clip(self.facts, w, num(2)? as i32);
            }
            "hold" | "release" => {
                for b in &words[1..] {
                    let bit = button_bit(b).ok_or_else(|| format!("no button {b}"))?;
                    if words[0] == "hold" {
                        self.cmd.buttons |= bit;
                    } else {
                        self.cmd.buttons &= !bit;
                    }
                }
            }
            "perk" | "noperk" => {
                let p = num(1)? as u32;
                if p >= 64 {
                    return Err(format!("perk {p} out of range"));
                }
                if words[0] == "perk" {
                    self.env.perks |= 1u64 << p;
                } else {
                    self.env.perks &= !(1u64 << p);
                }
            }
            "input" => {
                self.cmd.last_input = match arg(1)? {
                    "pad" => crate::cmd::last_input::GAMEPAD,
                    "mouse" => crate::cmd::last_input::MOUSECLICK,
                    "keyboard" => crate::cmd::last_input::KEYBOARD,
                    other => return Err(format!("input pad|mouse|keyboard, not {other}")),
                }
            }
            "zombies" => self.env.zombies = arg(1)? == "on",
            "pmtype" => self.env.pm_type = num(1)? as i32,
            "pmflags" => self.env.pm_flags = num(1)? as u32,
            "stick" => {
                self.cmd.forwardmove = num(1)?.clamp(-127, 127) as i8;
                self.cmd.rightmove = num(2)?.clamp(-127, 127) as i8;
            }
            "water" => self.env.waterlevel = num(1)? as i32,
            "jumptime" => self.env.jump_time = num(1)? as i32,
            // The slide starts on the next tick.
            "slidenext" => self.env.slide_time = self.now + self.cmd.msec,
            // The last dive ended at the current time.
            "dtpend" => self.env.last_dtp_end = self.now,
            "blocked" => self.env.riotshield_blocked = num(1)? != 0,
            "ground" => self.env.on_ground = num(1)? != 0,
            "plantok" => self.env.plant_spot_ok = num(1)? != 0,
            "tacok" => self.env.tac_insert_allowed = num(1)? != 0,
            "console" => self.env.is_console = num(1)? != 0,
            "vehicle" => self.env.in_vehicle = num(1)? != 0,
            "lunge" => {
                // lunge ENT DIST, or lunge none
                if arg(1)? == "none" {
                    self.cmd.melee_charge_ent = None;
                    self.cmd.melee_charge_dist = 0;
                } else {
                    self.cmd.melee_charge_ent = Some(num(1)? as u16);
                    self.cmd.melee_charge_dist = num(2)? as u8;
                }
            }
            "run" => {
                for _ in 0..num(1)? {
                    self.step();
                }
            }
            "expect" => self.expect(n, words)?,
            "everhad" => {
                let w = self.weapon_id(arg(1)?)?;
                self.ps.set_ever_had(w, num(2)? != 0);
            }
            "offhand" => {
                let w = self.weapon_id(arg(2)?)?;
                match arg(1)? {
                    "primary" => self.ps.offhand_primary = w,
                    "secondary" => self.ps.offhand_secondary = w,
                    o => return Err(format!("offhand primary|secondary, not {o}")),
                }
            }
            other => return Err(format!("unknown command {other}")),
        }
        Ok(())
    }

    fn give(&mut self, w: u32) {
        // BO2: a melee-inventory weapon becomes the melee weapon (the game's bridge does the same).
        if self.facts.get(w).core.inventory_type == crate::facts::inventory::MELEE {
            self.ps.melee_weapon = w;
            return;
        }
        if self.ps.give(w) {
            let clip = self.facts.get(w).core.clip_size;
            self.ps.set_ammo_in_clip(self.facts, w, clip);
        }
    }

    /// Run a whole script. Errors name the script line.
    pub fn run_script(mut self, script: &str) -> Result<TraceResult, String> {
        for (i, raw) in script.lines().enumerate() {
            let n = i + 1;
            let text = raw.split('#').next().unwrap_or("").trim();
            if text.is_empty() {
                continue;
            }
            self.out.lines.push(format!("> {text}"));
            let words: Vec<&str> = text.split_whitespace().collect();
            self.command(n, &words)
                .map_err(|e| format!("line {n}: {e}"))?;
        }
        Ok(self.out)
    }
}

/// A two-gun table for tests and for trying the runner without a dump. The times are round
/// test numbers, NOT BO2's: real traces load the facts dump from the fastfiles.
pub fn demo_facts() -> FactsTable {
    let mut t = FactsTable::new();
    for (i, name) in [(1u32, "demo_smg"), (2, "demo_pistol")] {
        let mut f = T6WeaponFacts::default();
        f.core.clip_size = 30;
        f.switch.raise = 500;
        f.switch.drop = 400;
        f.switch.first_raise = 1000;
        f.switch.quick_raise = 300;
        f.switch.quick_drop = 250;
        f.switch.alt_raise = 500;
        f.switch.alt_drop = 400;
        f.switch.empty_raise = 500;
        f.switch.empty_drop = 400;
        t.set(i, name, f);
    }
    t
}


/// One trace line for one tick: the right hand, the left hand, ADS columns, then what the tick
/// produced. The game writes the same line (BO2MP_GUN_TRACE) so a game trace and a runner trace
/// compare line by line.
pub fn frame_line(facts: &FactsTable, now: i32, ps: &T6WeaponPs, out: &FrameOut) -> String {
    let mut line = format!(
        "{:>6} {:<14} R {:<20} t{:>5} d{:>5} {:<16} | L {:<12} t{:>5} d{:>5} {:<10}",
        now,
        facts.name(ps.weapon),
        state::name(ps.weaponstate),
        ps.weapon_time,
        ps.weapon_delay,
        anim::name(ps.weap_anim),
        state::name(ps.weaponstate_left),
        ps.weapon_time_left,
        ps.weapon_delay_left,
        anim::name(ps.weap_anim_left),
    );
    let ads = crate::pm::ads::trace_columns(ps);
    if !ads.is_empty() {
        line.push_str(" | ads ");
        line.push_str(&ads);
    }
    for d in tick::describe(out) {
        line.push_str(" | ");
        line.push_str(&d);
    }
    line.trim_end().to_owned()
}

#[cfg(test)]
mod tests {
    use super::{Runner, demo_facts};

    #[test]
    fn switch_drops_raises_then_idles() {
        let facts = demo_facts();
        let script = "
            spawn demo_smg
            give demo_pistol
            run 2
            expect state READY
            select demo_pistol
            run 1
            expect state DROPPING
            run 8
            expect weapon demo_pistol
            run 20 # never held before: the first raise (1000 here)
            expect state READY
            expect anim IDLE
            select demo_smg
            run 9
            expect state RAISING
            expect anim RAISE
        ";
        let r = Runner::new(&facts).run_script(script).expect("script runs");
        let text = r.lines.join("\n");
        assert_eq!(r.failures, 0, "{text}");
        assert!(text.contains("RAISING"), "{text}");
    }

    #[test]
    fn ever_had_gun_takes_the_normal_raise() {
        let facts = demo_facts();
        let script = "
            spawn demo_smg
            give demo_pistol
            everhad demo_pistol 1
            expect clip demo_pistol 30
            select demo_pistol
            run 18 # drop 400 + raise 500 at 50 ms
            expect state RAISING
            run 1
            expect state READY
        ";
        let r = Runner::new(&facts).run_script(script).expect("script runs");
        assert_eq!(
            r.failures,
            0,
            "{}",
            r.lines.join(
                "
"
            )
        );
    }

    #[test]
    fn old_buttons_follow_the_last_tick() {
        let facts = demo_facts();
        let mut runner = Runner::new(&facts);
        runner.cmd.buttons = crate::cmd::button::ATTACK;
        assert!(runner.cmd.pressed(crate::cmd::button::ATTACK));
        runner.step();
        assert!(!runner.cmd.pressed(crate::cmd::button::ATTACK));
        assert!(runner.cmd.was_held(crate::cmd::button::ATTACK));
    }

    #[test]
    fn bad_command_names_the_line() {
        let facts = demo_facts();
        let e = Runner::new(&facts)
            .run_script("msec 50\nwiggle")
            .unwrap_err();
        assert!(e.starts_with("line 2:"), "{e}");
    }
}
