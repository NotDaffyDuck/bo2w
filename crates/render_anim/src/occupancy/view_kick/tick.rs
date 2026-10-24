//! The once-per-frame stepping of the local player's view kick, sway and scope state.
//!
//! This runs before the camera is placed. It sends the aim what the camera owes it
//! (so bullets go where the sights are), fires the gun's kick for this frame's shots,
//! and lets the kick settle and the sway follow the view.

use asset_core::AssetNamespace;
use asset_game::WeaponKickFacts;
use assets::PreparedWeapons;
use bevy::prelude::*;
use frame::LifeStarted;
use hud_t6::flashbang::shellshock_remaining_ms;
use net::{AppliedEntityEventWalk, FrameClock, LocalPresentClient, LookState, PresentedSnapshot};
use playerstate_t6::game::{PlayerState, get_viewmodel_weapon_index, pm_flags};
use weapon_t6::recoil::{
    CenterInput, FireRecoilInput, KickState, RecoilDvars, fire_recoil, gun_spring, settle_view_kick,
    view_kick_center_speed,
};
use weapon_t6::state::{FIRING, READY, is_any_firing};
use weapon_t6::sway::sway_shellshock_landing_scale;

use super::flags::shot_log_on;
use super::params::{dual_wield_kick, kick_params};
use super::state::{PendingViewHurt, SessionViewKick, bo2_camera};

/// A new life of the local player starts the view from rest.
pub fn reset_view_kick_on_life_started(
    local: Res<LocalPresentClient>,
    mut started: MessageReader<LifeStarted>,
    mut kick: ResMut<SessionViewKick>,
    mut hurt: ResMut<PendingViewHurt>,
) {
    let me = local.0.0;
    for event in started.read() {
        if event.client != me {
            continue;
        }
        kick.clear_for_new_life();
        hurt.0 = 0;
    }
}

/// While stunned or shocked, a Black Ops II gun sways by its own `swayShellShockScale`,
/// easing back to normal as the shock wears off. A gun with no such scale sways normally.
pub(super) fn shock_sway_scale(remaining_ms: i32, duration_ms: i32, gun_scale: f32) -> f32 {
    if gun_scale > 0.0 {
        sway_shellshock_landing_scale(remaining_ms, duration_ms, gun_scale)
    } else {
        1.0
    }
}

/// Gives the aim what the camera measured last frame that it must take: the hit's pitch,
/// the walk bob and the iron-sight gun turn.
///
/// Done before anything can return early and after a switch to an older gun, so the
/// aim gives back exactly what these put in.
fn send_aim_debts(kick: &mut SessionViewKick, look: Option<&mut LookState>) {
    let [pitch, yaw] = kick.take_aim_debts();
    if let Some(look) = look {
        look.kick_pending[0] += pitch;
        look.kick_pending[1] += yaw;
    }
}

/// The kick of this frame's shots for a gun that has no Black Ops II facts.
///
/// A Black Ops II gun with a second gun in the other hand adds the two guns' hip ranges.
fn seed_shots_the_old_way(
    kick: &mut SessionViewKick,
    ps: &PlayerState,
    reg: &PreparedWeapons,
    viewmodel: u32,
    own: &WeaponKickFacts,
    shots: u32,
) {
    let mut params = kick_params(own);
    let left_hand = reg.0.dual_wield_weapon_of(viewmodel);
    if bo2_camera(kick)
        && left_hand != 0
        && let Some(left) = reg.0.facts_of(left_hand)
    {
        params = dual_wield_kick(
            params,
            &kick_params(&left.kick),
            is_any_firing(ps.weapon_state_t6),
            is_any_firing(ps.weapon_state_left_t6),
        );
    }
    let restricted = ps.weapon_restrict_kick_time > 0;
    for _ in 0..shots {
        kick.state
            .seed_fire(&params, ps.f_weapon_pos_frac, ps.weap_flags, ps.recoil_scale, restricted);
    }
    kick.seeded_this_frame = shots;
    if shot_log_on() {
        diag::info!(
            Fpv,
            "shot kick: camera off aim by [{:.2}, {:.2}] ads {:.2}",
            kick.state.kick_angles[0],
            kick.state.kick_angles[1],
            ps.f_weapon_pos_frac
        );
    }
}

/// For a Black Ops II gun the kick moves the aim: queues this frame's change for the
/// next command (and the scope drift the camera measured last frame), so the bullets
/// go where the sights are.
fn send_kick_to_aim(kick: &mut SessionViewKick, look: &mut LookState) {
    for axis in 0..2 {
        let target = kick.state.kick_angles[axis];
        look.kick_pending[axis] += target - kick.aim_applied[axis];
        kick.aim_applied[axis] = target;
        look.kick_pending[axis] += kick.idle_aim_pending[axis];
        kick.idle_aim_pending[axis] = 0.0;
    }
}

/// Variable Zoom: a melee press while scoped toggles the zoom.
fn follow_zoom_presses(kick: &mut SessionViewKick, look: &mut LookState) {
    look.melee_is_zoom = kick.vzoom_scoped;
    look.breath_is_switch = kick.optic_ready;
    if look.zoom_presses == kick.vzoom_presses {
        return;
    }
    if kick.vzoom_scoped {
        kick.vzoom_in = !kick.vzoom_in;
    }
    kick.vzoom_presses = look.zoom_presses;
}

/// The view angles the gun sway follows: the view without the parts that belong to the aim.
///
/// The sway follows the view and its kick, not the drift, hit, bob and gun turn that the
/// aim carries for the server's shot.
fn sway_follows(kick: &SessionViewKick, ps: &PlayerState) -> [f32; 3] {
    if !kick.aim_kick {
        return ps.viewangles;
    }
    let [pitch, yaw] = kick.aim_movement();
    [ps.viewangles[0] - pitch, ps.viewangles[1] - yaw, ps.viewangles[2]]
}

/// Steps the local player's kick, sway and scope state one client frame.
pub fn tick_session_view_kick(
    clock: Res<FrameClock>,
    presented: Res<PresentedSnapshot>,
    local: Res<LocalPresentClient>,
    weapons: Option<Res<PreparedWeapons>>,
    walk: Option<Res<AppliedEntityEventWalk>>,
    t6_facts: Option<Res<sim::t6_viewmodel::T6GunFactsRes>>,
    mut kick: ResMut<SessionViewKick>,
    mut look: Option<ResMut<LookState>>,
) {
    let Some(ps) = presented.player(local.0) else {
        return;
    };
    let viewmodel = get_viewmodel_weapon_index(ps);
    if viewmodel != kick.last_weapon_id {
        kick.on_weapon_changed(viewmodel);
    }
    send_aim_debts(&mut kick, look.as_deref_mut());
    kick.t6.facts = None;

    let Some(reg) = weapons.as_deref() else {
        return;
    };
    let Some(facts) = reg.0.facts_of(viewmodel) else {
        return;
    };
    if !facts.body_resolved {
        return;
    }

    let dt = clock.frametime_secs();
    let frame_ms = clock.frametime();
    let frac = ps.f_weapon_pos_frac;
    kick.seeded_this_frame = 0;
    let shots = walk.map_or(0, |w| w.local_fire);

    // bo2mp gun brain: a gun BO2's facts know kicks by BO2's own rules.
    let own_table = t6_facts
        .as_deref()
        .filter(|table| !table.0.name(viewmodel).is_empty());
    let own_spread = own_table.map(|table| table.0.get(viewmodel).spread);
    kick.t6_land_frac = frac;
    kick.t6_land_factor = own_spread
        .as_ref()
        .map(|spread| weapon_t6::recoil::gun_landing_scale(spread, frac) / weapon_t6::view_bob::VIEWWEAPON_LAND_SCALE);
    match (own_table, own_spread.as_ref()) {
        (Some(table), Some(spread)) => {
            let left_hand = reg.0.dual_wield_weapon_of(viewmodel);
            tick_t6_kick(&mut kick, ps, spread, left_hand, table, shots, clock.time(), frame_ms);
        }
        _ if shots > 0 => seed_shots_the_old_way(&mut kick, ps, reg, viewmodel, &facts.kick, shots),
        _ => {}
    }

    if own_spread.is_none() {
        let slot = i32::from(viewmodel != 0);
        kick.state.advance(&kick_params(&facts.kick), slot, frac, frame_ms);
    }
    kick.aim_kick = reg.0.namespace_of(viewmodel) == Some(AssetNamespace::T6);
    if kick.aim_kick
        && let Some(look) = look.as_deref_mut()
    {
        send_kick_to_aim(&mut kick, look);
    }
    if let Some(look) = look.as_deref_mut() {
        follow_zoom_presses(&mut kick, look);
    }

    let sway_view = sway_follows(&kick, ps);
    // bo2mp gun brain: BO2's own sway for a gun the T6 facts know (the legacy sway below
    // still runs for its other readers).
    if bo2_camera(&kick)
        && let Some(table) = own_table
    {
        let own = table.0.get(viewmodel);
        kick.t6.facts = Some((own.spread, own.ads));
        let shock_left = if ps.shellshock_duration > 0 {
            ps.shellshock_time
                .wrapping_add(ps.shellshock_duration)
                .wrapping_sub(clock.time())
                .max(0)
        } else {
            0
        };
        kick.t6.tick_sway(viewmodel, frac, sway_view, dt, frame_ms, shock_left);
    }

    let scoped = facts.overlay_reticle != 0;
    if bo2_camera(&kick) && facts.aim_down_sight && scoped && frac > 0.0 {
        kick.sway.follow_view(sway_view);
        return;
    }
    let shock_scale = if bo2_camera(&kick) {
        let remaining = shellshock_remaining_ms(clock.time(), ps.shellshock_time, ps.shellshock_duration)
            .filter(|r| *r >= 1)
            .unwrap_or(0);
        shock_sway_scale(remaining, ps.shellshock_duration, facts.sway.sway_shell_shock_scale)
    } else {
        1.0
    };
    kick.sway.advance(
        facts.sway.hip_params(),
        facts.sway.ads_params(),
        sway_view,
        frac,
        facts.aim_down_sight,
        scoped,
        shock_scale,
        dt,
    );
}

/// The player's stance as the kick-centering speed table reads it: 0 standing, 1 prone, 2 crouched.
fn stance_index(pm: u32) -> i32 {
    if pm & pm_flags::PRONE != 0 {
        1
    } else if pm & pm_flags::CROUCH != 0 {
        2
    } else {
        0
    }
}

/// bo2mp gun brain: one client frame of BO2's kick.
///
/// This frame's shots (CG_FireWeapon), the right gun's shot counter
/// (CG_AddPlayerWeaponFx), then the camera kick settling (CG_KickAngles) and the gun's
/// recoil spring. The result is copied into `kick.state`, which the camera, the aim and
/// the gun placement already read.
#[allow(clippy::too_many_arguments)]
fn tick_t6_kick(
    kick: &mut SessionViewKick,
    ps: &PlayerState,
    spread: &weapon_t6::pm::spread::SpreadFacts,
    left_hand: u32,
    table: &sim::t6_viewmodel::T6GunFactsRes,
    shots: u32,
    now: i32,
    frame_ms: i32,
) {
    // Every BO2 firing state counts as one "firing" for the counter and the dual-wield ranges.
    let simple = |state: i32| if is_any_firing(state) { FIRING } else { READY };
    let right_state = simple(ps.weapon_state_t6);
    let left_state = simple(ps.weapon_state_left_t6);
    let left_gun = (left_hand != 0).then(|| table.0.get(left_hand).spread);
    let dvars = RecoilDvars::default();
    let frac = ps.f_weapon_pos_frac;

    for _ in 0..shots {
        let held = kick.t6_held;
        let input = FireRecoilInput {
            facts: spread,
            dual: left_gun.as_ref(),
            frac,
            restrict_kick_time: ps.weapon_restrict_kick_time,
            held,
            now,
            charge_level: None,
            gas: false,
            weaponstate: right_state,
            weaponstate_left: left_state,
        };
        let mut seed = kick.t6_seed;
        fire_recoil(&mut kick.t6_kick, &input, &mut seed, &dvars);
        kick.t6_seed = seed;
        kick.t6_held = true;
        if shot_log_on() {
            let k: &KickState = &kick.t6_kick;
            diag::info!(
                Fpv,
                "t6 kick: ads {:.2} held {} reduction {:.3} view avel [{:.2}, {:.2}, {:.2}] gun speed [{:.2}, {:.2}] restrict {}",
                frac,
                held as u8,
                k.ads_recoil_reduction,
                k.view_avel[0],
                k.view_avel[1],
                k.view_avel[2],
                k.gun_speed[0],
                k.gun_speed[1],
                ps.weapon_restrict_kick_time
            );
        }
    }
    kick.seeded_this_frame = shots;
    if !is_any_firing(ps.weapon_state_t6) && !is_any_firing(ps.weapon_state_left_t6) {
        kick.t6_held = false;
    }

    if frame_ms > 0 {
        let center = view_kick_center_speed(
            &CenterInput {
                facts: Some(spread),
                dual: left_gun.as_ref(),
                frac,
                charge_level: None,
                weaponstate: right_state,
                weaponstate_left: left_state,
                stance: stance_index(ps.pm_flags),
            },
            &dvars,
        );
        settle_view_kick(&mut kick.t6_kick, frame_ms, center);
        gun_spring(&mut kick.t6_kick, spread, frac, frame_ms as f32 * 0.001);
    }
    kick.state.kick_angles = kick.t6_kick.view_angles;
    kick.state.kick_avel = kick.t6_kick.view_avel;
    kick.state.gun_angles = kick.t6_kick.gun_angles;
    kick.state.gun_speed = kick.t6_kick.gun_speed;
}
