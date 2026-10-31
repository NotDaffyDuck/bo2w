//! bo2mp: Black Ops II's master mix EQ (Options > Audio > Audio Presets). Every sound
//! passes through the current master's EQ: a low shelf, two peaks and a high shelf (gain
//! in dB, frequency in Hz, Q), then `eq_gain`. The preset (`snd_menu_presets`) picks the
//! master by name (`SND_GetMasterByConfig`), falling back to "default"
//! (`SND_GetMasterCurrent`).
//!
//! An EQ is a linear filter, so shaping each sound before it is mixed sounds the same as
//! shaping the mix. Each voice runs it at its own sample rate. With no Black Ops II master
//! loaded (Modern Warfare 2) there is none and every sound passes untouched.
//!
//! Not matched: the master's compressor, limiter and bus gains. Black Ops II applies them
//! to the mix in the order scale (bus gains), pfutz, voice filter, compressor, EQ, gfutz,
//! limiter; their formulas are in the game's mixer, which is not in any source we read.
//! The bus gains alone are known (`SD_GetMixParam`: each voice's dry level times its bus's
//! gain, the wet level times the reverb bus's), but they set the levels going into the
//! compressor (Treyarch Mix: voice 5.6, effects 0.15), so they stay off until it is built.
//! Super Crunch differs from Treyarch Mix only in its compressor and limiter.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, RwLock};

use asset_audio::{SndEqBand, SndMaster};
use bevy::prelude::*;

use crate::play_core::AudioCatalogRes;

/// `snd_menu_presets` 0-4 (`CoD.AudioSettings`), by master name.
const PRESET_MASTERS: [&str; 5] = [
    "treyarch_mix",
    "bass_boost",
    "high_boost",
    "super_crunch",
    "headphones",
];

static CURRENT: RwLock<Option<Arc<SndMaster>>> = RwLock::new(None);
/// Bumped whenever `CURRENT` changes; each voice rebuilds its filters when it moves.
static VERSION: AtomicU32 = AtomicU32::new(0);

pub(crate) fn wire_up(host: &mut App) {
    host.add_systems(Update, pick_master);
}

/// The master the preset names, else "default"; none without Black Ops II's masters.
fn master_for_preset(masters: &[SndMaster], preset: u8) -> Option<&SndMaster> {
    let by_name = |name: &str| masters.iter().find(|m| m.name.eq_ignore_ascii_case(name));
    PRESET_MASTERS
        .get(preset as usize)
        .and_then(|name| by_name(name))
        .or_else(|| by_name("default"))
}

/// A bank without masters (a map's own sounds) keeps the frontend's: Black Ops II's
/// masters live in its always-loaded code zone.
fn pick_master(
    settings: Res<frame::PlayerPrefsBundle>,
    bank: Option<Res<AudioCatalogRes>>,
    mut picked: Local<Option<(u8, String)>>,
) {
    let Some(bank) = bank else { return };
    if !settings.is_changed() && !bank.is_changed() {
        return;
    }
    let Some(master) = master_for_preset(&bank.0.masters, settings.audio_preset) else {
        return;
    };
    let key = (settings.audio_preset, master.name.clone());
    if picked.as_ref() == Some(&key) {
        return;
    }
    diag::info!(
        Audio,
        "audio: bo2mp master mix: preset {} -> {}",
        settings.audio_preset, master.name
    );
    *picked = Some(key);
    if let Ok(mut current) = CURRENT.write() {
        *current = Some(Arc::new(master.clone()));
    }
    VERSION.fetch_add(1, Ordering::Release);
}

/// One normalized biquad (`SndDspBiQuadCoef`: b0, b1, b2, a1, a2 over a0).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Shape {
    LowShelf,
    Peak,
    HighShelf,
}

impl Biquad {
    /// The band at `rate` Hz; none when it is off or flat.
    fn band(shape: Shape, band: &SndEqBand, rate: f32) -> Option<Self> {
        if !band.enabled || !band.gain_db.is_finite() || band.gain_db.abs() < 1e-4 {
            return None;
        }
        let a = 10f64.powf(f64::from(band.gain_db) / 40.0);
        let rate = f64::from(rate.max(1.0));
        let freq = f64::from(band.freq).clamp(10.0, rate * 0.49);
        let q = f64::from(band.q).clamp(0.01, 32.0);
        let w0 = std::f64::consts::TAU * freq / rate;
        let (sn, cs) = w0.sin_cos();
        let alpha = sn / (2.0 * q);
        let sq = 2.0 * a.sqrt() * alpha;
        let (b0, b1, b2, a0, a1, a2) = match shape {
            Shape::Peak => (
                1.0 + alpha * a,
                -2.0 * cs,
                1.0 - alpha * a,
                1.0 + alpha / a,
                -2.0 * cs,
                1.0 - alpha / a,
            ),
            Shape::LowShelf => (
                a * ((a + 1.0) - (a - 1.0) * cs + sq),
                2.0 * a * ((a - 1.0) - (a + 1.0) * cs),
                a * ((a + 1.0) - (a - 1.0) * cs - sq),
                (a + 1.0) + (a - 1.0) * cs + sq,
                -2.0 * ((a - 1.0) + (a + 1.0) * cs),
                (a + 1.0) + (a - 1.0) * cs - sq,
            ),
            Shape::HighShelf => (
                a * ((a + 1.0) + (a - 1.0) * cs + sq),
                -2.0 * a * ((a - 1.0) + (a + 1.0) * cs),
                a * ((a + 1.0) + (a - 1.0) * cs - sq),
                (a + 1.0) - (a - 1.0) * cs + sq,
                2.0 * ((a - 1.0) - (a + 1.0) * cs),
                (a + 1.0) - (a - 1.0) * cs - sq,
            ),
        };
        Some(Self {
            b0: (b0 / a0) as f32,
            b1: (b1 / a0) as f32,
            b2: (b2 / a0) as f32,
            a1: (a1 / a0) as f32,
            a2: (a2 / a0) as f32,
        })
    }

    /// Its gain (linear) at `freq` Hz.
    #[cfg(test)]
    fn gain_at(&self, freq: f64, rate: f64) -> f64 {
        let w = std::f64::consts::TAU * freq / rate;
        let z1 = (w.cos(), -w.sin());
        let z2 = ((2.0 * w).cos(), -(2.0 * w).sin());
        let num = (
            f64::from(self.b0) + f64::from(self.b1) * z1.0 + f64::from(self.b2) * z2.0,
            f64::from(self.b1) * z1.1 + f64::from(self.b2) * z2.1,
        );
        let den = (
            1.0 + f64::from(self.a1) * z1.0 + f64::from(self.a2) * z2.0,
            f64::from(self.a1) * z1.1 + f64::from(self.a2) * z2.1,
        );
        num.0.hypot(num.1) / den.0.hypot(den.1)
    }
}

/// A master's EQ at one sample rate.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MasterEq {
    bands: Vec<Biquad>,
    gain: f32,
}

impl MasterEq {
    pub(crate) fn new(master: &SndMaster, rate: f32) -> Self {
        let bands = [
            (Shape::LowShelf, &master.low),
            (Shape::Peak, &master.peak1),
            (Shape::Peak, &master.peak2),
            (Shape::HighShelf, &master.hi),
        ]
        .into_iter()
        .filter_map(|(shape, band)| Biquad::band(shape, band, rate))
        .collect();
        let gain = if master.eq_gain.is_finite() && master.eq_gain > 0.0 {
            master.eq_gain
        } else {
            1.0
        };
        Self { bands, gain }
    }

    #[cfg(test)]
    fn gain_at(&self, freq: f64, rate: f64) -> f64 {
        self.bands.iter().map(|b| b.gain_at(freq, rate)).product::<f64>() * f64::from(self.gain)
    }
}

/// One voice's EQ: the current master's filters at its rate, and each channel's history.
#[derive(Clone, Debug)]
pub(crate) struct VoiceEq {
    version: u32,
    rate: f32,
    chan_count: usize,
    next_channel: usize,
    eq: Option<MasterEq>,
    /// x1, x2, y1, y2 per channel per band.
    history: Vec<[f32; 4]>,
}

impl VoiceEq {
    pub(crate) fn new(rate: u32, chan_count: u16) -> Self {
        let mut eq = Self {
            version: VERSION.load(Ordering::Acquire),
            rate: rate.max(1) as f32,
            chan_count: usize::from(chan_count.max(1)),
            next_channel: 0,
            eq: None,
            history: Vec::new(),
        };
        eq.rebuild();
        eq
    }

    fn rebuild(&mut self) {
        let master = CURRENT.read().ok().and_then(|m| m.clone());
        self.eq = master.map(|m| MasterEq::new(&m, self.rate));
        let bands = self.eq.as_ref().map_or(0, |e| e.bands.len());
        self.history.resize(self.chan_count * bands, [0.0; 4]);
    }

    /// The next interleaved sample, through the current master's EQ.
    #[inline]
    pub(crate) fn process(&mut self, x: f32) -> f32 {
        let version = VERSION.load(Ordering::Acquire);
        if version != self.version {
            self.version = version;
            self.rebuild();
        }
        self.filter(x)
    }

    #[inline]
    fn filter(&mut self, x: f32) -> f32 {
        let ch = self.next_channel;
        self.next_channel = (ch + 1) % self.chan_count;
        let Some(eq) = &self.eq else { return x };
        let n = eq.bands.len();
        let mut y = x;
        for (band, h) in eq.bands.iter().zip(&mut self.history[ch * n..ch * n + n]) {
            let mut out =
                band.b0 * y + band.b1 * h[0] + band.b2 * h[1] - band.a1 * h[2] - band.a2 * h[3];
            if out.abs() < 1e-20 {
                out = 0.0;
            }
            h[1] = h[0];
            h[0] = y;
            h[3] = h[2];
            h[2] = out;
            y = out;
        }
        y * eq.gain
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f64 = 48_000.0;

    fn band(gain_db: f32, freq: f32, q: f32) -> SndEqBand {
        SndEqBand {
            enabled: true,
            gain_db,
            freq,
            q,
        }
    }

    fn db(g: f64) -> f64 {
        20.0 * g.log10()
    }

    /// Black Ops II's `treyarch_mix` (and "default") EQ, read from code_post_gfx_mp.
    fn treyarch_mix() -> SndMaster {
        SndMaster {
            name: "treyarch_mix".into(),
            low: band(5.0, 132.3, 1.168),
            peak1: band(5.0, 357.9, 0.221),
            peak2: band(-8.0, 2772.5, 9.795),
            hi: band(3.0, 4621.9, 1.754),
            eq_gain: 1.0,
        }
    }

    #[test]
    fn a_peak_reaches_its_gain_at_its_frequency() {
        let b = Biquad::band(Shape::Peak, &band(-8.0, 2772.5, 9.795), RATE as f32).unwrap();
        assert!((db(b.gain_at(2772.5, RATE)) + 8.0).abs() < 0.01);
        assert!(db(b.gain_at(100.0, RATE)).abs() < 0.05);
    }

    #[test]
    fn shelves_reach_their_gain_at_their_ends() {
        let lo = Biquad::band(Shape::LowShelf, &band(5.0, 132.3, 1.168), RATE as f32).unwrap();
        assert!((db(lo.gain_at(1.0, RATE)) - 5.0).abs() < 0.01);
        assert!(db(lo.gain_at(15_000.0, RATE)).abs() < 0.05);
        let hi = Biquad::band(Shape::HighShelf, &band(3.0, 4621.9, 1.754), RATE as f32).unwrap();
        assert!((db(hi.gain_at(23_999.0, RATE)) - 3.0).abs() < 0.05);
        assert!(db(hi.gain_at(20.0, RATE)).abs() < 0.01);
    }

    #[test]
    fn a_flat_or_disabled_band_is_skipped() {
        assert!(Biquad::band(Shape::Peak, &band(0.0, 1000.0, 1.0), 48_000.0).is_none());
        let mut off = band(6.0, 1000.0, 1.0);
        off.enabled = false;
        assert!(Biquad::band(Shape::Peak, &off, 48_000.0).is_none());
    }

    #[test]
    fn the_filter_runs_as_its_response_says() {
        // A 1 kHz sine through the Treyarch mix settles at the EQ's gain there, on both
        // channels of an interleaved stereo voice.
        let eq = MasterEq::new(&treyarch_mix(), RATE as f32);
        let mut voice = VoiceEq {
            version: 0,
            rate: RATE as f32,
            chan_count: 2,
            next_channel: 0,
            history: vec![[0.0; 4]; 2 * eq.bands.len()],
            eq: Some(eq.clone()),
        };
        let (mut peak_l, mut peak_r) = (0f32, 0f32);
        for i in 0..48_000 {
            let x = (std::f64::consts::TAU * 1000.0 * f64::from(i) / RATE).sin() as f32 * 0.25;
            let l = voice.filter(x);
            let r = voice.filter(x * 0.5);
            if i > 24_000 {
                peak_l = peak_l.max(l.abs());
                peak_r = peak_r.max(r.abs());
            }
        }
        let expect = eq.gain_at(1000.0, RATE) * 0.25;
        assert!((f64::from(peak_l) - expect).abs() < 0.005, "{peak_l} vs {expect}");
        assert!((f64::from(peak_r) - expect * 0.5).abs() < 0.005, "{peak_r} vs {expect}");
    }

    #[test]
    fn no_master_passes_every_sample_untouched() {
        let mut voice = VoiceEq {
            version: 0,
            rate: 48_000.0,
            chan_count: 2,
            next_channel: 0,
            eq: None,
            history: Vec::new(),
        };
        for x in [0.5f32, -0.25, 1.0, 0.0] {
            assert_eq!(voice.filter(x), x);
        }
    }

    #[test]
    fn presets_pick_their_masters_then_default() {
        let mut bass = treyarch_mix();
        bass.name = "bass_boost".into();
        let mut def = treyarch_mix();
        def.name = "default".into();
        let masters = vec![def, bass];
        assert_eq!(master_for_preset(&masters, 1).unwrap().name, "bass_boost");
        assert_eq!(master_for_preset(&masters, 4).unwrap().name, "default");
        assert!(master_for_preset(&[], 0).is_none());
    }

    #[test]
    fn treyarch_mix_response() {
        let eq = MasterEq::new(&treyarch_mix(), RATE as f32);
        let mut line = String::new();
        let mut max = f64::MIN;
        for f in [
            30.0, 60.0, 132.0, 250.0, 358.0, 700.0, 1000.0, 2000.0, 2772.5, 4000.0, 8000.0,
            16000.0,
        ] {
            let g = db(eq.gain_at(f, RATE));
            max = max.max(g);
            line.push_str(&format!("{f}Hz {g:+.1}dB  "));
        }
        println!("treyarch_mix: {line}max {max:+.1}dB");
        assert!(max < 12.0);
    }
}
