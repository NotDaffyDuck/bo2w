//! The effect system's known shortfalls, each one recorded when something
//! hits it so a run can report what it silently went without.

use core::fmt;

use diag::gap::{self as ledger, GapLedger};

const GAP_COUNT: usize = <FxGap as ledger::Gap>::ALL.len();

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FxGap {
    TrailDefLookup,
    TrailCodeMesh,
    ElemCollideMotion,
    ElemMotionLookup,
    ElemImpactSpawn,
    ElemDeathSpawn,
    ElemLightingFrac,
    ElemEmitSpawn,
    ElemEmitOrientQuat,
    ElemEmitRandVariance,
    ElemSoundSpawn,
    ElemDecalSpawn,
    MarkFragments,
    ElemRunnerSpawn,
}

impl ledger::Gap for FxGap {
    const ALL: &'static [FxGap] = &[
        FxGap::TrailDefLookup,
        FxGap::TrailCodeMesh,
        FxGap::ElemCollideMotion,
        FxGap::ElemMotionLookup,
        FxGap::ElemImpactSpawn,
        FxGap::ElemDeathSpawn,
        FxGap::ElemLightingFrac,
        FxGap::ElemEmitSpawn,
        FxGap::ElemEmitOrientQuat,
        FxGap::ElemEmitRandVariance,
        FxGap::ElemSoundSpawn,
        FxGap::ElemDecalSpawn,
        FxGap::MarkFragments,
        FxGap::ElemRunnerSpawn,
    ];

    fn name(self) -> &'static str {
        match self {
            FxGap::TrailDefLookup => "trail-def-lookup",
            FxGap::TrailCodeMesh => "trail-code-mesh",
            FxGap::ElemCollideMotion => "elem-collide-motion",
            FxGap::ElemMotionLookup => "elem-motion-lookup",
            FxGap::ElemImpactSpawn => "elem-impact-spawn",
            FxGap::ElemDeathSpawn => "elem-death-spawn",
            FxGap::ElemLightingFrac => "elem-lighting-frac",
            FxGap::ElemEmitSpawn => "elem-emit-spawn",
            FxGap::ElemEmitOrientQuat => "elem-emit-orient-quat",
            FxGap::ElemEmitRandVariance => "elem-emit-rand-variance",
            FxGap::ElemSoundSpawn => "elem-sound-spawn",
            FxGap::ElemDecalSpawn => "elem-decal-spawn",
            FxGap::MarkFragments => "mark-fragments",
            FxGap::ElemRunnerSpawn => "elem-runner-spawn",
        }
    }

    fn is_standing(self) -> bool {
        false
    }
}

/// Which kind of child effect a part tried to play.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChildSpawn {
    Impact,
    Death,
    Emitted,
}

impl fmt::Display for ChildSpawn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ChildSpawn::Impact => "effectOnImpact",
            ChildSpawn::Death => "effectOnDeath",
            ChildSpawn::Emitted => "effectEmitted",
        })
    }
}

/// The step of building a CPU-side ribbon mesh that was refused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CodeMeshStep {
    Bind,
    VertReserve,
    IndexReserve,
}

impl fmt::Display for CodeMeshStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            CodeMeshStep::Bind => "material bind",
            CodeMeshStep::VertReserve => "vert reserve",
            CodeMeshStep::IndexReserve => "index reserve",
        })
    }
}

/// What happened, with the part's definition slot where there is one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FxGapCause {
    TrailDefNotFound { def_index: u8 },
    TrailCodeMeshRefused { step: CodeMeshStep },
    NoWorldClipForCollide { def_index: u8 },
    ElemDefNotFound { def_index: u8 },
    ChildSpawnRefused { child: ChildSpawn, def_index: u8 },
    NoLightGridSample { def_index: u8 },
    EmitOrientQuatNotUnpacked { def_index: u8 },
    EmitRandVarianceNotApplied { def_index: u8 },
    ElemSoundSpawnSkipped { def_index: u8 },
    ElemDecalSpawnSkipped { def_index: u8 },
    MarkFragmentsSkipped { def_index: u8 },
    ElemRunnerSpawnSkipped { def_index: u8 },
}

impl ledger::GapCause for FxGapCause {
    type Gap = FxGap;

    fn gap(&self) -> FxGap {
        match self {
            FxGapCause::TrailDefNotFound { .. } => FxGap::TrailDefLookup,
            FxGapCause::TrailCodeMeshRefused { .. } => FxGap::TrailCodeMesh,
            FxGapCause::NoWorldClipForCollide { .. } => FxGap::ElemCollideMotion,
            FxGapCause::ElemDefNotFound { .. } => FxGap::ElemMotionLookup,
            FxGapCause::ChildSpawnRefused { child, .. } => match child {
                ChildSpawn::Impact => FxGap::ElemImpactSpawn,
                ChildSpawn::Death => FxGap::ElemDeathSpawn,
                ChildSpawn::Emitted => FxGap::ElemEmitSpawn,
            },
            FxGapCause::NoLightGridSample { .. } => FxGap::ElemLightingFrac,
            FxGapCause::EmitOrientQuatNotUnpacked { .. } => FxGap::ElemEmitOrientQuat,
            FxGapCause::EmitRandVarianceNotApplied { .. } => FxGap::ElemEmitRandVariance,
            FxGapCause::ElemSoundSpawnSkipped { .. } => FxGap::ElemSoundSpawn,
            FxGapCause::ElemDecalSpawnSkipped { .. } => FxGap::ElemDecalSpawn,
            FxGapCause::MarkFragmentsSkipped { .. } => FxGap::MarkFragments,
            FxGapCause::ElemRunnerSpawnSkipped { .. } => FxGap::ElemRunnerSpawn,
        }
    }
}

impl fmt::Display for FxGapCause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            FxGapCause::TrailDefNotFound { def_index } => {
                write!(f, "part {def_index}: its trail definition is not in the catalog")
            }
            FxGapCause::TrailCodeMeshRefused { step } => {
                write!(f, "the CPU ribbon mesh was refused at the {step}")
            }
            FxGapCause::NoWorldClipForCollide { def_index } => {
                write!(f, "part {def_index} collides but was given no world to clip against")
            }
            FxGapCause::ElemDefNotFound { def_index } => {
                write!(f, "part {def_index}: no definition to work its motion out from")
            }
            FxGapCause::ChildSpawnRefused { child, def_index } => {
                write!(f, "part {def_index} could not start its {child} child")
            }
            FxGapCause::NoLightGridSample { def_index } => {
                write!(f, "part {def_index} wants scene lighting but there is no light grid")
            }
            FxGapCause::EmitOrientQuatNotUnpacked { def_index } => {
                write!(f, "part {def_index} authors an orientation quaternion that is never read")
            }
            FxGapCause::EmitRandVarianceNotApplied { def_index } => {
                write!(f, "part {def_index} authors emit-spacing variance that is never applied")
            }
            FxGapCause::ElemSoundSpawnSkipped { def_index } => {
                write!(f, "part {def_index} is a sound and its alias would not play")
            }
            FxGapCause::ElemDecalSpawnSkipped { def_index } => {
                write!(f, "part {def_index} chose a decal but the impact mark never started")
            }
            FxGapCause::MarkFragmentsSkipped { def_index } => {
                write!(f, "part {def_index}: its impact mark needs fragment generation, which is unsupported")
            }
            FxGapCause::ElemRunnerSpawnSkipped { def_index } => {
                write!(f, "part {def_index} is a runner with no child effect name")
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct FxGaps(GapLedger<FxGapCause, GAP_COUNT>);

impl FxGaps {
    pub fn raise(&mut self, cause: FxGapCause) {
        self.0.raise(cause);
    }

    /// How many times this gap has been raised.
    pub fn hits(&self, gap: FxGap) -> u64 {
        self.0.hits(gap)
    }
}
