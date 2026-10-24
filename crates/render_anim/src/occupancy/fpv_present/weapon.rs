//! Which weapon's model the first-person view shows, and the per-weapon
//! decisions that hang off it: whether a rocket is still in the launcher and
//! whether the gun is drawn at all behind an overlay.

use std::sync::atomic::{AtomicU32, Ordering};

use hud_t6::iris::{
    WeaponOverlayDef, get_weap_reticle_zoom, iris_overlay_configured, view_weapon_draw_gun,
};
use playerstate_t6::game::{PlayerState, get_viewmodel_weapon_index};
use weapon_t6::ammo_table::{clip_table_key, get_clip_for_hand};
use weapon_t6::fire_type::viewmodel_rocket_should_be_attached;

use crate::anim::fpv_prepared::FpvWeaponTable;

/// The client whose view is being drawn, remembered so the melee choice below
/// can ask the simulation which melee weapon that client was given.
pub(super) static LOCAL_CLIENT: AtomicU32 = AtomicU32::new(0);

/// The weapon whose viewmodel to show for this player state. Most of the time
/// that is the viewmodel weapon itself. During a melee strike it is the melee
/// weapon granted to the player (a knife or knuckles) when there is one, else
/// the weapon's own melee model.
pub(crate) fn fpv_viewmodel_weapon(ps: &PlayerState, weapons: &FpvWeaponTable) -> u32 {
    let shown = get_viewmodel_weapon_index(ps);
    let striking = matches!(
        ps.weapon_state_t6,
        weapon_t6::state::MELEE_INIT | weapon_t6::state::MELEE_FIRE
    );
    if !(striking && shown == ps.weapon) {
        return shown;
    }
    let own_melee = weapons.melee_weapon_of(shown);
    let granted = sim::melee_weapon(LOCAL_CLIENT.load(Ordering::Relaxed));
    // A granted melee weapon wins, unless the held gun is its own knife.
    let granted_applies =
        granted != 0 && own_melee != shown && weapons.facts_of(granted).is_some();
    if granted_applies { granted } else { own_melee }
}

/// Whether the launcher's rocket should be modelled on the gun right now: it
/// is in the tube except while reloading, before the new one is shown.
pub(super) fn fpv_rocket_should_attach(
    weapons: &FpvWeaponTable,
    weapon_id: u32,
    ps: Option<&PlayerState>,
) -> bool {
    // With no player state there is nothing to hide the rocket for.
    let Some(ps) = ps else { return true };
    let Some(facts) = weapons.facts_of(weapon_id) else {
        return false;
    };
    let shown = fpv_viewmodel_weapon(ps, weapons);
    let key = clip_table_key(facts.clip_index, shown);
    viewmodel_rocket_should_be_attached(
        get_clip_for_hand(&ps.ammoclip, key, 0),
        ps.weapon_state_t6,
        ps.weapon_time,
        facts.reload_time_ms,
        facts.reload_show_rocket_time_ms,
    )
}

/// Decides whether the gun model is drawn: false while a scope overlay covers
/// it. The second field names the reason when the gun is skipped. `None` when
/// the weapon table has nothing on the shown weapon.
pub(super) fn drawgun_admit(
    ps: &PlayerState,
    weapons: Option<&FpvWeaponTable>,
    heading_into_ads: bool,
) -> Option<(bool, Option<&'static str>)> {
    let table = weapons?;
    let shown = fpv_viewmodel_weapon(ps, table);
    let facts = table.facts_of(shown)?;

    let overlay = WeaponOverlayDef {
        overlay_material: u32::from(table.overlay_is_hud_iris(shown)),
        overlay_reticle: u32::try_from(facts.overlay_reticle).unwrap_or(0),
        overlay_width: facts.ads_overlay_width,
        overlay_height: facts.ads_overlay_height,
        ..WeaponOverlayDef::default()
    };
    // The reticle fades over a fraction of the zoom; which end depends on
    // whether the player is going into or out of the sights.
    let fade_window = if heading_into_ads {
        facts.ads_zoom_in_frac
    } else {
        facts.ads_zoom_out_frac
    };
    let reticle_up = if fade_window > 0.0 {
        get_weap_reticle_zoom(ps.f_weapon_pos_frac, fade_window).is_some()
    } else {
        // No window: only fully aimed counts, and there is no division.
        ps.f_weapon_pos_frac >= 1.0
    };
    let iris = iris_overlay_configured(&overlay) && reticle_up;
    Some((
        view_weapon_draw_gun(iris, true, false),
        iris.then_some("iris"),
    ))
}

/// [`drawgun_admit`] as the 0 / 1 value the pose product carries.
pub(super) fn drawgun_flag(
    ps: &PlayerState,
    weapons: Option<&FpvWeaponTable>,
    heading_into_ads: bool,
) -> Option<i32> {
    drawgun_admit(ps, weapons, heading_into_ads).map(|(drawn, _)| i32::from(drawn))
}
