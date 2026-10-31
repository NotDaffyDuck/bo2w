//! Showing the first-person status in the launch report.

use bevy::prelude::*;
use frame::StartupReport;

use crate::occupancy::gun_present::GunStatusShortfall;

/// When the status changes, write its label into the launch report so the
/// diagnostics probe can print why the first-person view looks the way it does.
///
/// A status of "none yet", or a run without a launch report, does nothing.
pub fn mirror_fpv_status_probe(
    status_in: Res<GunStatusShortfall>,
    mut report: Option<ResMut<StartupReport>>,
) {
    if !status_in.is_changed() {
        return;
    }
    let (Some(state), Some(report)) = (status_in.0.as_ref(), report.as_mut()) else {
        return;
    };
    report.stepping_lag = state.summary();
}
