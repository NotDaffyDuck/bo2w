use serde::{Serialize, Serializer};

/// A measurement that is either a number or known to be missing. The missing
/// case is written out as text, so a reader cannot take it for zero.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MetricValue {
    /// Nothing measures the GPU yet, so only `Unavailable` is produced.
    #[allow(dead_code)]
    Finite(f64),
    Unavailable,
}

impl Serialize for MetricValue {
    fn serialize<S: Serializer>(&self, out: S) -> Result<S::Ok, S::Error> {
        match *self {
            MetricValue::Finite(value) => out.serialize_f64(value),
            MetricValue::Unavailable => out.serialize_str("Unavailable"),
        }
    }
}

/// One frame of the sampling window, one line of `acceptance.jsonl`.
#[derive(Clone, Debug, Serialize)]
pub struct AcceptanceSample {
    pub i: u32,
    pub wall_frame_ms: f64,
    pub visible_cells: u32,
    pub bsp_visible_surfaces: u32,
    pub bsp_admitted_surfaces: u32,
    pub bsp_run_n: u32,
    pub bsp_input_gap_n: u32,
    pub bsp_ranges_unavailable: u32,
    pub surfaces: u32,
    pub triangles: u32,
    pub rebinds: u32,
    pub submitted_batches: u32,
    pub rewritten_index_bytes: u64,
    pub visibility_changes: u32,
    pub gpu_frame_ms: MetricValue,
    pub draw_calls: MetricValue,
    pub process_allocations: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Percentiles {
    pub p50: f64,
    pub p95: f64,
    pub max: f64,
}

impl Percentiles {
    /// Nearest-rank percentiles of a series of finite values.
    pub fn of(values: &[f64]) -> Result<Percentiles, String> {
        if values.is_empty() {
            return Err("percentiles: empty series".into());
        }
        if values.iter().any(|value| !value.is_finite()) {
            return Err("percentiles: non-finite value".into());
        }
        let mut ascending = values.to_vec();
        ascending.sort_by(f64::total_cmp);
        let count = ascending.len();
        let at_percent = |percent: usize| {
            let rank = (percent * count).div_ceil(100).max(1);
            ascending[rank.min(count) - 1]
        };
        Ok(Percentiles {
            p50: at_percent(50),
            p95: at_percent(95),
            max: ascending[count - 1],
        })
    }
}
