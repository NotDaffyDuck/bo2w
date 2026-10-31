//! A sound alias as the mixer sees it: a name, and one row per variant with
//! the numbers that variant is played by.

/// A falloff curve: gain against distance, as (distance, gain) knots.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SnapshotAudioCurve {
    pub name: String,
    pub knots: Vec<(f32, f32)>,
}

/// Where a variant's in-memory clip would be. Black Ops II streams every
/// clip from its bank files, so this never points anywhere; it stays so the
/// clip store can still ask.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LoadedClipSlot;

impl LoadedClipSlot {
    pub fn extent_position(self) -> Option<usize> {
        None
    }
}

/// One variant of an alias.
#[derive(Clone, Debug, Default)]
pub struct SnapshotSoundAlias {
    /// An alias played over this one's own clip.
    pub layered_alias: Option<String>,
    pub loaded: LoadedClipSlot,
    /// The bank file path and the entry id (eight hex digits) the clip
    /// streams from; `None` when no bank holds it.
    pub streamed: Option<(String, String)>,
    pub clip_file: Option<String>,
    pub volume_low: f32,
    pub volume_high: f32,
    pub pitch_lower_bound: f32,
    pub pitch_upper_bound: f32,
    pub reach_lower: f32,
    pub reach_upper: f32,
    pub alias_bits: Option<u32>,
    /// Chance weight among the alias's variants, 0 to 1.
    pub chance_weight: f32,
    pub volume_falloff: Option<SnapshotAudioCurve>,
    /// The curve the volume follows inside the variant's minimum distance.
    pub near_falloff: Option<SnapshotAudioCurve>,
    pub voice_priority: Option<VoicePriority>,
    pub cap_total: Option<u8>,
    pub actor_cap_total: Option<u8>,
    /// The menu volume this variant answers to: 0 effects, 1 music, 2 voice,
    /// 3 interface, 4 cinematic.
    pub menu_category: Option<u8>,
    /// Fixed stereo gains for a variant that does not pan by direction.
    pub stereo_pan: Option<StereoPan>,
    /// The duck (by id) this variant starts while it plays.
    pub duck: Option<u32>,
    /// The duck group this variant is turned down in, an index into
    /// `AudioRoster::duck_groups`.
    pub duck_group: Option<u8>,
    /// bo2zm: how much of the sound goes to the room's echo (0..1).
    pub reverb_send: f32,
}

/// One alias and its variants.
#[derive(Clone, Debug, Default)]
pub struct SnapshotSoundEntry {
    pub name: String,
    pub aliases: Vec<SnapshotSoundAlias>,
}

impl SnapshotSoundEntry {
    /// The entity channel a variant plays on. Alias rows carry none yet.
    pub fn channel_of_row(&self, _variant: usize) -> Option<u32> {
        None
    }
}

/// One band of a master mix's equaliser.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SndEqBand {
    pub enabled: bool,
    pub gain_db: f32,
    pub freq: f32,
    pub q: f32,
}

/// A master mix: a low shelf, two peaks and a high shelf, then an overall
/// gain. The audio preset names the one in use.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SndMaster {
    pub name: String,
    pub low: SndEqBand,
    pub peak1: SndEqBand,
    pub peak2: SndEqBand,
    pub hi: SndEqBand,
    pub eq_gain: f32,
}

/// A duck: while it is on, each duck group is turned down toward its
/// `attenuation` (1 leaves it alone) and low-passed toward its `filter`,
/// fading in over `ease_on` seconds and out over `ease_off`.
#[derive(Clone, Debug, PartialEq)]
pub struct SndDuck {
    pub name: String,
    pub id: u32,
    pub ease_on: f32,
    pub ease_off: f32,
    pub start_delay: f32,
    /// Inches; above zero the duck weakens with the listener's distance from
    /// its sound.
    pub fade_range: f32,
    /// Seconds; zero lasts as long as its sound plays.
    pub length: f32,
    pub update_while_paused: bool,
    pub attenuation: [f32; 32],
    pub filter: [f32; 32],
}

/// Left and right gains a pan table row gives a clip, for a mono clip and a
/// stereo one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StereoPan {
    pub mono: [f32; 2],
    pub stereo: [f32; 2],
}

impl StereoPan {
    /// The gains for a row's weights, in the order front, back, left, right. Stereo
    /// output has no centre or LFE speaker, so a row that feeds only those
    /// comes out silent.
    pub fn from_weights(weights: [f32; 4]) -> Self {
        let [front, back, left, right] = weights;
        // Both gain pairs are the speaker weights times one scale each.
        let scaled = |scale: f32| [left * scale, right * scale];
        let reach = front + back;
        Self {
            mono: scaled(reach.clamp(0.0, 1.0)),
            stereo: scaled(reach * 0.5),
        }
    }

    /// (left, right) for a clip of `channels` channels.
    pub fn gains(&self, channels: u16) -> (f32, f32) {
        let [l, r] = [self.mono, self.stereo][usize::from(channels > 1)];
        (l, r)
    }
}

/// How loud a voice must be to keep its slot: a priority that rises from
/// `values[0]` to `values[1]` as its audible volume crosses the thresholds.
#[derive(Clone, Debug)]
pub struct VoicePriority {
    pub thresholds: [u8; 2],
    pub values: [u8; 2],
    pub distance_max: f32,
}

impl VoicePriority {
    /// The priority `range` inches away; no range counts as inaudible.
    pub fn priority_for(&self, range: Option<f32>) -> f32 {
        let closeness = match range {
            Some(away) if self.distance_max > 0.0 => (1.0 - away / self.distance_max).clamp(0.0, 1.0),
            _ => 0.0,
        };
        let [from, to] = self.thresholds.map(|edge| f32::from(edge) / 255.0);
        let [bottom, top] = self.values.map(f32::from);
        // How far along the ramp: a ramp with no width steps at its start.
        let along = match to - from {
            width if width > 0.0 => ((closeness - from) / width).clamp(0.0, 1.0),
            _ => f32::from(closeness > from),
        };
        bottom + (top - bottom) * along
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gains_of(pan: [f32; 4], channels: u16) -> (f32, f32) {
        StereoPan::from_weights(pan).gains(channels)
    }

    fn priority_at(thresholds: [u8; 2], values: [u8; 2], range: f32, distance: Option<f32>) -> f32 {
        VoicePriority {
            thresholds,
            values,
            distance_max: range,
        }
        .priority_for(distance)
    }

    #[test]
    fn pans_follow_the_clip_width() {
        let cases = [
            ([1.0, 0.0, 1.0, 1.0], 1, (1.0, 1.0)),
            ([1.0, 0.0, 1.0, 1.0], 2, (0.5, 0.5)),
            ([1.0, 1.0, 1.0, 1.0], 1, (1.0, 1.0)),
            ([1.0, 1.0, 1.0, 1.0], 2, (1.0, 1.0)),
            ([1.0, 0.0, 0.0, 1.0], 1, (0.0, 1.0)),
            ([0.0, 0.0, 0.0, 0.0], 2, (0.0, 0.0)),
        ];
        for (pan, channels, want) in cases {
            assert_eq!(
                gains_of(pan, channels),
                want,
                "pan {pan:?} on {channels} channels"
            );
        }
    }

    #[test]
    fn priority_rises_between_the_thresholds() {
        let at = |distance| priority_at([0, 255], [10, 20], 100.0, distance);
        assert_eq!(
            [at(None), at(Some(100.0)), at(Some(0.0))],
            [10.0, 10.0, 20.0]
        );
        assert!((at(Some(50.0)) - 15.0).abs() < 1e-4);
    }

    #[test]
    fn priority_without_a_range_is_the_floor() {
        assert_eq!(priority_at([128, 255], [3, 9], 0.0, Some(1.0)), 3.0);
    }

    #[test]
    fn a_ramp_with_no_width_steps_once() {
        let at = |distance| priority_at([128, 128], [3, 9], 100.0, Some(distance));
        assert_eq!([at(90.0), at(10.0)], [3.0, 9.0]);
    }

    #[test]
    fn the_loaded_slot_is_always_empty() {
        assert_eq!(SnapshotSoundAlias::default().loaded.extent_position(), None);
        assert_eq!(SnapshotSoundEntry::default().channel_of_row(0), None);
    }
}

