use serde::{Serialize, Serializer};

/// A measurement that is either a number or known to be missing. The missing
/// case is written out as text, so a reader cannot take it for zero.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MeasuredReading {
    /// Kept for when the GPU timings get measured; nothing produces it yet.
    #[allow(dead_code)]
    Value(f64),
    /// Written to the ledger as the text `"Unavailable"`.
    Missing,
}

impl Serialize for MeasuredReading {
    fn serialize<S: Serializer>(&self, out: S) -> Result<S::Ok, S::Error> {
        if let MeasuredReading::Value(number) = *self { out.serialize_f64(number) } else { out.serialize_str("Unavailable") }
    }
}

/// One frame of the sampling window, one line of `acceptance.jsonl`.
#[derive(Clone, Debug, Serialize)]
pub struct AcceptanceReading {
    pub i: u32,
    #[serde(rename = "wall_frame_ms")]
    pub frame_wall_ms: f64,
    #[serde(rename = "visible_cells")]
    pub cells_seen: u32,
    #[serde(rename = "bsp_visible_surfaces")]
    pub bsp_surfaces_seen: u32,
    #[serde(rename = "bsp_admitted_surfaces")]
    pub bsp_surfaces_let_in: u32,
    #[serde(rename = "bsp_run_n")]
    pub bsp_runs: u32,
    #[serde(rename = "bsp_input_gap_n")]
    pub bsp_gaps: u32,
    #[serde(rename = "bsp_ranges_unavailable")]
    pub bsp_no_ranges: u32,
    #[serde(rename = "surfaces")]
    pub surfaces_drawn: u32,
    #[serde(rename = "triangles")]
    pub triangles_drawn: u32,
    #[serde(rename = "rebinds")]
    pub rebind_count: u32,
    #[serde(rename = "submitted_batches")]
    pub batches_sent: u32,
    #[serde(rename = "rewritten_index_bytes")]
    pub index_bytes_rewritten: u64,
    #[serde(rename = "visibility_changes")]
    pub vis_flips: u32,
    pub gpu_frame_ms: MeasuredReading,
    pub draw_calls: MeasuredReading,
    #[serde(rename = "process_allocations")]
    pub alloc_count: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct QuantileSummary {
    #[serde(rename = "p50")]
    pub median: f64,
    #[serde(rename = "p95")]
    pub upper: f64,
    pub max: f64,
}

impl QuantileSummary {
    /// Nearest-rank percentiles of a series of finite values.
    pub fn of(values: &[f64]) -> Result<QuantileSummary, String> {
        let problem = match values {
            [] => Some("empty series"),
            _ if !values.iter().all(|value| value.is_finite()) => Some("non-finite value"),
            _ => None,
        };
        if let Some(problem) = problem {
            return Err(format!("percentiles: {problem}"));
        }
        let mut ascending = values.to_vec();
        ascending.sort_by(f64::total_cmp);
        let count = ascending.len();
        let at_percent = |percent: usize| {
            let rank = (percent * count).div_ceil(100).max(1);
            ascending[rank.min(count) - 1]
        };
        Ok(QuantileSummary {
            median: at_percent(50),
            upper: at_percent(95),
            max: ascending[count - 1],
        })
    }
}
