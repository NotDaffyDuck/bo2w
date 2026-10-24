//! The named spans and counters. A new call site adds its name here.

/// A stretch of work inside a frame: opened with [`Span::begin`] and closed
/// with [`Span::end`], or held open by the guard [`Span::enter`] returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Span {
    FixedTocAdvance,
    FixedTocBookkeeping,
    FixedTocFanout,
    FixedTocGather,
    FixedTocIngress,
    FixedTocSnapshot,
    FixedTocStep,
    FramesDiagMs,
    FramesEffectsMs,
    FramesFixedMs,
    FramesPostupdateMs,
    FramesPredictMs,
    FramesPresentMs,
    FramesPreupdateMs,
    FramesUpdateMs,
    FramesWallFrameMs,
    HostFxPresentCpuMs,
    HostFxUpdateCpuMs,
    HostPostExecuteMs,
    HostPostRebuildMs,
    HostStaticSunFxMs,
    HostStaticSunMs,
    RenderCullCpuMs,
    RenderDispatchWorldMs,
    RenderExtractBodyMs,
    RenderReceiveWorldMs,
    RenderRenderRenderMs,
    RenderRenderThreadMs,
    TocInput,
    TocLoad,
    TocReceive,
    TocReconcile,
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
    pub fn enter(self) -> SpanGuard {
        self.begin();
        SpanGuard(self)
    }
}

/// Closes its span when dropped.
#[must_use = "the span ends as soon as the guard is dropped"]
pub struct SpanGuard(Span);

impl Drop for SpanGuard {
    fn drop(&mut self) {
        self.0.end();
    }
}

/// A value sampled once or more in a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Counter {
    CounterDraws,
    CounterFxElemAllocFail,
    CounterFxElemLive,
    CounterProcessAllocations,
    CounterSubmittedBatches,
    FxLayoutOverlay,
    HudStageMaxScheduleAt,
    HudStageMaxScheduleMs,
    HudSurfacesScheduleMs,
    HudTessBodyMs,
    HudTessJobs,
    PresentPublishMs,
    RenderGpuColourMs,
    RenderGpuFloatzMs,
    RenderGpuFrameMs,
    RenderGpuPostfxMs,
    RenderGpuSpotMs,
    RenderGpuSunMs,
    RenderGraphPresentMs,
    RenderGraphRenderMs,
    RenderGraphSubmitIntervalMs,
    RenderGraphSubmitPendingN,
    RenderPrepareViewsMs,
    RenderPresentedFramesBehind,
    RenderPresentedStateAgeMs,
    SpotShadowGpu,
    SpotShadowGpuMiss,
    SpotShadowSlotN,
    UiApplyDeferredMs,
    UiHudSetupMs,
    UiHudVisibilityMs,
    XmodelColourCameraFrustum,
    XmodelColourNoLighting,
    XmodelLayoutOverlay,
}

impl Counter {
    /// Sample `value` in the open frame.
    #[inline]
    pub fn emit(self, _value: f64) {}

    /// Sample `value` for an earlier frame, named by the
    /// [`open_index`](crate::frames::open_index) read when its work began.
    #[inline]
    pub fn emit_at(self, _value: f64, _frame: u64) {}
}
