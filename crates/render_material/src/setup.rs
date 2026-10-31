/// The technique slot a pass was authored for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MethodType(pub u8);

/// Which family of state setup a technique slot gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SetupVariantBranch {
    ModelLitSetup,
    DepthOnly,
    Generic,
}

impl SetupVariantBranch {
    pub fn classify(shading_technique_kind: MethodType) -> Self {
        let slot = shading_technique_kind.0;
        // TODO(fact): which BO2 slots are depth-only; BO2's 0 is the standard technique.
        match slot {
            _ if lighting_t6::slot_is_lit_substitute(slot) => Self::ModelLitSetup,
            0 | 1 => Self::DepthOnly,
            _ => Self::Generic,
        }
    }
}
