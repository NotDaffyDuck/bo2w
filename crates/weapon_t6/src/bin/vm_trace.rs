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
use weapon_t6::trace::Runner;
use weapon_t6::viewmodel::{AnimSet, Clip, Viewmodel, VmDvars, VmFrame, VmTable, slot};

type Clips = [Option<Clip>; slot::COUNT];

fn read_anims(text: &str) -> Vec<Clips> {
    let mut out = vec![[None; slot::COUNT]];
    for line in text.lines() {
        let mut clips = [None; slot::COUNT];
        if let Some((_, list)) = line.split_once('\t') {
            for item in list.split(';').filter(|s| !s.is_empty()) {
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
    r: Runner<'f>,
    vm: Viewmodel,
    table: &'f VmTable,
    clips: &'f [Clips],
    left_of: HashMap<u32, u32>,
    sets: HashMap<u32, AnimSet>,
    dvars: VmDvars,
    lines: Vec<String>,
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
            let clip = self.r.facts.get(w).core.clip_size;
            self.r.ps.set_ammo_in_clip(w, clip);
        }
    }

    fn step(&mut self) {
        self.r.step();
        let w = self.r.ps.weapon;
        let set = self.set_for(w);
        let ps = &self.r.ps;
        let left_gun = self.left_of.get(&w).copied().unwrap_or(0);
        let f = VmFrame {
            weapon: w,
            model: w,
            weap_anim: ps.weap_anim,
            weap_anim_left: ps.weap_anim_left,
            weaponstate: ps.weaponstate,
            weaponstate_left: ps.weaponstate_left,
            weapon_time: ps.weapon_time,
            weapon_time_left: ps.weapon_time_left,
            ammo_in_clip: ps.ammo_in_clip(w),
            ammo_in_clip_left: ps.ammo_in_clip(left_gun),
            intro_shots_fired: ps.intro_shots_fired,
            weapon_pos_frac: ps.f_weapon_pos_frac,
            perks: self.r.env.perks,
            off_hand_weapon: ps.off_hand_weapon,
            pm_type: self.r.env.pm_type,
            frametime_ms: self.r.cmd.msec,
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
        self.lines.push(format!(
            "{:>6} {:<14} {:<16} {:<14} | R {:<14} L {:<12} | {}",
            self.r.now,
            self.r.facts.name(w),
            state::name(ps.weaponstate),
            anim::name(ps.weap_anim),
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
            "msec" => self.r.cmd.msec = num(1)? as i32,
            "spawn" => {
                let w = self.id(arg(1)?)?;
                self.give(w);
                self.r.ps.weapon = w;
                self.r.cmd.weapon = w;
                self.r.ps.weaponstate = state::READY;
                self.r.ps.weaponstate_left = state::READY;
                self.r.ps.set_ever_had_all(true);
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
                self.r.ps.set_ammo_in_clip(w, num(2)? as i32);
            }
            "stock" => {
                let w = self.id(arg(1)?)?;
                self.r.ps.set_ammo_not_in_clip(w, num(2)? as i32);
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
            "pmtype" => self.r.env.pm_type = num(1)? as i32,
            "lefthand" => {
                let (w, l) = (self.id(arg(1)?)?, self.id(arg(2)?)?);
                self.left_of.insert(w, l);
                self.sets.remove(&w);
            }
            "anim" => {
                let a = anim_number(arg(1)?).ok_or_else(|| format!("no anim {}", words[1]))?;
                self.r.ps.weap_anim = anim::start(self.r.ps.weap_anim, a);
                if let Ok(s) = arg(2) {
                    self.r.ps.weaponstate =
                        state_number(s).ok_or_else(|| format!("no state {s}"))?;
                    // The state runs for the gun's own time for that move, as BO2 sets it.
                    let w = self.r.ps.weapon;
                    if let (Some(sl), Some(vf)) = (slot::find(words[1]), self.table.get(w)) {
                        self.r.ps.weapon_time = vf.times[sl];
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
                    "state" => state::name(self.r.ps.weaponstate),
                    "anim" => anim::name(self.r.ps.weap_anim),
                    "weapon" => self.r.facts.name(self.r.ps.weapon),
                    _ => return Err(format!("expect what? {what}")),
                };
                if got.eq_ignore_ascii_case(want) {
                    self.lines.push(format!("ok   line {n}: {what} {want}"));
                } else {
                    self.failures += 1;
                    self.lines.push(format!("FAIL line {n}: {what} is {got}, want {want}"));
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
        "attack" | "fire" => button::ATTACK,
        "ads" => button::ADS,
        "melee" => button::MELEE,
        "reload" => button::RELOAD,
        "use" => button::USE,
        "sprint" => button::SPRINT,
        "frag" => button::FRAG,
        "smoke" => button::SMOKE,
        "prone" => button::PRONE,
        "crouch" => button::CROUCH,
        "jump" => button::JUMP,
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
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 3 {
        eprintln!("usage: vm_trace SCRIPT FACTS_DUMP ANIMS_DUMP");
        return ExitCode::from(2);
    }
    let read = |p: &str| std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"));
    let (script, facts_text, anims_text) = match (read(&args[0]), read(&args[1]), read(&args[2])) {
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
    let r = Runner::new(&facts);
    let dvars = VmDvars::from_t6(&r.env.dvars);
    let mut vm = Vm {
        r,
        vm: Viewmodel::new(),
        table: &table,
        clips: &clips,
        left_of: HashMap::new(),
        sets: HashMap::new(),
        dvars,
        lines: Vec::new(),
        failures: 0,
    };
    for (i, raw) in script.lines().enumerate() {
        let text = raw.split('#').next().unwrap_or("").trim();
        if text.is_empty() {
            continue;
        }
        vm.lines.push(format!("> {text}"));
        let words: Vec<&str> = text.split_whitespace().collect();
        if let Err(e) = vm.command(i + 1, &words) {
            eprintln!("line {}: {e}", i + 1);
            return ExitCode::from(2);
        }
    }
    for l in &vm.lines {
        println!("{l}");
    }
    println!("{} expect failure(s)", vm.failures);
    if vm.failures == 0 { ExitCode::SUCCESS } else { ExitCode::from(1) }
}
