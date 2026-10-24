//! What happened to each request to start a sound, and a short rolling log
//! of those answers for tests and diagnostics.

use std::collections::VecDeque;
use std::fmt;
use std::time::Duration;

use asset_core::AssetNamespace;
use bevy::prelude::*;

use crate::backend::AudioScope;

/// How many recent start decisions are kept.
const START_DECISION_CAP: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartOutcome {
    Submitted,
    Pending,
    Suppressed(SuppressReason),
    Failed(StartFailure),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuppressReason {
    Inaudible,
    VoiceLimit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoundClass {
    Weapon,
    World,
    Ui,
}

impl SoundClass {
    /// How long a one-shot of this class may wait for its clip to be decoded.
    pub fn oneshot_wait(self) -> Duration {
        Duration::from_millis(match self {
            Self::Weapon => 200,
            Self::World => 350,
            Self::Ui => 500,
        })
    }

    pub fn scope(self) -> AudioScope {
        match self {
            Self::Ui => AudioScope::Menu,
            Self::Weapon | Self::World => AudioScope::Match,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartFailure {
    BankMissing,
    MissingAlias,
    NoPcm,
    NoListener,
    NoFalloffCurve,
    FalloffEval,
    DecodeFailed,
    Expired,
}

impl StartOutcome {
    pub fn is_terminal_success(&self) -> bool {
        *self == Self::Submitted
    }

    /// Whether the caller should try the next-best binding for the sound.
    pub fn allows_binding_fallback(&self) -> bool {
        matches!(
            self,
            Self::Failed(StartFailure::MissingAlias | StartFailure::NoPcm)
        )
    }

    /// Whether the request is still alive or was deliberately dropped,
    /// rather than having failed.
    pub fn is_open(&self) -> bool {
        matches!(self, Self::Submitted | Self::Pending | Self::Suppressed(_))
    }

    fn label(&self) -> &'static str {
        match self {
            Self::Submitted => "Submitted",
            Self::Pending => "Pending",
            Self::Suppressed(SuppressReason::Inaudible) => "SuppressedInaudible",
            Self::Suppressed(SuppressReason::VoiceLimit) => "SuppressedVoiceLimit",
            Self::Failed(StartFailure::BankMissing) => "FailedBankMissing",
            Self::Failed(StartFailure::MissingAlias) => "FailedMissingAlias",
            Self::Failed(StartFailure::NoPcm) => "FailedNoPcm",
            Self::Failed(StartFailure::NoListener) => "FailedNoListener",
            Self::Failed(StartFailure::NoFalloffCurve) => "FailedNoFalloffCurve",
            Self::Failed(StartFailure::FalloffEval) => "FailedFalloffEval",
            Self::Failed(StartFailure::DecodeFailed) => "FailedDecode",
            Self::Failed(StartFailure::Expired) => "ExpiredAwaitingDecode",
        }
    }
}

impl fmt::Display for StartOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

#[derive(Clone, Debug)]
pub struct StartDecision {
    pub namespace: AssetNamespace,
    pub alias: String,
    pub variant: Option<usize>,
    pub outcome: StartOutcome,
    /// A second alias tried alongside, and how that went.
    pub secondary: Option<(String, StartOutcome)>,
    pub detail: Option<String>,
}

impl StartDecision {
    /// One log line describing the decision.
    pub fn line(&self) -> String {
        let variant = match self.variant {
            Some(index) => index.to_string(),
            None => "-".to_owned(),
        };
        let mut line = format!(
            "audio: start alias=`{}:{}` variant={variant} result={}",
            self.namespace.as_str(),
            self.alias,
            self.outcome
        );
        if let Some((alias, outcome)) = &self.secondary {
            line += &format!(" secondary=`{alias}` result={outcome}");
        }
        if let Some(detail) = &self.detail {
            line += " ";
            line += detail;
        }
        line
    }
}

#[derive(Resource, Default, Debug)]
pub struct StartDecisions {
    recent: VecDeque<StartDecision>,
}

impl StartDecisions {
    pub fn record(&mut self, decision: StartDecision) {
        // BO2MP_SOUND_LOG=1 echoes every start decision to the log (for tests).
        static ECHO: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if *ECHO.get_or_init(|| std::env::var_os("BO2MP_SOUND_LOG").is_some()) {
            diag::info!(Audio, "{}", decision.line());
        }
        if self.recent.len() >= START_DECISION_CAP {
            self.recent.pop_front();
        }
        self.recent.push_back(decision);
    }

    pub fn len(&self) -> usize {
        self.recent.len()
    }

    pub fn is_empty(&self) -> bool {
        self.recent.is_empty()
    }

    pub fn lines(&self) -> impl Iterator<Item = String> + '_ {
        self.recent.iter().map(StartDecision::line)
    }
}
