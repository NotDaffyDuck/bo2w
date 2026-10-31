//! vm_trace SCRIPT FACTS_DUMP ANIMS_DUMP: run the weapon brain on a gun_trace-style script and
//! play its anim requests on the viewmodel anim player ([`weapon_t6::viewmodel`]), printing
//! what each hand plays and every slot's weight and play position per tick.
//!
//! ANIMS_DUMP: one line per weapon, same order as the facts dump:
//! `name TAB slot:length_ms:loop:anim;...` (the slots the weapon names, from the zones).
//!
//! Commands: msec, spawn, give, take, select, clip, stock, hold, release, perk, noperk,
//! pmtype, run as in gun_trace, plus
//!   lefthand GUN LEFTGUN   dual wield: GUN's left hand plays LEFTGUN's anims
//!   anim NAME [STATE]      set the request (and state, run for the gun's time for that move)
//!                          by hand, for moves the brain does not drive yet (sprint)
//!   expect slot NAME / lslot NAME / state NAME / anim NAME / weapon NAME
//! Exit code 1 when an `expect` line fails, 2 on a script or file error.

use std::collections::HashMap;
use std::process::ExitCode;

use weapon_t6::anim;
use weapon_t6::facts::FactsTable;
use weapon_t6::state;
use weapon_t6::trace::ScenarioDriver;
use weapon_t6::viewmodel::{AnimSet, Clip, Viewmodel, VmDvars, VmFrame, VmTable, slot};

fn read_anims(text: &str) -> Vec<[Option<Clip>; slot::COUNT]> {
    let mut out = vec![[None; slot::COUNT]];
    for row in text.lines() {
        let mut clips = [None; slot::COUNT];
        if let Some((_slot, names)) = row.split_once('\t') {
            for item in names.split(';').filter(|s| !s.is_empty()) {
                let mut p = item.split(':');
                let (Some(s), Some(ms), Some(lp)) = (p.next(), p.next(), p.next()) else {
                    continue;
                };
                let (Ok(s), Ok(ms)) = (s.parse::<usize>(), ms.parse::<f32>()) else {
                    continue;
                };
                if s < slot::COUNT {
                    clips[s] = Some(Clip {
                        length_ms: ms,
                        looping: lp == "1",
                    });
                }
            }
        }
        out.push(clips);
    }
    out
}

struct Vm<'f> {
    r: ScenarioDriver<'f>,
    vm: Viewmodel,
    table: &'f VmTable,
    clips: &'f [[Option<Clip>; slot::COUNT]],
    left_of: HashMap<u32, u32>,
    sets: HashMap<u32, AnimSet>,
    dvars: VmDvars,
    log: Vec<String>,
    failures: usize,
}

impl Vm<'_> {
    fn id(&self, name: &str) -> Result<u32, String> {
        if name.eq_ignore_ascii_case("none") {
            return Ok(0);
        }
        self.r.facts.find(name).ok_or_else(|| format!("no weapon named {name}"))
    }

    fn set_for(&mut self, w: u32) -> AnimSet {
        if let Some(s) = self.sets.get(&w) {
            return s.clone();
        }
        let none = [None; slot::COUNT];
        let right = self.clips.get(w as usize).unwrap_or(&none);
        let left = self
            .left_of
            .get(&w)
            .and_then(|l| self.clips.get(*l as usize))
            .unwrap_or(&none);
        let s = AnimSet::build(right, left);
        self.sets.insert(w, s.clone());
        s
    }

    fn give(&mut self, w: u32) {
        if self.r.ps.give(w) {
            let clip = self.r.facts.get(w).core.magazine_capacity;
            self.r.ps.set_ammo_in_clip(self.r.facts, w, clip);
        }
    }

    fn step(&mut self) {
        self.r.step();
        let w = self.r.ps.weapon;
        let set = self.set_for(w);
        let player = &self.r.ps;
        let left_gun = self.left_of.get(&w).copied().unwrap_or(0);
        let f = VmFrame {
            weapon: w,
            model: w,
            gun_clip: player.gun_clip,
            weap_anim_left: player.weap_anim_left,
            gun_phase: player.gun_phase,
            weaponstate_left: player.weaponstate_left,
            gun_timer_ms: player.gun_timer_ms,
            weapon_time_left: player.weapon_time_left,
            ammo_in_clip: player.ammo_in_clip(self.r.facts, w),
            ammo_in_clip_left: player.ammo_in_clip(self.r.facts, left_gun),
            intro_shots_fired: player.intro_shots_fired,
            gun_raise_share: player.weapon_raise_fraction,
            perks: self.r.env.perks,
            off_hand_weapon: player.off_hand_weapon,
            movement_mode: self.r.env.movement_mode,
            step_duration_ms: self.r.cmd.frame_ms,
            dvars: self.dvars,
            ..VmFrame::default()
        };
        self.vm.frame(&f, self.table, &set);
        let mut pose = self.vm.pose();
        pose.sort_by(|a, b| b.1.total_cmp(&a.1));
        let slots: Vec<String> = pose
            .iter()
            .filter(|p| p.1 >= 0.005)
            .map(|(s, wt, t)| format!("{} {:.2}@{:.2}", slot::name(*s), wt, t))
            .collect();
        self.log.push(format!(
            "{:>6} {:<14} {:<16} {:<14} | R {:<14} L {:<12} | {}",
            self.r.now,
            self.r.facts.name(w),
            state::name(player.gun_phase),
            anim::name(player.gun_clip),
            slot::name(self.vm.hands[0].slot),
            slot::name(self.vm.hands[1].slot),
            slots.join("  "),
        ));
    }

    fn command(&mut self, n: usize, words: &[&str]) -> Result<(), String> {
        let arg = |i: usize| words.get(i).copied().ok_or_else(|| "missing argument".to_owned());
        let num = |i: usize| -> Result<i64, String> {
            let s = arg(i)?;
            s.parse::<i64>().map_err(|e| format!("{s}: {e}"))
        };
        match words[0] {
            "msec" => self.r.cmd.frame_ms = num(1)? as i32,
            "spawn" => {
                let w = self.id(arg(1)?)?;
                self.give(w);
                self.r.ps.weapon = w;
                self.r.cmd.weapon = w;
                self.r.ps.gun_phase = state::READY;
                self.r.ps.weaponstate_left = state::READY;
                self.r.ps.mark_all_owned(true);
            }
            "give" => {
                let w = self.id(arg(1)?)?;
                self.give(w);
            }
            "take" => {
                let w = self.id(arg(1)?)?;
                self.r.ps.take(w);
            }
            "select" => self.r.cmd.weapon = self.id(arg(1)?)?,
            "clip" => {
                let w = self.id(arg(1)?)?;
                self.r.ps.set_ammo_in_clip(self.r.facts, w, num(2)? as i32);
            }
            "stock" => {
                let w = self.id(arg(1)?)?;
                self.r.ps.set_ammo_not_in_clip(self.r.facts, w, num(2)? as i32);
            }
            "hold" | "release" => {
                for b in &words[1..] {
                    let bit = button(b).ok_or_else(|| format!("no button {b}"))?;
                    if words[0] == "hold" {
                        self.r.cmd.buttons |= bit;
                    } else {
                        self.r.cmd.buttons &= !bit;
                    }
                }
            }
            "perk" | "noperk" => {
                let p = num(1)? as u32;
                if p >= 64 {
                    return Err(format!("perk {p} out of range"));
                }
                if words[0] == "perk" {
                    self.r.env.perks |= 1u64 << p;
                } else {
                    self.r.env.perks &= !(1u64 << p);
                }
            }
            "pmtype" => self.r.env.movement_mode = num(1)? as i32,
            "stick" => self.r.cmd.thrust_ahead = num(1)?.clamp(-127, 127) as i8,
            "lefthand" => {
                let (w, l) = (self.id(arg(1)?)?, self.id(arg(2)?)?);
                self.left_of.insert(w, l);
                self.sets.remove(&w);
            }
            "anim" => {
                let a = anim_number(arg(1)?).ok_or_else(|| format!("no anim {}", words[1]))?;
                self.r.ps.gun_clip = anim::start(self.r.ps.gun_clip, a);
                if let Ok(s) = arg(2) {
                    self.r.ps.gun_phase =
                        state_number(s).ok_or_else(|| format!("no state {s}"))?;
                    // The state runs for the gun's own time for that move, as BO2 sets it.
                    let w = self.r.ps.weapon;
                    if let (Some(sl), Some(vf)) = (slot::find(words[1]), self.table.fetch(w)) {
                        self.r.ps.gun_timer_ms = vf.times[sl];
                    }
                }
            }
            "run" => {
                for _ in 0..num(1)? {
                    self.step();
                }
            }
            "expect" => {
                let what = arg(1)?;
                let want = arg(2)?;
                let got = match what {
                    "slot" => slot::name(self.vm.hands[0].slot),
                    "lslot" => slot::name(self.vm.hands[1].slot),
                    "state" => state::name(self.r.ps.gun_phase),
                    "anim" => anim::name(self.r.ps.gun_clip),
                    "weapon" => self.r.facts.name(self.r.ps.weapon),
                    _ => return Err(format!("expect what? {what}")),
                };
                if got.eq_ignore_ascii_case(want) {
                    self.log.push(format!("ok   line {n}: {what} {want}"));
                } else {
                    self.failures += 1;
                    self.log.push(format!("FAIL line {n}: {what} is {got}, want {want}"));
                }
            }
            other => return Err(format!("unknown command {other}")),
        }
        Ok(())
    }
}

fn button(name: &str) -> Option<u32> {
    use weapon_t6::cmd::button;
    Some(match name {
        "attack" | "fire" => button::FIRE_BIT,
        "ads" => button::AIM_BIT,
        "melee" => button::CLOSE_COMBAT,
        "reload" => button::REFILL_BIT,
        "use" => button::INTERACT_KEY,
        "sprint" => button::SPRINT,
        "frag" => button::SAMPLE_FRAG_ASSET,
        "smoke" => button::OBSCURANT,
        "prone" => button::LYING_FLAT,
        "crouch" => button::DUCK_BUTTON_BIT,
        "jump" => button::LEAP,
        _ => return None,
    })
}

fn anim_number(name: &str) -> Option<i32> {
    (0..anim::COUNT as i32).find(|&a| anim::name(a).eq_ignore_ascii_case(name))
}

fn state_number(name: &str) -> Option<i32> {
    (0..state::COUNT as i32).find(|&s| state::name(s).eq_ignore_ascii_case(name))
}

fn main() -> ExitCode {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() < 3 {
        eprintln!("usage: vm_trace SCRIPT FACTS_DUMP ANIMS_DUMP");
        return ExitCode::from(2);
    }
    let read = |p: &str| std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"));
    let (script_text, facts_text, anims_text) = match (read(&args[0]), read(&args[1]), read(&args[2])) {
        (Ok(a), Ok(b), Ok(c)) => (a, b, c),
        (a, b, c) => {
            for e in [a.err(), b.err(), c.err()].into_iter().flatten() {
                eprintln!("{e}");
            }
            return ExitCode::from(2);
        }
    };
    let facts = FactsTable::from_dump(&facts_text);
    let table = VmTable::from_dump(&facts_text);
    let clips = read_anims(&anims_text);
    let r = ScenarioDriver::new(&facts);
    let dvars = VmDvars::from_t6(&r.env.dvars);
    let mut vm = Vm {
        r,
        vm: Viewmodel::new(),
        table: &table,
        clips: &clips,
        left_of: HashMap::new(),
        sets: HashMap::new(),
        dvars,
        log: Vec::new(),
        failures: 0,
    };
    for (i, raw) in script_text.lines().enumerate() {
        let cmd = raw.split('#').next().unwrap_or("").trim();
        if cmd.is_empty() {
            continue;
        }
        vm.log.push(format!("> {cmd}"));
        let words: Vec<&str> = cmd.split_whitespace().collect();
        if let Err(e) = vm.command(i + 1, &words) {
            eprintln!("line {}: {e}", i + 1);
            return ExitCode::from(2);
        }
    }
    for l in &vm.log {
        println!("{l}");
    }
    println!("{} expect failure(s)", vm.failures);
    if vm.failures == 0 { ExitCode::SUCCESS } else { ExitCode::from(1) }
}
