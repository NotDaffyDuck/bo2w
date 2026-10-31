use std::path::PathBuf;

/// One screenshot someone asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScreenshotOrder {
    pub path: PathBuf,
    /// Quit once this request and every one before it is on disk.
    pub quit_when_shot_taken: bool,
}

impl ScreenshotOrder {
    /// The request named by `BO2MP_SCREENSHOT`, if that is set.
    pub fn from_env() -> Option<ScreenshotOrder> {
        let path = PathBuf::from(std::env::var_os("BO2MP_SCREENSHOT")?);
        Some(ScreenshotOrder {
            path,
            quit_when_shot_taken: false,
        })
    }
}
