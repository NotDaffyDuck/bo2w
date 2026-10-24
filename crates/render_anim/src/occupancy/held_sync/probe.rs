//! Showing the first-person status in the launch report.

use bevy::prelude::*;
use frame::LaunchReport;

use crate::occupancy::fpv_present::FpvStatusGap;

/// When the status changes, write its label into the launch report so the
/// diagnostics probe can print why the first-person view looks the way it does.
///
/// A status of "none yet", or a run without a launch report, does nothing.
pub fn sync_fpv_status_to_probe(
    status: Res<FpvStatusGap>,
    mut report: Option<ResMut<LaunchReport>>,
) {
    if !status.is_changed() {
        return;
    }
    let (Some(state), Some(report)) = (status.0.as_ref(), report.as_mut()) else {
        return;
    };
    report.sim_gap = state.label();
}
