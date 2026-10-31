//! A tagged holding queue for datagrams.

/// Datagrams waiting to go out, each given the next tag in order; tags wrap at `u32::MAX`.
#[derive(Debug, Default)]
pub struct LossySimulatedQueue {
    held_back: Vec<(u32, Vec<u8>)>,
    upcoming_marker: u32,
}

impl LossySimulatedQueue {
    /// Queue a datagram and return the tag it was given.
    pub fn push(&mut self, bytes: Vec<u8>) -> u32 {
        let tag = self.upcoming_marker;
        self.upcoming_marker = tag.wrapping_add(1);
        self.held_back.push((tag, bytes));
        tag
    }

    pub fn len(&self) -> usize {
        self.held_back.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_count_up_from_zero() {
        let mut queue = LossySimulatedQueue::default();
        assert_eq!(queue.len(), 0);
        assert_eq!(queue.push(vec![1]), 0);
        assert_eq!(queue.push(vec![2]), 1);
        assert_eq!(queue.len(), 2);
    }

    #[test]
    fn tags_wrap_after_the_largest() {
        let mut queue = LossySimulatedQueue { held_back: Vec::new(), upcoming_marker: u32::MAX };
        assert_eq!(queue.push(Vec::new()), u32::MAX);
        assert_eq!(queue.push(Vec::new()), 0);
    }
}
