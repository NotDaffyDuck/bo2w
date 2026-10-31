/// What one adoption did, for the prediction metrics.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TakeoverReport {
    /// Players whose state was replaced.
    pub people: usize,
    /// Clients whose match record was replaced.
    pub seats: usize,
    /// Projectiles taken over from the snapshot.
    pub thrown: usize,
    /// Predicted things the snapshot had no match for, which were dropped.
    pub let_go: usize,
    /// The snapshot was made against other content than this world holds.
    pub content_differs: bool,
}

impl TakeoverReport {
    /// Whether the adoption replaced nothing and dropped nothing.
    pub fn is_untouched(&self) -> bool {
        self.people + self.seats + self.thrown + self.let_go == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_adopted_to_begin_with() {
        let report = TakeoverReport::default();
        assert!(report.is_untouched());
        assert!(!report.content_differs);
    }

    #[test]
    fn every_count_matters_to_equality() {
        let base = TakeoverReport { people: 2, seats: 2, thrown: 1, ..TakeoverReport::default() };
        assert!(!base.is_untouched());
        assert_ne!(base, TakeoverReport { let_go: 1, ..base });
        assert_ne!(base, TakeoverReport { content_differs: true, ..base });
    }
}
