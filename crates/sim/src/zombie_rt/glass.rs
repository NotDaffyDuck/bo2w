//! bo2mp: a Black Ops II map's glass and its scripts. The scripts' blasts
//! reach the panes (BO2's `G_RadiusDamage` ends with the glass, and
//! `glassradiusdamage` hits only the glass), and each pane that shatters
//! reaches the scripts as BO2's `codecallback_glasssmash` (Frostbite, Rush
//! and Vertigo listen for its `glass_smash`).

use bevy_ecs::prelude::World;
use gsc_t6::ScriptValue;

use super::{frame, mp, tick, with_vm};

/// A blast's share for the panes in `radius` of `origin`: full `max` at the
/// centre down to `min` at the edge, of the kind `means` names.
pub(super) fn script_blast(game: &mut World, spot: [f32; 3], reach: f32, max: f32, min: f32, means: &str) {
    let now = tick(game);
    crate::harm::apply_shared_glass_blast(&mut frame(game), now, spot, max, min, reach, means);
}

/// Each frame: the panes that shattered since the last one tell the
/// scripts where, and which way the hit pushed them.
pub(super) fn flush(world: &mut World) {
    let smashes = frame(world).world_objects_mut().take_glass_smashes();
    if smashes.is_empty() {
        return;
    }
    // The client's half: Vertigo's blinds.
    for (at, _) in &smashes {
        super::mp_fxanims::glass_smash(world, *at);
    }
    with_vm(world, |vm, world| {
        let cb = mp::callbacks(world);
        let level = ScriptValue::Object(vm.level);
        for (at, dir) in smashes {
            let ran = vm
                .spawn_named(
                    world,
                    &cb,
                    "codecallback_glasssmash",
                    level.clone(),
                    vec![ScriptValue::Vec3(at), ScriptValue::Vec3(dir)],
                )
                .is_some();
            if std::env::var_os("BO2MP_T6_HITLOG").is_some() {
                diag::info!(Sim, "bo2mp t6 glass: {cb}::codecallback_glasssmash at {at:?} dir {dir:?}, ran {ran}");
            }
        }
    });
}
