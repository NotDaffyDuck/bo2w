//! GPU pass timings, read back from Bevy's diagnostics store.
//!
//! Bevy's render-diagnostics plugin resolves GPU timestamps a few frames late and hands
//! them over in batches, so what the store holds is the newest delivered batch and not
//! this frame's work.

use std::time::Instant;

use bevy::diagnostic::{DiagnosticPath, DiagnosticsStore};

/// Names the passes give their GPU time spans.
const SPAN_COLOUR: &str = "bo2_colour";
const SPAN_EMISSIVE: &str = "bo2_emissive";
const SPAN_SUN: &str = "bo2_sun";
const SPAN_SPOT: &str = "bo2_spot";
const SPAN_FLOATZ: &str = "bo2_floatz";
const SPAN_POSTFX: &str = "bo2_postfx";

/// The last path component of every GPU timing in the store.
const GPU_TIMING: &str = "elapsed_gpu";

/// GPU milliseconds per pass; a pass with no measurement in the batch stays `None`.
#[derive(Default)]
pub(super) struct PassTimes {
    pub colour: Option<f32>,
    pub sun: Option<f32>,
    pub spot: Option<f32>,
    pub floatz: Option<f32>,
    pub postfx: Option<f32>,
}

impl PassTimes {
    fn add(&mut self, span: &str, ms: f64) {
        let total = match span {
            // The emissive pass is drawn into the same target as colour.
            SPAN_COLOUR | SPAN_EMISSIVE => &mut self.colour,
            SPAN_SUN => &mut self.sun,
            SPAN_SPOT => &mut self.spot,
            SPAN_FLOATZ => &mut self.floatz,
            SPAN_POSTFX => &mut self.postfx,
            _ => return,
        };
        *total = Some(total.unwrap_or(0.0) + ms as f32);
    }

    pub fn counters(&self) -> [(perf::Counter, Option<f32>); 5] {
        [
            (perf::Counter::RenderGpuColourMs, self.colour),
            (perf::Counter::RenderGpuSunMs, self.sun),
            (perf::Counter::RenderGpuSpotMs, self.spot),
            (perf::Counter::RenderGpuFloatzMs, self.floatz),
            (perf::Counter::RenderGpuPostfxMs, self.postfx),
        ]
    }
}

/// The newest delivered batch of GPU timings, summed.
pub(super) struct GpuBatch {
    /// Every timing in the batch added together.
    pub total_ms: Option<f32>,
    pub passes: PassTimes,
    /// The latest measurement time in the batch. It identifies a delivery, not a source
    /// frame: equal durations are never compared, and the same delivery seen on the next
    /// tick is recognised by this stamp and not reported again.
    pub delivered: Option<Instant>,
}

pub(super) fn latest_batch(store: &DiagnosticsStore) -> GpuBatch {
    let timings = || {
        store
            .iter()
            .filter(|d| d.path().components().last() == Some(GPU_TIMING))
            .filter_map(|d| Some((d.path(), d.measurement()?)))
    };
    let delivered = timings().map(|(_, m)| m.time).max();

    let mut batch = GpuBatch {
        total_ms: None,
        passes: PassTimes::default(),
        delivered,
    };
    let mut total = 0.0f64;
    for (path, m) in timings().filter(|(_, m)| Some(m.time) == delivered) {
        total += m.value;
        batch.total_ms = Some(total as f32);
        let parts: Vec<&str> = path.components().collect();
        if let Some(span) = parts.len().checked_sub(2).map(|i| parts[i]) {
            batch.passes.add(span, m.value);
        }
    }
    batch
}

/// The main opaque pass's own GPU time, whichever batch it came from.
pub(super) fn opaque_pass_ms(store: &DiagnosticsStore) -> Option<f32> {
    let path = DiagnosticPath::from_components(["render", "main_opaque_pass_3d", GPU_TIMING]);
    store.get_measurement(&path).map(|m| m.value as f32)
}

/// The Black Ops II world pass's GPU time, for when no named pass reported one.
pub(super) fn world_pass_ms(store: &DiagnosticsStore) -> Option<f32> {
    store
        .iter()
        .find(|d| {
            let parts: Vec<&str> = d.path().components().collect();
            parts.contains(&"bo2zm_world") && parts.last() == Some(&GPU_TIMING)
        })
        .and_then(|d| d.measurement())
        .map(|m| m.value as f32)
}
