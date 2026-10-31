//! What happened to each request to start a sound, and a short rolling log
//! of those answers for tests and diagnostics.

use std::collections::VecDeque;
use std::fmt;
use std::time::Duration;

use asset_core::AssetCategoryScope;
use bevy::prelude::*;

use crate::backend::SoundAudience;

/// How many recent start decisions are kept.
const DECISION_LOG_LEN: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlaybackVerdict {
    Handed,
    Pending,
    Suppressed(MuteCause),
    Failed(PlaybackBlocker),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MuteCause {
    Inaudible,
    AtVoiceCap,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoiseCategory {
    Weapon,
    World,
    Ui,
}

impl NoiseCategory {
    /// How long a one-shot of this class may wait for its clip to be decoded.
    pub fn oneshot_grace(self) -> Duration {
        Duration::from_millis(match self {
            Self::Weapon => 200,
            Self::World => 350,
            Self::Ui => 500,
        })
    }

    pub fn scope(self) -> SoundAudience {
        match self {
            Self::Ui => SoundAudience::Menu,
            Self::Weapon | Self::World => SoundAudience::Match,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlaybackBlocker {
    NoCatalog,
    UnknownCue,
    NoPcm,
    NoEars,
    AttenuationCurveAbsent,
    AttenuationMath,
    UnreadableBytes,
    Expired,
}

impl PlaybackVerdict {
    pub const fn is_terminal_success(&self) -> bool {
        matches!(*self, Self::Handed)
    }

    /// Whether the caller should try the next-best binding for the sound.
    pub const fn allows_binding_fallback(&self) -> bool {
        match self {
            Self::Failed(why) => matches!(why, PlaybackBlocker::UnknownCue | PlaybackBlocker::NoPcm),
            _ => false,
        }
    }

    /// Whether the request is still alive or was deliberately dropped,
    /// rather than having failed.
    pub const fn is_open(&self) -> bool {
        !matches!(self, Self::Failed(_))
    }

    fn caption(&self) -> &'static str {
        match self {
            Self::Handed => "Submitted",
            Self::Pending => "Pending",
            Self::Suppressed(MuteCause::Inaudible) => "SuppressedInaudible",
            Self::Suppressed(MuteCause::AtVoiceCap) => "SuppressedVoiceLimit",
            Self::Failed(PlaybackBlocker::NoCatalog) => "FailedBankMissing",
            Self::Failed(PlaybackBlocker::UnknownCue) => "FailedMissingAlias",
            Self::Failed(PlaybackBlocker::NoPcm) => "FailedNoPcm",
            Self::Failed(PlaybackBlocker::NoEars) => "FailedNoListener",
            Self::Failed(PlaybackBlocker::AttenuationCurveAbsent) => "FailedNoFalloffCurve",
            Self::Failed(PlaybackBlocker::AttenuationMath) => "FailedFalloffEval",
            Self::Failed(PlaybackBlocker::UnreadableBytes) => "FailedDecode",
            Self::Failed(PlaybackBlocker::Expired) => "ExpiredAwaitingDecode",
        }
    }
}

impl std::fmt::Display for PlaybackVerdict {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str(self.caption())
    }
}

#[derive(Clone, Debug)]
pub struct PlaybackVerdictEntry {
    pub namespace: AssetCategoryScope,
    pub alias: String,
    pub variant_ix: Option<usize>,
    pub outcome: PlaybackVerdict,
    /// A second alias tried alongside, and how that went.
    pub alt_slot: Option<(String, PlaybackVerdict)>,
    pub detail: Option<String>,
}

impl PlaybackVerdictEntry {
    /// One log line describing the decision.
    pub fn line(&self) -> String {
        let variant = self.variant_ix.map_or_else(|| "-".to_owned(), |v| v.to_string());
        let Self { namespace, alias, outcome, .. } = self;
        let ns = namespace.as_str();
        let mut parts = vec![format!(
            "audio: start alias=`{ns}:{alias}` variant={variant} result={outcome}"
        )];
        if let Some((second, result)) = &self.alt_slot {
            parts.push(format!("secondary=`{second}` result={result}"));
        }
        parts.extend(self.detail.clone());
        parts.join(" ")
    }
}

#[derive(Resource, Default, Debug)]
pub struct PlaybackVerdictLog(VecDeque<PlaybackVerdictEntry>);

impl PlaybackVerdictLog {
    pub fn miss_row(&mut self, entry: PlaybackVerdictEntry) {
        // BO2MP_SOUND_LOG=1 echoes every start decision to the log (for tests).
        static ECHO: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if *ECHO.get_or_init(|| std::env::var_os("BO2MP_SOUND_LOG").is_some()) {
            diag::info!(Audio, "{}", entry.line());
        }
        if self.0.len() >= DECISION_LOG_LEN {
            self.0.pop_front();
        }
        self.0.push_back(entry);
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn lines(&self) -> impl Iterator<Item = String> {
        self.0.iter().map(PlaybackVerdictEntry::line)
    }
}
