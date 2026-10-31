//! Firing + rechamber, by BO2's rules (docs/gun/TRIGGER_SLOT.md has the full timing spec).
//!
//! One hand of one player. `tick` runs the parts of BO2's weapon tick that this file owns, in BO2's order:
//! spin, pending pull, timers, continuous fire, rechamber, then the fire check. States owned by other files
//! (reload, melee, offhand, sprint ...) are left alone; a reload BO2 would start on an empty clip is reported
//! as `Event::StartReload` for the reload code to pick up.

use crate::state;

/// How long the gun waits each frame while the trigger is still held down.
const TRIGGER_WAIT_MS: i32 = 1;

/// BO2's fire types (`weapFireType_t`).
pub mod fire_type {
    pub const FULLAUTO: i32 = 0;
    pub const SINGLESHOT: i32 = 1;
    pub const BURST2: i32 = 2;
    pub const BURST3: i32 = 3;
    pub const BURST4: i32 = 4;
    pub const BURST5: i32 = 5;
    pub const STACKED: i32 = 6;
    pub const MINIGUN: i32 = 7;
    pub const CHARGESHOT: i32 = 8;
    pub const JETGUN: i32 = 9;
}

/// BO2's player weapon anim numbers used by firing (`weapAnim`, low bits; 0x400 is the restart toggle).
pub mod clip_ids {
    pub const REST: i32 = 0;
    pub const ATTACK: i32 = 2;
    pub const ATTACK_LASTSHOT: i32 = 4;
    pub const BOLT_CYCLE: i32 = 6;
    pub const ADS_ATTACK: i32 = 7;
    pub const ADS_ATTACK_LASTSHOT: i32 = 8;
    pub const ADS_BOLT_CYCLE: i32 = 9;
    pub const CONT_FIRE_IN: i32 = 38;
    pub const CONT_FIRE_LOOP: i32 = 39;
    pub const CONT_FIRE_OUT: i32 = 40;
    pub const MANTLE: i32 = 54;
    pub const TOGGLE: i32 = 0x400;
}

/// Rate of fire perk multiplier (`perk_weapRateMultiplier` default).
pub const RATE_OF_FIRE_MULT: f32 = 0.75;
/// Extra lockout after a dry-fire click, in ms.
pub const DRY_FIRE_MS: i32 = 500;
/// shotCount never goes past this.
pub const MAX_SHOT_COUNT: i32 = 7;
/// introShotsFired never goes past this.
pub const MAX_INTRO_SHOTS: i32 = 250;

/// The firing numbers of one weapon (from its weapon file, attachments already applied to `fire_time`).
#[derive(Clone, Debug, Default)]
pub struct FireDef {
    pub trigger_mode: i32,
    pub fire_time: i32,
    pub fire_delay: i32,
    pub last_fire_time: i32,
    pub intro_fire_time: i32,
    pub intro_fire_length: i32,
    pub burst_delay: i32,
    pub manual_cycle: bool,
    pub shots_before_rechamber: i32,
    pub rechamber_time: i32,
    pub rechamber_bolt_time: i32,
    pub continuous_fire: bool,
    pub cont_fire_in_time: i32,
    pub cont_fire_out_time: i32,
    pub spin_up_time: i32,
    pub spin_down_time: i32,
    pub magazine_capacity: i32,
    pub unlimited_ammo: bool,
    /// Weapon flag that turns off the wait for a trigger release.
    pub no_trigger_wait: bool,
    /// Refuses the automatic reload while fully aimed.
    pub no_ads_auto_reload: bool,
    pub fire_only_when_scoped: bool,
    /// Time to aim in fully, in ms (after the aim-in scale).
    pub ads_in_time: i32,
}

/// One hand's firing state (the player-state fields BO2 keeps for it).
#[derive(Clone, Debug, Default)]
pub struct Hand {
    pub phase: i32,
    pub timer_ms: i32,
    pub weapon_ready_wait: i32,
    pub rounds_fired: i32,
    pub intro_shots_fired: i32,
    pub shots_from_chamber: i32,
    pub needs_rechamber: bool,
    pub pending_pull: bool,
    pub mag_rounds: i32,
    pub spare_rounds: i32,
    /// Player anim number with the 0x400 toggle bit.
    pub anim: i32,
    /// Minigun spin, 0..1.
    pub spin: f32,
    /// The "barrel spinning" player flag.
    pub spinning: bool,
}

/// This tick's input for the hand.
#[derive(Clone, Debug, Default)]
pub struct Input {
    pub frame_ms: i32,
    pub fire: bool,
    pub final_round_fired: bool,
    pub ads: bool,
    /// Aim blend, 0 = hip, 1 = fully aimed.
    pub pos_frac: f32,
    pub rate_of_fire: bool,
    pub overheated: bool,
    pub mantling: bool,
    /// Something outside this file forbids the shot (friendly in the way, grenade held, no-fire player flag).
    pub blocked: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A shot left the gun. `shot` is the shot count of this pull; `emptied` = this shot emptied the clip.
    Fire { shot: i32, emptied: bool },
    /// Dry-fire click: clip and stock both short.
    Empty,
    /// Clip short but stock has rounds: BO2 starts a reload here.
    StartReload,
    /// Rechamber started.
    Rechamber,
    /// Bolt point of a rechamber reached (shell out).
    Bolt,
    /// Clip and stock both empty after a shot: switch away.
    AutoSwitch,
}

fn start_anim(h: &mut Hand, a: i32) {
    h.anim = (!h.anim & clip_ids::TOGGLE) | a;
}

/// Back to the idle anim unless it is already playing.
fn continue_anim(h: &mut Hand) {
    if h.anim & !clip_ids::TOGGLE != clip_ids::REST {
        start_anim(h, clip_ids::REST);
    }
}

fn idle(h: &mut Hand) {
    h.timer_ms = Default::default();
    h.weapon_ready_wait = 0;
    h.phase = state::READY;
    start_anim(h, clip_ids::REST);
}

fn uses_burst(def: &FireDef) -> bool {
    (fire_type::BURST2..=fire_type::BURST5).contains(&def.trigger_mode) || def.trigger_mode == fire_type::CHARGESHOT
}

/// Has this pull fired all the shots its fire type allows?
pub fn shot_limit_reached(def: &FireDef, h: &Hand) -> bool {
    match def.trigger_mode {
        fire_type::SINGLESHOT | fire_type::CHARGESHOT => h.rounds_fired >= 1,
        fire_type::BURST2..=fire_type::BURST5 => h.rounds_fired >= def.trigger_mode,
        fire_type::STACKED => h.rounds_fired >= def.magazine_capacity,
        _ => false,
    }
}

/// A burst has started and still owes shots (it keeps firing with the trigger up).
pub fn burst_in_progress(def: &FireDef, h: &Hand) -> bool {
    !matches!(def.trigger_mode, fire_type::FULLAUTO | fire_type::MINIGUN | fire_type::JETGUN)
        && h.rounds_fired != 0
        && !shot_limit_reached(def, h)
}

fn waiting_for_trigger_release(def: &FireDef, h: &Hand, inp: &Input) -> bool {
    let offhand = (state::OFFHAND_INIT..=state::OFFHAND_END).contains(&h.phase);
    let crawl = (state::CRAWL_IN..=state::CRAWL_LEFT).contains(&h.phase);
    !offhand
        && !crawl
        && !def.no_trigger_wait
        && !h.pending_pull
        && shot_limit_reached(def, h)
        && inp.fire
        && (h.mag_rounds > 0 || def.unlimited_ammo)
}

fn is_last_shot(def: &FireDef, h: &Hand) -> bool {
    let below = match def.trigger_mode {
        fire_type::BURST2..=fire_type::BURST5 => def.trigger_mode + 1,
        _ => 2,
    };
    h.mag_rounds < below
}

fn rechamber_blocks_fire(def: &FireDef, h: &Hand) -> bool {
    def.manual_cycle && h.needs_rechamber
}

fn update_spin(def: &FireDef, h: &mut Hand, inp: &Input) {
    if def.trigger_mode != fire_type::MINIGUN {
        return;
    }
    let rate = if inp.fire || inp.ads {
        h.spinning = true;
        1.0 / def.spin_up_time.max(1) as f32
    } else {
        h.spinning = false;
        -1.0 / def.spin_down_time.max(1) as f32
    };
    h.spin = (h.spin + inp.frame_ms as f32 * rate).clamp(0.0, 1.0);
}

fn update_pending_pull(def: &FireDef, h: &mut Hand, inp: &Input) {
    if def.trigger_mode != fire_type::FULLAUTO
        && def.trigger_mode != fire_type::SINGLESHOT
        && inp.fire
        && !inp.final_round_fired
    {
        h.pending_pull = true;
    }
}

/// This tick's timer step in ms, perk-scaled the way BO2 scales firing and rechamber states.
pub fn step_ms(h: &Hand, inp: &Input) -> i32 {
    let fire_states = matches!(
        h.phase,
        state::FIRING | state::RECHAMBER_START | state::RECHAMBERING | state::CONT_FIRE_LOOP
    );
    if inp.rate_of_fire && fire_states {
        (inp.frame_ms as f64 / RATE_OF_FIRE_MULT as f64 + 9.313226e-10).round() as i32
    } else {
        inp.frame_ms
    }
}

/// Counts the timers down. Returns true on the tick the weapon delay runs out (the held shot is released).
fn adjust_times(def: &FireDef, h: &mut Hand, inp: &Input) -> bool {
    let step = step_ms(h, inp);
    if h.timer_ms != 0 {
        h.timer_ms -= step;
        if def.trigger_mode == fire_type::STACKED && burst_in_progress(def, h) {
            h.timer_ms = Default::default();
        } else if h.timer_ms > 0 {
            return count_delay(h, step);
        }
        if matches!(h.phase, state::FIRING | state::CONT_FIRE_LOOP) && uses_burst(def) && !burst_in_progress(def, h) {
            // Burst over: the burst wait starts and nothing else runs this tick.
            h.timer_ms = if def.burst_delay == 0 { 1 } else { def.burst_delay };
            continue_anim(h);
            h.phase = state::READY;
            return false;
        }
        if waiting_for_trigger_release(def, h, inp) {
            h.timer_ms = TRIGGER_WAIT_MS;
            match h.phase {
                state::RECHAMBERING => finish_rechamber(h),
                state::FIRING
                | state::RECHAMBER_START
                | state::CONT_FIRE_LOOP
                | state::MELEE_INIT
                | state::MELEE_FIRE
                | state::MELEE_END => {
                    continue_anim(h);
                    h.phase = state::READY;
                }
                s if state::is_reload_family(s) => {
                    h.timer_ms = Default::default();
                    h.rounds_fired = 0;
                }
                _ => {}
            }
        } else {
            if (!inp.fire || h.pending_pull) && !burst_in_progress(def, h) {
                h.rounds_fired = 0;
            }
            h.timer_ms = Default::default();
        }
    }
    count_delay(h, step)
}

fn count_delay(h: &mut Hand, step: i32) -> bool {
    if h.weapon_ready_wait != 0 {
        h.weapon_ready_wait -= step;
        if h.weapon_ready_wait < 1 {
            h.weapon_ready_wait = 0;
            return true;
        }
    }
    false
}

fn should_be_firing(def: &FireDef, h: &mut Hand, inp: &Input, released: bool) -> bool {
    let held = inp.fire;
    let owed = released || burst_in_progress(def, h);
    let fire = if def.trigger_mode == fire_type::MINIGUN {
        held && h.spin >= 1.0
    } else {
        !rechamber_blocks_fire(def, h) && (held || owed)
    };
    if !fire && !released && h.phase == state::FIRING {
        if !inp.mantling {
            continue_anim(h);
        }
        h.phase = state::READY;
    }
    fire
}

fn check_continuous_fire(def: &FireDef, h: &mut Hand, inp: &Input, released: bool) {
    if !def.continuous_fire {
        return;
    }
    let s = h.phase;
    let fire_or_melee = matches!(
        s,
        state::FIRING | state::RECHAMBER_START | state::RECHAMBERING | state::MELEE_INIT | state::MELEE_FIRE | state::MELEE_END
    );
    let other = (state::RAISING..=state::DROPPING_ALTSWITCH).contains(&s)
        || (state::OFFHAND_INIT..=state::OFFHAND_END).contains(&s)
        || (state::RIOTSHIELD_PREPARE..=state::RIOTSHIELD_END).contains(&s)
        || (state::DTP_IN..=state::DTP_OUT).contains(&s)
        || s == state::DETONATING;
    if fire_or_melee || other {
        return;
    }
    // BO2 asks should-be-firing here without letting it touch the state.
    let mut probe = h.clone();
    let firing = should_be_firing(def, &mut probe, inp, released);
    if inp.overheated || !firing {
        if matches!(h.phase, state::CONT_FIRE_IN | state::CONT_FIRE_LOOP) {
            h.phase = state::CONT_FIRE_OUT;
            h.timer_ms = def.cont_fire_out_time;
            start_anim(h, clip_ids::CONT_FIRE_OUT);
        }
    } else if !(state::CONT_FIRE_IN..=state::CONT_FIRE_OUT).contains(&h.phase) {
        h.phase = state::CONT_FIRE_IN;
        h.timer_ms = def.cont_fire_in_time;
        h.weapon_ready_wait = 0;
        start_anim(h, clip_ids::CONT_FIRE_IN);
    }
}

fn finish_rechamber(h: &mut Hand) {
    start_anim(h, clip_ids::REST);
    h.needs_rechamber = false;
    h.phase = state::READY;
    h.shots_from_chamber = 0;
}

/// Bolt-action rechamber. Returns true when the tick must stop here.
fn check_rechamber(def: &FireDef, h: &mut Hand, inp: &Input, released: bool, ev: &mut Vec<Event>) -> bool {
    let s = h.phase;
    if (state::OFFHAND_INIT..=state::OFFHAND_END).contains(&s)
        || (state::DTP_IN..=state::DTP_OUT).contains(&s)
        || (state::RIOTSHIELD_PREPARE..=state::RIOTSHIELD_END).contains(&s)
        || !def.manual_cycle
        || !h.needs_rechamber
    {
        return false;
    }
    if h.phase == state::RECHAMBER_START && released {
        h.phase = state::RECHAMBERING;
        ev.push(Event::Bolt);
        if h.timer_ms != 0 {
            return true;
        }
    } else if h.timer_ms != 0 {
        let busy = matches!(
            h.phase,
            state::FIRING
                | state::RECHAMBER_START
                | state::RECHAMBERING
                | state::CONT_FIRE_LOOP
                | state::MELEE_INIT
                | state::MELEE_FIRE
                | state::MELEE_END
        );
        if busy || h.weapon_ready_wait != 0 {
            return false;
        }
    }
    if h.phase == state::RECHAMBERING {
        finish_rechamber(h);
        return false;
    }
    if h.phase == state::READY || (state::CRAWL_IN..=state::CRAWL_OUT).contains(&h.phase) {
        start_anim(h, if inp.pos_frac <= 0.75 { clip_ids::BOLT_CYCLE } else { clip_ids::ADS_BOLT_CYCLE });
        h.phase = state::RECHAMBER_START;
        h.timer_ms = def.rechamber_time;
        if def.shots_before_rechamber != 0 {
            h.shots_from_chamber = 0;
        }
        let bolt = def.rechamber_bolt_time;
        h.weapon_ready_wait = if bolt != 0 && bolt < def.rechamber_time { bolt } else { 1 };
        ev.push(Event::Rechamber);
    }
    false
}

/// Enough rounds in the clip? If not: reload or dry-fire click, and the click lockout.
fn check_firing_ammo(def: &FireDef, h: &mut Hand, inp: &Input, ev: &mut Vec<Event>) -> bool {
    if h.mag_rounds >= 1 {
        return true;
    }
    if !(def.no_ads_auto_reload && inp.pos_frac >= 1.0) {
        if h.spare_rounds >= 1 {
            ev.push(Event::StartReload);
            return false;
        }
        ev.push(Event::Empty);
    }
    if !inp.mantling {
        h.needs_rechamber = false;
        continue_anim(h);
        h.timer_ms += DRY_FIRE_MS;
    }
    false
}

fn start_firing(def: &FireDef, h: &mut Hand, inp: &Input) {
    if h.phase != state::FIRING {
        h.intro_shots_fired = 0;
    }
    h.timer_ms = if is_last_shot(def, h) && def.last_fire_time != 0 {
        def.last_fire_time
    } else if def.intro_fire_length != 0 && h.intro_shots_fired < def.intro_fire_length {
        def.intro_fire_time
    } else {
        def.fire_time
    };
    if def.manual_cycle && def.shots_before_rechamber == 0 {
        h.needs_rechamber = true;
    }
    if h.phase != state::FIRING && h.phase != state::CONT_FIRE_LOOP {
        h.weapon_ready_wait = def.fire_delay;
    }
    if def.fire_only_when_scoped && def.trigger_mode != fire_type::MINIGUN {
        let left = ((1.0 - inp.pos_frac) * def.ads_in_time as f32) as i32;
        h.weapon_ready_wait = h.weapon_ready_wait.max(left);
    }
    if !(state::CONT_FIRE_IN..=state::CONT_FIRE_OUT).contains(&h.phase) {
        h.phase = state::FIRING;
    }
    if def.manual_cycle && def.shots_before_rechamber > 0 {
        h.shots_from_chamber += 1;
        if h.shots_from_chamber >= def.shots_before_rechamber {
            h.needs_rechamber = true;
        }
    }
    if h.weapon_ready_wait == 0 && def.trigger_mode != fire_type::FULLAUTO {
        h.rounds_fired = (h.rounds_fired + 1).min(MAX_SHOT_COUNT);
    }
}

fn set_fire_anim(h: &mut Hand, inp: &Input) {
    if h.anim & !clip_ids::TOGGLE == clip_ids::MANTLE {
        return;
    }
    let empty = h.mag_rounds == 0;
    let a = match (inp.pos_frac <= 0.75, empty) {
        (true, false) => clip_ids::ATTACK,
        (true, true) => clip_ids::ATTACK_LASTSHOT,
        (false, false) => clip_ids::ADS_ATTACK,
        (false, true) => clip_ids::ADS_ATTACK_LASTSHOT,
    };
    start_anim(h, a);
}

fn fire_weapon(def: &FireDef, h: &mut Hand, inp: &Input, ev: &mut Vec<Event>) {
    if inp.overheated {
        return;
    }
    if !def.unlimited_ammo && !check_firing_ammo(def, h, inp, ev) {
        return;
    }
    start_firing(def, h, inp);
    if h.weapon_ready_wait != 0 {
        return;
    }
    h.pending_pull = false;
    if !def.unlimited_ammo {
        h.mag_rounds -= 1;
    }
    h.intro_shots_fired = (h.intro_shots_fired + 1).min(MAX_INTRO_SHOTS);
    if !(state::CONT_FIRE_IN..=state::CONT_FIRE_OUT).contains(&h.phase) {
        set_fire_anim(h, inp);
    }
    ev.push(Event::Fire { shot: h.rounds_fired, emptied: h.mag_rounds == 0 });
    if h.mag_rounds == 0 && h.spare_rounds == 0 && !def.unlimited_ammo {
        ev.push(Event::AutoSwitch);
    }
}

/// One weapon tick for the parts firing owns. Events this tick go into `ev`.
pub fn tick(def: &FireDef, h: &mut Hand, inp: &Input, ev: &mut Vec<Event>) {
    update_spin(def, h, inp);
    update_pending_pull(def, h, inp);
    let released = adjust_times(def, h, inp);
    if !burst_in_progress(def, h) {
        check_continuous_fire(def, h, inp, released);
    }
    if check_rechamber(def, h, inp, released, ev) {
        return;
    }
    if (h.timer_ms != 0 || h.weapon_ready_wait != 0) && !released {
        return;
    }
    match h.phase {
        state::CONT_FIRE_IN => {
            h.phase = state::CONT_FIRE_LOOP;
            h.timer_ms = Default::default();
            h.weapon_ready_wait = 0;
            start_anim(h, clip_ids::CONT_FIRE_LOOP);
        }
        state::CONT_FIRE_OUT => {
            idle(h);
            return;
        }
        state::READY | state::FIRING | state::RECHAMBER_START | state::RECHAMBERING | state::CONT_FIRE_LOOP => {}
        _ => return,
    }
    if !inp.blocked && should_be_firing(def, h, inp, released) {
        fire_weapon(def, h, inp, ev);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs `ms` of game time in `frame`-ms ticks; `fire(t)` says if the trigger is down at time t.
    /// Returns the times shots left the gun.
    fn run(def: &FireDef, h: &mut Hand, frame: i32, ms: i32, mut fire: impl FnMut(i32) -> bool, base: Input) -> Vec<i32> {
        let mut shots = Vec::new();
        let mut last = false;
        let mut t = 0;
        while t < ms {
            let f = fire(t);
            let inp = Input { frame_ms: frame, fire: f, final_round_fired: last, ..base.clone() };
            let mut ev = Vec::new();
            tick(def, h, &inp, &mut ev);
            for e in ev {
                if let Event::Fire { .. } = e {
                    shots.push(t);
                }
            }
            last = f;
            t += frame;
        }
        shots
    }

    fn gun(fire_type: i32, fire_time: i32) -> FireDef {
        let mut def = FireDef::default();
        def.trigger_mode = fire_type;
        def.fire_time = fire_time;
        def.magazine_capacity = 30;
        def
    }

    fn loaded(clip: i32) -> Hand {
        let mut hand = Hand::default();
        hand.mag_rounds = clip;
        hand.spare_rounds = 90;
        hand
    }

    #[test]
    fn full_auto_held_fires_every_fire_time() {
        let def = gun(fire_type::FULLAUTO, 100);
        let mut h = loaded(30);
        let shots = run(&def, &mut h, 1, 1000, |_| true, Input::default());
        assert_eq!(shots, vec![0, 100, 200, 300, 400, 500, 600, 700, 800, 900]);
    }

    #[test]
    fn full_auto_fire_delay_only_on_first_shot() {
        let mut def = gun(fire_type::FULLAUTO, 100);
        def.fire_delay = 30;
        let mut h = loaded(30);
        let shots = run(&def, &mut h, 1, 450, |_| true, Input::default());
        assert_eq!(shots, vec![30, 130, 230, 330, 430]);
    }

    #[test]
    fn semi_auto_held_fires_once() {
        let def = gun(fire_type::SINGLESHOT, 100);
        let mut h = loaded(30);
        let shots = run(&def, &mut h, 1, 1000, |_| true, Input::default());
        assert_eq!(shots, vec![0]);
    }

    #[test]
    fn semi_auto_needs_release_and_fire_time() {
        let def = gun(fire_type::SINGLESHOT, 100);
        let mut h = loaded(30);
        // Presses at 0-19, 50-69 (gun still busy: lost), 150-169.
        let pressed = |t: i32| t < 20 || (50..70).contains(&t) || (150..170).contains(&t);
        let shots = run(&def, &mut h, 1, 300, pressed, Input::default());
        assert_eq!(shots, vec![0, 150]);
    }

    #[test]
    fn burst3_tap_fires_three_then_waits() {
        let mut def = gun(fire_type::BURST3, 60);
        def.burst_delay = 200;
        let mut h = loaded(30);
        let shots = run(&def, &mut h, 1, 1000, |t| t < 10, Input::default());
        assert_eq!(shots, vec![0, 60, 120]);
        assert_eq!(h.rounds_fired, 0);
    }

    #[test]
    fn burst3_held_needs_release() {
        let mut def = gun(fire_type::BURST3, 60);
        def.burst_delay = 200;
        let mut h = loaded(30);
        let shots = run(&def, &mut h, 1, 1000, |_| true, Input::default());
        assert_eq!(shots, vec![0, 60, 120]);
    }

    #[test]
    fn burst3_repress_held_chains_after_burst_delay() {
        let mut def = gun(fire_type::BURST3, 60);
        def.burst_delay = 200;
        let mut h = loaded(30);
        // Tap at 0, re-press at 200 (after the third shot, inside the burst wait) and hold.
        let shots = run(&def, &mut h, 1, 700, |t| !(10..200).contains(&t), Input::default());
        // Burst ends when the third shot's fire time runs out (180); burst wait 200 -> next burst at 380.
        assert_eq!(shots, vec![0, 60, 120, 380, 440, 500]);
    }

    #[test]
    fn last_shot_uses_last_fire_time_then_dry_fire() {
        let mut def = gun(fire_type::FULLAUTO, 100);
        def.last_fire_time = 300;
        let mut h = loaded(2);
        h.spare_rounds = 0;
        let mut ev_all = Vec::new();
        let mut last = false;
        for t in 0..1000 {
            let mut inp = Input::default();
            inp.frame_ms = 1;
            inp.fire = true;
            inp.final_round_fired = last;
            let mut ev = Vec::new();
            tick(&def, &mut h, &inp, &mut ev);
            for e in ev {
                ev_all.push((t, e));
            }
            last = true;
        }
        assert_eq!(ev_all[0], (0, Event::Fire { shot: 0, emptied: false }));
        assert_eq!(ev_all[1], (100, Event::Fire { shot: 0, emptied: true }));
        assert_eq!(ev_all[2], (100, Event::AutoSwitch));
        // Last shot costs 300, then the click; each click adds 500.
        assert_eq!(ev_all[3], (400, Event::Empty));
        assert_eq!(ev_all[4], (900, Event::Empty));
    }

    #[test]
    fn bolt_action_rechambers_and_blocks_fire() {
        let def = FireDef {
            trigger_mode: fire_type::SINGLESHOT,
            fire_time: 100,
            manual_cycle: true,
            rechamber_time: 1000,
            rechamber_bolt_time: 600,
            magazine_capacity: 5,
            ..Default::default()
        };
        let mut h = loaded(5);
        let mut log = Vec::new();
        let mut last = false;
        for t in 0..1500 {
            let f = true;
            let mut inp = Input::default();
            inp.frame_ms = 1;
            inp.fire = f;
            inp.final_round_fired = last;
            let mut ev = Vec::new();
            tick(&def, &mut h, &inp, &mut ev);
            for e in ev {
                log.push((t, e));
            }
            last = f;
        }
        assert_eq!(log[0], (0, Event::Fire { shot: 1, emptied: false }));
        // Fire time ends at 100: held trigger -> READY, and the rechamber starts in the same tick.
        assert_eq!(log[1], (100, Event::Rechamber));
        assert_eq!(log[2], (700, Event::Bolt));
        // Rechamber done at 1100; trigger still held from the first shot -> no second shot.
        assert_eq!(log.len(), 3);
        assert_eq!(h.phase, state::READY);
        assert!(!h.needs_rechamber);
    }

    #[test]
    fn rate_of_fire_scales_fire_time() {
        let def = gun(fire_type::FULLAUTO, 100);
        let mut h = loaded(30);
        let mut base = Input::default();
        base.rate_of_fire = true;
        // 10 ms frames: step rounds 13.3 to 13, so a 100 ms fire time ends after 8 frames.
        let shots = run(&def, &mut h, 10, 400, |_| true, base);
        assert_eq!(shots, vec![0, 80, 160, 240, 320]);
        let mut h = loaded(30);
        let mut base = Input::default();
        base.rate_of_fire = true;
        let shots = run(&def, &mut h, 1, 300, |_| true, base);
        // 1 ms frames: step rounds 1.33 to 1, so the perk only shows on longer frames (as in BO2).
        assert_eq!(shots, vec![0, 100, 200]);
    }

    #[test]
    fn intro_shots_use_intro_fire_time() {
        let mut def = gun(fire_type::FULLAUTO, 100);
        def.intro_fire_time = 200;
        def.intro_fire_length = 2;
        let mut h = loaded(30);
        let shots = run(&def, &mut h, 1, 700, |_| true, Input::default());
        assert_eq!(shots, vec![0, 200, 400, 500, 600]);
    }

    #[test]
    fn minigun_spins_up_before_firing() {
        let def = FireDef {
            trigger_mode: fire_type::MINIGUN,
            fire_time: 50,
            spin_up_time: 500,
            spin_down_time: 1000,
            magazine_capacity: 100,
            ..Default::default()
        };
        let mut h = loaded(100);
        let shots = run(&def, &mut h, 1, 700, |_| true, Input::default());
        // Spin reaches 1.0 after 500 ms of holding.
        assert!((499..=500).contains(&shots[0]), "first shot {}", shots[0]);
        assert!(shots.windows(2).all(|w| w[1] - w[0] == 50));
    }

    #[test]
    fn continuous_fire_in_loop_out() {
        let def = FireDef {
            trigger_mode: fire_type::FULLAUTO,
            fire_time: 100,
            continuous_fire: true,
            cont_fire_in_time: 300,
            cont_fire_out_time: 200,
            magazine_capacity: 100,
            ..Default::default()
        };
        let mut h = loaded(100);
        let shots = run(&def, &mut h, 1, 600, |t| t < 500, Input::default());
        assert_eq!(shots, vec![300, 400]);
        assert_eq!(h.phase, state::CONT_FIRE_OUT);
        run(&def, &mut h, 1, 300, |_| false, Input::default());
        assert_eq!(h.phase, state::READY);
    }
}
