//! The predicting client: its own copy of the world, the commands it has run ahead, and the
//! forward step that runs a new command.

use playerstate_t6::game::{PlayerState, UserCmd};
use sim::{ClientId, SimWorld, StepReason, Tick, TickInput};

use super::ground::pmove_runs_for;
use super::history::{CmdSeq, DEFAULT_HISTORY_CAP, MoveHistory, MoveRecord};
use super::metrics::PredictionMetrics;
use crate::ServerTime;
use crate::client::predicted_error::PredictedError;
use crate::client::view_z_smooth::ViewZSmoothing;

/// A command is never stepped for longer than this many milliseconds at once.
const LONGEST_STEP_MS: i32 = 200;

#[derive(Debug)]
pub struct ClientPrediction {
    pub(super) local: ClientId,
    pub(super) world: SimWorld,
    pub(super) history: MoveHistory,
    pub(super) armed: bool,
    pub(super) next_seq: CmdSeq,
    pub(super) next_tick: u32,
    pub(super) snapshots_since_ack: u32,
    pub(super) predicted_error: PredictedError,
    /// The camera easing over stair steps.
    pub(super) view_z: ViewZSmoothing,
    pub(super) metrics: PredictionMetrics,
    pub(super) predicted_local: Option<PlayerState>,
    pub(super) last_cmd: Option<UserCmd>,
    /// The command of the newest acknowledgement that matched.
    pub(super) acknowledged_cmd: Option<UserCmd>,
    /// The last command dropped from the history, with its sequence number. It lets late acks
    /// for dropped commands retire instead of reading as broken.
    pub(super) replay_floor: Option<(CmdSeq, UserCmd)>,
    pub(super) last_predict_msec: Option<i32>,
    pub(super) had_local_last_snap: bool,
    /// Scratch input reused by every step to avoid allocating.
    pub(super) tick_input: TickInput,
}

impl ClientPrediction {
    pub fn new(local: ClientId) -> Self {
        Self::with_history_cap(local, DEFAULT_HISTORY_CAP)
    }

    pub fn with_history_cap(local: ClientId, cap: usize) -> Self {
        let mut world = SimWorld::new();
        world.suppress_snapshot_publish();
        Self {
            local,
            world,
            history: MoveHistory::with_cap(cap),
            armed: false,
            next_seq: CmdSeq(1),
            next_tick: 0,
            snapshots_since_ack: 0,
            predicted_error: PredictedError::default(),
            view_z: ViewZSmoothing::default(),
            metrics: PredictionMetrics::default(),
            predicted_local: None,
            last_cmd: None,
            acknowledged_cmd: None,
            replay_floor: None,
            last_predict_msec: None,
            had_local_last_snap: false,
            tick_input: TickInput::default(),
        }
    }

    pub fn local(&self) -> ClientId {
        self.local
    }

    pub fn set_local(&mut self, local: ClientId) {
        self.local = local;
    }

    pub fn is_armed(&self) -> bool {
        self.armed
    }

    /// Forgets everything but who the local player is.
    pub fn disarm(&mut self) {
        *self = Self::new(self.local);
    }

    /// Copies the match content from `authority` and starts predicting.
    pub fn arm_from_content(&mut self, authority: &SimWorld) {
        self.world.initialize_prediction_from(authority);
        self.armed = true;
    }

    pub fn world(&self) -> &SimWorld {
        &self.world
    }

    /// Where this client draws the living other players (id, origin, velocity); the knife lunge
    /// predicts at them.
    pub fn set_lunge_seen(
        &mut self,
        rows: impl IntoIterator<Item = (ClientId, [f32; 3], [f32; 3])>,
    ) {
        self.world.set_lunge_seen(rows);
    }

    pub fn history(&self) -> &MoveHistory {
        &self.history
    }

    pub fn metrics(&self) -> PredictionMetrics {
        self.metrics
    }

    pub fn predicted_error(&self) -> &PredictedError {
        &self.predicted_error
    }

    pub fn predicted_error_mut(&mut self) -> &mut PredictedError {
        &mut self.predicted_error
    }

    /// The camera easing over stair steps.
    pub fn view_z(&self) -> &ViewZSmoothing {
        &self.view_z
    }

    pub fn predicted_local(&self) -> Option<&PlayerState> {
        self.predicted_local.as_ref()
    }

    pub fn last_cmd(&self) -> Option<&UserCmd> {
        self.last_cmd.as_ref()
    }

    pub fn last_predict_msec(&self) -> Option<i32> {
        self.last_predict_msec
    }

    pub fn had_local_last_snap(&self) -> bool {
        self.had_local_last_snap
    }

    /// The local player as the prediction world holds it now.
    pub(super) fn local_state(&self) -> Option<PlayerState> {
        self.world.player(self.local).copied()
    }

    /// Runs `cmd` as the next command and returns the sequence number to send it under.
    ///
    /// Returns nothing while disarmed or when the history is full. While the player watches
    /// the kill camera the command is numbered and sent but not run.
    pub fn predict(
        &mut self,
        mut cmd: UserCmd,
        server_time: ServerTime,
    ) -> Option<(CmdSeq, UserCmd)> {
        if !self.armed || self.history.len() >= self.history.cap() {
            return None;
        }
        let now = server_time.ms();
        cmd.server_time = now;
        let input = self.local_state().unwrap_or(PlayerState::ZERO);
        let elapsed = now.wrapping_sub(input.command_time);
        let msec = elapsed.clamp(0, LONGEST_STEP_MS);
        self.last_predict_msec = Some(msec);

        let seq = self.next_seq;
        self.next_seq = seq.next();
        let tick = Tick(self.next_tick);
        self.next_tick = self.next_tick.wrapping_add(1);

        if self.predicted_local.is_some_and(|ps| !pmove_runs_for(&ps)) {
            self.last_cmd = Some(cmd);
            self.replay_floor = Some((seq, cmd));
            return Some((seq, cmd));
        }

        let output = if msec > 0 {
            let output = self
                .step_predicted(tick, cmd, msec, StepReason::PredictNew)
                .unwrap_or(input);
            // Only a new move's step is eased; a replayed one was eased when it first ran.
            let rise = self.world.pmove_view_change(self.local);
            self.view_z.note_step(rise, cmd.server_time);
            output
        } else {
            input
        };
        self.view_z.settle(now, output.pm_type);

        let evicted = self.history.push(MoveRecord {
            seq,
            tick,
            cmd,
            input,
            output,
        });
        if evicted {
            self.metrics.evicted_moves += 1;
        }
        self.metrics.predicted_moves += 1;
        self.metrics.deepest_history = self.metrics.deepest_history.max(self.history.len());
        self.predicted_local = Some(output);
        self.last_cmd = Some(cmd);

        Some((seq, cmd))
    }

    /// Runs one command through the movement step on the prediction world and returns the local
    /// player afterwards.
    pub(super) fn step_predicted(
        &mut self,
        tick: Tick,
        cmd: UserCmd,
        msec: i32,
        reason: StepReason,
    ) -> Option<PlayerState> {
        let local = self.local;
        let mut input = std::mem::take(&mut self.tick_input);
        input.cmds.clear();
        input.actions.clear();
        input.cmds.push((local, cmd));
        let _ = sim::try_step(&mut self.world, tick, &input, msec, reason);
        self.tick_input = input;
        self.world.player(local).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn armed() -> ClientPrediction {
        let mut prediction = ClientPrediction::new(ClientId(1));
        prediction.arm_from_content(&SimWorld::new());
        prediction
    }

    #[test]
    fn a_new_prediction_is_disarmed_and_empty() {
        let prediction = ClientPrediction::new(ClientId(2));
        assert_eq!(prediction.local(), ClientId(2));
        assert!(!prediction.is_armed());
        assert!(prediction.history().is_empty());
        assert_eq!(prediction.metrics(), PredictionMetrics::default());
        assert!(prediction.predicted_local().is_none());
        assert!(prediction.last_cmd().is_none());
        assert_eq!(prediction.last_predict_msec(), None);
        assert!(!prediction.had_local_last_snap());
    }

    #[test]
    fn the_history_limit_is_the_one_asked_for() {
        let prediction = ClientPrediction::with_history_cap(ClientId(1), 8);
        assert_eq!(prediction.history().cap(), 8);
        assert_eq!(ClientPrediction::new(ClientId(1)).history().cap(), DEFAULT_HISTORY_CAP);
    }

    #[test]
    fn a_disarmed_prediction_runs_nothing() {
        let mut prediction = ClientPrediction::new(ClientId(1));
        assert!(
            prediction
                .predict(UserCmd::default(), ServerTime::from_ms(100))
                .is_none()
        );
        assert_eq!(prediction.metrics().predicted_moves, 0);
    }

    #[test]
    fn the_local_client_can_change() {
        let mut prediction = ClientPrediction::new(ClientId(1));
        prediction.set_local(ClientId(5));
        assert_eq!(prediction.local(), ClientId(5));
    }

    #[test]
    fn arming_then_disarming_goes_back_to_the_start_for_the_same_client() {
        let mut prediction = armed();
        assert!(prediction.is_armed());
        prediction.disarm();
        assert!(!prediction.is_armed());
        assert_eq!(prediction.local(), ClientId(1));
    }

    #[test]
    fn commands_are_numbered_from_one_and_stamped_with_the_server_time() {
        let mut prediction = armed();
        let (first, cmd) = prediction
            .predict(UserCmd::default(), ServerTime::from_ms(50))
            .expect("armed prediction runs");
        assert_eq!(first, CmdSeq(1));
        assert_eq!(cmd.server_time, 50);
        let (second, _) = prediction
            .predict(UserCmd::default(), ServerTime::from_ms(66))
            .expect("armed prediction runs");
        assert_eq!(second, CmdSeq(2));
        assert_eq!(prediction.history().len(), 2);
        assert_eq!(prediction.metrics().predicted_moves, 2);
        assert_eq!(prediction.metrics().deepest_history, 2);
        assert_eq!(prediction.last_cmd().map(|c| c.server_time), Some(66));
    }

    #[test]
    fn a_step_is_never_longer_than_200_ms() {
        let mut prediction = armed();
        prediction.predict(UserCmd::default(), ServerTime::from_ms(5000));
        assert_eq!(prediction.last_predict_msec(), Some(LONGEST_STEP_MS));
    }

    #[test]
    fn time_running_backwards_steps_nothing() {
        let mut prediction = armed();
        prediction.predict(UserCmd::default(), ServerTime::from_ms(0));
        assert_eq!(prediction.last_predict_msec(), Some(0));
    }

    #[test]
    fn a_full_history_stops_new_commands() {
        let mut prediction = ClientPrediction::with_history_cap(ClientId(1), 2);
        prediction.arm_from_content(&SimWorld::new());
        assert!(prediction.predict(UserCmd::default(), ServerTime::from_ms(10)).is_some());
        assert!(prediction.predict(UserCmd::default(), ServerTime::from_ms(20)).is_some());
        assert!(prediction.predict(UserCmd::default(), ServerTime::from_ms(30)).is_none());
        assert_eq!(prediction.history().len(), 2);
    }
}
