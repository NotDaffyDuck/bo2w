/// gm-14: with nobody connected after this long, BO2 ends the level.
pub const PLAYER_WAIT_TIMEOUT_MS: u32 = 120_000;

/// gm-15: BO2 reads "prematchperiod" from the gametype settings. TODO(fact): its value.
pub const PREMATCH_PERIOD_MS: u32 = 5_000;

/// gm-12.
pub const USE_START_SPAWNS_AT_START: bool = true;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrematchStep {
    Waiting { elapsed_ms: u32 },

    Starting { elapsed_ms: u32 },

    Done,
}

impl Default for PrematchStep {
    fn default() -> Self {
        Self::Waiting { elapsed_ms: 0 }
    }
}

impl PrematchStep {
    /// gm-14, gm-15: wait for one connected player, then run the prematch period.
    #[must_use]
    pub fn advance(self, dt_ms: u32, connected_players: u32, prematch_period_ms: u32) -> Self {
        match self {
            Self::Done => Self::Done,
            Self::Waiting { elapsed_ms } => {
                if connected_players >= 1 {
                    Self::Starting { elapsed_ms: 0 }
                } else {
                    Self::Waiting {
                        elapsed_ms: elapsed_ms.saturating_add(dt_ms),
                    }
                }
            }
            Self::Starting { elapsed_ms } => {
                let elapsed = elapsed_ms.saturating_add(dt_ms);
                if elapsed >= prematch_period_ms {
                    Self::Done
                } else {
                    Self::Starting {
                        elapsed_ms: elapsed,
                    }
                }
            }
        }
    }

    /// gm-14: still waiting for players past the timeout (BO2 exits the level here).
    #[must_use]
    pub const fn player_wait_timed_out(self) -> bool {
        matches!(self, Self::Waiting { elapsed_ms } if elapsed_ms > PLAYER_WAIT_TIMEOUT_MS)
    }

    #[must_use]
    pub const fn dump_token(self) -> &'static str {
        match self {
            Self::Waiting { .. } => "waiting",
            Self::Starting { .. } => "starting",
            Self::Done => "done",
        }
    }
}
