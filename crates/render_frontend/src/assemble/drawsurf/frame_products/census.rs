//! Opt-in diagnostic: how the colour list's reflection-probe indices are
//! spread over the draw kinds. Enabled with `BO2MP_PRODUCT_LOG`.

use std::collections::BTreeMap;

use bevy::prelude::*;

use super::super::retained::{RetainedDrawItem, RetainedDrawKind};
use super::super::tess::xmodel::XMODEL_OBJECT_ID_VIEWMODEL;
use super::{FrameProductKind, RenderFrameProducts};

/// Draw counts by probe index.
type Histogram = BTreeMap<u8, usize>;

/// One histogram per kind of geometry the census reports on.
#[derive(Default)]
struct Census {
    world: Histogram,
    smodel: Histogram,
    viewmodel: Histogram,
    xmodel: Histogram,
    /// Draws whose sort-key probe index differs from the probe bound to
    /// the draw.
    unbound: usize,
}

impl Census {
    fn of(draws: &[RetainedDrawItem]) -> Self {
        let mut census = Self::default();
        for draw in draws {
            let fields = dpvs_t6::unpack(dpvs_t6::GfxDrawSurf { packed: draw.key });
            let bucket = match draw.kind {
                RetainedDrawKind::World { .. } => &mut census.world,
                RetainedDrawKind::Smodel { .. } => &mut census.smodel,
                RetainedDrawKind::XModel { .. } if fields.object_id == XMODEL_OBJECT_ID_VIEWMODEL => {
                    &mut census.viewmodel
                }
                RetainedDrawKind::XModel { .. } => &mut census.xmodel,
                _ => continue,
            };
            let index = fields.reflection_probe_index;
            if draw.surface_samplers.reflection_probe.map(|id| id.0) != Some(index) {
                census.unbound += 1;
            }
            *bucket.entry(index).or_default() += 1;
        }
        census
    }

    fn line(&self) -> String {
        format!(
            "probe index census: world={} smodel={} fpv={} xmodel_other={} packed_ne_bound={} (zero names cube 0, not absence)",
            describe(&self.world),
            describe(&self.smodel),
            describe(&self.viewmodel),
            describe(&self.xmodel),
            self.unbound,
        )
    }
}

fn describe(hist: &Histogram) -> String {
    let total: usize = hist.values().sum();
    let zero = hist.get(&0).copied().unwrap_or(0);
    let body: Vec<String> = hist.iter().map(|(index, n)| format!("{index}:{n}")).collect();
    format!("n={total} zero={zero} {{{}}}", body.join(" "))
}

fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("BO2MP_PRODUCT_LOG").is_some())
}

/// Logs the census whenever it changes.
pub(crate) fn log_probe_index_census(
    frame: Res<RenderFrameProducts>,
    mut last: Local<Option<String>>,
) {
    if !enabled() {
        return;
    }
    let draws = &frame.product(FrameProductKind::Colour).ordered_draws;
    if draws.is_empty() {
        return;
    }
    let line = Census::of(draws).line();
    if last.as_ref() != Some(&line) {
        diag::warn!(World, "{line}");
        *last = Some(line);
    }
}

#[cfg(test)]
mod tests {
    use super::super::lanes::tests::row;
    use super::*;

    #[test]
    fn an_empty_histogram_reads_as_nothing() {
        assert_eq!(describe(&Histogram::new()), "n=0 zero=0 {}");
    }

    #[test]
    fn indices_are_listed_in_order_with_the_zero_count_called_out() {
        let hist: Histogram = [(0, 2), (3, 5), (1, 1)].into_iter().collect();
        assert_eq!(describe(&hist), "n=8 zero=2 {0:2 1:1 3:5}");
    }

    #[test]
    fn rows_without_a_bound_probe_are_counted_as_unbound() {
        // `row` builds world rows with no probe bound, and key 0 names probe 0.
        let census = Census::of(&[row(0, 1), row(0, 2)]);
        assert_eq!(census.unbound, 2);
        assert_eq!(census.world.get(&0), Some(&2));
    }
}
