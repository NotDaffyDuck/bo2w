//! hud_t6's crosshair keeps its own copies of BO2's weapon states so it stays
//! libm-only. They must match weapon_t6::state (facts/weaponstate-t6.md:3); if
//! this fails, the fix goes in hud_t6.

use hud_t6::reticle_plan;
use weapon_t6::state;

#[test]
fn crosshair_weaponstate_twins() {
    assert_eq!(reticle_plan::WEAPONSTATE_RAISING, state::RAISING);
    assert_eq!(reticle_plan::WEAPONSTATE_DROPPING_ALTSWITCH, state::DROPPING_ALTSWITCH);
    assert_eq!(reticle_plan::WEAPONSTATE_RELOADING, state::RELOADING);
    assert_eq!(reticle_plan::WEAPONSTATE_RELOAD_START, state::RELOAD_START);
    assert_eq!(reticle_plan::WEAPONSTATE_MELEE_INIT, state::MELEE_INIT);
    assert_eq!(reticle_plan::WEAPONSTATE_MELEE_FIRE, state::MELEE_FIRE);
    assert_eq!(reticle_plan::WEAPONSTATE_MELEE_END, state::MELEE_END);
    assert_eq!(reticle_plan::WEAPONSTATE_LOWREADY_RAISE, state::LOWREADY_RAISE);
    assert_eq!(reticle_plan::WEAPONSTATE_LOWREADY_DROP, state::LOWREADY_DROP);
}
