//! bo2mp: the parsed Black Ops II player animation script as a resource.
//! The script itself is `anim_t6`.

pub use anim_t6::*;

/// The parsed script as a resource (each machine reads it from the zones
/// it loaded; `None` when they hold none).
#[derive(bevy_ecs::prelude::Resource, Clone, Default)]
pub struct T6PlayerAnimsRes(pub Option<std::sync::Arc<T6PlayerAnims>>);

impl T6PlayerAnimsRes {
    /// Parse `(mp/playeranim.script, mp/playeranimtypes.txt)`; a script that
    /// does not parse is logged and left out.
    pub fn from_text(text: Option<&(String, String)>, speeds: &[(String, f32)]) -> Self {
        let Some((script, types)) = text else {
            return Self(None);
        };
        match T6PlayerAnims::parse(script, types) {
            Ok(mut parsed) => {
                parsed.set_speeds(speeds);
                Self(Some(std::sync::Arc::new(parsed)))
            }
            Err(e) => {
                diag::warn!(Sim, "body anim script mp/playeranim.script could not be read: {e}");
                Self(None)
            }
        }
    }
}
