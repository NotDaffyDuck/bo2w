//! Focused render observation: `BO2MP_PERF_FOCUS=script_model:<id>` picks one
//! script model whose fate each frame is recorded in `DrawFocus`.

use bevy::prelude::*;
use render_frame::FocusedFrameSnapshot;

const FOCUS_ENV: &str = "BO2MP_PERF_FOCUS";

/// What the focused model's frame came to.
#[derive(Resource, Debug, Default)]
pub struct DrawFocus {
    pub frame: Option<FocusedFrameSnapshot>,
}

impl DrawFocus {
    /// Starts the frame for `owner_id`, not yet seen.
    pub fn begin_frm(&mut self, owner_id_in: u32) {
        let unseen = FocusedFrameSnapshot::never_found(owner_id_in, "owner_not_found");
        self.frame.replace(unseen);
    }

    /// Records why the model `owner_id` (drawn as `model`) is not planned.
    pub fn decline(&mut self, owner_id_v: u32, shown_model: &str, refusal_why: &'static str) {
        let mut refused = FocusedFrameSnapshot::never_found(owner_id_v, refusal_why);
        refused.model = Some(shown_model.to_owned());
        self.frame = Some(refused);
    }
}

/// The owner id being observed, if any.
#[derive(Resource, Debug, Default, PartialEq, Eq)]
pub(super) struct FocusSelect {
    script_model: Option<u32>,
}

impl FocusSelect {
    pub(super) fn from_env() -> Result<Self, String> {
        use std::env::VarError;
        let spec = match std::env::var(FOCUS_ENV) {
            Err(VarError::NotPresent) => return Ok(Self::default()),
            Err(VarError::NotUnicode(_)) => {
                return Err(format!("{FOCUS_ENV} is not valid UTF-8"));
            }
            Ok(spec) => spec,
        };
        if !perf::recording() {
            let why = "focused facts exist only in .pftrace";
            return Err(format!("{FOCUS_ENV} requires BO2MP_PERF=1; {why}"));
        }
        parse_focus_spec(&spec).map(|id| Self {
            script_model: Some(id),
        })
    }

    pub(super) fn script_model_id(&self) -> Option<u32> {
        self.script_model
    }
}

/// `script_model:<id>` to the id.
fn parse_focus_spec(raw: &str) -> Result<u32, String> {
    let invalid = |why: &str| format!("invalid {FOCUS_ENV}={raw:?}; {why}");
    let Some((kind, owner)) = raw.split_once(':') else {
        return Err(invalid("expected script_model:<u32>"));
    };
    let Ok(owner) = owner.parse::<u32>() else {
        return Err(invalid("owner id must be u32"));
    };
    if kind != "script_model" {
        return Err(format!(
            "unsupported {FOCUS_ENV} kind {kind:?}; only script_model works"
        ));
    }
    Ok(owner)
}

mod tests {
    #![cfg(test)]
    use super::*;

    #[test]
    fn a_script_model_spec_names_its_owner() {
        assert_eq!(parse_focus_spec("script_model:1073741824"), Ok(1 << 30));
    }

    #[test]
    fn a_spec_without_a_colon_is_refused() {
        assert!(
            parse_focus_spec("script_model")
                .unwrap_err()
                .contains("expected")
        );
    }

    #[test]
    fn an_owner_that_is_not_a_u32_is_refused() {
        assert!(
            parse_focus_spec("script_model:-3")
                .unwrap_err()
                .contains("u32")
        );
        assert!(
            parse_focus_spec("script_model:x")
                .unwrap_err()
                .contains("u32")
        );
    }

    #[test]
    fn only_script_models_can_be_focused() {
        let error = parse_focus_spec("vehicle:4").unwrap_err();
        assert!(error.contains("unsupported") && error.contains("vehicle"));
    }

    #[test]
    fn a_frame_begins_unresolved_and_a_refusal_names_the_model() {
        let mut observed = DrawFocus::default();
        assert!(observed.frame.is_none());
        observed.begin_frm(7);
        let begun = observed.frame.as_ref().expect("begun");
        assert_eq!(begun.outcome, "owner_not_found");
        assert_eq!(begun.owner_no, 7);
        assert_eq!(begun.model, None);
        observed.decline(7, "crate", "hidden");
        let refused = observed.frame.as_ref().expect("refused");
        assert_eq!(refused.model.as_deref(), Some("crate"));
        assert_eq!(refused.outcome, "hidden");
    }

    #[test]
    fn no_selection_by_default() {
        assert_eq!(FocusSelect::default().script_model_id(), None);
    }
}
