//! Opt-in diagnostic: how the colour list's reflection-probe indices are
//! spread over the draw kinds. Enabled with `BO2MP_PRODUCT_LOG`.

use std::collections::BTreeMap;
use std::fmt;

use bevy::prelude::*;

use super::super::kept_lanes::{HeldDrawItem, HeldDrawKind};
use super::super::mesh_build::figure_mesh::MODEL_OBJECT_ID_FIRST_PERSON_WEAPON;
use super::FrameOutputKind;
use super::FrameRenderOutputs;

/// Draw counts by probe index.

/// One histogram per kind of geometry the census reports on.
#[derive(Default)]
struct Census {
    world: BTreeMap<u8, usize>,
    smodel: BTreeMap<u8, usize>,
    viewmodel: BTreeMap<u8, usize>,
    xmodel: BTreeMap<u8, usize>,
    /// Draws whose sort-key probe index differs from the probe bound to
    /// the draw.
    unbound: usize,
}

impl Census {
    fn of(draws: &[HeldDrawItem]) -> Self {
        let mut tally = Self::default();
        for draw_item in draws {
            let fields = dpvs_t6::unpack(dpvs_t6::SortedDraw { packed: draw_item.key });
            let bucket = match draw_item.kind {
                HeldDrawKind::World { .. } => &mut tally.world,
                HeldDrawKind::Smodel { .. } => &mut tally.smodel,
                HeldDrawKind::XModel { .. } if fields.object_tag == MODEL_OBJECT_ID_FIRST_PERSON_WEAPON => {
                    &mut tally.viewmodel
                }
                HeldDrawKind::XModel { .. } => &mut tally.xmodel,
                _ => continue,
            };
            let index = fields.env_probe_index;
            if draw_item.facet_samplers.reflection_probe.map(|id| id.0) != Some(index) {
                tally.unbound += 1;
            }
            *bucket.entry(index).or_default() += 1;
        }
        tally
    }

    fn line(&self) -> String {
        format!(
            "env probe census: world={} smodel={} fpv={} xmodel_other={} packed_ne_bound={} (a 0 means cube 0, not a missing probe)",
            Spread(&self.world),
            Spread(&self.smodel),
            Spread(&self.viewmodel),
            Spread(&self.xmodel),
            self.unbound,
        )
    }
}

/// One draw kind's probe indices as a read-out: how many rows, how many on
/// cube 0, and the count at each index in order.
struct Spread<'a>(&'a BTreeMap<u8, usize>);

impl fmt::Display for Spread<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let total: usize = self.0.values().sum();
        let on_cube_zero = self.0.get(&0).copied().unwrap_or(0);
        write!(f, "n={total} zero={on_cube_zero} {{")?;
        for (place, (index, count)) in self.0.iter().enumerate() {
            let gap = if place == 0 { "" } else { " " };
            write!(f, "{gap}{index}:{count}")?;
        }
        f.write_str("}")
    }
}

fn census_logging_on() -> bool {
    static CENSUS_ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *CENSUS_ON.get_or_init(|| std::env::var_os("BO2MP_PRODUCT_LOG").is_some())
}

/// Logs the census whenever it changes.
pub(crate) fn log_env_probe_index_census(
    published: Res<FrameRenderOutputs>,
    mut last: Local<Option<String>>,
) {
    if !census_logging_on() {
        return;
    }
    let sequence = &published.product(FrameOutputKind::Colour).draw_sequence;
    if sequence.is_empty() {
        return;
    }
    let census = Census::of(sequence).line();
    if last.as_ref() != Some(&census) {
        diag::warn!(World, "{census}");
        *last = Some(census);
    }
}

#[cfg(test)]
mod tests {
    use super::super::lane_set::tests::row;
    use super::*;

    #[test]
    fn an_empty_histogram_reads_as_nothing() {
        assert_eq!(Spread(&BTreeMap::new()).to_string(), "n=0 zero=0 {}");
    }

    #[test]
    fn indices_are_listed_in_order_with_the_zero_count_called_out() {
        let hist: BTreeMap<u8, usize> = [(0, 2), (3, 5), (1, 1)].into_iter().collect();
        assert_eq!(Spread(&hist).to_string(), "n=8 zero=2 {0:2 1:1 3:5}");
    }

    #[test]
    fn rows_without_a_bound_probe_are_counted_as_unbound() {
        // `row` builds world rows with no probe bound, and key 0 names probe 0.
        let census = Census::of(&[row(0, 1), row(0, 2)]);
        assert_eq!(census.unbound, 2);
        assert_eq!(census.world.get(&0), Some(&2));
    }
}
