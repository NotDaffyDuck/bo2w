//! The predicting client: its own copy of the world, the commands it has run ahead, and the
//! forward step that runs a new command.

use playerstate_t6::game::{AvatarState, InputCommand};
use sim::{ClientId, SimWorld, AdvanceCause, Tick, StepInputs};

use super::ground::pmove_runs_for;
use super::history::{InputCommandNumber, DEFAULT_MOVE_LOG_LIMIT, ForecastMoveLog, ForecastMoveEntry};
use super::metrics::ForecastCounters;
use crate::HostInstant;
use crate::client::correction_fade::ForecastDrift;
use crate::client::view_z_smooth::ViewZSmoothing;

/// A command is never stepped for longer than this many milliseconds at once.
const LONGEST_STEP_MS: i32 = 200;

#[derive(Debug)]
pub struct LocalForecast {
    pub(super) me: ClientId,
    pub(super) sandbox: SimWorld,
    pub(super) replay_moves: ForecastMoveLog,
    pub(super) forecast_on: bool,
    pub(super) seq_next: InputCommandNumber,
    pub(super) tick_next: u32,
    pub(super) quiet_snaps: u32,
    pub(super) view_error: ForecastDrift,
    /// The camera easing over stair steps.
    pub(super) stair_ease: ViewZSmoothing,
    pub(super) stats: ForecastCounters,
    pub(super) predicted_ps: Option<AvatarState>,
    pub(super) newest_cmd: Option<InputCommand>,
    /// The command of the newest acknowledgement that matched.
    pub(super) acked_cmd: Option<InputCommand>,
    /// The last command dropped from the history, with its sequence number. It lets late receipts_batch
    /// for dropped commands retire instead of reading as broken.
    pub(super) floor: Option<(InputCommandNumber, InputCommand)>,
    pub(super) last_step_ms: Option<i32>,
    pub(super) saw_local: bool,
    /// Scratch input reused by every step to avoid allocating.
    pub(super) scratch_input: StepInputs,
}

impl LocalForecast {
    pub fn new(who: ClientId) -> Self {
        Self::with_log_limit(who, DEFAULT_MOVE_LOG_LIMIT)
    }

    pub fn with_log_limit(who: ClientId, cap: usize) -> Self {
        let mut sandbox = SimWorld::new();
        sandbox.hold_wire_meta();
        Self {
            me: who,
            sandbox,
            replay_moves: ForecastMoveLog::holding_at_most(cap),
            forecast_on: false,
            seq_next: InputCommandNumber(1),
            tick_next: 0,
            quiet_snaps: 0,
            view_error: ForecastDrift::default(),
            stair_ease: ViewZSmoothing::default(),
            stats: ForecastCounters::default(),
            predicted_ps: None,
            newest_cmd: None,
            acked_cmd: None,
            floor: None,
            last_step_ms: None,
            saw_local: false,
            scratch_input: StepInputs::default(),
        }
    }

    pub fn local(&self) -> ClientId {
        self.me
    }

    pub fn assign_scoped(&mut self, who: ClientId) {
        self.me = who;
    }

    pub fn is_forecast_live(&self) -> bool {
        self.forecast_on
    }

    /// Forgets everything but who the local player is.
    pub fn stand_down(&mut self) {
        *self = Self::with_log_limit(self.me, DEFAULT_MOVE_LOG_LIMIT);
    }

    /// Copies the match content from `authority` and starts predicting.
    pub fn prime_from_assets(
        &mut self,
        authority: &SimWorld,
    ) {
        self.sandbox.initialize_prediction_from(authority);
        self.forecast_on = true;
    }

    pub fn world(&self) -> &SimWorld {
        &self.sandbox
    }

    /// Where this client draws the living other players (id, origin, velocity); the knife lunge
    /// predicts at them.
    pub fn set_lunge_seen(
        &mut self,
        rows: impl IntoIterator<Item = (ClientId, [f32; 3], [f32; 3])>,
    ) {
        self.sandbox.set_lunge_seen(rows);
    }

    pub fn history(&self) -> &ForecastMoveLog {
        &self.replay_moves
    }

    pub fn metrics(&self) -> ForecastCounters {
        self.stats
    }

    pub fn predicted_error(&self) -> &ForecastDrift {
        &self.view_error
    }

    pub fn forecast_miss_mut(&mut self) -> &mut ForecastDrift {
        &mut self.view_error
    }

    /// The camera easing over stair steps.
    pub fn view_z(&self) -> &ViewZSmoothing {
        &self.stair_ease
    }

    pub fn predicted_local(&self) -> Option<&AvatarState> {
        self.predicted_ps.as_ref()
    }

    pub fn last_cmd(&self) -> Option<&InputCommand> {
        self.newest_cmd.as_ref()
    }

    pub fn latest_forecast_millis(&self) -> Option<i32> {
        self.last_step_ms
    }

    pub fn self_present_in_prior_state(&self) -> bool {
        self.saw_local
    }

    /// The local player as the prediction world holds it now.
    pub(super) fn local_state(&self) -> Option<AvatarState> {
        self.sandbox.player(self.me).copied()
    }

    /// Runs `cmd` as the next command and returns the sequence number to send it under.
    ///
    /// Returns nothing while disarmed or when the history is full. While the player watches
    /// the kill camera the command is numbered and sent but not run.
    pub fn run_ahead(
        &mut self,
        mut cmd: InputCommand,
        authority_clock_ms: HostInstant,
    ) -> Option<(InputCommandNumber, InputCommand)> {
        if !self.forecast_on || self.replay_moves.len() >= self.replay_moves.capacity_limit() {
            return None;
        }
        let stamp = authority_clock_ms.as_millis();
        cmd.authority_clock_ms = stamp;
        let before = self.local_state().unwrap_or(AvatarState::ZERO);
        let step_ms = (stamp.wrapping_sub(before.last_cmd_ms)).clamp(0, LONGEST_STEP_MS);
        self.last_step_ms = Some(step_ms);

        let (move_seq, tick) = self.take_next_ids();
        if self.predicted_ps.is_some_and(|ps| !pmove_runs_for(&ps)) {
            // Watching the kill camera: numbered and sent, never run.
            self.newest_cmd = Some(cmd);
            self.floor = Some((move_seq, cmd));
            return Some((move_seq, cmd));
        }

        let after = self.run_fresh(tick, cmd, before, step_ms);
        self.stair_ease.settle(stamp, after.movement_mode);
        self.file_record(ForecastMoveEntry {
            seq: move_seq,
            at_tick: tick,
            cmd,
            input: before,
            output: after,
        });
        Some((move_seq, cmd))
    }

    /// Hands out the next command number and the next tick, advancing both counters.
    fn take_next_ids(&mut self) -> (InputCommandNumber, Tick) {
        let seq = self.seq_next;
        self.seq_next = seq.next();
        let tick = Tick(self.tick_next);
        self.tick_next = self.tick_next.wrapping_add(1);
        (seq, tick)
    }

    /// Steps a brand new command (when it spans any time) and returns the player afterwards.
    fn run_fresh(&mut self, tick: Tick, cmd: InputCommand, before: AvatarState, step_ms: i32) -> AvatarState {
        if step_ms == 0 {
            return before;
        }
        let stepped = self
            .advance_forecast(tick, cmd, step_ms, AdvanceCause::GuessAhead)
            .unwrap_or(before);
        // Only a new move's step is eased; a replayed one was eased when it first ran.
        let rise = self.sandbox.pmove_view_change(self.me);
        self.stair_ease.record_rise(rise, cmd.authority_clock_ms);
        stepped
    }

    /// Stores a freshly run command and updates the counters that describe the history.
    fn file_record(&mut self, record: ForecastMoveEntry) {
        let (cmd, output) = (record.cmd, record.output);
        let evicted = self.replay_moves.add_entry(record);
        self.stats.displaced_inputs += u64::from(evicted);
        self.stats.forecast_inputs += 1;
        let depth = self.replay_moves.len();
        self.stats.oldest_retained_frame = self.stats.oldest_retained_frame.max(depth);
        self.predicted_ps = Some(output);
        self.newest_cmd = Some(cmd);
    }

    /// Runs one command through the movement step on the prediction world and returns the local
    /// player afterwards.
    pub(super) fn advance_forecast(
        &mut self,
        tick: Tick,
        cmd: InputCommand,
        msec: i32,
        reason: AdvanceCause,
    ) -> Option<AvatarState> {
        let me = self.me;
        let mut scratch = std::mem::take(&mut self.scratch_input);
        scratch.cmds.clear();
        scratch.actions.clear();
        scratch.cmds.push((me, cmd));
        let _ = sim::try_step(&mut self.sandbox, tick, &scratch, msec, reason);
        self.scratch_input = scratch;
        self.sandbox.player(me).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn armed() -> LocalForecast {
        let mut prediction = LocalForecast::new(ClientId(1));
        prediction.prime_from_assets(&SimWorld::new());
        prediction
    }

    #[test]
    fn a_new_prediction_is_disarmed_and_empty() {
        let prediction = LocalForecast::new(ClientId(2));
        assert_eq!(prediction.local(), ClientId(2));
        assert!(!prediction.is_forecast_live());
        assert!(prediction.history().is_empty());
        assert_eq!(prediction.metrics(), ForecastCounters::default());
        assert!(prediction.predicted_local().is_none());
        assert!(prediction.last_cmd().is_none());
        assert_eq!(prediction.latest_forecast_millis(), None);
        assert!(!prediction.self_present_in_prior_state());
    }

    #[test]
    fn the_history_limit_is_the_one_asked_for() {
        let prediction = LocalForecast::with_log_limit(ClientId(1), 8);
        assert_eq!(prediction.history().capacity_limit(), 8);
        assert_eq!(LocalForecast::new(ClientId(1)).history().capacity_limit(), DEFAULT_MOVE_LOG_LIMIT);
    }

    #[test]
    fn a_disarmed_prediction_runs_nothing() {
        let mut prediction = LocalForecast::new(ClientId(1));
        assert!(
            prediction
                .run_ahead(InputCommand::default(), HostInstant::from_millis_value(100))
                .is_none()
        );
        assert_eq!(prediction.metrics().forecast_inputs, 0);
    }

    #[test]
    fn the_local_client_can_change() {
        let mut prediction = LocalForecast::new(ClientId(1));
        prediction.assign_scoped(ClientId(5));
        assert_eq!(prediction.local(), ClientId(5));
    }

    #[test]
    fn arming_then_disarming_goes_back_to_the_start_for_the_same_client() {
        let mut prediction = armed();
        assert!(prediction.is_forecast_live());
        prediction.stand_down();
        assert!(!prediction.is_forecast_live());
        assert_eq!(prediction.local(), ClientId(1));
    }

    #[test]
    fn commands_are_numbered_from_one_and_stamped_with_the_server_time() {
        let mut prediction = armed();
        let (first, cmd) = prediction
            .run_ahead(InputCommand::default(), HostInstant::from_millis_value(50))
            .expect("armed prediction runs");
        assert_eq!(first, InputCommandNumber(1));
        assert_eq!(cmd.authority_clock_ms, 50);
        let (second, _) = prediction
            .run_ahead(InputCommand::default(), HostInstant::from_millis_value(66))
            .expect("armed prediction runs");
        assert_eq!(second, InputCommandNumber(2));
        assert_eq!(prediction.history().len(), 2);
        assert_eq!(prediction.metrics().forecast_inputs, 2);
        assert_eq!(prediction.metrics().oldest_retained_frame, 2);
        assert_eq!(prediction.last_cmd().map(|c| c.authority_clock_ms), Some(66));
    }

    #[test]
    fn a_step_is_never_longer_than_200_ms() {
        let mut prediction = armed();
        prediction.run_ahead(InputCommand::default(), HostInstant::from_millis_value(5000));
        assert_eq!(prediction.latest_forecast_millis(), Some(LONGEST_STEP_MS));
    }

    #[test]
    fn time_running_backwards_steps_nothing() {
        let mut prediction = armed();
        prediction.run_ahead(InputCommand::default(), HostInstant::from_millis_value(0));
        assert_eq!(prediction.latest_forecast_millis(), Some(0));
    }

    #[test]
    fn a_full_history_stops_new_commands() {
        let mut prediction = LocalForecast::with_log_limit(ClientId(1), 2);
        prediction.prime_from_assets(&SimWorld::new());
        assert!(prediction.run_ahead(InputCommand::default(), HostInstant::from_millis_value(10)).is_some());
        assert!(prediction.run_ahead(InputCommand::default(), HostInstant::from_millis_value(20)).is_some());
        assert!(prediction.run_ahead(InputCommand::default(), HostInstant::from_millis_value(30)).is_none());
        assert_eq!(prediction.history().len(), 2);
    }
}
