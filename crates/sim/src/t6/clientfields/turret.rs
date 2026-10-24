//! bo2mp: the Guardian's client script (`_turret.csc`): its dish opens when
//! it is placed, folds when it is picked up or stunned and drops when it is
//! destroyed (client flags 2, 3 and 4 on a turret, each `setanim` at full
//! weight, no blend), and it hums while it fires (`turret_microwave_sounds`:
//! a start, the loop, a stop).
//!
//! Not ported: BO2 plays the hum on a line along its 750-unit beam, heard
//! from the nearest point of it, and from `tag_flash`; ours plays from the
//! turret. Its stop sound when the turret goes away mid-hum.

use bevy_ecs::prelude::World;

use super::Zm;
use crate::EventAudience;

const OPEN_FLAG: i32 = 2;
const CLOSE_FLAG: i32 = 3;
const DESTROYED_FLAG: i32 = 4;
const START: &str = "wpn_micro_turret_start";
const HUM: &str = "wpn_micro_turret_loop";
const STOP: &str = "wpn_micro_turret_stop";

/// Its flag callbacks: open and close only as they are set, destroyed on
/// either.
pub(super) fn flag(world: &mut World, n: u32, bit: i32, on: bool) {
    let clip = match bit {
        OPEN_FLAG if on => "o_hpm_open",
        CLOSE_FLAG if on => "o_hpm_close",
        DESTROYED_FLAG => "o_hpm_destroyed",
        _ => return,
    };
    super::super::mp_fxanims::play(world, n, clip, false);
}

/// `turret_microwave_sounds`: 1 starts the hum, 0 stops it.
pub(super) fn sounds(world: &mut World, n: u32, value: i64) {
    let mut zm = world.resource_mut::<Zm>();
    let Some(e) = zm.ents.get_mut(&n) else {
        return;
    };
    let on = value != 0;
    if on == e.loop_sound.as_deref().is_some_and(|a| a == HUM) {
        return;
    }
    e.loop_sound = on.then(|| HUM.to_owned());
    let at = e.origin;
    drop(zm);
    super::super::natives_fx::sound(world, EventAudience::All, if on { START } else { STOP }, at);
}
