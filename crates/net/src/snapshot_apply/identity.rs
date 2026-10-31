//! How each kind of thing is told apart across snapshots, and for how long that name holds.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IdentityRuleEntry {
    pub topic: &'static str,
    pub key_form: &'static str,
    pub span_rule: &'static str,
}

const fn row(
    topic: &'static str,
    key_form: &'static str,
    span_rule: &'static str,
) -> IdentityRuleEntry {
    IdentityRuleEntry { topic, key_form, span_rule }
}

pub const IDENTITY_RULES: &[IdentityRuleEntry] = &[
    row(
        "player life",
        "(ClientId, LifeSequence)",
        "from accepted spawn until Died; a respawn bumps LifeSequence",
    ),
    row(
        "dynamic entity and archived entity",
        "EntityRef(number, generation)",
        "from allocation until free; reusing a slot keeps the number and bumps the generation",
    ),
    row(
        "authoritative projectile",
        "(ProjectileId, EntityRef)",
        "from launch until the terminal event is retained or freed; a predicted ProjectileId stays local",
    ),
    row(
        "presentation entity event",
        "EventSequence",
        "a single authoritative emission; repeated snapshots never deliver it again",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn identity_of(topic: &str) -> &'static str {
        IDENTITY_RULES
            .iter()
            .find(|row| row.topic == topic)
            .map(|row| row.key_form)
            .unwrap_or_else(|| panic!("no row for {topic:?}"))
    }

    #[test]
    fn four_subjects_each_have_a_name_and_a_lifetime() {
        assert_eq!(IDENTITY_RULES.len(), 4);
        for row in IDENTITY_RULES {
            assert!(!row.topic.is_empty() && !row.key_form.is_empty() && !row.span_rule.is_empty());
        }
    }

    #[test]
    fn a_life_is_a_client_and_a_life_number() {
        assert_eq!(identity_of("player life"), "(ClientId, LifeSequence)");
    }

    #[test]
    fn a_reused_entity_keeps_its_number_and_bumps_its_generation() {
        let row = IDENTITY_RULES
            .iter()
            .find(|row| row.key_form == "EntityRef(number, generation)")
            .expect("entity row");
        assert!(row.span_rule.contains("keeps the number and bumps the generation"));
    }

    #[test]
    fn events_are_named_by_their_sequence() {
        assert_eq!(identity_of("presentation entity event"), "EventSequence");
    }
}
