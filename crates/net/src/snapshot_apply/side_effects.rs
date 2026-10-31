//! What replaying inputs may do to things the player can see or hear.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AftereffectRerunPolicy {
    /// Run again as part of the replay.
    Recompute,
    /// Produced by a prediction step; never shown.
    MuteForecast,
    /// Shown once, however many snapshots repeat it.
    DiscardRepeatedHostTruth,
    /// Read fresh from current state each frame.
    Resample,
    /// Not wired to anything; asking for it is an error.
    DeclineDisconnected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AftereffectRuleEntry {
    pub effect_name: &'static str,
    pub rule: AftereffectRerunPolicy,
}

const fn row(effect_name: &'static str, rule: AftereffectRerunPolicy) -> AftereffectRuleEntry {
    AftereffectRuleEntry { effect_name, rule }
}

pub const AFTEREFFECT_RULES: &[AftereffectRuleEntry] = &[
    row(
        "gameplay mutation: projectile flight, damage, lifecycle, score",
        AftereffectRerunPolicy::Recompute,
    ),
    row(
        "entity events produced by a prediction-world step",
        AftereffectRerunPolicy::MuteForecast,
    ),
    row(
        "authoritative EntityEventRecord keyed by EventSequence",
        AftereffectRerunPolicy::DiscardRepeatedHostTruth,
    ),
    row(
        "reliable control journal: encoded but not consumed by the client",
        AftereffectRerunPolicy::DeclineDisconnected,
    ),
    row(
        "camera, smoothing between frames, loop audio, plus any other presentation that runs continuously",
        AftereffectRerunPolicy::Resample,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn rule_for(output_prefix: &str) -> AftereffectRerunPolicy {
        AFTEREFFECT_RULES
            .iter()
            .find(|row| row.effect_name.starts_with(output_prefix))
            .map(|row| row.rule)
            .unwrap_or_else(|| panic!("no row starting {output_prefix:?}"))
    }

    #[test]
    fn gameplay_is_recomputed_and_predicted_events_are_hidden() {
        assert_eq!(rule_for("gameplay mutation"), AftereffectRerunPolicy::Recompute);
        assert_eq!(rule_for("entity events produced"), AftereffectRerunPolicy::MuteForecast);
    }

    #[test]
    fn authoritative_events_are_shown_once() {
        assert_eq!(
            rule_for("authoritative EntityEventRecord"),
            AftereffectRerunPolicy::DiscardRepeatedHostTruth
        );
    }

    #[test]
    fn continuous_presentation_is_resampled_and_the_control_journal_is_refused() {
        assert_eq!(rule_for("camera, smoothing between frames"), AftereffectRerunPolicy::Resample);
        assert_eq!(rule_for("reliable control journal"), AftereffectRerunPolicy::DeclineDisconnected);
    }

    #[test]
    fn every_replay_rule_is_used_by_some_row() {
        let used: Vec<_> = AFTEREFFECT_RULES.iter().map(|row| row.rule).collect();
        for rule in [
            AftereffectRerunPolicy::Recompute,
            AftereffectRerunPolicy::MuteForecast,
            AftereffectRerunPolicy::DiscardRepeatedHostTruth,
            AftereffectRerunPolicy::Resample,
            AftereffectRerunPolicy::DeclineDisconnected,
        ] {
            assert!(used.contains(&rule), "{rule:?}");
        }
    }
}
