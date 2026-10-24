/// The technique slot a pass was authored for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TechType(pub u8);

/// Which family of state setup a technique slot gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SetupArm {
    LitModelLighting,
    DepthPrepass,
    Generic,
}

impl SetupArm {
    pub fn classify(tech_type: TechType) -> Self {
        let slot = tech_type.0;
        // TODO(fact): which BO2 slots are depth-only; BO2's 0 is the standard technique.
        match slot {
            _ if lighting_t6::is_lit_remap_slot(slot) => Self::LitModelLighting,
            0 | 1 => Self::DepthPrepass,
            _ => Self::Generic,
        }
    }
}
