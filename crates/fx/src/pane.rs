//! Broken window glass: the pane shards live in `glass_t6`; this host adds
//! which panes have shattered so they stop taking marks.

pub use glass_t6::{GlassWorldContact, GlassWorldTrace};

#[derive(Clone, Debug, Default)]
pub struct PaneShardOwner {
    /// The falling shards.
    pub t6: glass_t6::T6Glass,
    /// Indexed by pane; a shattered pane takes no more marks.
    pub shattered: Vec<bool>,
}

impl PaneShardOwner {
    /// Breaks the shards a segment passes through; returns how many broke.
    pub fn hit_segment(&mut self, from_pt: [f32; 3], stop: [f32; 3], seed: u64) -> usize {
        self.t6.hit_segment(from_pt, stop, seed)
    }

    /// Lets the shards fall for `msec`, landing on whatever `world` reports.
    pub fn advance_in_world(&mut self, msec: i32, world: Option<&dyn GlassWorldTrace>) {
        self.t6.advance(msec, world);
    }

    pub fn pane_broken(&self, pane: u32) -> bool {
        self.shattered.get(pane as usize).copied().unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_marked_panes_count_as_broken() {
        let mut host = PaneShardOwner {
            shattered: vec![false; 2],
            ..Default::default()
        };
        host.shattered[1] = true;
        assert!(!host.pane_broken(0));
        assert!(host.pane_broken(1));
        assert!(!host.pane_broken(7), "a pane past the list is not broken");
    }
}
