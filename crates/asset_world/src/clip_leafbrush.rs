//! Reading the leaf-brush tables of a clip map.

use crate::clip_error::SolidsTraceFault;

/// The deepest a leaf-brush node chain is followed.
pub(crate) const LEAFBRUSH_NODE_MAX_DEPTH: usize = 64;

/// The place and length of the brushes a leaf adds to the shared leaf-brush
/// table, given the table's length before and after the leaf's brushes went
/// in.
pub(crate) fn leafbrush_range(len_before: usize, len_after: usize) -> Result<(u32, u16), SolidsTraceFault> {
    let count = len_after.checked_sub(len_before).map(u16::try_from);
    match (u32::try_from(len_before), count) {
        (Ok(first), Some(Ok(count))) => Ok((first, count)),
        _ => Err(SolidsTraceFault::LeafSolidIndexExceeded),
    }
}

/// The nodes to look at after the one at `index`: the next row when `count`
/// is negative (the node has brushes of its own and the chain goes on), then
/// each non-zero relative jump.
pub(crate) fn leafbrush_node_visits(index: usize, count: i16, jump_a: usize, jump_b: usize) -> Vec<usize> {
    let next_row = (count < 0).then_some(index + 1);
    let jumps = [jump_a, jump_b].into_iter().filter(|&jump| jump != 0).map(|jump| index + jump);
    next_row.into_iter().chain(jumps).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_range_is_where_the_leaf_started_and_how_much_it_added() {
        assert_eq!(leafbrush_range(10, 14), Ok((10, 4)));
        assert_eq!(leafbrush_range(7, 7), Ok((7, 0)));
    }

    #[test]
    fn a_range_that_cannot_be_indexed_is_refused() {
        let overflow = Err(SolidsTraceFault::LeafSolidIndexExceeded);
        assert_eq!(leafbrush_range(5, 4), overflow);
        assert_eq!(leafbrush_range(0, usize::from(u16::MAX) + 1), overflow);
        if usize::BITS > 32 {
            assert_eq!(leafbrush_range(usize::try_from(u64::from(u32::MAX) + 1).unwrap(), usize::MAX), overflow);
        }
    }

    #[test]
    fn a_negative_count_visits_the_next_row_first() {
        assert_eq!(leafbrush_node_visits(8, -1, 3, 5), [9, 11, 13]);
        assert_eq!(leafbrush_node_visits(8, 2, 3, 5), [11, 13]);
    }

    #[test]
    fn a_zero_jump_goes_nowhere() {
        assert_eq!(leafbrush_node_visits(8, 0, 0, 5), [13]);
        assert_eq!(leafbrush_node_visits(8, 0, 4, 0), [12]);
        assert!(leafbrush_node_visits(8, 0, 0, 0).is_empty());
    }
}
