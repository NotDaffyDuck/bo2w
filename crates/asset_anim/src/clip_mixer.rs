//! A fixed row of animation slots. Each slot plays one clip on its own clock
//! and speed, and fades toward a blend weight over a set time.

use std::sync::Arc;

use crate::motion_clip::SkeletalClip;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipPlanningFault {
    NodeRejected { node_ix: usize, len: usize },
}

impl core::fmt::Display for ClipPlanningFault {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let Self::NodeRejected { node_ix: node, len } = self;
        write!(f, "animation slot {node} is out of range (there are {len})")
    }
}

impl std::error::Error for ClipPlanningFault {}

/// A slot that is playing: it has a clip and a weight above zero.
#[derive(Debug, Clone, Copy)]
pub struct PlayingMotion<'a> {
    pub slot_no: usize,
    pub reel: &'a SkeletalClip,
    pub time: f32,
    pub pace: f32,
    pub level: f32,
}

/// A blend that is under way: the level it is heading for and the seconds
/// still to run.
#[derive(Debug, Clone, Copy, Default)]
struct Blend {
    goal: f32,
    seconds: f32,
}

/// A slot's playhead and how fast it runs.
#[derive(Debug, Clone, Copy)]
struct Clock {
    at: f32,
    speed: f32,
}

impl Default for Clock {
    fn default() -> Self {
        Clock { at: 0.0, speed: 1.0 }
    }
}

#[derive(Debug, Default)]
struct Slot {
    held: Option<Arc<SkeletalClip>>,
    clock: Clock,
    level: f32,
    blend: Blend,
}

impl Slot {
    /// Moves the clock on by `dt` seconds at the slot's speed. A clip with no
    /// length stays at its start; a looping one wraps; any other stops at its
    /// ends.
    fn step_clock(&mut self, dt: f32) {
        let Some(span) = self.held.as_deref().map(|c| (c.duration(), c.looping)) else {
            return;
        };
        let (length, wraps) = span;
        let target = self.clock.at + dt * self.clock.speed;
        self.clock.at = match (length > f32::EPSILON, wraps) {
            (false, _) => 0.0,
            (true, true) => target.rem_euclid(length),
            (true, false) => target.clamp(0.0, length),
        };
    }

    /// Moves the level a straight-line share of the way to the goal.
    fn step_blend(&mut self, dt: f32) {
        let Blend { goal, seconds } = self.blend;
        if seconds <= 0.0 {
            return;
        }
        let spent = dt.min(seconds);
        let remaining = seconds - spent;
        let done = remaining <= f32::EPSILON;
        self.level = if done { goal } else { self.level + (goal - self.level) * spent / seconds };
        self.blend.seconds = if done { 0.0 } else { remaining };
    }

    /// One tick; the clip's notifies the clock passed over are added to `fired`.
    /// A slot fading out to nothing fires none.
    fn beat(&mut self, dt: f32, fired: &mut Vec<String>) {
        if self.level <= 0.0 {
            self.step_blend(dt);
            return;
        }
        let from = self.clock.at;
        self.step_clock(dt);
        let reports = self.blend.goal > 0.0;
        if let Some(clip) = self.held.as_deref().filter(|_| reports) {
            fired.extend(clip.notes_between(from, self.clock.at));
        }
        self.step_blend(dt);
    }
}

#[derive(Debug)]
pub struct ClipPlanner {
    slot_bank: Vec<Slot>,
}

impl ClipPlanner {
    pub fn new(n_slots: usize) -> Self {
        let slot_bank = (0..n_slots).map(|_| Slot::default()).collect();
        Self { slot_bank }
    }

    /// Puts a clip in a slot and starts it from its beginning.
    pub fn load_clip(&mut self, slot_no: usize, reel: Arc<SkeletalClip>) -> Result<(), ClipPlanningFault> {
        let slot = self.pick_mut(slot_no)?;
        slot.held = Some(reel);
        slot.clock.at = 0.0;
        Ok(())
    }

    pub fn seek(&mut self, slot_no: usize, at: f32) -> Result<(), ClipPlanningFault> {
        self.pick_mut(slot_no)?.clock.at = at.max(0.0);
        Ok(())
    }

    pub fn set_speed(&mut self, slot_no: usize, speed: f32) -> Result<(), ClipPlanningFault> {
        self.pick_mut(slot_no)?.clock.speed = speed;
        Ok(())
    }

    /// Sets the weight a slot fades to and the seconds it takes. A fade of no
    /// time lands on the weight at once.
    pub fn assign_target_blend(
        &mut self,
        slot_ix: usize,
        goal_level: f32,
        seconds: f32,
    ) -> Result<(), ClipPlanningFault> {
        let slot = self.pick_mut(slot_ix)?;
        let ease = Blend { goal: goal_level.max(0.0), seconds: seconds.max(0.0) };
        if ease.seconds == 0.0 {
            slot.level = ease.goal;
        }
        slot.blend = ease;
        Ok(())
    }

    /// Runs every slot on by `step_secs` seconds. Returns the names of the notifies
    /// the playing clips passed over.
    pub fn run_for(&mut self, step_secs: f32) -> Vec<String> {
        let step = step_secs.max(0.0);
        let mut notes = Vec::new();
        for slot in &mut self.slot_bank {
            slot.beat(step, &mut notes);
        }
        notes
    }

    /// The slots that are playing, in slot order.
    pub fn active(&self) -> impl Iterator<Item = PlayingMotion<'_>> {
        self.slot_bank
            .iter()
            .enumerate()
            .filter(|pair| pair.1.level > 0.0)
            .filter_map(|(node, slot)| {
                let clip = slot.held.as_deref()?;
                let Clock { at, speed } = slot.clock;
                Some(PlayingMotion { slot_no: node, reel: clip, time: at, pace: speed, level: slot.level })
            })
    }

    pub fn weight(&self, which: usize) -> Result<f32, ClipPlanningFault> {
        self.pick(which).map(|slot| slot.level)
    }

    pub fn playhead(&self, slot_no: usize) -> Result<f32, ClipPlanningFault> {
        self.pick(slot_no).map(|slot| slot.clock.at)
    }

    fn pick(&self, seek_ix: usize) -> Result<&Slot, ClipPlanningFault> {
        let len = self.slot_bank.len();
        self.slot_bank.get(seek_ix).ok_or(ClipPlanningFault::NodeRejected { node_ix: seek_ix, len })
    }

    fn pick_mut(&mut self, take_ix: usize) -> Result<&mut Slot, ClipPlanningFault> {
        let len = self.slot_bank.len();
        self.slot_bank.get_mut(take_ix).ok_or(ClipPlanningFault::NodeRejected { node_ix: take_ix, len })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motion_clip::{ClipEventMark, RootShift};

    /// Two seconds long, with a notify halfway.
    fn reel(looping: bool) -> Arc<SkeletalClip> {
        Arc::new(SkeletalClip {
            name: "test".to_owned(),
            fps: 10.0,
            frame_total: 20,
            looping,
            tracks: Vec::new(),
            notifies: vec![ClipEventMark {
                name: "halfway".to_owned(),
                time: 0.5,
            }],
            root_motion_shift: RootShift::Default,
        })
    }

    fn playing(looping: bool) -> ClipPlanner {
        let mut mixer = ClipPlanner::new(2);
        mixer.load_clip(0, reel(looping)).unwrap();
        mixer.assign_target_blend(0, 1.0, 0.0).unwrap();
        mixer
    }

    #[test]
    fn a_slot_that_does_not_exist_is_an_error() {
        let mut mixer = ClipPlanner::new(2);
        assert_eq!(
            mixer.set_speed(5, 1.0),
            Err(ClipPlanningFault::NodeRejected { node_ix: 5, len: 2 })
        );
        assert!(mixer.weight(2).is_err());
    }

    #[test]
    fn a_fade_closes_the_gap_in_equal_steps_then_lands() {
        let mut mixer = ClipPlanner::new(1);
        mixer.load_clip(0, reel(true)).unwrap();
        mixer.assign_target_blend(0, 1.0, 2.0).unwrap();
        assert_eq!(mixer.weight(0), Ok(0.0));
        mixer.run_for(1.0);
        assert_eq!(mixer.weight(0), Ok(0.5));
        mixer.run_for(5.0);
        assert_eq!(mixer.weight(0), Ok(1.0));
    }

    #[test]
    fn a_fade_of_no_time_lands_at_once_and_a_negative_goal_is_zero() {
        let mut mixer = playing(true);
        assert_eq!(mixer.weight(0), Ok(1.0));
        mixer.assign_target_blend(0, -3.0, 0.0).unwrap();
        assert_eq!(mixer.weight(0), Ok(0.0));
    }

    #[test]
    fn a_looping_clip_wraps_and_another_stops_at_its_end() {
        let mut looping = playing(true);
        looping.run_for(2.5);
        assert_eq!(looping.playhead(0), Ok(0.5));

        let mut once = playing(false);
        once.run_for(3.0);
        assert_eq!(once.playhead(0), Ok(2.0));
    }

    #[test]
    fn the_rate_scales_the_clock() {
        let mut mixer = playing(true);
        mixer.set_speed(0, 0.5).unwrap();
        mixer.run_for(1.0);
        assert_eq!(mixer.playhead(0), Ok(0.5));
    }

    #[test]
    fn a_slot_with_no_weight_does_not_run_and_is_not_active() {
        let mut mixer = ClipPlanner::new(1);
        mixer.load_clip(0, reel(true)).unwrap();
        mixer.run_for(1.0);
        assert_eq!(mixer.playhead(0), Ok(0.0));
        assert_eq!(mixer.active().count(), 0);

        mixer.assign_target_blend(0, 0.25, 0.0).unwrap();
        let live: Vec<_> = mixer.active().map(|a| (a.slot_no, a.level)).collect();
        assert_eq!(live, [(0, 0.25)]);
    }

    #[test]
    fn passing_a_notify_reports_it_once() {
        let mut mixer = playing(false);
        assert!(mixer.run_for(0.5).is_empty());
        assert_eq!(mixer.run_for(1.0), ["halfway"]);
        assert!(mixer.run_for(0.1).is_empty());
    }
}
