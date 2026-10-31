//! The effect system's known shortfalls, each one recorded when something
//! hits it so a run can report what it silently went without.

use core::fmt;

use diag::gap::{self, LackLedger};

const GAP_COUNT: usize = <EffectHole as gap::Gap>::ALL_KINDS.len();

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EffectHole {
    TrailDefMiss,
    StreakProceduralMesh,
    CollideMotion,
    MotionLookup,
    ImpactChild,
    DeathChild,
    LightFraction,
    EmitChild,
    EmitOrientation,
    EmitVariance,
    SoundChild,
    DecalChild,
    FragmentMarks,
    RunnerChild,
}

/// Each gap with the name the run report lists it under.
const GAP_NAMES: [(EffectHole, &str); 14] = {
    use EffectHole::*;
    [
        (TrailDefMiss, "trail-def-lookup"),
        (StreakProceduralMesh, "trail-code-mesh"),
        (CollideMotion, "elem-collide-motion"),
        (MotionLookup, "elem-motion-lookup"),
        (ImpactChild, "elem-impact-spawn"),
        (DeathChild, "elem-death-spawn"),
        (LightFraction, "elem-lighting-frac"),
        (EmitChild, "elem-emit-spawn"),
        (EmitOrientation, "elem-emit-orient-quat"),
        (EmitVariance, "elem-emit-rand-variance"),
        (SoundChild, "elem-sound-spawn"),
        (DecalChild, "elem-decal-spawn"),
        (FragmentMarks, "mark-fragments"),
        (RunnerChild, "elem-runner-spawn"),
    ]
};

/// The gaps alone, in table order.
const GAP_LIST: [EffectHole; GAP_NAMES.len()] = {
    let mut list = [EffectHole::TrailDefMiss; GAP_NAMES.len()];
    let mut at = 0;
    while at < list.len() {
        list[at] = GAP_NAMES[at].0;
        at += 1;
    }
    list
};

impl gap::Gap for EffectHole {
    const ALL_KINDS: &'static [EffectHole] = &GAP_LIST;

    fn name(self) -> &'static str {
        let named = GAP_NAMES.iter().find(|entry| entry.0 == self);
        named.map_or("unknown", |entry| entry.1)
    }

    fn still_open(self) -> bool {
        false
    }
}

/// Which kind of child effect a part tried to play.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChildKind {
    Impact,
    Death,
    Emitted,
}

impl ChildKind {
    /// The field of the effect definition that names this child.
    fn field(self) -> &'static str {
        ["effectOnImpact", "effectOnDeath", "effectEmitted"][self as usize]
    }
}

impl fmt::Display for ChildKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.field())
    }
}

/// The step of building a CPU-side ribbon mesh that was refused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProceduralMeshPhase {
    Bind,
    CornerClaim,
    TriangleSlotClaim,
}

impl fmt::Display for ProceduralMeshPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = ["material bind", "vert reserve", "index reserve"][*self as usize];
        f.write_str(label)
    }
}

/// What happened, with the part's definition slot where there is one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EffectHoleReason {
    TrailDefAbsent { definition_slot: u8 },
    TrailCodeMeshRefused { step: ProceduralMeshPhase },
    CollideWithoutWorld { definition_slot: u8 },
    ElemDefAbsent { definition_slot: u8 },
    ChildRefused { child: ChildKind, definition_slot: u8 },
    NoLightGridSample { definition_slot: u8 },
    OrientationUnread { definition_slot: u8 },
    VarianceNeverApplied { definition_slot: u8 },
    ElemSoundSpawnSkipped { definition_slot: u8 },
    DecalNeverStarted { definition_slot: u8 },
    FragmentsUnsupported { definition_slot: u8 },
    RunnerNeverStarted { definition_slot: u8 },
}

impl EffectHoleReason {
    /// The part this happened to, when it happened to one.
    fn definition_slot(&self) -> Option<u8> {
        match *self {
            EffectHoleReason::TrailCodeMeshRefused { .. } => None,
            EffectHoleReason::TrailDefAbsent { definition_slot }
            | EffectHoleReason::CollideWithoutWorld { definition_slot }
            | EffectHoleReason::ElemDefAbsent { definition_slot }
            | EffectHoleReason::ChildRefused { definition_slot, .. }
            | EffectHoleReason::NoLightGridSample { definition_slot }
            | EffectHoleReason::OrientationUnread { definition_slot }
            | EffectHoleReason::VarianceNeverApplied { definition_slot }
            | EffectHoleReason::ElemSoundSpawnSkipped { definition_slot }
            | EffectHoleReason::DecalNeverStarted { definition_slot }
            | EffectHoleReason::FragmentsUnsupported { definition_slot }
            | EffectHoleReason::RunnerNeverStarted { definition_slot } => Some(definition_slot),
        }
    }
}

impl gap::ShortfallOrigin for EffectHoleReason {
    type Gap = EffectHole;

    fn gap(&self) -> EffectHole {
        match *self {
            EffectHoleReason::ElemDefAbsent { .. } => EffectHole::MotionLookup,
            EffectHoleReason::CollideWithoutWorld { .. } => EffectHole::CollideMotion,
            EffectHoleReason::TrailDefAbsent { .. } => EffectHole::TrailDefMiss,
            EffectHoleReason::TrailCodeMeshRefused { .. } => EffectHole::StreakProceduralMesh,
            EffectHoleReason::ChildRefused { child: kind, .. } => match kind {
                ChildKind::Emitted => EffectHole::EmitChild,
                ChildKind::Death => EffectHole::DeathChild,
                ChildKind::Impact => EffectHole::ImpactChild,
            },
            EffectHoleReason::NoLightGridSample { .. } => EffectHole::LightFraction,
            EffectHoleReason::OrientationUnread { .. } => EffectHole::EmitOrientation,
            EffectHoleReason::VarianceNeverApplied { .. } => EffectHole::EmitVariance,
            EffectHoleReason::ElemSoundSpawnSkipped { .. } => EffectHole::SoundChild,
            EffectHoleReason::DecalNeverStarted { .. } => EffectHole::DecalChild,
            EffectHoleReason::FragmentsUnsupported { .. } => EffectHole::FragmentMarks,
            EffectHoleReason::RunnerNeverStarted { .. } => EffectHole::RunnerChild,
        }
    }
}

impl EffectHoleReason {
    /// What went short, worded to follow "part N".
    fn shortfall(&self) -> &'static str {
        match self {
            EffectHoleReason::TrailDefAbsent { .. } => ": its trail definition is not in the catalog",
            EffectHoleReason::CollideWithoutWorld { .. } => " collides but was given no world to clip against",
            EffectHoleReason::ElemDefAbsent { .. } => ": no definition to work its motion out from",
            EffectHoleReason::NoLightGridSample { .. } => " wants scene lighting but there is no light grid",
            EffectHoleReason::OrientationUnread { .. } => {
                " authors an orientation quaternion that is never read"
            }
            EffectHoleReason::VarianceNeverApplied { .. } => {
                " authors emit-spacing variance that is never applied"
            }
            EffectHoleReason::ElemSoundSpawnSkipped { .. } => " is a sound and its alias would not play",
            EffectHoleReason::DecalNeverStarted { .. } => " chose a decal but the impact mark never started",
            EffectHoleReason::FragmentsUnsupported { .. } => {
                ": its impact mark needs fragment generation, which is unsupported"
            }
            EffectHoleReason::RunnerNeverStarted { .. } => " is a runner with no child effect name",
            EffectHoleReason::TrailCodeMeshRefused { .. } | EffectHoleReason::ChildRefused { .. } => "",
        }
    }
}

impl fmt::Display for EffectHoleReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TrailCodeMeshRefused { step } => {
                write!(f, "the CPU ribbon mesh was refused at the {step}")
            }
            Self::ChildRefused { child, .. } => {
                let part = self.definition_slot().expect("every other cause names a part");
                write!(f, "part {part} could not start its {child} child")
            }
            other => {
                let part = other.definition_slot().expect("every other cause names a part");
                write!(f, "part {part}{}", other.shortfall())
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct GapBook(LackLedger<EffectHoleReason, GAP_COUNT>);

impl GapBook {
    pub fn log_hole(&mut self, why: EffectHoleReason) {
        self.0.raise(why)
    }

    /// How many times this gap has been raised.
    pub fn hit_total(&self, hole: EffectHole) -> u64 {
        self.0.hits(hole)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diag::gap::{Gap, ShortfallOrigin};

    #[test]
    fn gap_names_and_order() {
        let names: Vec<_> = EffectHole::ALL_KINDS.iter().map(|g| g.name()).collect();
        assert_eq!(names.len(), 14);
        assert_eq!(names[0], "trail-def-lookup");
        assert_eq!(names[9], "elem-emit-rand-variance");
        assert_eq!(names[13], "elem-runner-spawn");
        assert!(EffectHole::ALL_KINDS.iter().all(|g| !g.still_open()));
    }

    #[test]
    fn child_labels() {
        assert_eq!(ChildKind::Impact.to_string(), "effectOnImpact");
        assert_eq!(ChildKind::Death.to_string(), "effectOnDeath");
        assert_eq!(ChildKind::Emitted.to_string(), "effectEmitted");
        assert_eq!(ProceduralMeshPhase::Bind.to_string(), "material bind");
        assert_eq!(ProceduralMeshPhase::CornerClaim.to_string(), "vert reserve");
        assert_eq!(ProceduralMeshPhase::TriangleSlotClaim.to_string(), "index reserve");
    }

    #[test]
    fn causes_read_and_map() {
        let cases = [
            (
                EffectHoleReason::TrailDefAbsent { definition_slot: 3 },
                "trail-def-lookup",
                "part 3: its trail definition is not in the catalog",
            ),
            (
                EffectHoleReason::TrailCodeMeshRefused { step: ProceduralMeshPhase::TriangleSlotClaim },
                "trail-code-mesh",
                "the CPU ribbon mesh was refused at the index reserve",
            ),
            (
                EffectHoleReason::ChildRefused { child: ChildKind::Death, definition_slot: 7 },
                "elem-death-spawn",
                "part 7 could not start its effectOnDeath child",
            ),
            (
                EffectHoleReason::ChildRefused { child: ChildKind::Emitted, definition_slot: 1 },
                "elem-emit-spawn",
                "part 1 could not start its effectEmitted child",
            ),
            (
                EffectHoleReason::FragmentsUnsupported { definition_slot: 9 },
                "mark-fragments",
                "part 9: its impact mark needs fragment generation, which is unsupported",
            ),
            (
                EffectHoleReason::RunnerNeverStarted { definition_slot: 0 },
                "elem-runner-spawn",
                "part 0 is a runner with no child effect name",
            ),
            (
                EffectHoleReason::NoLightGridSample { definition_slot: 2 },
                "elem-lighting-frac",
                "part 2 wants scene lighting but there is no light grid",
            ),
        ];
        for (cause, gap_name, text) in cases {
            assert_eq!(cause.gap().name(), gap_name);
            assert_eq!(cause.to_string(), text);
        }
    }

    #[test]
    fn ledger_counts_by_gap() {
        let mut gaps = GapBook::default();
        gaps.log_hole(EffectHoleReason::ElemDefAbsent { definition_slot: 1 });
        gaps.log_hole(EffectHoleReason::ElemDefAbsent { definition_slot: 2 });
        assert_eq!(gaps.hit_total(EffectHole::MotionLookup), 2);
        assert_eq!(gaps.hit_total(EffectHole::FragmentMarks), 0);
    }
}
