use std::path::PathBuf;

/// One screenshot someone asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureRequest {
    pub path: PathBuf,
    /// Quit once this request and every one before it is on disk.
    pub exit_after_capture: bool,
}

impl CaptureRequest {
    /// The request named by `BO2MP_SCREENSHOT`, if that is set.
    pub fn from_env() -> Option<CaptureRequest> {
        let path = PathBuf::from(std::env::var_os("BO2MP_SCREENSHOT")?);
        Some(CaptureRequest {
            path,
            exit_after_capture: false,
        })
    }
}
