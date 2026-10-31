//! BO2's camera merge of visible BSP surfaces into draw-surf runs (R_AddBspDrawSurfs and
//! R_AllocDrawSurf). Line numbers below are in facts dpvs-drawsurf-t6.md.
//!
//! A run is a list of surface indices that share one whole 64-bit key (§4 points 5-7, lines
//! 185-187). Adjacency, first vertex, lightmap, probe, primary light and setup key are not
//! compared (lines 190-193), so the indices need not be contiguous and an invisible surface
//! between two members does not split them.

use alloc::vec::Vec;

use crate::SortedDraw;

/// A pending run holds at most 128 u16 indices and is flushed when full (facts line 187).
pub const BSP_DRAW_SURF_RUN_MAX: usize = 0x80;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorldFaceRun<K> {
    pub kind: K,

    /// Offset of the run's first index in the shared surface list.
    pub list_start: u32,

    pub len: u16,

    pub facet_draw: SortedDraw,

    /// The run was flushed at 128 and this one has the same key, so R_AllocDrawSurf extends
    /// the same list entry instead of starting a new one (facts line 110).
    pub extends_entry: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LevelTreeFacetDrawCensus {
    pub span_total: u32,
    pub seen_total: u32,
    pub accepted_total: u32,
    pub runs_seen: u32,

    /// Draw-surf list entries: runs that do not extend the entry before them.
    pub entry_record_total: u32,

    /// Surfaces with no vis byte, no key, or an index above u16.
    pub input_gap_tally: u32,
}

/// Appends the visible surfaces of `begin..end` to `list` and their runs to `runs`.
/// Each call starts with no pending run and flushes its last one (facts §4 point 7).
pub fn queue_world_faces_for_view<K: Copy>(
    kind: K,
    first_ix: u32,
    end: u32,
    facet_vis: &[u8],
    facet_draws: &[SortedDraw],
    list: &mut Vec<u16>,
    runs: &mut Vec<WorldFaceRun<K>>,
) -> LevelTreeFacetDrawCensus {
    let mut tally = LevelTreeFacetDrawCensus {
        span_total: end.saturating_sub(first_ix),
        ..LevelTreeFacetDrawCensus::default()
    };
    let mut pending: Option<WorldFaceRun<K>> = None;

    for face in first_ix..end {
        let i = face as usize;
        let Some(&vis) = facet_vis.get(i) else {
            tally.input_gap_tally = tally.input_gap_tally.saturating_add(1);
            continue;
        };
        if vis == 0 {
            continue;
        }
        tally.seen_total = tally.seen_total.saturating_add(1);
        let (Some(&facet_draw), Ok(id)) = (facet_draws.get(i), u16::try_from(face)) else {
            tally.input_gap_tally = tally.input_gap_tally.saturating_add(1);
            continue;
        };
        tally.accepted_total = tally.accepted_total.saturating_add(1);

        if let Some(run) = pending.as_mut()
            && run.facet_draw == facet_draw
            && usize::from(run.len) < BSP_DRAW_SURF_RUN_MAX
        {
            run.len += 1;
            list.push(id);
            continue;
        }
        let extends_entry = pending.is_some_and(|run| run.facet_draw == facet_draw);
        if let Some(run) = pending.take() {
            push_run(run, runs, &mut tally);
        }
        pending = Some(WorldFaceRun {
            kind,
            list_start: list.len() as u32,
            len: 1,
            facet_draw,
            extends_entry,
        });
        list.push(id);
    }
    if let Some(run) = pending {
        push_run(run, runs, &mut tally);
    }
    tally
}

fn push_run<K>(
    run: WorldFaceRun<K>,
    runs: &mut Vec<WorldFaceRun<K>>,
    census: &mut LevelTreeFacetDrawCensus,
) {
    census.runs_seen = census.runs_seen.saturating_add(1);
    if !run.extends_entry {
        census.entry_record_total = census.entry_record_total.saturating_add(1);
    }
    runs.push(run);
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;

    fn members(list: &[u16], run: &WorldFaceRun<()>) -> Vec<u16> {
        let start = run.list_start as usize;
        list[start..start + usize::from(run.len)].to_vec()
    }

    #[test]
    fn invisible_surface_does_not_split_a_run() {
        let vis = [1, 0, 1];
        let keys = [7, 9, 7].map(SortedDraw::unpack_from_bits);
        let (mut which_list, mut spans) = (Vec::new(), Vec::new());
        let census = queue_world_faces_for_view((), 0, 3, &vis, &keys, &mut which_list, &mut spans);
        assert_eq!(spans.len(), 1);
        assert_eq!(members(&which_list, &spans[0]), vec![0, 2]);
        assert_eq!(
            (
                census.seen_total,
                census.accepted_total,
                census.runs_seen,
                census.entry_record_total
            ),
            (2, 2, 1, 1)
        );
    }

    #[test]
    fn key_change_closes_the_run() {
        let vis = [1; 4];
        let keys = [1, 1, 2, 1].map(SortedDraw::unpack_from_bits);
        let (mut face_list, mut run_list) = (Vec::new(), Vec::new());
        let census = queue_world_faces_for_view((), 0, 4, &vis, &keys, &mut face_list, &mut run_list);
        assert_eq!(run_list.len(), 3);
        assert_eq!(members(&face_list, &run_list[0]), vec![0, 1]);
        assert_eq!(members(&face_list, &run_list[1]), vec![2]);
        assert_eq!(members(&face_list, &run_list[2]), vec![3]);
        assert!(run_list.iter().all(|run| !run.extends_entry));
        assert_eq!(census.entry_record_total, 3);
    }

    #[test]
    fn full_run_flushes_at_128_and_extends_the_entry() {
        let vis = [1; 130];
        let keys = [SortedDraw::unpack_from_bits(5); 130];
        let (mut draws, mut stripes) = (Vec::new(), Vec::new());
        let census = queue_world_faces_for_view((), 0, 130, &vis, &keys, &mut draws, &mut stripes);
        assert_eq!(stripes.len(), 2);
        assert_eq!((stripes[0].len, stripes[1].len), (128, 2));
        assert!(!stripes[0].extends_entry);
        assert!(stripes[1].extends_entry);
        assert_eq!(stripes[1].list_start, 128);
        assert_eq!((census.runs_seen, census.entry_record_total), (2, 1));
    }

    #[test]
    fn missing_vis_or_key_is_an_input_gap() {
        let vis = [1, 1, 1];
        let keys = [3, 3].map(SortedDraw::unpack_from_bits);
        let (mut items, mut run_set) = (Vec::new(), Vec::new());
        let census = queue_world_faces_for_view((), 0, 5, &vis, &keys, &mut items, &mut run_set);
        assert_eq!(census.input_gap_tally, 3);
        assert_eq!(items, vec![0, 1]);
        assert_eq!(run_set.len(), 1);
    }

    #[test]
    fn runs_point_past_the_existing_list() {
        let vis = [1, 1];
        let keys = [4, 6].map(SortedDraw::unpack_from_bits);
        let mut list = vec![9, 9, 9];
        let mut runs = Vec::new();
        queue_world_faces_for_view((), 0, 2, &vis, &keys, &mut list, &mut runs);
        assert_eq!((runs[0].list_start, runs[1].list_start), (3, 4));
        assert_eq!(list, vec![9, 9, 9, 0, 1]);
    }

    #[test]
    fn each_call_starts_a_new_entry() {
        let vis = [1, 1];
        let keys = [8, 8].map(SortedDraw::unpack_from_bits);
        let (mut face_ids, mut run_spans) = (Vec::new(), Vec::new());
        queue_world_faces_for_view((), 0, 1, &vis, &keys, &mut face_ids, &mut run_spans);
        queue_world_faces_for_view((), 1, 2, &vis, &keys, &mut face_ids, &mut run_spans);
        assert_eq!(run_spans.len(), 2);
        assert!(!run_spans[1].extends_entry);
    }
}
