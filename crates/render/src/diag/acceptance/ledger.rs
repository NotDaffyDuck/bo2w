//! The files an acceptance run leaves behind: the per-frame trace and the
//! summary that sits above it.

use std::fs;
use std::io::{BufWriter, Write};

use serde::Serialize;

use super::metrics::{AcceptanceSample, MetricValue, Percentiles};
use super::{
    ACCEPTANCE_HEIGHT, ACCEPTANCE_SAMPLE_FRAMES, ACCEPTANCE_WARM_FRAMES, ACCEPTANCE_WIDTH,
    AcceptanceRun,
};

const FORMAT: &str = "bo2mp-render-acceptance-v2";
pub(super) const TRACE_FILE: &str = "acceptance.jsonl";
pub(super) const SUMMARY_FILE: &str = "summary.json";
pub(super) const PICTURE_FILE: &str = "acceptance.png";
pub(super) const FAILURE_FILE: &str = "FAILURE.txt";

#[derive(Serialize)]
struct Summary<'a> {
    format: &'static str,
    map: &'a str,
    width: u32,
    height: u32,
    present_mode: &'static str,
    warm_frames: u32,
    sample_frames: u32,
    samples_written: u32,
    wall_frame_ms: Percentiles,
    gpu_frame_ms: MetricValue,
    draw_calls: MetricValue,
    png: &'static str,
    trace: &'static str,
    process_allocations_end: u64,
}

/// Writes `acceptance.jsonl` and `summary.json` into the run's directory.
pub(super) fn write_ledger(run: &AcceptanceRun, samples: &[AcceptanceSample]) -> Result<(), String> {
    if samples.len() != ACCEPTANCE_SAMPLE_FRAMES as usize {
        return Err(format!(
            "expected {} samples, have {}",
            ACCEPTANCE_SAMPLE_FRAMES,
            samples.len()
        ));
    }

    let trace = fs::File::create(run.artifact_dir.join(TRACE_FILE))
        .map_err(|error| format!("open jsonl: {error}"))?;
    let mut trace = BufWriter::new(trace);
    for sample in samples {
        serde_json::to_writer(&mut trace, sample).map_err(|error| format!("jsonl: {error}"))?;
        trace
            .write_all(b"\n")
            .map_err(|error| format!("jsonl write: {error}"))?;
    }
    trace
        .flush()
        .map_err(|error| format!("jsonl flush: {error}"))?;

    let wall: Vec<f64> = samples.iter().map(|sample| sample.wall_frame_ms).collect();
    let summary = Summary {
        format: FORMAT,
        map: &run.map,
        width: ACCEPTANCE_WIDTH,
        height: ACCEPTANCE_HEIGHT,
        present_mode: "AutoNoVsync",
        warm_frames: ACCEPTANCE_WARM_FRAMES,
        sample_frames: ACCEPTANCE_SAMPLE_FRAMES,
        samples_written: samples.len() as u32,
        wall_frame_ms: Percentiles::of(&wall)?,
        gpu_frame_ms: MetricValue::Unavailable,
        draw_calls: MetricValue::Unavailable,
        png: PICTURE_FILE,
        trace: TRACE_FILE,
        process_allocations_end: samples.last().map_or(0, |sample| sample.process_allocations),
    };
    let mut body =
        serde_json::to_string_pretty(&summary).map_err(|error| format!("summary: {error}"))?;
    body.push('\n');
    fs::write(run.artifact_dir.join(SUMMARY_FILE), body)
        .map_err(|error| format!("summary write: {error}"))
}
