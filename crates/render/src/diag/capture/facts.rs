//! When a frame is fit to be photographed.

use frame::AppScreen;

/// Frames the picture has to stay up before it is taken.
pub(super) const SETTLE_FRAMES: u32 = 30;

/// Facts about the frame that just ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FrameFacts {
    pub screen: AppScreen,
    pub loading_overlay: bool,
    pub has_world: bool,
    pub submitted_batches: u32,
    pub g0_world: u32,
}

/// Why a capture has to keep waiting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Hold {
    MatchLoading,
    LoadingOverlayUp,
    WorldNotSubmitted,
    Settling { frames_left: u32 },
}

impl Hold {
    pub(super) fn label(self) -> String {
        match self {
            Hold::MatchLoading => "match still loading".to_owned(),
            Hold::LoadingOverlayUp => "loading overlay still drawn".to_owned(),
            Hold::WorldNotSubmitted => "world handed over, no batch submitted".to_owned(),
            Hold::Settling { frames_left } => format!("settling ({frames_left} frames left)"),
        }
    }
}

impl FrameFacts {
    fn world_submitted(self) -> bool {
        self.submitted_batches > 0 || self.g0_world > 0
    }

    /// What stops the content from being on screen, if anything does.
    fn blocker(self) -> Option<Hold> {
        if self.screen == AppScreen::Loading {
            Some(Hold::MatchLoading)
        } else if self.loading_overlay {
            Some(Hold::LoadingOverlayUp)
        } else if self.has_world && !self.world_submitted() {
            Some(Hold::WorldNotSubmitted)
        } else {
            None
        }
    }

    pub(super) fn content_up(self) -> bool {
        self.blocker().is_none()
    }

    /// Whether to keep waiting, given how many settled frames have gone by.
    pub(super) fn hold(self, settled_frames: u32) -> Option<Hold> {
        if let Some(blocker) = self.blocker() {
            return Some(blocker);
        }
        let frames_left = SETTLE_FRAMES.checked_sub(settled_frames)?;
        (frames_left > 0).then_some(Hold::Settling { frames_left })
    }
}
