//! Which weapon's model the first-person view shows, and the per-weapon
//! decisions that hang off it: whether a rocket is still in the launcher and
//! whether the gun is drawn at all behind an overlay.

use std::sync::atomic::{AtomicU32, Ordering};

use hud_t6::iris::{
    WeaponOverlayDef, get_weap_reticle_zoom, iris_overlay_configured, view_weapon_draw_gun,
};
use playerstate_t6::game::{AvatarState, get_viewmodel_weapon_index};
use weapon_t6::ammo_table::{clip_table_key, get_clip_for_hand};
use weapon_t6::fire_type::rocket_stays_on_launcher;

use crate::motion::fpv_prepared::FpvWeaponTable;

/// The client whose view is being drawn, remembered so the melee choice below
/// can ask the simulation which melee weapon that client was given.
pub(super) static LOCAL_CLIENT: AtomicU32 = AtomicU32::new(0);

/// The weapon whose viewmodel to show for this player state. Most of the time
/// that is the viewmodel weapon itself. During a melee strike it is the melee
/// weapon granted to the player (a knife or knuckles) when there is one, else
/// the weapon's own melee model.
pub(crate) fn fpv_viewmodel_weapon(ps: &AvatarState, weapons: &FpvWeaponTable) -> u32 {
    let shown_in = get_viewmodel_weapon_index(ps);
    let striking = matches!(
        ps.weapon_state_t6,
        weapon_t6::state::MELEE_INIT | weapon_t6::state::MELEE_FIRE
    );
    if !(striking && shown_in == ps.weapon) {
        return shown_in;
    }
    let own_melee = weapons.melee_weapon_of(shown_in);
    let granted = sim::melee_weapon(LOCAL_CLIENT.load(Ordering::Relaxed));
    // A granted melee weapon wins, unless the held gun is its own knife.
    let granted_applies =
        granted != 0 && own_melee != shown_in && weapons.info_of(granted).is_some();
    if granted_applies { granted } else { own_melee }
}

/// Whether the launcher's rocket should be modelled on the gun right now: it
/// is in the tube except while reloading, before the new one is shown.
pub(super) fn fpv_rocket_should_attach(
    weapons: &FpvWeaponTable,
    gun_catalog_no: u32,
    pstate: Option<&AvatarState>,
) -> bool {
    // With no player state there is nothing to hide the rocket for.
    let Some(pstate) = pstate else { return true };
    let Some(facts) = weapons.info_of(gun_catalog_no) else {
        return false;
    };
    let shown = fpv_viewmodel_weapon(pstate, weapons);
    let key = clip_table_key(facts.motion_clip_slot, shown);
    rocket_stays_on_launcher(
        get_clip_for_hand(&pstate.magazine_rows, key, 0),
        pstate.weapon_state_t6,
        pstate.weapon_timer,
        facts.reload_duration_millis,
        facts.rocket_reappear_millis,
    )
}

/// Decides whether the gun model is drawn: false while a scope overlay covers
/// it. The second field names the reason when the gun is skipped. `None` when
/// the weapon table has nothing on the shown weapon.
pub(super) fn drawgun_admit(
    ps: &AvatarState,
    weapons: Option<&FpvWeaponTable>,
    heading_into_ads: bool,
) -> Option<(bool, Option<&'static str>)> {
    let table = weapons?;
    let shown = fpv_viewmodel_weapon(ps, table);
    let facts = table.info_of(shown)?;

    let overlay = WeaponOverlayDef {
        hud_layer_surface: u32::from(table.hud_iris_overlay(shown)),
        hud_crosshair_layer: u32::try_from(facts.hud_crosshair_layer).unwrap_or(0),
        overlay_width: facts.scope_pic_w,
        overlay_height: facts.scope_pic_h,
        ..WeaponOverlayDef::default()
    };
    // The reticle fades over a fraction of the zoom; which end depends on
    // whether the player is going into or out of the sights.
    let fade_window = if heading_into_ads {
        facts.aim_in_share
    } else {
        facts.aim_out_share
    };
    let reticle_up = if fade_window > 0.0 {
        get_weap_reticle_zoom(ps.weapon_raise_fraction, fade_window).is_some()
    } else {
        // No window: only fully aimed counts, and there is no division.
        ps.weapon_raise_fraction >= 1.0
    };
    let iris = iris_overlay_configured(&overlay) && reticle_up;
    // bo2zm 10-13: BO2MP_NOGUN=1 is cg_drawgun 0 for proof shots (bodies, not the gun).
    static DRAW_GUN: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let draw_gun = *DRAW_GUN.get_or_init(|| std::env::var("BO2MP_NOGUN").map_or(true, |v| v != "1"));
    Some((
        view_weapon_draw_gun(iris, draw_gun, false),
        iris.then_some("iris"),
    ))
}

/// [`drawgun_admit`] as the 0 / 1 value the pose product carries.
pub(super) fn drawgun_flag(
    ps: &AvatarState,
    weapons: Option<&FpvWeaponTable>,
    heading_into_ads: bool,
) -> Option<i32> {
    drawgun_admit(ps, weapons, heading_into_ads).map(|(drawn, _)| i32::from(drawn))
}
