//! Reloading + its cancels, by BO2's rules (docs/gun/RELOAD.md has the full timing spec).
//!
//! One hand of one player. The reload check (`check`) runs at its place in BO2's weapon tick (after weapon
//! change, before melee); the finish (`finish`) runs from the state switch for states 11-16 when both timers
//! are 0 or the ammo delay fired this tick. `step_ms` is the reload-state timer step (the fast-reload perk).
//! Vehicle-seat reloads and fuel-tank guns are not ported (BO2 MP gives neither to a player hand).

use crate::state;

/// BO2's player weapon anim numbers used by reloading (`weapAnim`, low bits; 0x400 is the restart toggle).
pub mod refill_clips {
    pub const REST: i32 = 0;
    pub const RELOAD: i32 = 17;
    pub const RELOAD_DRY: i32 = 18;
    pub const RELOAD_BEGIN: i32 = 19;
    pub const RELOAD_FINISH: i32 = 20;
    pub const RELOAD_QUICK: i32 = 21;
    pub const RELOAD_QUICK_EMPTY: i32 = 22;
    pub const EMPTY_TAKE_OUT: i32 = 28;
    pub const TOGGLE: i32 = 0x400;
}

/// `perk_weapReloadMultiplier` default.
pub const RELOAD_MULT: f32 = 0.5;
/// A one-by-one start can only be interrupted once more than this share of it has played.
pub const START_INTERRUPT_IGNORE_FRAC: f32 = 0.4;

/// The reload numbers of one weapon (variant times already scaled by the attachments).
#[derive(Clone, Debug, Default)]
pub struct ReloadDef {
    /// Bullet gun (only these use the empty rows).
    pub bullet: bool,
    /// A melee weapon (blocks the reload check when dual wielded).
    pub melee_type: bool,
    pub dual_wield: bool,
    /// Weapon used as a melee weapon (no empty raise after a knife).
    pub use_as_melee: bool,
    pub magazine_capacity: i32,
    pub reload: i32,
    pub reload_empty: i32,
    pub reload_quick: i32,
    pub reload_quick_empty: i32,
    pub reload_add: i32,
    pub reload_empty_add: i32,
    pub reload_quick_add: i32,
    pub reload_quick_empty_add: i32,
    pub reload_start: i32,
    pub reload_begin_bonus: i32,
    pub reload_end: i32,
    pub rechamber_bolt_time: i32,
    pub empty_raise_time: i32,
    pub forbid_topup_reload: bool,
    /// One-by-one reload (weapon or attachment).
    pub segmented: bool,
    pub no_ads_auto_reload: bool,
    /// Rounds per main-reload step (0 = fill).
    pub ammo_add: i32,
    /// Rounds added during the start (0 = none).
    pub start_add: i32,
    pub manual_cycle: bool,
    pub shots_before_rechamber: i32,
    pub unlimited_ammo: bool,
    /// Fast Mag.
    pub dual_mag: bool,
    pub charge_shot: bool,
    /// Third-person reload anim is skipped for clip-only weapons.
    pub motion_clip_exclusive: bool,
}

/// One hand's reload state (the player-state fields BO2 keeps for it).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Hand {
    pub phase: i32,
    pub timer_ms: i32,
    pub weapon_ready_wait: i32,
    /// Player anim number with the 0x400 toggle bit.
    pub anim: i32,
    pub mag_rounds: i32,
    pub spare_rounds: i32,
    pub rounds_fired: i32,
    pub shots_from_chamber: i32,
    pub needs_rechamber: bool,
    /// The held weapon's quick-reload flag (Fast Mag alternation).
    pub quick_reload: bool,
    /// Charge-shot charge (cleared by a reload).
    pub charge: i32,
}

/// This tick's input for the hand.
#[derive(Clone, Debug)]
pub struct Input {
    pub frame_ms: i32,
    pub reload: bool,
    /// Script asked for a reload (weapon flag bit 1, consumed by the check).
    pub script_reload: bool,
    pub fire: bool,
    pub final_round_fired: bool,
    /// specialty_fastreload (player or attachment perk).
    pub fast_reload: bool,
    pub reload_mult: f32,
    /// Fully aimed in (aim blend == 1).
    pub fully_ads: bool,
    /// pm flag 0x800 or other flag 0x400: reload requests are ignored.
    pub request_locked: bool,
    /// pm flag 4, or deep water with weapons off: no reload check at all.
    pub check_blocked: bool,
    /// eFlags 0x300 (using a turret or object): no auto-reload.
    pub using_object: bool,
    /// eFlags2 0x10000000: no reload starts.
    pub no_reload: bool,
    pub mantling: bool,
    pub prone_change: bool,
    /// `tu_enforceRechamberOnSwitch` (default on).
    pub enforce_rechamber: bool,
}

impl Default for Input {
    fn default() -> Self {
        Self {
            frame_ms: 0,
            reload: false,
            script_reload: false,
            fire: false,
            final_round_fired: false,
            fast_reload: false,
            reload_mult: RELOAD_MULT,
            fully_ads: false,
            request_locked: false,
            check_blocked: false,
            using_object: false,
            no_reload: false,
            mantling: false,
            prone_change: false,
            enforce_rechamber: true,
        }
    }
}

/// BO2's event numbers raised by reloading.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    ResetAds = 17,
    Reload = 18,
    ReloadFromEmpty = 19,
    ReloadKickoff = 20,
    ReloadEnd = 21,
    /// The `reload_start` script notify.
    ReloadStartNotify = 22,
    /// The moment the ammo counts.
    ReloadAddAmmo = 23,
    EjectBrass = 38,
    /// Third-person reload anim (anim script event, not a player event).
    ThirdPersonReload = -1,
}

fn start_anim(h: &mut Hand, a: i32) {
    h.anim = (!h.anim & refill_clips::TOGGLE) | a;
}

fn to_ready(h: &mut Hand) {
    h.phase = state::READY;
    start_anim(h, refill_clips::REST);
}

fn in_start(h: &Hand) -> bool {
    matches!(
        h.phase,
        state::RELOAD_START | state::RELOAD_START_RECHAMBER | state::RELOAD_START_INTERUPT
    )
}

/// Reload states the fast-reload perk step applies to.
pub fn is_reload_state(s: i32) -> bool {
    (state::RELOADING..=state::RELOAD_QUICK_EMPTY).contains(&s)
}

/// This tick's timer step in ms for a hand in a reload state (plain `msec` otherwise).
pub fn step_ms(h: &Hand, inp: &Input) -> i32 {
    if !(inp.fast_reload && is_reload_state(h.phase)) {
        return inp.frame_ms;
    }
    if inp.reload_mult == 0.0 {
        return h.timer_ms.max(h.weapon_ready_wait);
    }
    (inp.frame_ms as f64 / inp.reload_mult as f64 + 9.313226e-10).round() as i32
}

/// Counts both timers down by `dt`. True when the delayed action fires this tick.
pub fn count_down(h: &mut Hand, dt: i32) -> bool {
    h.timer_ms = (h.timer_ms - dt).max(0);
    if h.weapon_ready_wait != 0 {
        h.weapon_ready_wait -= dt;
        if h.weapon_ready_wait < 1 {
            h.weapon_ready_wait = 0;
            return true;
        }
    }
    false
}

fn quick(def: &ReloadDef, h: &Hand) -> bool {
    h.quick_reload && def.dual_mag
}

fn empty_bullet(def: &ReloadDef, h: &Hand) -> bool {
    h.mag_rounds == 0 && def.bullet
}

/// AllowReload: spare ammo, clip not full, and the no-partial rule.
pub fn allow(def: &ReloadDef, h: &Hand) -> bool {
    if h.spare_rounds == 0 || h.mag_rounds >= def.magazine_capacity {
        return false;
    }
    if def.forbid_topup_reload {
        if def.ammo_add != 0 && def.ammo_add < def.magazine_capacity {
            return def.ammo_add <= def.magazine_capacity - h.mag_rounds;
        }
        return h.mag_rounds == 0;
    }
    true
}

/// May an automatic (empty-clip) reload start in this state?
fn auto_allowed(def: &ReloadDef, h: &Hand, inp: &Input, ev: &mut Vec<Event>) -> bool {
    match h.phase {
        1..=5
        | state::RELOADING
        | state::RELOADING_INTERUPT
        | state::RELOAD_END
        | state::SWIM_IN => false,
        state::RELOAD_START | state::RELOAD_START_RECHAMBER | state::RELOAD_START_INTERUPT => {
            if inp.prone_change && !def.motion_clip_exclusive {
                ev.push(Event::ThirdPersonReload);
            }
            false
        }
        s => {
            s != state::FIRING
                && s != state::CONT_FIRE_LOOP
                && !(state::SPRINT_RAISE..=state::SPRINT_DROP).contains(&s)
                && !(state::DEPLOYING..=state::BREAKING_DOWN).contains(&s)
                && !inp.using_object
                && !def.unlimited_ammo
                && !(def.no_ads_auto_reload && inp.fully_ads)
        }
    }
}

/// The reload check at its place in the weapon tick (BO2's CheckForReload). Returns true when the
/// request was read (past the blockers and not locked): the caller then clears the script request flags.
pub fn check(def: &ReloadDef, h: &mut Hand, inp: &Input, ev: &mut Vec<Event>) -> bool {
    if h.phase == state::MELEE_END && h.mag_rounds == 0 && !def.use_as_melee {
        h.phase = state::RAISING;
        h.timer_ms = def.empty_raise_time;
        start_anim(h, refill_clips::EMPTY_TAKE_OUT);
        return false;
    }
    let s = h.phase;
    if (state::OFFHAND_INIT..=state::OFFHAND_END).contains(&s)
        || (state::MELEE_INIT..=state::MELEE_END).contains(&s)
        || (state::DTP_IN..=state::DTP_OUT).contains(&s)
        || (state::RIOTSHIELD_PREPARE..=state::RIOTSHIELD_END).contains(&s)
        || inp.check_blocked
        || (def.melee_type && def.dual_wield)
    {
        return false;
    }
    let requested = !inp.request_locked && (inp.script_reload || inp.reload);

    // A fresh fire press stops a one-by-one reload after the current step.
    if def.segmented && inp.fire && !inp.final_round_fired {
        if matches!(s, state::RELOAD_START | state::RELOAD_START_RECHAMBER) && def.reload_start != 0
        {
            let played = (def.reload_start - h.timer_ms) as f32 / def.reload_start as f32;
            if played > START_INTERRUPT_IGNORE_FRAC {
                h.phase = state::RELOAD_START_INTERUPT;
            }
        } else if s == state::RELOADING {
            h.phase = state::RELOADING_INTERUPT;
        }
    }

    let asked = requested && allow(def, h);
    let auto = h.mag_rounds == 0 && h.spare_rounds != 0 && auto_allowed(def, h, inp, ev);
    if asked || auto {
        begin(def, h, inp, ev);
    }
    !inp.request_locked
}

/// BeginWeaponReload: only from the states BO2 lets a reload start in.
pub fn begin(def: &ReloadDef, h: &mut Hand, inp: &Input, ev: &mut Vec<Event>) {
    let s = h.phase;
    let ok = s == state::READY
        || (state::CRAWL_FORWARD..=state::CRAWL_LEFT).contains(&s)
        || s == state::FIRING
        || s == state::CONT_FIRE_LOOP
        || s == state::RECHAMBER_START
        || s == state::RECHAMBERING
        || (state::SPRINT_RAISE..=state::SPRINT_DROP).contains(&s)
        || (state::LOWREADY_RAISE..=state::LOWREADY_DROP).contains(&s);
    if !ok || inp.no_reload || inp.mantling {
        return;
    }
    h.rounds_fired = 0;
    ev.push(Event::ResetAds);
    ev.push(Event::ReloadStartNotify);
    if def.charge_shot {
        h.charge = 0;
    }
    if def.reload_start == 0 || !def.segmented {
        set_reloading(def, h, ev);
    } else {
        start_anim(h, refill_clips::RELOAD_BEGIN);
        h.timer_ms = def.reload_start;
        h.phase = if inp.enforce_rechamber && def.manual_cycle && h.needs_rechamber {
            state::RELOAD_START_RECHAMBER
        } else {
            state::RELOAD_START
        };
        ev.push(Event::ReloadKickoff);
        set_add_delay(def, h);
    }
    if !def.motion_clip_exclusive {
        ev.push(Event::ThirdPersonReload);
    }
}

/// SetReloadingState: the main reload (or the next shell).
pub fn set_reloading(def: &ReloadDef, h: &mut Hand, ev: &mut Vec<Event>) {
    if def.unlimited_ammo {
        return;
    }
    let q = quick(def, h);
    if empty_bullet(def, h) {
        if q {
            start_anim(h, refill_clips::RELOAD_QUICK_EMPTY);
            h.timer_ms = def.reload_quick_empty;
        } else {
            start_anim(h, refill_clips::RELOAD_DRY);
            h.timer_ms = def.reload_empty;
        }
        ev.push(Event::ReloadFromEmpty);
    } else {
        if q {
            start_anim(h, refill_clips::RELOAD_QUICK);
            h.timer_ms = def.reload_quick;
        } else {
            start_anim(h, refill_clips::RELOAD);
            h.timer_ms = def.reload;
        }
        ev.push(Event::Reload);
    }
    h.phase = if h.phase == state::RELOAD_START_INTERUPT {
        state::RELOADING_INTERUPT
    } else {
        state::RELOADING
    };
    set_add_delay(def, h);
}

/// The main-reload ammo point: add time clamped by the NORMAL total (BO2 uses the normal time even when quick).
fn main_add_point(def: &ReloadDef, h: &Hand, use_quick: bool) -> i32 {
    let q = use_quick && quick(def, h);
    let base_add = if q {
        def.reload_quick_add
    } else {
        def.reload_add
    };
    let (total, add) = if empty_bullet(def, h) {
        let add = if q && def.reload_quick_empty_add != 0 {
            def.reload_quick_empty_add
        } else if def.reload_empty_add != 0 {
            def.reload_empty_add
        } else {
            base_add
        };
        (def.reload_empty, add)
    } else {
        (def.reload, base_add)
    };
    if add != 0 && add < total { add } else { total }
}

fn start_add_point(def: &ReloadDef) -> i32 {
    if def.reload_begin_bonus == 0 {
        0
    } else {
        def.reload_begin_bonus.min(def.reload_start)
    }
}

/// SetWeaponReloadAddAmmoDelay: when the ammo goes in.
fn set_add_delay(def: &ReloadDef, h: &mut Hand) {
    let mut wait_ms = if in_start(h) {
        start_add_point(def)
    } else {
        main_add_point(def, h, true)
    };
    if def.manual_cycle && h.needs_rechamber {
        if wait_ms == 0 {
            wait_ms = h.timer_ms;
        }
        wait_ms = wait_ms.min(def.rechamber_bolt_time);
        if wait_ms == 0 {
            wait_ms = 1;
        }
        h.weapon_ready_wait = wait_ms;
    } else if wait_ms != 0 {
        h.weapon_ready_wait = wait_ms;
    }
}

/// ReloadClip: move rounds from the stock into the clip.
fn reload_clip(def: &ReloadDef, h: &mut Hand, ev: &mut Vec<Event>) {
    let start = in_start(h);
    if start && def.start_add == 0 {
        return;
    }
    let mut n = h.spare_rounds.min(def.magazine_capacity - h.mag_rounds);
    if start {
        if def.start_add < def.magazine_capacity && def.start_add < n {
            n = def.start_add;
        }
    } else if def.ammo_add != 0 && def.ammo_add < def.magazine_capacity && def.ammo_add < n {
        n = def.ammo_add;
    }
    if n != 0 {
        h.spare_rounds -= n;
        h.mag_rounds += n;
        ev.push(Event::ReloadAddAmmo);
    }
}

/// ReloadDelayedAction: bolt work, then the ammo; Fast Mag flips after the ammo.
pub fn delayed_action(def: &ReloadDef, h: &mut Hand, ev: &mut Vec<Event>) {
    if def.manual_cycle {
        if def.shots_before_rechamber != 0 {
            h.shots_from_chamber = 0;
        }
        if h.needs_rechamber {
            h.needs_rechamber = false;
            if def.charge_shot {
                h.charge = 0;
            }
            ev.push(Event::EjectBrass);
            if in_start(h) && def.reload_begin_bonus == 0 {
                return;
            }
            if h.timer_ms != 0 {
                let add = if in_start(h) {
                    start_add_point(def)
                } else {
                    main_add_point(def, h, false)
                };
                let bolt = if add <= def.rechamber_bolt_time {
                    1
                } else {
                    def.rechamber_bolt_time
                };
                if add - bolt > 0 {
                    h.weapon_ready_wait = add - bolt;
                    return;
                }
            }
            reload_clip(def, h, ev);
            return;
        }
    }
    reload_clip(def, h, ev);
    // Fast Mag flips here. BO2 tests and flips the selected weapon (ps->weapon) whichever hand
    // reloaded, so the hand's flag always flips and the player-state write-back applies the
    // selected weapon's dual mag test (pm/reload.rs). `quick` still gates on this hand's own.
    h.quick_reload = !h.quick_reload;
}

/// End of a one-by-one reload: closing anim, or straight to ready.
fn end_or_ready(def: &ReloadDef, h: &mut Hand, ev: &mut Vec<Event>) {
    if def.reload_end == 0 {
        to_ready(h);
    } else {
        h.phase = state::RELOAD_END;
        start_anim(h, refill_clips::RELOAD_FINISH);
        h.timer_ms = def.reload_end;
        ev.push(Event::ReloadEnd);
    }
}

/// FinishReloadStart (13/14/15).
pub fn finish_start(
    def: &ReloadDef,
    h: &mut Hand,
    inp: &Input,
    delayed: bool,
    ev: &mut Vec<Event>,
) {
    if delayed {
        delayed_action(def, h, ev);
    }
    if h.timer_ms != 0 {
        return;
    }
    if def.segmented && inp.fire {
        h.phase = state::RELOAD_START_INTERUPT;
    }
    if (h.phase != state::RELOAD_START_INTERUPT || h.mag_rounds == 0) && allow(def, h) {
        set_reloading(def, h, ev);
        return;
    }
    h.needs_rechamber = false;
    end_or_ready(def, h, ev);
}

/// FinishReload (11/12).
pub fn finish_reload(
    def: &ReloadDef,
    h: &mut Hand,
    inp: &Input,
    delayed: bool,
    ev: &mut Vec<Event>,
) {
    if delayed {
        delayed_action(def, h, ev);
    }
    if h.timer_ms != 0 {
        return;
    }
    if def.segmented && inp.fire {
        h.phase = state::RELOADING_INTERUPT;
    }
    h.needs_rechamber = false;
    if def.manual_cycle && def.shots_before_rechamber != 0 {
        h.shots_from_chamber = 0;
    }
    if def.charge_shot {
        h.charge = 0;
    }
    if !def.segmented {
        to_ready(h);
        return;
    }
    if h.phase != state::RELOADING_INTERUPT && allow(def, h) {
        set_reloading(def, h, ev);
        return;
    }
    end_or_ready(def, h, ev);
}

/// FinishReloadEnd (16).
pub fn finish_end(h: &mut Hand) {
    to_ready(h);
}

/// The state-switch part for the reload states. Returns false for a state this file does not own.
pub fn finish(
    def: &ReloadDef,
    h: &mut Hand,
    inp: &Input,
    delayed: bool,
    ev: &mut Vec<Event>,
) -> bool {
    match h.phase {
        state::RELOADING | state::RELOADING_INTERUPT => finish_reload(def, h, inp, delayed, ev),
        state::RELOAD_START | state::RELOAD_START_RECHAMBER | state::RELOAD_START_INTERUPT => {
            finish_start(def, h, inp, delayed, ev)
        }
        state::RELOAD_END => finish_end(h),
        _ => return false,
    }
    true
}

/// The reload slice of one weapon tick in BO2's order: timers, reload check, then the state finish
/// (when both timers are 0 or the delay fired). Other files' checks run between these in the real tick.
pub fn tick(def: &ReloadDef, h: &mut Hand, inp: &Input, ev: &mut Vec<Event>) {
    let dt = step_ms(h, inp);
    let delayed = count_down(h, dt);
    check(def, h, inp, ev);
    if !delayed && (h.timer_ms != 0 || h.weapon_ready_wait != 0) {
        return;
    }
    finish(def, h, inp, delayed, ev);
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: i32 = 16;

    fn ar() -> ReloadDef {
        ReloadDef {
            bullet: true,
            magazine_capacity: 30,
            reload: 2000,
            reload_empty: 2600,
            reload_add: 1200,
            reload_empty_add: 1500,
            reload_quick: 1500,
            reload_quick_empty: 1900,
            reload_quick_add: 900,
            reload_quick_empty_add: 1100,
            ..Default::default()
        }
    }

    fn shotgun() -> ReloadDef {
        ReloadDef {
            bullet: true,
            magazine_capacity: 7,
            reload: 600,
            reload_empty: 600,
            reload_add: 300,
            reload_start: 500,
            reload_end: 700,
            segmented: true,
            ammo_add: 1,
            ..Default::default()
        }
    }

    fn hand(clip: i32, stock: i32) -> Hand {
        Hand {
            mag_rounds: clip,
            spare_rounds: stock,
            ..Default::default()
        }
    }

    /// Runs ticks until `until` holds or `max` ms pass; returns elapsed ms.
    fn run(
        def: &ReloadDef,
        h: &mut Hand,
        inp: &Input,
        ev: &mut Vec<Event>,
        max: i32,
        until: impl Fn(&Hand) -> bool,
    ) -> i32 {
        let mut t = 0;
        while t < max {
            tick(def, h, inp, ev);
            t += inp.frame_ms;
            if until(h) {
                break;
            }
        }
        t
    }

    fn press() -> Input {
        Input {
            frame_ms: MS,
            reload: true,
            ..Default::default()
        }
    }

    fn idle_in() -> Input {
        Input {
            frame_ms: MS,
            ..Default::default()
        }
    }

    #[test]
    fn normal_reload_ammo_at_add_point_ready_at_end() {
        let d = ar();
        let mut h = hand(10, 90);
        let mut ev = vec![];
        tick(&d, &mut h, &press(), &mut ev);
        assert_eq!(h.phase, state::RELOADING);
        assert_eq!(h.anim & !refill_clips::TOGGLE, refill_clips::RELOAD);
        assert_eq!(
            ev,
            vec![
                Event::ResetAds,
                Event::ReloadStartNotify,
                Event::Reload,
                Event::ThirdPersonReload
            ]
        );
        let t = run(&d, &mut h, &idle_in(), &mut ev, 5000, |h| h.mag_rounds == 30);
        assert!((1200 - MS..=1200 + MS).contains(&(t + MS)), "ammo at {t}");
        assert_eq!(h.phase, state::RELOADING);
        assert_eq!(h.spare_rounds, 70);
        let t2 = run(&d, &mut h, &idle_in(), &mut ev, 5000, |h| {
            h.phase == state::READY
        });
        assert!(
            (2000 - 2 * MS..=2000 + MS).contains(&(t + MS + t2)),
            "ready at {}",
            t + MS + t2
        );
    }

    #[test]
    fn empty_reload_uses_empty_row() {
        let d = ar();
        let mut h = hand(0, 90);
        let mut ev = vec![];
        tick(&d, &mut h, &idle_in(), &mut ev);
        assert_eq!(h.phase, state::RELOADING, "auto-reload on empty");
        assert_eq!(h.anim & !refill_clips::TOGGLE, refill_clips::RELOAD_DRY);
        assert!(ev.contains(&Event::ReloadFromEmpty));
        assert_eq!(h.timer_ms, 2600);
        assert_eq!(h.weapon_ready_wait, 1500);
    }

    #[test]
    fn fast_reload_perk_halves_the_reload() {
        let d = ar();
        let mut h = hand(10, 90);
        let mut ev = vec![];
        let inp = Input {
            fast_reload: true,
            ..press()
        };
        tick(&d, &mut h, &inp, &mut ev);
        let t = run(
            &d,
            &mut h,
            &Input {
                fast_reload: true,
                ..idle_in()
            },
            &mut ev,
            5000,
            |h| h.phase == state::READY,
        );
        assert!(
            (1000 - 2 * MS..=1000 + 2 * MS).contains(&(t + MS)),
            "ready at {}",
            t + MS
        );
    }

    #[test]
    fn fast_mag_alternates_quick_normal() {
        let d = ReloadDef {
            dual_mag: true,
            ..ar()
        };
        let mut h = Hand {
            quick_reload: true,
            ..hand(10, 200)
        };
        let mut kinds = vec![];
        for _ in 0..3 {
            let mut ev = vec![];
            h.mag_rounds = 10;
            tick(&d, &mut h, &press(), &mut ev);
            kinds.push(h.anim & !refill_clips::TOGGLE);
            run(&d, &mut h, &idle_in(), &mut ev, 5000, |h| {
                h.phase == state::READY
            });
        }
        assert_eq!(
            kinds,
            vec![refill_clips::RELOAD_QUICK, refill_clips::RELOAD, refill_clips::RELOAD_QUICK]
        );
    }

    #[test]
    fn quick_add_clamped_by_normal_total() {
        // Quick reload with no quick add time waits for the NORMAL reload time (BO2 quirk).
        let d = ReloadDef {
            dual_mag: true,
            reload_quick_add: 0,
            reload_add: 0,
            ..ar()
        };
        let mut h = Hand {
            quick_reload: true,
            ..hand(10, 90)
        };
        let mut ev = vec![];
        tick(&d, &mut h, &press(), &mut ev);
        assert_eq!(h.timer_ms, 1500);
        assert_eq!(h.weapon_ready_wait, 2000);
    }

    #[test]
    fn cancel_before_add_point_keeps_old_clip() {
        let d = ar();
        let mut h = hand(10, 90);
        let mut ev = vec![];
        tick(&d, &mut h, &press(), &mut ev);
        run(&d, &mut h, &idle_in(), &mut ev, 1000, |_| false);
        // A switch/knife/grenade replaces the state; the delay never fires.
        h.phase = state::DROPPING;
        h.weapon_ready_wait = 0;
        assert_eq!(h.mag_rounds, 10);
    }

    #[test]
    fn shotgun_one_by_one() {
        let d = shotgun();
        let mut h = hand(4, 20);
        let mut ev = vec![];
        tick(&d, &mut h, &press(), &mut ev);
        assert_eq!(h.phase, state::RELOAD_START);
        assert_eq!(h.anim & !refill_clips::TOGGLE, refill_clips::RELOAD_BEGIN);
        let mut seen = vec![h.phase];
        let mut t = MS;
        while h.phase != state::READY && t < 10000 {
            tick(&d, &mut h, &idle_in(), &mut ev);
            t += MS;
            if *seen.last().unwrap() != h.phase {
                seen.push(h.phase);
            }
        }
        assert_eq!(h.mag_rounds, 7);
        assert_eq!(h.spare_rounds, 17);
        assert_eq!(
            seen,
            vec![
                state::RELOAD_START,
                state::RELOADING,
                state::RELOAD_END,
                state::READY
            ]
        );
        assert_eq!(ev.iter().filter(|e| **e == Event::ReloadAddAmmo).count(), 3);
        // 500 start + 3 x 600 shells + 700 end = 3000; each of the 5 steps ends on the next 16 ms frame.
        assert!((3000..=3000 + 5 * MS).contains(&t), "done at {t}");
    }

    #[test]
    fn shotgun_fire_after_40_percent_interrupts_start() {
        let d = shotgun();
        let mut h = hand(4, 20);
        let mut ev = vec![];
        tick(&d, &mut h, &press(), &mut ev);
        // 30% in: ignored.
        run(&d, &mut h, &idle_in(), &mut ev, 150, |_| false);
        let fire = Input {
            fire: true,
            ..idle_in()
        };
        tick(&d, &mut h, &fire, &mut ev);
        assert_eq!(h.phase, state::RELOAD_START);
        // 50% in, fresh press: interrupt.
        run(&d, &mut h, &idle_in(), &mut ev, 80, |_| false);
        tick(&d, &mut h, &fire, &mut ev);
        assert_eq!(h.phase, state::RELOAD_START_INTERUPT);
        run(&d, &mut h, &idle_in(), &mut ev, 5000, |h| {
            h.phase == state::READY
        });
        // Clip was not empty, so no shell goes in; straight to the end anim.
        assert_eq!(h.mag_rounds, 4);
        assert!(ev.contains(&Event::ReloadEnd));
    }

    #[test]
    fn shotgun_fire_mid_shell_stops_after_that_shell() {
        let d = shotgun();
        let mut h = hand(2, 20);
        let mut ev = vec![];
        tick(&d, &mut h, &press(), &mut ev);
        run(&d, &mut h, &idle_in(), &mut ev, 5000, |h| {
            h.phase == state::RELOADING
        });
        tick(
            &d,
            &mut h,
            &Input {
                fire: true,
                ..idle_in()
            },
            &mut ev,
        );
        assert_eq!(h.phase, state::RELOADING_INTERUPT);
        run(&d, &mut h, &idle_in(), &mut ev, 5000, |h| {
            h.phase == state::READY
        });
        assert_eq!(h.mag_rounds, 3);
    }

    #[test]
    fn empty_shotgun_interrupted_start_still_loads_one() {
        let d = shotgun();
        let mut h = hand(0, 20);
        let mut ev = vec![];
        tick(&d, &mut h, &idle_in(), &mut ev);
        run(&d, &mut h, &idle_in(), &mut ev, 300, |_| false);
        tick(
            &d,
            &mut h,
            &Input {
                fire: true,
                ..idle_in()
            },
            &mut ev,
        );
        assert_eq!(h.phase, state::RELOAD_START_INTERUPT);
        run(&d, &mut h, &idle_in(), &mut ev, 5000, |h| {
            h.phase == state::READY
        });
        assert_eq!(h.mag_rounds, 1);
    }

    #[test]
    fn bolt_gun_ejects_then_loads() {
        let d = ReloadDef {
            manual_cycle: true,
            rechamber_bolt_time: 400,
            ..ar()
        };
        let mut h = Hand {
            needs_rechamber: true,
            ..hand(3, 30)
        };
        let mut ev = vec![];
        tick(&d, &mut h, &press(), &mut ev);
        assert_eq!(h.weapon_ready_wait, 400);
        run(&d, &mut h, &idle_in(), &mut ev, 5000, |h| {
            h.phase == state::READY
        });
        let brass = ev.iter().position(|e| *e == Event::EjectBrass).unwrap();
        let ammo = ev.iter().position(|e| *e == Event::ReloadAddAmmo).unwrap();
        assert!(brass < ammo);
        assert_eq!(h.mag_rounds, 30);
        assert!(!h.needs_rechamber);
    }

    #[test]
    fn no_auto_reload_while_sprinting_or_aimed_no_ads_gun() {
        let d = ReloadDef {
            no_ads_auto_reload: true,
            ..ar()
        };
        let mut ev = vec![];
        let mut h = Hand {
            phase: state::SPRINT_LOOP,
            ..hand(0, 90)
        };
        tick(&d, &mut h, &idle_in(), &mut ev);
        assert_eq!(h.phase, state::SPRINT_LOOP);
        let mut h = hand(0, 90);
        tick(
            &d,
            &mut h,
            &Input {
                fully_ads: true,
                ..idle_in()
            },
            &mut ev,
        );
        assert_eq!(h.phase, state::READY);
        tick(&d, &mut h, &idle_in(), &mut ev);
        assert_eq!(h.phase, state::RELOADING);
        // But the button works from sprint.
        let mut h = Hand {
            phase: state::SPRINT_LOOP,
            ..hand(5, 90)
        };
        tick(&d, &mut h, &press(), &mut ev);
        assert_eq!(h.phase, state::RELOADING);
    }

    #[test]
    fn knife_end_on_empty_does_empty_raise() {
        let d = ReloadDef {
            empty_raise_time: 500,
            ..ar()
        };
        let mut h = Hand {
            phase: state::MELEE_END,
            ..hand(0, 90)
        };
        let mut ev = vec![];
        check(&d, &mut h, &idle_in(), &mut ev);
        assert_eq!(h.phase, state::RAISING);
        assert_eq!(h.timer_ms, 500);
        assert_eq!(h.anim & !refill_clips::TOGGLE, refill_clips::EMPTY_TAKE_OUT);
    }

    #[test]
    fn no_partial_reload_rules() {
        let d = ReloadDef {
            forbid_topup_reload: true,
            ..ar()
        };
        assert!(!allow(&d, &hand(5, 90)));
        assert!(allow(&d, &hand(0, 90)));
        let d = ReloadDef {
            forbid_topup_reload: true,
            ammo_add: 10,
            ..ar()
        };
        assert!(allow(&d, &hand(20, 90)));
        assert!(!allow(&d, &hand(21, 90)));
    }
}
