//! A sound alias as the mixer sees it: a name, and one row per variant with
//! the numbers that variant is played by.

/// A falloff curve: gain against distance, as (distance, gain) knots.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CapturedSndCurve {
    pub name: String,
    pub knots: Vec<(f32, f32)>,
}

/// Where a variant's in-memory clip would be. Black Ops II streams every
/// clip from its bank files, so this never points anywhere; it stays so the
/// clip store can still ask.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LoadedClipSlot;

impl LoadedClipSlot {
    pub fn bound_index(self) -> Option<usize> {
        None
    }
}

/// One variant of an alias.
#[derive(Clone, Debug, Default)]
pub struct CapturedAlias {
    /// An alias played over this one's own clip.
    pub secondary: Option<String>,
    pub loaded: LoadedClipSlot,
    /// The bank file path and the entry id (eight hex digits) the clip
    /// streams from; `None` when no bank holds it.
    pub streamed: Option<(String, String)>,
    pub file_name: Option<String>,
    pub vol_min: f32,
    pub vol_max: f32,
    pub pitch_min: f32,
    pub pitch_max: f32,
    pub dist_min: f32,
    pub dist_max: f32,
    pub flags: Option<u32>,
    /// Chance weight among the alias's variants, 0 to 1.
    pub probability: f32,
    pub volume_falloff: Option<CapturedSndCurve>,
    /// The curve the volume follows inside the variant's minimum distance.
    pub near_falloff: Option<CapturedSndCurve>,
    pub voice_priority: Option<VoicePriority>,
    pub limit_count: Option<u8>,
    pub entity_limit_count: Option<u8>,
    /// The menu volume this variant answers to: 0 effects, 1 music, 2 voice,
    /// 3 interface, 4 cinematic.
    pub menu_category: Option<u8>,
    /// Fixed stereo gains for a variant that does not pan by direction.
    pub stereo_pan: Option<StereoPan>,
    /// The duck (by id) this variant starts while it plays.
    pub duck: Option<u32>,
    /// The duck group this variant is turned down in, an index into
    /// `SoundCatalog::duck_groups`.
    pub duck_group: Option<u8>,
}

/// One alias and its variants.
#[derive(Clone, Debug, Default)]
pub struct CapturedSound {
    pub name: String,
    pub aliases: Vec<CapturedAlias>,
}

impl CapturedSound {
    /// The entity channel a variant plays on. Alias rows carry none yet.
    pub fn ent_channel(&self, _variant: usize) -> Option<u32> {
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
/// fading in over `fade_in` seconds and out over `fade_out`.
#[derive(Clone, Debug, PartialEq)]
pub struct SndDuck {
    pub name: String,
    pub id: u32,
    pub fade_in: f32,
    pub fade_out: f32,
    pub start_delay: f32,
    /// Inches; above zero the duck weakens with the listener's distance from
    /// its sound.
    pub distance: f32,
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
    /// The gains for a row's front, back, left and right weights. Stereo
    /// output has no centre or LFE speaker, so a row that feeds only those
    /// comes out silent.
    pub fn from_pan(front: f32, back: f32, left: f32, right: f32) -> Self {
        let total = front + back;
        let (mono, stereo) = (total.clamp(0.0, 1.0), total / 2.0);
        Self {
            mono: [left * mono, right * mono],
            stereo: [left * stereo, right * stereo],
        }
    }

    /// (left, right) for a clip of `channels` channels.
    pub fn gains(&self, channels: u16) -> (f32, f32) {
        let pair = if channels > 1 { self.stereo } else { self.mono };
        (pair[0], pair[1])
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
    /// The priority at `distance` inches; no distance counts as inaudible.
    pub fn evaluate(&self, distance: Option<f32>) -> f32 {
        let audible = match distance {
            Some(d) if self.distance_max > 0.0 => 1.0 - (d / self.distance_max).clamp(0.0, 1.0),
            _ => 0.0,
        };
        let [lo, hi] = self.thresholds.map(|t| f32::from(t) / 255.0);
        let [floor, ceil] = self.values.map(f32::from);
        if audible <= lo {
            floor
        } else if audible >= hi {
            ceil
        } else {
            floor + (ceil - floor) * (audible - lo) / (hi - lo)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pans_follow_the_clip_width() {
        let front = StereoPan::from_pan(1.0, 0.0, 1.0, 1.0);
        assert_eq!(front.gains(1), (1.0, 1.0));
        assert_eq!(front.gains(2), (0.5, 0.5));
        let quad = StereoPan::from_pan(1.0, 1.0, 1.0, 1.0);
        assert_eq!(quad.gains(1), (1.0, 1.0));
        assert_eq!(quad.gains(2), (1.0, 1.0));
        assert_eq!(StereoPan::from_pan(1.0, 0.0, 0.0, 1.0).gains(1), (0.0, 1.0));
        assert_eq!(StereoPan::from_pan(0.0, 0.0, 0.0, 0.0).gains(2), (0.0, 0.0));
    }

    #[test]
    fn priority_rises_between_the_thresholds() {
        let priority = VoicePriority {
            thresholds: [0, 255],
            values: [10, 20],
            distance_max: 100.0,
        };
        assert_eq!(priority.evaluate(None), 10.0);
        assert_eq!(priority.evaluate(Some(100.0)), 10.0);
        assert_eq!(priority.evaluate(Some(0.0)), 20.0);
        assert!((priority.evaluate(Some(50.0)) - 15.0).abs() < 1e-4);
    }

    #[test]
    fn priority_without_a_range_is_the_floor() {
        let priority = VoicePriority {
            thresholds: [128, 255],
            values: [3, 9],
            distance_max: 0.0,
        };
        assert_eq!(priority.evaluate(Some(1.0)), 3.0);
    }

    #[test]
    fn the_loaded_slot_is_always_empty() {
        assert_eq!(CapturedAlias::default().loaded.bound_index(), None);
        assert_eq!(CapturedSound::default().ent_channel(0), None);
    }
}
