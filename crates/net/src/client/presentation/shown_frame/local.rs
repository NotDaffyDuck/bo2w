use bevy::prelude::Resource;
use sim::ClientId;

/// Which way the local player's state was chosen this frame, for the census output.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct OwnViewerTally {
    pub picked_view: Option<&'static str>,
    pub frame_gap_millis: Option<i32>,
    pub forecast_step_gap: Option<i32>,
}

/// Where the gun and the crosshair sit relative to the view this frame.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct ViewweaponAim {
    pub live: bool,
    pub weapon_tilt_angle: f32,
    pub weapon_swing_angle: f32,
    pub crosshair_x: f32,
    pub crosshair_y: f32,
    pub from_combined_direction: bool,
}

/// The client whose view is presented; client 0 until told otherwise.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct HostViewClient(pub ClientId);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_presented_client_defaults_to_zero() {
        assert_eq!(HostViewClient::default().0, ClientId(0));
    }
}
