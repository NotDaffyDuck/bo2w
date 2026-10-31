use std::collections::HashMap;

use asset_game::FxDefinitions;
use bevy::prelude::*;
use fx::EffectSystemHost;
use fx_t6::AfterglowSprite;

use crate::effect_play::EffectElementInfoMemo;

/// Declares the resources that wrap the effect data prepared when a world loads; each
/// starts empty and is dropped again when the match is torn down.
macro_rules! prepared_resources {
    ($($(#[$note:meta])* $name:ident($data:ty);)*) => {$(
        $(#[$note])*
        #[derive(Resource, Default)]
        pub struct $name(pub $data);
    )*};
}

prepared_resources! {
    /// Every effect definition the loaded world can name.
    ReadyFxBook(FxDefinitions);
    /// The models effects attach to their elements.
    ReadyFxProps(asset_game::EffectMeshRoster);
    /// Per-definition element facts, filled as effects play.
    PreparedFxElemTable(EffectElementInfoMemo);
    /// The impact table, when the world carries one of its own.
    ReadyHitFx(Option<asset_game::SelfContainedImpactFxTable>);
}

/// Declares a kind of event and the table that counts each kind in its own saturating slot.
macro_rules! counter_table {
    (
        $(#[$kind_note:meta])* $kind:ident;
        $(#[$table_note:meta])* $table:ident;
        $($(#[$note:meta])* $variant:ident),+ $(,)?
    ) => {
        $(#[$kind_note])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum $kind {
            $($(#[$note])* $variant),+
        }

        impl $kind {
            const ALL: &'static [$kind] = &[$($kind::$variant),+];
        }

        $(#[$table_note])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub struct $table([u32; $kind::ALL.len()]);

        impl Default for $table {
            fn default() -> Self {
                Self([0; $kind::ALL.len()])
            }
        }

        impl $table {
            pub fn count_of(&self, kind: $kind) -> u32 {
                self.0[kind as usize]
            }

            /// The counter itself, for code that adds to it through a reference.
            pub fn slot(&mut self, kind: $kind) -> &mut u32 {
                &mut self.0[kind as usize]
            }

            pub fn add_to(&mut self, kind: $kind, times: u32) {
                let slot = self.slot(kind);
                *slot = slot.saturating_add(times);
            }

            pub fn bump(&mut self, kind: $kind) {
                self.add_to(kind, 1);
            }
        }
    };
}

/// Colour handles and sort keys for one way of naming a material.
#[derive(Debug)]
pub struct ColorMaps<K> {
    pub colors: HashMap<K, Handle<Image>>,
    pub sort_keys: HashMap<K, u8>,
}

impl<K> Default for ColorMaps<K> {
    fn default() -> Self {
        Self {
            colors: HashMap::new(),
            sort_keys: HashMap::new(),
        }
    }
}

impl<K> ColorMaps<K> {
    pub fn is_empty(&self) -> bool {
        self.colors.is_empty() && self.sort_keys.is_empty()
    }
}

/// The map's materials as effects name them: by name, or by asset id.
#[derive(Resource, Default)]
pub struct EffectLevelColorImages {
    pub by_name: ColorMaps<String>,
    pub by_asset: ColorMaps<usize>,
}

impl EffectLevelColorImages {
    /// Whether no material has a colour image yet: the world's are not prepared.
    pub fn is_empty(&self) -> bool {
        self.by_name.colors.is_empty() && self.by_asset.colors.is_empty()
    }
}

/// How far around the camera, in inches, an `fx_dump` report looks unless told otherwise.
const DEFAULT_DUMP_RADIUS: f32 = 800.0;

/// A report the console asked for, held until the next frame fills it. The radius asked for
/// last is the one the next request without a radius uses.
#[derive(Resource, Debug)]
pub struct EffectDumpRequest {
    reach: f32,
    waiting: bool,
}

impl Default for EffectDumpRequest {
    fn default() -> Self {
        Self {
            reach: DEFAULT_DUMP_RADIUS,
            waiting: false,
        }
    }
}

impl EffectDumpRequest {
    /// Queues a report, around `reach` when one is given, and says how far it will look.
    pub fn queue(&mut self, reach: Option<f32>) -> f32 {
        self.reach = reach.unwrap_or(self.reach);
        self.waiting = true;
        self.reach
    }

    /// How far the next report looks.
    pub fn reach(&self) -> f32 {
        self.reach
    }

    pub fn is_queued(&self) -> bool {
        self.waiting
    }

    /// Takes the queued report: the radius it looks around, or nothing when none was asked for.
    pub fn take(&mut self) -> Option<f32> {
        std::mem::take(&mut self.waiting).then_some(self.reach)
    }
}

/// One of the dvars that switch decal marks and element culling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkSwitch {
    /// `fx_marks`: the outer gate of every impact mark.
    Marks,
    /// `fx_marks_smodels`: marks on static models.
    StaticModels,
    /// `fx_marks_ents`: marks on entities.
    Entities,
    /// `fx_mark_profile`: the mark census.
    Profile,
    /// `fx_cull_elem_draw`: culling of elements before they draw.
    CullElemDraw,
}

impl MarkSwitch {
    const fn bit(self) -> u8 {
        1 << self as u8
    }
}

/// Which mark switches are on; all are, except the profiling census.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct EffectStainTunables(u8);

impl Default for EffectStainTunables {
    fn default() -> Self {
        Self(
            MarkSwitch::Marks.bit()
                | MarkSwitch::StaticModels.bit()
                | MarkSwitch::Entities.bit()
                | MarkSwitch::CullElemDraw.bit(),
        )
    }
}

impl EffectStainTunables {
    pub const fn is_on(self, switch: MarkSwitch) -> bool {
        self.0 & switch.bit() != 0
    }

    pub fn set(&mut self, switch: MarkSwitch, on: bool) {
        if on {
            self.0 |= switch.bit();
        } else {
            self.0 &= !switch.bit();
        }
    }

    /// These switches with `switch` put to `on`.
    pub fn with(mut self, switch: MarkSwitch, on: bool) -> Self {
        self.set(switch, on);
        self
    }
}

/// The laser sight's dvars, BO2's defaults (fx_t6 `FxLaserDvars`).
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct LaserSettings(pub fx_t6::FxLaserDvars);

/// The dynamic lights effects hand to the scene this frame, up to the scene's cap.
#[derive(Resource, Clone, Debug, Default)]
pub struct EffectsDynamicLightSet {
    pub scene: Vec<lighting_t6::LampRecord>,
    over_cap: u32,
}

impl EffectsDynamicLightSet {
    /// Forgets last frame's lights.
    pub fn begin_frame(&mut self) {
        self.scene.clear();
        self.over_cap = 0;
    }

    /// Adds the light `pack` builds, if the scene admits one of this radius; a light turned
    /// away only because the scene is full is counted.
    pub fn admit(
        &mut self,
        level_present: bool,
        reach_r: f32,
        pack: impl FnOnce() -> lighting_t6::LampRecord,
    ) {
        let live = self.scene.len() as u32;
        match lighting_t6::add_omni_light_to_scene_allows(level_present, reach_r, live) {
            Ok(()) => self.scene.push(pack()),
            Err(lighting_t6::PointLampRefusal::Cap) => {
                self.over_cap = self.over_cap.saturating_add(1);
            }
            Err(_) => {}
        }
    }

    /// How many lights this frame the scene turned away for being full.
    pub fn over_cap(&self) -> u32 {
        self.over_cap
    }
}

/// What became of a post light.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PostLightFate {
    /// More lights than the post pass takes.
    Declined,
    /// Too short to make any geometry.
    TooShort,
    /// Its material has no colour image.
    Unbound,
    /// In the plan.
    Drawn,
}

/// The lights effects ask the post pass to draw this frame, and what became of them.
#[derive(Resource, Clone, Debug, Default)]
pub struct PostEffectLightSet {
    pub lights: Vec<AfterglowSprite>,
    fates: [u32; 4],
}

impl PostEffectLightSet {
    pub fn bump_by(&mut self, light: AfterglowSprite) {
        if fx_t6::post_light_add_allows(self.lights.len()) {
            self.lights.push(light);
        } else {
            self.log_fate(PostLightFate::Declined);
        }
    }

    pub fn log_fate(&mut self, fate: PostLightFate) {
        let count = &mut self.fates[fate as usize];
        *count = count.saturating_add(1);
    }

    pub fn count(&self, fate: PostLightFate) -> u32 {
        self.fates[fate as usize]
    }

    /// Starts the effects tick over: the lights asked for and the ones refused are forgotten.
    pub fn begin_tick(&mut self) {
        self.lights.clear();
        self.fates[PostLightFate::Declined as usize] = 0;
    }

    /// Starts the frame over, forgetting every light and what became of it.
    pub fn begin_frame(&mut self) {
        self.lights.clear();
        self.fates = [0; 4];
    }
}

#[derive(Resource)]
pub struct EffectsSystemHandle(pub EffectSystemHost);

impl Default for EffectsSystemHandle {
    fn default() -> Self {
        Self(EffectSystemHost::new())
    }
}

counter_table! {
    /// What the effects journal counts.
    JournalCount;
    /// The running tallies of the effects the game events asked for.
    JournalCounts;
    MuzzlePlayed,
    MuzzleMissed,
    /// Flashes that rode the weapon's bone rather than a fixed spot.
    MuzzleBolted,
    BrassPlayed,
    BrassMissed,
    Pellets,
    ImpactPlayed,
    ImpactMissed,
    ImpactTableMisses,
    BlastPlayed,
    BlastMissed,
    BlastSoundMissed,
    FireSoundMissed,
    TracerSpawned,
    /// The tracers that left the weapon's flash tag rather than the aim ray.
    TracerFromTag,
    TracerSkippedInterval,
    TracerSkippedShort,
    TracerSkippedNoDef,
    /// Placed one-shots that named no loaded effect.
    CreateFxMissedDefs,
    /// Effect meshes left out of a frame for want of a material in the sorted catalog.
    MeshNoOrdinal,
    /// Effect meshes whose material draws in another region than the emissive pass.
    MeshNotEmissive,
    /// Sprites and trails drawn without a material.
    UnboundDraws,
}

impl JournalCounts {
    /// The tracers that were not spawned, whatever the reason.
    pub fn tracers_skipped(&self) -> u32 {
        [
            JournalCount::TracerSkippedInterval,
            JournalCount::TracerSkippedShort,
            JournalCount::TracerSkippedNoDef,
        ]
        .into_iter()
        .fold(0, |sum, kind| sum.saturating_add(self.count_of(kind)))
    }
}

/// The map's placed one-shots: whether they have been played, and when.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CreateFxLedger {
    pub done: bool,
    pub ms_now: Option<i32>,
}

/// What the effects journal has seen so far, and what it has already said about it.
#[derive(Resource, Debug, Default)]
pub struct EffectJournalCursor {
    counts: JournalCounts,
    pub createfx: CreateFxLedger,
    summary_logged: bool,
    impact_warned: bool,
}

impl EffectJournalCursor {
    pub fn tally_of(&self, kind: JournalCount) -> u32 {
        self.counts.count_of(kind)
    }

    /// The counter itself, for code that adds to it through a reference.
    pub fn slot(&mut self, kind: JournalCount) -> &mut u32 {
        self.counts.slot(kind)
    }

    pub fn bump(&mut self, kind: JournalCount) {
        self.counts.bump(kind);
    }

    pub fn add(&mut self, kind: JournalCount, times: u32) {
        self.counts.add_to(kind, times);
    }

    /// The tracers that were not spawned, whatever the reason.
    pub fn tracers_skipped(&self) -> u32 {
        self.counts.tracers_skipped()
    }

    /// Forgets the placed one-shots, so the map's are played again.
    pub fn reset_createfx(&mut self) {
        self.createfx = CreateFxLedger::default();
        *self.slot(JournalCount::CreateFxMissedDefs) = 0;
    }

    /// True the first time it is asked: the summary line is logged once.
    pub fn claim_summary(&mut self) -> bool {
        !std::mem::replace(&mut self.summary_logged, true)
    }

    /// True the first time it is asked: an impact table gap is warned about once.
    pub fn claim_impact_warning(&mut self) -> bool {
        !std::mem::replace(&mut self.impact_warned, true)
    }
}

counter_table! {
    /// What became of a tracer beam.
    BeamFate;
    /// One frame's tracer beams: how many were queued, drawn, and why the rest were not.
    BeamReport;
    Queued,
    Drawn,
    NoMaterial,
    NoColor,
    NoOrdinal,
    NoGlow,
    Unprepared,
}

/// The tracer the last shot looked up.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TracerSample {
    pub name: Option<String>,
    pub shader: Option<String>,
    pub bind: Option<String>,
    pub has_color: Option<bool>,
    pub speed: Option<f32>,
    pub ray_extent: Option<f32>,
}

/// What the `fx_dump` report says about tracers.
#[derive(Resource, Clone, Debug, Default)]
pub struct BattleEffectsSnapshot {
    pub streak_live: u32,
    pub beams: BeamReport,
    pub sample: TracerSample,
}

#[derive(Resource, Default)]
pub struct EffectsEyePosition(pub [f32; 3]);
