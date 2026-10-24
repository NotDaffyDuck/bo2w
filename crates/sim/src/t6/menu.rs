//! bo2mp mod menu: the engine functions its options need that the game's
//! own scripts never reach here (BO2's names where BO2 has one: `kick`,
//! `map`, `setslowmotion`, `player_recoilscaleon`; `menu_rapidfire` is
//! ours). Bound after every other set, so `hide`/`show` here also cover
//! players (the entity ones turn players away).

use std::sync::Mutex;

use bevy_ecs::prelude::World;
use gsc_t6::{Value, Vm};

use super::{Zm, arg, client, entnum, frame, text};

/// A player whose body nobody else draws (`player hide()`).
pub const PLAYER_HIDDEN: u32 = playerstate_t6::game::eflags::HIDDEN;
/// Rapid fire from the menu: every gun fires three times as fast.
pub const PERK_MENU_RAPID_FIRE: u32 = 0x4000_0000;

/// A map change the menu asked for: the map, the game type, and the bots
/// (friends, enemies, difficulty) as the bot dvars stand.
#[derive(Clone, Debug, Default)]
pub struct MenuMapChange {
    pub map: String,
    pub gametype: String,
    pub bots: (u32, u32, u32),
}

static MAP_CHANGE: Mutex<Option<MenuMapChange>> = Mutex::new(None);
static KICKS: Mutex<Vec<u32>> = Mutex::new(Vec::new());
/// The game speed and where `setslowmotion` is taking it.
static SLOW_MOTION: Mutex<SlowMotion> = Mutex::new(SlowMotion::NORMAL);

/// Black Ops II's slow motion (Com_SetSlowMotion): the game speed slides in a
/// straight line from `start` to `end`, taking `ramp_ms` of real time for the
/// whole slide, after `delay_ms`. A slide that begins away from `start` keeps
/// the same pace from where the speed already is.
#[derive(Clone, Copy, Debug, PartialEq)]
struct SlowMotion {
    speed: f32,
    start: f32,
    end: f32,
    ramp_ms: f32,
    delay_ms: f32,
    sliding: bool,
}

impl SlowMotion {
    const NORMAL: Self = Self {
        speed: 1.0,
        start: 1.0,
        end: 1.0,
        ramp_ms: 0.0,
        delay_ms: 0.0,
        sliding: false,
    };

    /// ScrCmd_SetSlowMotion: `end` defaults to 1, the time to 1 s. Slowing
    /// down waits the first 150 ms of it (all of it if shorter), then slides.
    fn set(&mut self, start: f32, end: f32, secs: f32) {
        let ms = (secs * 1000.0) as i32;
        let delay = if start > end { ms.min(150) } else { 0 };
        self.delay_ms = delay as f32;
        if start == end {
            *self = Self { speed: end, start, end, ..Self::NORMAL };
            return;
        }
        self.start = start;
        self.end = end;
        self.ramp_ms = (ms - delay) as f32;
        self.sliding = true;
    }

    /// One frame of `real_ms` (Com_Frame): count the delay down, else slide,
    /// stopping on `end`.
    fn advance(&mut self, real_ms: f32) -> f32 {
        if self.delay_ms >= 1.0 {
            self.delay_ms -= real_ms;
        } else if self.sliding {
            let done = self.ramp_ms <= 0.0 || {
                self.speed += (self.end - self.start) * real_ms / self.ramp_ms;
                if self.start < self.end {
                    self.speed >= self.end
                } else {
                    self.speed <= self.end
                }
            };
            if done {
                self.speed = self.end;
                self.sliding = false;
            }
        }
        self.speed
    }
}

/// The map and game type `map()` asked for (taken once).
pub fn take_menu_map_change() -> Option<MenuMapChange> {
    MAP_CHANGE.lock().ok()?.take()
}

/// The bots `kick()` asked to remove (taken once).
pub fn take_menu_kicks() -> Vec<u32> {
    KICKS.lock().map(|mut k| std::mem::take(&mut *k)).unwrap_or_default()
}

/// The game speed scripts asked for, after one frame of `real_ms` real time
/// moves any slow-motion slide on: 1.0 normal, 0.25 quarter speed.
pub fn advance_menu_timescale(real_ms: f32) -> f32 {
    SLOW_MOTION.lock().map_or(1.0, |mut s| s.advance(real_ms))
}

/// Back to normal speed (a match ends or a new one loads).
pub fn reset_menu_timescale() {
    if let Ok(mut s) = SLOW_MOTION.lock() {
        *s = SlowMotion::NORMAL;
    }
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
    // hide/show: an entity as before; a player's body vanishes for everyone
    // else (bots stop seeing him through `ignoreme`, set by the script).
    vm.bind("hide", true, hide);
    vm.bind("show", true, show);
    m!("player_recoilscaleon", |vm, world, s, a| {
        let id = client(vm, world, s)?;
        let scale = arg(a, 0).as_float().unwrap_or(0.0).clamp(0.0, 100.0) as i32;
        if let Some(ps) = frame(world).player_mut(id) {
            ps.recoil_scale = scale;
            ps.weap_flags |= playerstate_t6::game::weap_flags::RECOIL_SCALE;
        }
        Ok(Value::Undefined)
    });
    m!("player_recoilscaleoff", |vm, world, s, _| {
        let id = client(vm, world, s)?;
        if let Some(ps) = frame(world).player_mut(id) {
            ps.weap_flags &= !playerstate_t6::game::weap_flags::RECOIL_SCALE;
        }
        Ok(Value::Undefined)
    });
    m!("menu_rapidfire", |vm, world, s, a| {
        let id = client(vm, world, s)?;
        let on = gsc_t6::truthy(arg(a, 0));
        if let Some(ps) = frame(world).player_mut(id) {
            if on {
                ps.perks[1] |= PERK_MENU_RAPID_FIRE;
            } else {
                ps.perks[1] &= !PERK_MENU_RAPID_FIRE;
            }
        }
        Ok(Value::Undefined)
    });
    // setslowmotion( start, end = 1, seconds = 1 ): the game speed slides
    // from `start` to `end` (the final killcam: 1 to 0.25 over 2 s, back
    // over 1 s; the menu: at once).
    f!("setslowmotion", |_, _, _, a| {
        let start = arg(a, 0).as_float().unwrap_or(1.0).clamp(0.1, 4.0);
        let end = arg(a, 1).as_float().unwrap_or(1.0).clamp(0.1, 4.0);
        let secs = arg(a, 2).as_float().unwrap_or(1.0).max(0.0);
        if let Ok(mut s) = SLOW_MOTION.lock() {
            s.set(start, end, secs);
        }
        diag::info!(Sim, "bo2mp: game speed {start:.2} -> {end:.2} over {secs:.2} s");
        Ok(Value::Undefined)
    });
    // kick( entnum ): a bot leaves the match (players are never kicked).
    // bo2mp: ban( entnum ) (the team-kill and suicide kicks) is a kick here.
    for name in ["kick", "ban"] {
        vm.bind(name, false, |vm, world, _, a| {
            let n = match arg(a, 0) {
                Value::Int(n) => Some(*n as u32),
                v => entnum(vm, v),
            };
            if let Some(n) = n
                && world
                    .resource::<Zm>()
                    .players
                    .get(&n)
                    .is_some_and(|p| p.test_client)
                && let Ok(mut k) = KICKS.lock()
                && !k.contains(&n)
            {
                k.push(n);
            }
            Ok(Value::Undefined)
        });
    }
    // map( name [, gametype] ): load another map; the game type is the
    // argument, else the g_gametype dvar (the one playing unless a script
    // set it).
    f!("map", |vm, _, _, a| {
        let map = text(vm, a, 0).to_ascii_lowercase();
        if map.is_empty() {
            return Ok(Value::Undefined);
        }
        let mut gametype = text(vm, a, 1).to_ascii_lowercase();
        if gametype.is_empty() {
            gametype = vm.dvars.get("g_gametype").cloned().unwrap_or_default();
        }
        let dvar = |k: &str| vm.dvars.get(k).and_then(|v| v.trim().parse::<u32>().ok()).unwrap_or(0);
        let bots = (dvar("bot_friends"), dvar("bot_enemies"), dvar("bot_difficulty"));
        diag::info!(Sim, "bo2mp menu: map {map} {gametype} bots {bots:?}");
        if let Ok(mut m) = MAP_CHANGE.lock() {
            *m = Some(MenuMapChange { map, gametype, bots });
        }
        Ok(Value::Undefined)
    });
}

fn set_hidden(vm: &mut Vm<World>, world: &mut World, s: &Value, hidden: bool) -> Result<Value, String> {
    let Some(n) = entnum(vm, s) else {
        return Ok(Value::Undefined);
    };
    if world.resource::<Zm>().players.contains_key(&n) {
        if let Some(ps) = frame(world).player_mut(crate::world::ClientId(n)) {
            if hidden {
                ps.e_flags |= PLAYER_HIDDEN;
            } else {
                ps.e_flags &= !PLAYER_HIDDEN;
            }
        }
    } else if world.resource::<Zm>().ents.contains_key(&n) {
        let mut zm = world.resource_mut::<Zm>();
        if let Some(e) = zm.ents.get_mut(&n) {
            e.hidden = hidden;
        }
    }
    Ok(Value::Undefined)
}

fn hide(vm: &mut Vm<World>, world: &mut World, s: &Value, _: &[Value]) -> Result<Value, String> {
    set_hidden(vm, world, s, true)
}

fn show(vm: &mut Vm<World>, world: &mut World, s: &Value, _: &[Value]) -> Result<Value, String> {
    set_hidden(vm, world, s, false)
}

#[cfg(test)]
mod tests {
    use super::SlowMotion;

    fn run(s: &mut SlowMotion, ms: u32) -> f32 {
        let mut v = s.speed;
        for _ in 0..ms / 10 {
            v = s.advance(10.0);
        }
        v
    }

    #[test]
    fn the_final_killcam_slows_after_a_beat_then_slides_to_a_quarter() {
        let mut s = SlowMotion::NORMAL;
        s.set(1.0, 0.25, 2.0);
        // The first 150 ms hold full speed.
        assert_eq!(run(&mut s, 150), 1.0);
        // Then 1850 ms slide down in a straight line.
        let half = run(&mut s, 930);
        assert!((half - 0.625).abs() < 0.01, "{half}");
        assert_eq!(run(&mut s, 1000), 0.25);
        assert!(!s.sliding);
        // Back up: no beat, 1 s from a quarter to full.
        s.set(0.25, 1.0, 1.0);
        let mid = run(&mut s, 500);
        assert!((mid - 0.625).abs() < 0.01, "{mid}");
        assert_eq!(run(&mut s, 600), 1.0);
    }

    #[test]
    fn no_time_or_the_same_speed_changes_at_once() {
        let mut s = SlowMotion::NORMAL;
        s.set(1.0, 0.5, 0.0);
        assert_eq!(s.advance(16.0), 0.5);
        s.set(1.0, 1.0, 0.0);
        assert_eq!(s.speed, 1.0);
        assert_eq!(s.advance(16.0), 1.0);
    }
}
