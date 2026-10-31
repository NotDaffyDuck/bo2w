//! When a frame is fit to be photographed.

use frame::ShellScreen;

/// Frames the picture has to stay up before it is taken.
pub(super) const SETTLE_FRAMES: u32 = 30;

/// Facts about the frame that just ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FrameFacts {
    pub on_screen: ShellScreen,
    pub loading_screen_shown: bool,
    pub level_is_up: bool,
    pub batches_sent: u32,
    pub first_group_world_batches: u32,
}

/// Why a capture has to keep waiting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Hold {
    GameSessionLoading,
    LoadingScreenActive,
    MapGeometryNotQueued,
    WarmingUp { frames_left: u32 },
}

impl Hold {
    pub(super) fn label(self) -> String {
        match self {
            Hold::GameSessionLoading => "match still loading".to_owned(),
            Hold::LoadingScreenActive => "loading overlay still drawn".to_owned(),
            Hold::MapGeometryNotQueued => "world given to the renderer, nothing submitted yet".to_owned(),
            Hold::WarmingUp { frames_left } => format!("settling ({frames_left} frames left)"),
        }
    }
}

impl FrameFacts {
    fn world_submitted(self) -> bool {
        self.batches_sent > 0 || self.first_group_world_batches > 0
    }

    /// What stops the content from being on screen, if anything does.
    fn blocker(self) -> Option<Hold> {
        // Test aid: BO2MP_CAPTURE_DURING_LOAD=1 takes the shot as the screen is (BO2's loading screen).
        if std::env::var_os("BO2MP_CAPTURE_DURING_LOAD").is_some() {
            return None;
        }
        if self.on_screen == ShellScreen::Loading {
            Some(Hold::GameSessionLoading)
        } else if self.loading_screen_shown {
            Some(Hold::LoadingScreenActive)
        } else if self.level_is_up && !self.world_submitted() {
            Some(Hold::MapGeometryNotQueued)
        } else {
            None
        }
    }

    pub(super) fn content_up(self) -> bool {
        self.blocker().is_none()
    }

    /// Whether to keep waiting, given how many settled frames have gone by.
    pub(super) fn hold(self, quiet_frame_count: u32) -> Option<Hold> {
        if let Some(blocker) = self.blocker() {
            return Some(blocker);
        }
        let frames_left = SETTLE_FRAMES.checked_sub(quiet_frame_count)?;
        (frames_left > 0).then_some(Hold::WarmingUp { frames_left })
    }
}
