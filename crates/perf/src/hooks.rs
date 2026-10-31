//! The named spans and counters. A new call site adds its name here.

/// A stretch of work inside a frame: opened with [`Span::begin`] and closed
/// with [`Span::end`], or held open by the guard [`Span::enter`] returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Span {
    TickPhaseAdvance,
    TickPhaseHousekeeping,
    TickPhaseSpread,
    TickPhaseCollect,
    TickPhaseIntake,
    TickPhaseCapture,
    TickPhaseStep,
    FrameMetricsMillis,
    FrameEffectsMillis,
    FrameTicksMillis,
    FrameAfterRefreshMillis,
    FrameForecastMillis,
    FrameShowMillis,
    FrameBeforeRefreshMillis,
    FrameRefreshMillis,
    FrameClockMillis,
    ServerEffectsShowCpuMillis,
    ServerEffectsRefreshCpuMillis,
    ServerAfterRunMillis,
    ServerAfterRegenMillis,
    HostStaticSunFxMs,
    HostStaticSunMs,
    DrawHideCpuMillis,
    RenderDispatchWorldMs,
    RenderExtractBodyMs,
    RenderReceiveWorldMs,
    DrawRenderMs,
    DrawThreadMs,
    TocInput,
    TocLoad,
    TocIntake,
    TickPhaseSettle,
    TocSend,
    TocUi,
}

impl Span {
    #[inline]
    pub fn begin(self) {}

    #[inline]
    pub fn end(self) {}

    /// Open the span until the returned guard is dropped.
    #[inline]
    pub fn enter(self) -> TimedStretchCloser {
        self.begin();
        TimedStretchCloser(self)
    }
}

/// Closes its span when dropped.
#[must_use = "the span ends as soon as the guard is dropped"]
pub struct TimedStretchCloser(Span);

impl Drop for TimedStretchCloser {
    fn drop(&mut self) {
        self.0.end();
    }
}

/// A value sampled once or more in a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Counter {
    TallyDrawCalls,
    TallyEffectSlotShortage,
    TallyEffectSlotsInUse,
    TallyHeapRequests,
    TallySentBatches,
    EffectLayoutTopmost,
    HudStageMaxScheduleAt,
    HudStageMaxScheduleMs,
    HudSurfacesScheduleMs,
    HudTessBodyMs,
    HudTessJobs,
    PresentPublishMs,
    DrawGpuShadeMillis,
    DrawGpuDepthPrepassMillis,
    DrawGpuWholeMillis,
    DrawGpuAfterEffectsMillis,
    DrawGpuConeLightMillis,
    DrawGpuDaylightMillis,
    DrawGraphShowMillis,
    DrawGraphRunMillis,
    RenderGraphSubmitIntervalMs,
    RenderGraphSubmitPendingN,
    RenderPrepareViewsMs,
    RenderPresentedFramesBehind,
    RenderPresentedStateAgeMs,
    UiApplyDeferredMs,
    UiHudSetupMs,
    UiHudVisibilityMs,
    MeshShadeViewCone,
    MeshShadeUnlit,
    FigureLayoutTopmost,
}

impl Counter {
    /// Sample `value` in the open frame.
    #[inline]
    pub fn emit(self, _sample: f64) {}

    /// Sample `value` for an earlier frame, named by the
    /// [`open_index`](crate::frames::open_index) read when its work began.
    #[inline]
    pub fn emit_at(self, _sample: f64, _frame: u64) {}
}
