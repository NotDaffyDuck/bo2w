//! A fixed row of animation slots. Each slot plays one clip on its own clock
//! and speed, and fades toward a blend weight over a set time.

use std::sync::Arc;

use crate::xanim_clip::AnimClip;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipSchedulerError {
    BadNode { node: usize, len: usize },
}

impl core::fmt::Display for ClipSchedulerError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let Self::BadNode { node, len } = self;
        write!(f, "animation slot {node} is out of range (there are {len})")
    }
}

impl std::error::Error for ClipSchedulerError {}

/// A slot that is playing: it has a clip and a weight above zero.
#[derive(Debug, Clone, Copy)]
pub struct ActiveAnim<'a> {
    pub node: usize,
    pub clip: &'a AnimClip,
    pub time: f32,
    pub rate: f32,
    pub weight: f32,
}

/// Where a slot's weight is heading and how much of the trip is left.
#[derive(Debug, Clone, Copy, Default)]
struct Fade {
    target: f32,
    left: f32,
}

#[derive(Debug)]
struct Slot {
    clip: Option<Arc<AnimClip>>,
    time: f32,
    rate: f32,
    weight: f32,
    fade: Fade,
}

impl Default for Slot {
    fn default() -> Self {
        Self {
            clip: None,
            time: 0.0,
            rate: 1.0,
            weight: 0.0,
            fade: Fade::default(),
        }
    }
}

impl Slot {
    /// Moves the clock on by `dt` seconds at the slot's speed. A clip with no
    /// length stays at its start; a looping one wraps; any other stops at its
    /// ends.
    fn step_clock(&mut self, dt: f32) {
        let Some(clip) = self.clip.as_deref() else {
            return;
        };
        let length = clip.duration();
        let next = self.time + dt * self.rate;
        self.time = if length <= f32::EPSILON {
            0.0
        } else if clip.looping {
            next.rem_euclid(length)
        } else {
            next.clamp(0.0, length)
        };
    }

    /// Moves the weight a straight-line share of the way to its target.
    fn step_fade(&mut self, dt: f32) {
        if self.fade.left <= 0.0 {
            return;
        }
        let used = dt.min(self.fade.left);
        self.weight += (self.fade.target - self.weight) * used / self.fade.left;
        self.fade.left -= used;
        if self.fade.left <= f32::EPSILON {
            self.weight = self.fade.target;
            self.fade.left = 0.0;
        }
    }

    /// One tick; the clip's notifies the clock passed over are added to `fired`.
    /// A slot fading out to nothing fires none.
    fn tick(&mut self, dt: f32, fired: &mut Vec<String>) {
        if self.weight > 0.0 {
            let before = self.time;
            self.step_clock(dt);
            if self.fade.target > 0.0
                && let Some(clip) = self.clip.as_deref()
            {
                fired.extend(clip.crossed_notifies(before, self.time));
            }
        }
        self.step_fade(dt);
    }
}

#[derive(Debug)]
pub struct ClipScheduler {
    slots: Vec<Slot>,
}

impl ClipScheduler {
    pub fn new(node_count: usize) -> Self {
        Self {
            slots: std::iter::repeat_with(Slot::default).take(node_count).collect(),
        }
    }

    /// Puts a clip in a slot and starts it from its beginning.
    pub fn set_clip(&mut self, node: usize, clip: Arc<AnimClip>) -> Result<(), ClipSchedulerError> {
        let slot = self.slot_mut(node)?;
        slot.clip = Some(clip);
        slot.time = 0.0;
        Ok(())
    }

    pub fn set_time(&mut self, node: usize, time: f32) -> Result<(), ClipSchedulerError> {
        self.slot_mut(node)?.time = time.max(0.0);
        Ok(())
    }

    pub fn set_rate(&mut self, node: usize, rate: f32) -> Result<(), ClipSchedulerError> {
        self.slot_mut(node)?.rate = rate;
        Ok(())
    }

    /// Sets the weight a slot fades to and the seconds it takes. A fade of no
    /// time lands on the weight at once.
    pub fn set_goal_weight(
        &mut self,
        node: usize,
        goal_weight: f32,
        goal_time: f32,
    ) -> Result<(), ClipSchedulerError> {
        let slot = self.slot_mut(node)?;
        slot.fade = Fade {
            target: goal_weight.max(0.0),
            left: goal_time.max(0.0),
        };
        if slot.fade.left == 0.0 {
            slot.weight = slot.fade.target;
        }
        Ok(())
    }

    /// Runs every slot on by `dt` seconds. Returns the names of the notifies
    /// the playing clips passed over.
    pub fn advance(&mut self, dt: f32) -> Vec<String> {
        let dt = dt.max(0.0);
        let mut fired = Vec::new();
        for slot in &mut self.slots {
            slot.tick(dt, &mut fired);
        }
        fired
    }

    /// The slots that are playing, in slot order.
    pub fn active(&self) -> impl Iterator<Item = ActiveAnim<'_>> {
        self.slots
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.weight > 0.0)
            .filter_map(|(node, slot)| {
                Some(ActiveAnim {
                    node,
                    clip: slot.clip.as_deref()?,
                    time: slot.time,
                    rate: slot.rate,
                    weight: slot.weight,
                })
            })
    }

    pub fn weight(&self, node: usize) -> Result<f32, ClipSchedulerError> {
        self.slot(node).map(|slot| slot.weight)
    }

    pub fn time(&self, node: usize) -> Result<f32, ClipSchedulerError> {
        self.slot(node).map(|slot| slot.time)
    }

    fn slot(&self, node: usize) -> Result<&Slot, ClipSchedulerError> {
        let len = self.slots.len();
        self.slots.get(node).ok_or(ClipSchedulerError::BadNode { node, len })
    }

    fn slot_mut(&mut self, node: usize) -> Result<&mut Slot, ClipSchedulerError> {
        let len = self.slots.len();
        self.slots.get_mut(node).ok_or(ClipSchedulerError::BadNode { node, len })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xanim_clip::{ClipNotify, Translation};

    /// Two seconds long, with a notify halfway.
    fn clip(looping: bool) -> Arc<AnimClip> {
        Arc::new(AnimClip {
            name: "test".to_owned(),
            framerate: 10.0,
            numframes: 20,
            looping,
            tracks: Vec::new(),
            notifies: vec![ClipNotify {
                name: "halfway".to_owned(),
                time: 0.5,
            }],
            delta_translation: Translation::Default,
        })
    }

    fn playing(looping: bool) -> ClipScheduler {
        let mut mixer = ClipScheduler::new(2);
        mixer.set_clip(0, clip(looping)).unwrap();
        mixer.set_goal_weight(0, 1.0, 0.0).unwrap();
        mixer
    }

    #[test]
    fn a_slot_that_does_not_exist_is_an_error() {
        let mut mixer = ClipScheduler::new(2);
        assert_eq!(
            mixer.set_rate(5, 1.0),
            Err(ClipSchedulerError::BadNode { node: 5, len: 2 })
        );
        assert!(mixer.weight(2).is_err());
    }

    #[test]
    fn a_fade_closes_the_gap_in_equal_steps_then_lands() {
        let mut mixer = ClipScheduler::new(1);
        mixer.set_clip(0, clip(true)).unwrap();
        mixer.set_goal_weight(0, 1.0, 2.0).unwrap();
        assert_eq!(mixer.weight(0), Ok(0.0));
        mixer.advance(1.0);
        assert_eq!(mixer.weight(0), Ok(0.5));
        mixer.advance(5.0);
        assert_eq!(mixer.weight(0), Ok(1.0));
    }

    #[test]
    fn a_fade_of_no_time_lands_at_once_and_a_negative_goal_is_zero() {
        let mut mixer = playing(true);
        assert_eq!(mixer.weight(0), Ok(1.0));
        mixer.set_goal_weight(0, -3.0, 0.0).unwrap();
        assert_eq!(mixer.weight(0), Ok(0.0));
    }

    #[test]
    fn a_looping_clip_wraps_and_another_stops_at_its_end() {
        let mut looping = playing(true);
        looping.advance(2.5);
        assert_eq!(looping.time(0), Ok(0.5));

        let mut once = playing(false);
        once.advance(3.0);
        assert_eq!(once.time(0), Ok(2.0));
    }

    #[test]
    fn the_rate_scales_the_clock() {
        let mut mixer = playing(true);
        mixer.set_rate(0, 0.5).unwrap();
        mixer.advance(1.0);
        assert_eq!(mixer.time(0), Ok(0.5));
    }

    #[test]
    fn a_slot_with_no_weight_does_not_run_and_is_not_active() {
        let mut mixer = ClipScheduler::new(1);
        mixer.set_clip(0, clip(true)).unwrap();
        mixer.advance(1.0);
        assert_eq!(mixer.time(0), Ok(0.0));
        assert_eq!(mixer.active().count(), 0);

        mixer.set_goal_weight(0, 0.25, 0.0).unwrap();
        let live: Vec<_> = mixer.active().map(|a| (a.node, a.weight)).collect();
        assert_eq!(live, [(0, 0.25)]);
    }

    #[test]
    fn passing_a_notify_reports_it_once() {
        let mut mixer = playing(false);
        assert!(mixer.advance(0.5).is_empty());
        assert_eq!(mixer.advance(1.0), ["halfway"]);
        assert!(mixer.advance(0.1).is_empty());
    }
}
