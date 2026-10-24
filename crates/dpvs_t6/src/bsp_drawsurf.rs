//! BO2's camera merge of visible BSP surfaces into draw-surf runs (R_AddBspDrawSurfs and
//! R_AllocDrawSurf). Line numbers below are in facts dpvs-drawsurf-t6.md.
//!
//! A run is a list of surface indices that share one whole 64-bit key (§4 points 5-7, lines
//! 185-187). Adjacency, first vertex, lightmap, probe, primary light and setup key are not
//! compared (lines 190-193), so the indices need not be contiguous and an invisible surface
//! between two members does not split them.

use alloc::vec::Vec;

use crate::GfxDrawSurf;

/// A pending run holds at most 128 u16 indices and is flushed when full (facts line 187).
pub const BSP_DRAW_SURF_RUN_MAX: usize = 0x80;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BspDrawSurfRun<K> {
    pub kind: K,

    /// Offset of the run's first index in the shared surface list.
    pub list_start: u32,

    pub len: u16,

    pub draw_surf: GfxDrawSurf,

    /// The run was flushed at 128 and this one has the same key, so R_AllocDrawSurf extends
    /// the same list entry instead of starting a new one (facts line 110).
    pub extends_entry: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BspDrawSurfCensus {
    pub range_n: u32,
    pub visible_n: u32,
    pub admitted_n: u32,
    pub run_n: u32,

    /// Draw-surf list entries: runs that do not extend the entry before them.
    pub entry_n: u32,

    /// Surfaces with no vis byte, no key, or an index above u16.
    pub input_gap_n: u32,
}

/// Appends the visible surfaces of `begin..end` to `list` and their runs to `runs`.
/// Each call starts with no pending run and flushes its last one (facts §4 point 7).
pub fn add_bsp_draw_surfs_camera<K: Copy>(
    kind: K,
    begin: u32,
    end: u32,
    surface_vis: &[u8],
    draw_surfs: &[GfxDrawSurf],
    list: &mut Vec<u16>,
    runs: &mut Vec<BspDrawSurfRun<K>>,
) -> BspDrawSurfCensus {
    let mut census = BspDrawSurfCensus {
        range_n: end.saturating_sub(begin),
        ..BspDrawSurfCensus::default()
    };
    let mut pending: Option<BspDrawSurfRun<K>> = None;

    for surf in begin..end {
        let i = surf as usize;
        let Some(&vis) = surface_vis.get(i) else {
            census.input_gap_n = census.input_gap_n.saturating_add(1);
            continue;
        };
        if vis == 0 {
            continue;
        }
        census.visible_n = census.visible_n.saturating_add(1);
        let (Some(&draw_surf), Ok(id)) = (draw_surfs.get(i), u16::try_from(surf)) else {
            census.input_gap_n = census.input_gap_n.saturating_add(1);
            continue;
        };
        census.admitted_n = census.admitted_n.saturating_add(1);

        if let Some(run) = pending.as_mut()
            && run.draw_surf == draw_surf
            && usize::from(run.len) < BSP_DRAW_SURF_RUN_MAX
        {
            run.len += 1;
            list.push(id);
            continue;
        }
        let extends_entry = pending.is_some_and(|run| run.draw_surf == draw_surf);
        if let Some(run) = pending.take() {
            push_run(run, runs, &mut census);
        }
        pending = Some(BspDrawSurfRun {
            kind,
            list_start: list.len() as u32,
            len: 1,
            draw_surf,
            extends_entry,
        });
        list.push(id);
    }
    if let Some(run) = pending {
        push_run(run, runs, &mut census);
    }
    census
}

fn push_run<K>(
    run: BspDrawSurfRun<K>,
    runs: &mut Vec<BspDrawSurfRun<K>>,
    census: &mut BspDrawSurfCensus,
) {
    census.run_n = census.run_n.saturating_add(1);
    if !run.extends_entry {
        census.entry_n = census.entry_n.saturating_add(1);
    }
    runs.push(run);
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;

    fn key(packed: u64) -> GfxDrawSurf {
        GfxDrawSurf::from_packed(packed)
    }

    fn members(list: &[u16], run: &BspDrawSurfRun<()>) -> Vec<u16> {
        let start = run.list_start as usize;
        list[start..start + usize::from(run.len)].to_vec()
    }

    #[test]
    fn invisible_surface_does_not_split_a_run() {
        let vis = [1, 0, 1];
        let keys = [key(7), key(9), key(7)];
        let (mut list, mut runs) = (Vec::new(), Vec::new());
        let census = add_bsp_draw_surfs_camera((), 0, 3, &vis, &keys, &mut list, &mut runs);
        assert_eq!(runs.len(), 1);
        assert_eq!(members(&list, &runs[0]), vec![0, 2]);
        assert_eq!(
            (
                census.visible_n,
                census.admitted_n,
                census.run_n,
                census.entry_n
            ),
            (2, 2, 1, 1)
        );
    }

    #[test]
    fn key_change_closes_the_run() {
        let vis = [1; 4];
        let keys = [key(1), key(1), key(2), key(1)];
        let (mut list, mut runs) = (Vec::new(), Vec::new());
        let census = add_bsp_draw_surfs_camera((), 0, 4, &vis, &keys, &mut list, &mut runs);
        assert_eq!(runs.len(), 3);
        assert_eq!(members(&list, &runs[0]), vec![0, 1]);
        assert_eq!(members(&list, &runs[1]), vec![2]);
        assert_eq!(members(&list, &runs[2]), vec![3]);
        assert!(runs.iter().all(|run| !run.extends_entry));
        assert_eq!(census.entry_n, 3);
    }

    #[test]
    fn full_run_flushes_at_128_and_extends_the_entry() {
        let vis = [1; 130];
        let keys = [key(5); 130];
        let (mut list, mut runs) = (Vec::new(), Vec::new());
        let census = add_bsp_draw_surfs_camera((), 0, 130, &vis, &keys, &mut list, &mut runs);
        assert_eq!(runs.len(), 2);
        assert_eq!((runs[0].len, runs[1].len), (128, 2));
        assert!(!runs[0].extends_entry);
        assert!(runs[1].extends_entry);
        assert_eq!(runs[1].list_start, 128);
        assert_eq!((census.run_n, census.entry_n), (2, 1));
    }

    #[test]
    fn missing_vis_or_key_is_an_input_gap() {
        let vis = [1, 1, 1];
        let keys = [key(3), key(3)];
        let (mut list, mut runs) = (Vec::new(), Vec::new());
        let census = add_bsp_draw_surfs_camera((), 0, 5, &vis, &keys, &mut list, &mut runs);
        assert_eq!(census.input_gap_n, 3);
        assert_eq!(list, vec![0, 1]);
        assert_eq!(runs.len(), 1);
    }

    #[test]
    fn runs_point_past_the_existing_list() {
        let vis = [1, 1];
        let keys = [key(4), key(6)];
        let mut list = vec![9, 9, 9];
        let mut runs = Vec::new();
        add_bsp_draw_surfs_camera((), 0, 2, &vis, &keys, &mut list, &mut runs);
        assert_eq!((runs[0].list_start, runs[1].list_start), (3, 4));
        assert_eq!(list, vec![9, 9, 9, 0, 1]);
    }

    #[test]
    fn each_call_starts_a_new_entry() {
        let vis = [1, 1];
        let keys = [key(8), key(8)];
        let (mut list, mut runs) = (Vec::new(), Vec::new());
        add_bsp_draw_surfs_camera((), 0, 1, &vis, &keys, &mut list, &mut runs);
        add_bsp_draw_surfs_camera((), 1, 2, &vis, &keys, &mut list, &mut runs);
        assert_eq!(runs.len(), 2);
        assert!(!runs[1].extends_entry);
    }
}
