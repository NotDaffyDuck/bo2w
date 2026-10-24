use std::path::PathBuf;

use bevy::prelude::*;

use super::ACCEPTANCE_ENV;

/// Where an acceptance run puts its files, and the map it was asked to measure.
#[derive(Resource, Clone, Debug)]
pub struct AcceptanceRun {
    pub artifact_dir: PathBuf,
    pub map: String,
}

impl AcceptanceRun {
    /// The run named by `BO2MP_RENDER_ACCEPTANCE`, if it is set.
    pub fn from_env(map: impl Into<String>) -> Option<Self> {
        let artifact_dir = PathBuf::from(std::env::var_os(ACCEPTANCE_ENV)?);
        Some(Self {
            artifact_dir,
            map: map.into(),
        })
    }
}
