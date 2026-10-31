//! A keeper for the lowest scored of several candidates offered in turn.

/// The candidate with the lowest score offered so far; on a tie the first
/// offered stays. (A search for the highest offers the negated score.)
pub(crate) struct Closest<T> {
    kept: Option<(f32, T)>,
}

impl<T> Closest<T> {
    pub(crate) fn new() -> Self {
        Self { kept: None }
    }

    /// Would a candidate scored `score` take the place of the one kept?
    pub(crate) fn beaten_by(&self, score: f32) -> bool {
        self.kept.as_ref().is_none_or(|held| score < held.0)
    }

    /// Keep `item` when nothing is kept yet or `score` is lower.
    pub(crate) fn offer(&mut self, score: f32, item: T) {
        if self.beaten_by(score) {
            self.kept = Some((score, item));
        }
    }

    /// The kept candidate's score.
    pub(crate) fn score(&self) -> Option<f32> {
        self.kept.as_ref().map(|held| held.0)
    }

    /// The kept candidate with its score.
    pub(crate) fn into_scored(self) -> Option<(f32, T)> {
        self.kept
    }

    /// The kept candidate.
    pub(crate) fn into_item(self) -> Option<T> {
        self.kept.map(|held| held.1)
    }
}
