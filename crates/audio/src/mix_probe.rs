//! A silent voice that times the sound device's callbacks from inside the mix.
//! The device asks for a buffer at a time and the mixer fills it in one go, so
//! the probe's samples arrive in bursts: a burst's length is the time the mixer
//! spent on that callback, and the gap between bursts is how long the device
//! waited. A burst that takes longer than the sound it makes, or a long wait,
//! is a buffer the device ran out of (static).

use std::num::NonZero;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use bevy::audio::{AddAudioSource, AudioPlayer, ChannelCount, Decodable, PlaybackSettings, SampleRate, Source};
use bevy::prelude::*;

const PROBE_RATE: u32 = 48_000;
const PROBE_LANES: u16 = 2;
/// The clock is read once per this many samples.
const STAMP_EVERY: u32 = 8;
/// A pause this long between samples ends a callback.
const BURST_BREAK: Duration = Duration::from_millis(1);
/// The device waited this long between callbacks: its buffer ran dry.
const LATE_WAIT: Duration = Duration::from_millis(25);

/// Callbacks seen since the last report.
pub(crate) static CALLBACKS: AtomicU64 = AtomicU64::new(0);
/// Their mixer time, summed and the longest, in microseconds.
pub(crate) static WORK_SUM_US: AtomicU64 = AtomicU64::new(0);
pub(crate) static WORK_MAX_US: AtomicU64 = AtomicU64::new(0);
/// The longest wait from one callback's start to the next, in microseconds.
pub(crate) static WAIT_MAX_US: AtomicU64 = AtomicU64::new(0);
/// Callbacks that started LATE_WAIT or more after the one before.
pub(crate) static LATE: AtomicU64 = AtomicU64::new(0);
/// Callbacks whose mixing took longer than the sound they produced.
pub(crate) static OVERRUN: AtomicU64 = AtomicU64::new(0);

#[derive(Asset, TypePath, Clone)]
pub struct MixProbe;

pub struct MixProbeDecoder {
    count: u32,
    last_stamp: Option<Instant>,
    burst_start: Option<Instant>,
    burst_samples: u32,
}

impl MixProbeDecoder {
    fn stamp(&mut self, now: Instant) {
        let Some(last) = self.last_stamp.replace(now) else {
            self.burst_start = Some(now);
            return;
        };
        if now.duration_since(last) < BURST_BREAK {
            return;
        }
        // `last` was the end of the previous callback; `now` starts a new one.
        if let Some(start) = self.burst_start.replace(now) {
            let work = last.duration_since(start);
            let wait = now.duration_since(start);
            let sound = Duration::from_secs_f64(
                f64::from(self.burst_samples) / f64::from(PROBE_RATE * u32::from(PROBE_LANES)),
            );
            CALLBACKS.fetch_add(1, Ordering::Relaxed);
            WORK_SUM_US.fetch_add(work.as_micros() as u64, Ordering::Relaxed);
            WORK_MAX_US.fetch_max(work.as_micros() as u64, Ordering::Relaxed);
            WAIT_MAX_US.fetch_max(wait.as_micros() as u64, Ordering::Relaxed);
            LATE.fetch_add(u64::from(wait >= LATE_WAIT), Ordering::Relaxed);
            OVERRUN.fetch_add(u64::from(work > sound), Ordering::Relaxed);
        }
        self.burst_samples = 0;
    }
}

impl Iterator for MixProbeDecoder {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        self.burst_samples += 1;
        self.count = self.count.wrapping_add(1);
        if self.count % STAMP_EVERY == 0 {
            self.stamp(Instant::now());
        }
        Some(0.0)
    }
}

impl Source for MixProbeDecoder {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> ChannelCount {
        NonZero::new(PROBE_LANES).unwrap_or(NonZero::<u16>::MIN)
    }

    fn sample_rate(&self) -> SampleRate {
        NonZero::new(PROBE_RATE).unwrap_or(NonZero::<u32>::MIN)
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

impl Decodable for MixProbe {
    type Decoder = MixProbeDecoder;

    fn decoder(&self) -> Self::Decoder {
        MixProbeDecoder {
            count: 0,
            last_stamp: None,
            burst_start: None,
            burst_samples: 0,
        }
    }
}

pub(crate) fn install(app: &mut App) {
    app.add_audio_source::<MixProbe>();
    app.add_systems(Startup, start_probe);
}

fn start_probe(mut probes: ResMut<Assets<MixProbe>>, mut commands: Commands) {
    commands.spawn((
        Name::new("mix probe"),
        AudioPlayer(probes.add(MixProbe)),
        PlaybackSettings::ONCE,
    ));
}

/// The callbacks since the last call: count, mean and longest mixer time (ms),
/// longest wait (ms), late callbacks, overrun callbacks. Resets the tallies.
pub(crate) fn take_report() -> (u64, f64, f64, f64, u64, u64) {
    let n = CALLBACKS.swap(0, Ordering::Relaxed);
    let sum = WORK_SUM_US.swap(0, Ordering::Relaxed);
    let mean = if n == 0 { 0.0 } else { sum as f64 / n as f64 / 1000.0 };
    (
        n,
        mean,
        WORK_MAX_US.swap(0, Ordering::Relaxed) as f64 / 1000.0,
        WAIT_MAX_US.swap(0, Ordering::Relaxed) as f64 / 1000.0,
        LATE.swap(0, Ordering::Relaxed),
        OVERRUN.swap(0, Ordering::Relaxed),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reset() {
        let _ = take_report();
    }

    #[test]
    fn bursts_are_counted_and_timed() {
        reset();
        let mut probe = MixProbe.decoder();
        let t0 = Instant::now();
        // Two callbacks of 960 samples (10 ms of sound), 10 ms apart, then a
        // third 40 ms later (late), each taking 2 ms to mix.
        let at = |probe: &mut MixProbeDecoder, start_ms: u64| {
            for i in 0..960 / STAMP_EVERY {
                probe.burst_samples += STAMP_EVERY;
                let micros = start_ms * 1000 + u64::from(i) * 2000 / u64::from(960 / STAMP_EVERY - 1);
                probe.stamp(t0 + Duration::from_micros(micros));
            }
        };
        at(&mut probe, 0);
        at(&mut probe, 10);
        at(&mut probe, 50);
        at(&mut probe, 60);
        let (n, mean, max, wait, late, over) = take_report();
        assert_eq!(n, 3);
        assert!((1.9..2.1).contains(&mean) && (1.9..2.1).contains(&max), "{mean} {max}");
        assert!((39.0..41.0).contains(&wait), "{wait}");
        assert_eq!((late, over), (1, 0));
    }
}
