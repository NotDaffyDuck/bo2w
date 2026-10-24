#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TechType(pub u8);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SetupArm {
    LitModelLighting,
    DepthPrepass,
    Generic,
}

impl SetupArm {
    pub fn classify(tech_type: TechType) -> Self {
        let tech = tech_type.0;

        if lighting_t6::is_lit_remap_slot(tech) {
            Self::LitModelLighting
        // TODO(fact): which BO2 slots are depth-only; BO2's 0 is the standard technique.
        } else if tech < 2 {
            Self::DepthPrepass
        } else {
            Self::Generic
        }
    }
}
