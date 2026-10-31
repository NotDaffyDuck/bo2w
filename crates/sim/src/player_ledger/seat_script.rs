//! Where a spectating or killcam view sits.

/// What a script-driven view is pointed at, and how much of the recorded past it replays.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScriptSeat {
    pub spectator_client: i32,
    pub death_replay_target: i32,
    pub look_at_entity: i32,
    /// Whom an entity killcam follows while the attacker has no saved state (BO2 picks the victim).
    pub target_client: i32,
    pub archive_ms: i32,
    pub ps_offset_ms: i32,
    pub length_ms: i32,
    /// This replay is the one that ends the match.
    pub last_kill: bool,
}

impl Default for ScriptSeat {
    /// No client, entity or target picked, nothing to replay.
    fn default() -> Self {
        Self {
            spectator_client: -1,
            death_replay_target: -1,
            look_at_entity: -1,
            target_client: -1,
            archive_ms: 0,
            ps_offset_ms: 0,
            length_ms: 0,
            last_kill: false,
        }
    }
}

/// What the killcam overlay needs to know.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeathReelBoard {
    pub final_kill: bool,
}
/// Older spelling KillcamHud still calls; the rename sweep moves it over.
pub use self::DeathReelBoard as KillcamHud;
