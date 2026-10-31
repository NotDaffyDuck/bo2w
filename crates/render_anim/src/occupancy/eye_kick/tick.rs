//! The once-per-frame stepping of the local player's view kick, sway and scope state.
//!
//! This runs before the camera is placed. It sends the aim what the camera owes it
//! (so bullets go where the sights are), fires the gun's kick for this frame's shots,
//! and lets the kick settle and the sway follow the view.

use asset_core::AssetCategoryScope;
use asset_game::WeaponRecoilSummary;
use assets::StagedWeapons;
use bevy::prelude::*;
use frame::SpawnLifeBegun;
use hud_t6::flashbang::shellshock_left_ms;
use net::{InstalledEntityEventTraversal, FrameClock, HostViewClient, ViewAimAccumulator, ShownWorldFrame};
use playerstate_t6::game::{AvatarState, get_viewmodel_weapon_index, movement_state_bits};
use weapon_t6::recoil::{
    CenterInput, FireRecoilInput, KickState, RecoilDvars, fire_recoil, gun_spring, settle_view_kick,
    view_kick_center_speed,
};
use weapon_t6::state::{FIRING, READY, is_any_firing};
use weapon_t6::sway::daze_touchdown_factor;

use super::flags::shot_log_on;
use super::params::{dual_wield_kick, kick_params};
use super::state::{QueuedHurtFlinch, CameraRecoilState, bo2_camera};

/// A new life of the local player starts the view from rest.
pub fn clear_recoil_at_respawn(
    mut lives: MessageReader<SpawnLifeBegun>,
    mut hurt: ResMut<QueuedHurtFlinch>,
    mut kick: ResMut<CameraRecoilState>,
    me: Res<HostViewClient>,
) {
    let mine = lives.read().filter(|life| life.client == me.0.0).count();
    if mine > 0 {
        hurt.0 = 0;
        kick.reset_on_respawn();
    }
}

/// While stunned or shocked, a Black Ops II gun sways by its own `swayShellShockScale`,
/// easing back to normal as the shock wears off. A gun with no such scale sways normally.
pub(super) fn shock_sway_scale(ms_to_go: i32, span_ms: i32, gun_scale: f32) -> f32 {
    match gun_scale > 0.0 {
        true => daze_touchdown_factor(ms_to_go, span_ms, gun_scale),
        false => 1.0,
    }
}

/// Gives the aim what the camera measured last frame that it must take: the hit's pitch,
/// the walk bob and the iron-sight gun turn.
///
/// Done before anything can return early and after a switch to an older gun, so the
/// aim gives back exactly what these put in.
fn send_aim_debts(kick: &mut CameraRecoilState, look: Option<&mut ViewAimAccumulator>) {
    let debts = kick.take_aim_debts();
    let Some(look) = look else {
        return;
    };
    for (pending, owed) in look.kick_pending.iter_mut().zip(debts) {
        *pending += owed;
    }
}

/// The kick of this frame's shots for a gun that has no Black Ops II facts.
///
/// A Black Ops II gun with a second gun in the other hand adds the two guns' hip ranges.
fn seed_shots_the_old_way(
    kick_in: &mut CameraRecoilState,
    pstate: &AvatarState,
    weapons: &StagedWeapons,
    gun: u32,
    facts: &WeaponRecoilSummary,
    shots: u32,
) {
    let off_hand = weapons.0.dual_wield_weapon_of(gun);
    let off_facts = match bo2_camera(kick_in) && off_hand != 0 {
        true => weapons.0.summary_for(off_hand),
        false => None,
    };
    let params = match off_facts {
        Some(off) => dual_wield_kick(
            kick_params(facts),
            &kick_params(&off.kick),
            is_any_firing(pstate.weapon_state_t6),
            is_any_firing(pstate.weapon_state_left_t6),
        ),
        None => kick_params(facts),
    };
    let restricted = pstate.gun_lockout_kick_stamp > 0;
    (0..shots).for_each(|_| {
        kick_in.state
            .start_shot_kick(&params, pstate.weapon_raise_fraction, pstate.weapon_state_bits, pstate.recoil_scale, restricted)
    });
    kick_in.freshly_primed_count = shots;
    if shot_log_on() {
        diag::info!(
            ViewModelPerspective,
            "shot kick: camera off aim by [{:.2}, {:.2}] ads {:.2}",
            kick_in.state.recoil_tilt[0],
            kick_in.state.recoil_tilt[1],
            pstate.weapon_raise_fraction
        );
    }
}

/// For a Black Ops II gun the kick moves the aim: queues this frame's change for the
/// next command (and the scope drift the camera measured last frame), so the bullets
/// go where the sights are.
fn send_kick_to_aim(kick: &mut CameraRecoilState, look: &mut ViewAimAccumulator) {
    for axis in 0..=1 {
        let now = kick.state.recoil_tilt[axis];
        look.kick_pending[axis] += now - kick.aim_applied[axis];
        look.kick_pending[axis] += std::mem::take(&mut kick.idle_aim_pending[axis]);
        kick.aim_applied[axis] = now;
    }
}

/// Variable Zoom: a melee press while scoped toggles the zoom.
fn follow_zoom_presses(kick: &mut CameraRecoilState, look: &mut ViewAimAccumulator) {
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
fn sway_follows(kick: &CameraRecoilState, ps: &AvatarState) -> [f32; 3] {
    if !kick.aim_kick {
        return ps.look_angles;
    }
    let [pitch, yaw] = kick.aim_movement();
    [ps.look_angles[0] - pitch, ps.look_angles[1] - yaw, ps.look_angles[2]]
}

/// Steps the local player's kick, sway and scope state one client frame.
pub fn advance_session_view_kick(
    mut kick_v: ResMut<CameraRecoilState>,
    mut look: Option<ResMut<ViewAimAccumulator>>,
    clock: Res<FrameClock>,
    viewer: Res<HostViewClient>,
    shown_frame: Res<ShownWorldFrame>,
    walk: Option<Res<InstalledEntityEventTraversal>>,
    gun_defs: Option<Res<StagedWeapons>>,
    own_facts: Option<Res<sim::gun_view_facts::T6GunFactsRes>>,
) {
    let Some(pstate) = shown_frame.player(viewer.0) else {
        return;
    };
    let gun = get_viewmodel_weapon_index(pstate);
    if kick_v.last_weapon_id != gun {
        kick_v.on_weapon_changed(gun);
    }
    send_aim_debts(&mut kick_v, look.as_deref_mut());
    kick_v.t6.facts = None;

    let Some(gun_defs) = gun_defs.as_deref() else {
        return;
    };
    let Some(facts) = gun_defs.0.summary_for(gun).filter(|facts| facts.payload_linked) else {
        return;
    };

    let now = clock.time();
    let frame_ms = clock.step_millis_elapsed();
    let secs = clock.step_duration_seconds();
    let frac = pstate.weapon_raise_fraction;
    let shots = walk.map_or(0, |walk| walk.self_trigger_pull);
    kick_v.freshly_primed_count = 0;

    // bo2mp gun brain: a gun BO2's facts know kicks by BO2's own rules.
    let table = own_facts
        .as_deref()
        .filter(|own| !own.0.name(gun).is_empty());
    let spread = table.map(|own| own.0.get(gun).spread);
    kick_v.t6_land_frac = frac;
    kick_v.t6_land_factor = spread.as_ref().map(|spread| {
        let landing = weapon_t6::recoil::gun_landing_scale(spread, frac);
        landing / weapon_t6::view_bob::VIEWMODEL_TOUCHDOWN_FACTOR
    });
    match (table, spread.as_ref()) {
        (Some(table), Some(spread)) => {
            let off_hand = gun_defs.0.dual_wield_weapon_of(gun);
            tick_t6_kick(&mut kick_v, pstate, spread, off_hand, table, shots, now, frame_ms);
        }
        _ if shots > 0 => seed_shots_the_old_way(&mut kick_v, pstate, gun_defs, gun, &facts.kick, shots),
        _ => {}
    }

    if spread.is_none() {
        let slot = i32::from(gun != 0);
        kick_v.state.step_on(&kick_params(&facts.kick), slot, frac, frame_ms);
    }
    kick_v.aim_kick = gun_defs.0.scope_for(gun) == Some(AssetCategoryScope::T6);
    if let Some(look) = look.as_deref_mut() {
        if kick_v.aim_kick {
            send_kick_to_aim(&mut kick_v, look);
        }
        follow_zoom_presses(&mut kick_v, look);
    }

    let sway_view = sway_follows(&kick_v, pstate);
    // bo2mp gun brain: BO2's own sway for a gun the T6 facts know (the legacy sway below
    // still runs for its other readers).
    if let Some(table) = table.filter(|_| bo2_camera(&kick_v)) {
        let own = table.0.get(gun);
        kick_v.t6.facts = Some((own.spread, own.ads));
        let shock_left = match pstate.stun_length > 0 {
            true => (pstate.stun_start.wrapping_add(pstate.stun_length).wrapping_sub(now)).max(0),
            false => 0,
        };
        kick_v.t6.tick_sway(gun, frac, sway_view, secs, frame_ms, shock_left);
    }

    let scoped = facts.hud_crosshair_layer != 0;
    let in_scope = bo2_camera(&kick_v) && facts.scoped_aiming && scoped && frac > 0.0;
    if in_scope {
        kick_v.sway.follow_view(sway_view);
        return;
    }
    let shock_scale = match bo2_camera(&kick_v) {
        true => {
            let left = shellshock_left_ms(now, pstate.stun_start, pstate.stun_length)
                .map_or(0, |ms| if ms >= 1 { ms } else { 0 });
            shock_sway_scale(left, pstate.stun_length, facts.sway.drift_stun_k)
        }
        false => 1.0,
    };
    kick_v.sway.progress(
        facts.sway.unaimed_settings(),
        facts.sway.aim_down_settings(),
        sway_view,
        frac,
        facts.scoped_aiming,
        scoped,
        shock_scale,
        secs,
    );
}

/// The player's stance as the kick-centering speed table reads it: 0 standing, 1 prone, 2 crouched.
fn stance_index(pm: u32) -> i32 {
    if pm & movement_state_bits::LYING_FLAT != 0 {
        1
    } else if pm & movement_state_bits::DUCK_BUTTON_BIT != 0 {
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
#[allow(clippy::too_many_arguments, reason = "kept as written")]
fn tick_t6_kick(
    kick: &mut CameraRecoilState,
    ps: &AvatarState,
    spread: &weapon_t6::pm::spread::SpreadFacts,
    off_hand: u32,
    guns: &sim::gun_view_facts::T6GunFactsRes,
    shots: u32,
    now: i32,
    frame_ms: i32,
) {
    // Every BO2 firing state counts as one "firing" for the counter and the dual-wield ranges.
    let simple = |state: i32| match is_any_firing(state) {
        true => FIRING,
        false => READY,
    };
    let right_state = simple(ps.weapon_state_t6);
    let left_state = simple(ps.weapon_state_left_t6);
    let left_gun = (off_hand != 0).then(|| guns.0.get(off_hand).spread);
    let dvars = RecoilDvars::default();
    let frac = ps.weapon_raise_fraction;

    for _ in 0..shots {
        let held = std::mem::replace(&mut kick.t6_held, true);
        let input = FireRecoilInput {
            facts: spread,
            dual: left_gun.as_ref(),
            frac,
            restrict_kick_time: ps.gun_lockout_kick_stamp,
            held,
            now,
            charge_level: None,
            gas: false,
            gun_phase: right_state,
            weaponstate_left: left_state,
        };
        let mut seed = kick.t6_seed;
        fire_recoil(&mut kick.t6_kick, &input, &mut seed, &dvars);
        kick.t6_seed = seed;
        if shot_log_on() {
            let k: &KickState = &kick.t6_kick;
            diag::info!(
                ViewModelPerspective,
                "t6 kick: ads {:.2} held {} reduction {:.3} view avel [{:.2}, {:.2}, {:.2}] gun speed [{:.2}, {:.2}] restrict {}",
                frac,
                held as u8,
                k.ads_recoil_reduction,
                k.view_avel[0],
                k.view_avel[1],
                k.view_avel[2],
                k.gun_swing_rate[0],
                k.gun_swing_rate[1],
                ps.gun_lockout_kick_stamp
            );
        }
    }
    kick.freshly_primed_count = shots;
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
                gun_phase: right_state,
                weaponstate_left: left_state,
                stance: stance_index(ps.movement_state_bits),
            },
            &dvars,
        );
        settle_view_kick(&mut kick.t6_kick, frame_ms, center);
        gun_spring(&mut kick.t6_kick, spread, frac, frame_ms as f32 * 0.001);
    }
    kick.state.recoil_tilt = kick.t6_kick.look_rotation;
    kick.state.punch_spin_rate = kick.t6_kick.view_avel;
    kick.state.weapon_aim_angles = kick.t6_kick.weapon_aim_angles;
    kick.state.gun_swing_rate = kick.t6_kick.gun_swing_rate;
}
