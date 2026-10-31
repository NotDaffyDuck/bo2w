//! BO2's `CG_ScriptMover` draw choice: hidden, a brush model, or a model
//! with its render flags and laser.

use crate::missile::{EF2_DEMO_HIDDEN, EF_NODRAW};

/// `solid` value that marks a brush-model entity.
pub const SOLID_BMODEL: i32 = 0xff_ffff;
/// `eFlags` bit that keeps the model loaded but skips drawing it.
pub const EF_SKIP_DRAW_MODEL: i32 = 0x200;
/// `clientLinkInfo.flags` bit: a player is linked to this mover.
pub const CLIENT_LINK_ACTIVE: u8 = 1;

/// `eFlags2` bits `CG_ScriptMover` turns into render flags.
pub const EF2_LASER: i32 = 0x2000;
pub const EF2_RENDER_0X4000000: i32 = 0x400_0000;
pub const EF2_INFRARED: i32 = 0x2000_0000;

/// Render flags `CG_ScriptMover` passes to `R_AddDObjToScene`.
pub const RF_RAGDOLL: u32 = 4;
pub const RF_LIT_RIOTSHIELD: u32 = 0x2_0000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptMoverDraw {
    /// Not drawn. `keep_model` says whether BO2 still sets up the model
    /// (a model mover a player is linked to, or weapon-linked to).
    Hidden { keep_model: bool },
    /// Drawn as the map's brush model at the entity's pose.
    BrushModel,
    /// Drawn as its model, if it has one and `eFlags & 0x200` is clear.
    Model,
}

/// The first half of `CG_ScriptMover`: which way the mover draws.
#[must_use]
pub fn script_mover_draw(
    e_flags: i32,
    e_flags2: i32,
    solid: i32,
    demo_playing: bool,
    player_linked: bool,
) -> ScriptMoverDraw {
    if e_flags & EF_NODRAW != 0 || (demo_playing && e_flags2 & EF2_DEMO_HIDDEN != 0) {
        return ScriptMoverDraw::Hidden { keep_model: solid != SOLID_BMODEL && player_linked };
    }
    if solid == SOLID_BMODEL {
        ScriptMoverDraw::BrushModel
    } else {
        ScriptMoverDraw::Model
    }
}

/// Whether a model mover with a model goes into the scene at all.
#[must_use]
pub fn script_mover_model_drawn(e_flags: i32) -> bool {
    e_flags & EF_SKIP_DRAW_MODEL == 0
}

/// The render flags `CG_ScriptMover` builds for a drawn model.
/// `infrared` is what `CG_EntityInfrared` gives, used only when the
/// entity asks for it.
#[must_use]
pub fn script_mover_render_flags(
    ragdoll: bool,
    e_flags2: i32,
    lit_riotshield: bool,
    infrared: u32,
) -> u32 {
    let mut flags = if ragdoll { RF_RAGDOLL } else { 0 };
    if e_flags2 & EF2_RENDER_0X4000000 != 0 {
        flags |= EF2_RENDER_0X4000000 as u32;
    }
    if lit_riotshield {
        flags |= RF_LIT_RIOTSHIELD;
    }
    if e_flags2 & EF2_INFRARED != 0 {
        flags |= infrared;
    }
    flags
}

/// Whether a drawn model mover adds its laser.
#[must_use]
pub fn script_mover_laser(e_flags2: i32, laser_force_on: bool) -> bool {
    laser_force_on || e_flags2 & EF2_LASER != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draw_choice() {
        assert_eq!(script_mover_draw(0, 0, SOLID_BMODEL, false, false), ScriptMoverDraw::BrushModel);
        assert_eq!(script_mover_draw(0, 0, 0x20_2020, false, false), ScriptMoverDraw::Model);
        assert_eq!(
            script_mover_draw(EF_NODRAW, 0, SOLID_BMODEL, false, true),
            ScriptMoverDraw::Hidden { keep_model: false }
        );
        assert_eq!(
            script_mover_draw(0, EF2_DEMO_HIDDEN, 0, true, true),
            ScriptMoverDraw::Hidden { keep_model: true }
        );
        assert_eq!(script_mover_draw(0, EF2_DEMO_HIDDEN, 0, false, false), ScriptMoverDraw::Model);
        assert!(!script_mover_model_drawn(EF_SKIP_DRAW_MODEL));
    }

    #[test]
    fn render_flags_and_laser() {
        assert_eq!(script_mover_render_flags(true, 0, false, 0x40), RF_RAGDOLL);
        assert_eq!(script_mover_render_flags(false, EF2_INFRARED, true, 0x40), 0x2_0040);
        assert_eq!(script_mover_render_flags(false, 0, false, 0x40), 0);
        assert!(script_mover_laser(EF2_LASER, false) && script_mover_laser(0, true));
        assert!(!script_mover_laser(0, false));
    }
}
