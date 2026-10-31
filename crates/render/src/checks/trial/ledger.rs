//! The files an acceptance run leaves behind: the per-frame trace and the
//! summary that sits above it.

use std::fs;
use std::io::{BufWriter, Write};

use serde::Serialize;

use super::metrics::{AcceptanceReading, MeasuredReading, QuantileSummary};
use super::{
    TRIAL_RESOLUTION_ROWS, TRIAL_MEASURED_FRAME_COUNT, TRIAL_WARMUP_FRAME_COUNT, TRIAL_RESOLUTION_COLUMNS,
    AcceptanceTrial,
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
    #[serde(rename = "present_mode")]
    swap_policy: &'static str,
    warm_frames: u32,
    sample_frames: u32,
    #[serde(rename = "samples_written")]
    readings_written: u32,
    wall_frame_ms: QuantileSummary,
    gpu_frame_ms: MeasuredReading,
    draw_calls: MeasuredReading,
    png: &'static str,
    trace: &'static str,
    process_allocations_end: u64,
}

/// Writes `acceptance.jsonl` and `summary.json` into the run's directory.
pub(super) fn write_ledger(run: &AcceptanceTrial, readings: &[AcceptanceReading]) -> Result<(), String> {
    if readings.len() != TRIAL_MEASURED_FRAME_COUNT as usize {
        return Err(format!(
            "need {} readings, got {}",
            TRIAL_MEASURED_FRAME_COUNT,
            readings.len()
        ));
    }

    let trace = fs::File::create(run.output_folder.join(TRACE_FILE))
        .map_err(|error| format!("open jsonl: {error}"))?;
    let mut trace = BufWriter::new(trace);
    for one in readings {
        serde_json::to_writer(&mut trace, one).map_err(|error| format!("jsonl: {error}"))?;
        trace
            .write_all(b"\n")
            .map_err(|error| format!("jsonl write: {error}"))?;
    }
    trace
        .flush()
        .map_err(|error| format!("jsonl flush: {error}"))?;

    let wall: Vec<f64> = readings.iter().map(|one| one.frame_wall_ms).collect();
    let summary = Summary {
        format: FORMAT,
        map: &run.map,
        width: TRIAL_RESOLUTION_COLUMNS,
        height: TRIAL_RESOLUTION_ROWS,
        swap_policy: "AutoNoVsync",
        warm_frames: TRIAL_WARMUP_FRAME_COUNT,
        sample_frames: TRIAL_MEASURED_FRAME_COUNT,
        readings_written: readings.len() as u32,
        wall_frame_ms: QuantileSummary::of(&wall)?,
        gpu_frame_ms: MeasuredReading::Missing,
        draw_calls: MeasuredReading::Missing,
        png: PICTURE_FILE,
        trace: TRACE_FILE,
        process_allocations_end: readings.last().map_or(0, |last| last.alloc_count),
    };
    let mut body =
        serde_json::to_string_pretty(&summary).map_err(|error| format!("summary: {error}"))?;
    body.push('\n');
    fs::write(run.output_folder.join(SUMMARY_FILE), body)
        .map_err(|error| format!("summary write: {error}"))
}
