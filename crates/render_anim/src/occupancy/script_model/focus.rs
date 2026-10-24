//! Focused render observation: `BO2MP_PERF_FOCUS=script_model:<id>` picks one
//! script model whose fate each frame is recorded in `RenderFocus`.

use bevy::prelude::*;
use render_frame::RenderFocusFrame;

const FOCUS_ENV: &str = "BO2MP_PERF_FOCUS";

/// What the focused model's frame came to.
#[derive(Resource, Debug, Default)]
pub struct RenderFocus {
    pub frame: Option<RenderFocusFrame>,
}

impl RenderFocus {
    /// Starts the frame for `owner_id`, not yet seen.
    pub fn begin_frame(&mut self, owner_id: u32) {
        self.frame = Some(RenderFocusFrame::unresolved(owner_id, "owner_not_found"));
    }

    /// Records why the model `owner_id` (drawn as `model`) is not planned.
    pub fn refuse(&mut self, owner_id: u32, model: &str, outcome: &'static str) {
        self.frame = Some(RenderFocusFrame {
            model: Some(model.to_owned()),
            ..RenderFocusFrame::unresolved(owner_id, outcome)
        });
    }
}

/// The owner id being observed, if any.
#[derive(Resource, Debug, Default, PartialEq, Eq)]
pub(super) struct FocusSelect {
    script_model: Option<u32>,
}

impl FocusSelect {
    pub(super) fn from_env() -> Result<Self, String> {
        let raw = match std::env::var(FOCUS_ENV) {
            Ok(raw) => raw,
            Err(std::env::VarError::NotPresent) => return Ok(Self::default()),
            Err(std::env::VarError::NotUnicode(_)) => {
                return Err(format!("{FOCUS_ENV} is not valid UTF-8"));
            }
        };
        if !perf::recording() {
            return Err(format!(
                "{FOCUS_ENV} requires BO2MP_PERF=1; focused facts exist only in .pftrace"
            ));
        }
        Ok(Self {
            script_model: Some(parse_focus_spec(&raw)?),
        })
    }

    pub(super) fn script_model_id(&self) -> Option<u32> {
        self.script_model
    }
}

/// `script_model:<id>` to the id.
fn parse_focus_spec(raw: &str) -> Result<u32, String> {
    let (kind, id) = raw
        .split_once(':')
        .ok_or_else(|| format!("invalid {FOCUS_ENV}={raw:?}; expected script_model:<u32>"))?;
    let id = id
        .parse::<u32>()
        .map_err(|_| format!("invalid {FOCUS_ENV}={raw:?}; owner id must be u32"))?;
    match kind {
        "script_model" => Ok(id),
        _ => Err(format!(
            "unsupported {FOCUS_ENV} kind {kind:?}; supported: script_model"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_script_model_spec_names_its_owner() {
        assert_eq!(parse_focus_spec("script_model:1073741824"), Ok(0x4000_0000));
    }

    #[test]
    fn a_spec_without_a_colon_is_refused() {
        assert!(parse_focus_spec("script_model").unwrap_err().contains("expected"));
    }

    #[test]
    fn an_owner_that_is_not_a_u32_is_refused() {
        assert!(parse_focus_spec("script_model:-3").unwrap_err().contains("u32"));
        assert!(parse_focus_spec("script_model:x").unwrap_err().contains("u32"));
    }

    #[test]
    fn only_script_models_can_be_focused() {
        let error = parse_focus_spec("vehicle:4").unwrap_err();
        assert!(error.contains("unsupported") && error.contains("vehicle"));
    }

    #[test]
    fn a_frame_begins_unresolved_and_a_refusal_names_the_model() {
        let mut focus = RenderFocus::default();
        focus.begin_frame(7);
        let frame = focus.frame.as_ref().expect("frame");
        assert_eq!((frame.owner_id, frame.outcome), (7, "owner_not_found"));
        assert!(frame.model.is_none());
        focus.refuse(7, "crate", "hidden");
        let frame = focus.frame.as_ref().expect("frame");
        assert_eq!((frame.outcome, frame.model.as_deref()), ("hidden", Some("crate")));
    }

    #[test]
    fn no_selection_by_default() {
        assert_eq!(FocusSelect::default().script_model_id(), None);
    }
}
