use std::path::PathBuf;

use bevy::prelude::*;

use super::RENDER_CHECK_ENV;

/// Where an acceptance run puts its files, and the map it was asked to measure.
#[derive(Resource, Clone, Debug)]
pub struct AcceptanceTrial {
    pub output_folder: PathBuf,
    pub map: String,
}

impl AcceptanceTrial {
    /// The run named by `BO2MP_RENDER_ACCEPTANCE`, if it is set.
    pub fn from_env(level: impl Into<String>) -> Option<Self> {
        let output_folder = PathBuf::from(std::env::var_os(RENDER_CHECK_ENV)?);
        Some(Self {
            output_folder,
            map: level.into(),
        })
    }
}
