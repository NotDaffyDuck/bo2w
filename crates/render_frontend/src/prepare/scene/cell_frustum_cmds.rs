//! Queues a worker command for every cell the visibility walk admitted, so
//! the cell's dynamic models, brushes and scene entities get culled against
//! the cell's clip planes.

use bevy::prelude::*;
use render_frontend::{
    AddWorkerCmd, CellFrustumWorkerCmd, WORKER_CMD_CELL_DYN_BRUSH, WORKER_CMD_CELL_DYN_MODEL,
    WORKER_CMD_CELL_SCENE_ENT, enqueue_cell_frustum_cmds,
};

use crate::prepare::scene::cull::{DpvsFrameStats, apply_dpvs_cull};
use crate::prepare::scene::smodel_geom_cache::FrontendWorkerCmds;

/// The cells whose bit is set in `words` (32 cells a word, lowest bit first),
/// leaving out bits past `cell_count`.
#[must_use]
pub fn admitted_cell_indices(words: &[u32], cell_count: usize) -> Vec<u32> {
    let set_bits = words.iter().enumerate().flat_map(|(word_at, &word)| {
        (0..u32::BITS)
            .filter(move |bit| word >> bit & 1 != 0)
            .map(move |bit| word_at * u32::BITS as usize + bit as usize)
    });
    set_bits
        .filter(|&cell| cell < cell_count)
        .map(|cell| cell as u32)
        .collect()
}

pub fn register_cell_frustum_cmds(app: &mut App) {
    app.add_systems(
        Update,
        enqueue_admitted_cells
            .after(apply_dpvs_cull)
            .in_set(frame::WorkerCmdSet::CellStatic),
    );
}

/// The plane range a cell is culled against: its own clip if it has one,
/// otherwise the view frustum's planes.
fn plane_range(stats: &DpvsFrameStats, cell: u32) -> (u8, u8) {
    match stats.cell_clips.get(cell as usize) {
        Some(clip) if clip.plane_count > 0 => (clip.plane_count, clip.frustum_plane_count),
        _ => {
            let frustum = u8::try_from(stats.frustum_planes.len()).unwrap_or(u8::MAX);
            (frustum, frustum)
        }
    }
}

fn enqueue_admitted_cells(
    stats: Option<Res<DpvsFrameStats>>,
    mut worker_cmds: ResMut<FrontendWorkerCmds>,
) {
    for kind in [
        WORKER_CMD_CELL_DYN_BRUSH,
        WORKER_CMD_CELL_DYN_MODEL,
        WORKER_CMD_CELL_SCENE_ENT,
    ] {
        let _ = worker_cmds.queues.reset_type(kind);
    }
    let Some(stats) = stats.as_deref() else {
        return;
    };
    for cell in admitted_cell_indices(&stats.cell_vis, stats.cell_vis_count) {
        let (plane_count, plane_begin) = plane_range(stats, cell);
        let cmd = CellFrustumWorkerCmd {
            cell,
            plane_count,
            plane_begin,
            view: 0,
        };
        // A full queue ends the frame's enqueueing for good.
        match enqueue_cell_frustum_cmds(&mut worker_cmds.queues, cmd) {
            Ok(adds) if !adds.contains(&AddWorkerCmd::OverflowInline) => {}
            _ => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::ecs::system::RunSystemOnce;
    use dpvs_t6::CellClip;

    use super::*;

    fn clip(plane_count: u8, frustum_plane_count: u8) -> CellClip {
        CellClip {
            plane_count,
            frustum_plane_count,
            ..CellClip::EMPTY
        }
    }

    fn stats(words: &[u32], cell_count: usize) -> DpvsFrameStats {
        DpvsFrameStats {
            cell_vis: words.to_vec(),
            cell_vis_count: cell_count,
            ..DpvsFrameStats::default()
        }
    }

    fn queued(world: &World, kind: u8) -> i32 {
        world
            .resource::<FrontendWorkerCmds>()
            .queues
            .in_size(kind)
            .unwrap()
    }

    #[test]
    fn the_admitted_cells_are_the_set_bits_lowest_first() {
        assert_eq!(admitted_cell_indices(&[0b1011], 4), [0, 1, 3]);
        assert_eq!(admitted_cell_indices(&[0], 32), Vec::<u32>::new());
        assert_eq!(admitted_cell_indices(&[], 32), Vec::<u32>::new());
    }

    #[test]
    fn the_second_word_holds_cells_thirty_two_and_up() {
        assert_eq!(admitted_cell_indices(&[1 << 31, 0b101], 40), [31, 32, 34]);
        assert_eq!(admitted_cell_indices(&[0, 1 << 5], 64), [37]);
    }

    #[test]
    fn bits_past_the_cell_count_are_left_out() {
        assert_eq!(admitted_cell_indices(&[u32::MAX], 3), [0, 1, 2]);
        assert_eq!(admitted_cell_indices(&[u32::MAX, u32::MAX], 33), (0..33).collect::<Vec<_>>());
        assert!(admitted_cell_indices(&[u32::MAX], 0).is_empty());
    }

    #[test]
    fn a_cell_with_its_own_clip_is_culled_against_that_clip() {
        let mut frame = stats(&[], 0);
        frame.cell_clips = vec![clip(0, 0), clip(7, 4)];
        frame.frustum_planes = vec![[0.0; 4]; 5];
        assert_eq!(plane_range(&frame, 1), (7, 4));
    }

    #[test]
    fn a_cell_without_a_clip_is_culled_against_the_view_frustum() {
        let mut frame = stats(&[], 0);
        frame.cell_clips = vec![clip(0, 0)];
        frame.frustum_planes = vec![[0.0; 4]; 5];
        // An empty clip, and a cell past the clips.
        assert_eq!(plane_range(&frame, 0), (5, 5));
        assert_eq!(plane_range(&frame, 9), (5, 5));
    }

    #[test]
    fn a_view_with_too_many_planes_reports_the_most_a_byte_holds() {
        let mut frame = stats(&[], 0);
        frame.frustum_planes = vec![[0.0; 4]; 300];
        assert_eq!(plane_range(&frame, 0), (u8::MAX, u8::MAX));
    }

    fn enqueue(frame: Option<DpvsFrameStats>, world: Option<World>) -> World {
        let mut world = world.unwrap_or_default();
        world.init_resource::<FrontendWorkerCmds>();
        match frame {
            Some(frame) => world.insert_resource(frame),
            None => {
                world.remove_resource::<DpvsFrameStats>();
            }
        }
        world.run_system_once(enqueue_admitted_cells).unwrap();
        world
    }

    #[test]
    fn each_admitted_cell_is_queued_for_the_brush_model_and_scene_entity_workers() {
        let world = enqueue(Some(stats(&[0b1011], 4)), None);
        for kind in [
            WORKER_CMD_CELL_DYN_BRUSH,
            WORKER_CMD_CELL_DYN_MODEL,
            WORKER_CMD_CELL_SCENE_ENT,
        ] {
            assert_eq!(queued(&world, kind), 3);
        }
    }

    #[test]
    fn each_frame_starts_the_cell_queues_afresh() {
        let world = enqueue(Some(stats(&[0b1011], 4)), None);
        let world = enqueue(Some(stats(&[0b1], 4)), Some(world));
        assert_eq!(queued(&world, WORKER_CMD_CELL_DYN_MODEL), 1);
        // And a frame with no statistics at all leaves them empty.
        let world = enqueue(None, Some(world));
        assert_eq!(queued(&world, WORKER_CMD_CELL_DYN_BRUSH), 0);
        assert_eq!(queued(&world, WORKER_CMD_CELL_DYN_MODEL), 0);
        assert_eq!(queued(&world, WORKER_CMD_CELL_SCENE_ENT), 0);
    }

    #[test]
    fn a_full_queue_ends_the_frames_enqueueing() {
        let capacity = {
            let queues = FrontendWorkerCmds::default().queues;
            [
                WORKER_CMD_CELL_DYN_BRUSH,
                WORKER_CMD_CELL_DYN_MODEL,
                WORKER_CMD_CELL_SCENE_ENT,
            ]
            .map(|kind| queues.buf_count(kind).unwrap())
        };
        let cells = capacity.into_iter().max().unwrap() as usize + 50;
        let words = vec![u32::MAX; cells.div_ceil(32)];
        let world = enqueue(Some(stats(&words, cells)), None);
        for (kind, capacity) in [
            WORKER_CMD_CELL_DYN_BRUSH,
            WORKER_CMD_CELL_DYN_MODEL,
            WORKER_CMD_CELL_SCENE_ENT,
        ]
        .into_iter()
        .zip(capacity)
        {
            assert!(queued(&world, kind) <= capacity);
        }
        // The smallest ring filled up, which is what stopped the frame.
        let full = [
            WORKER_CMD_CELL_DYN_BRUSH,
            WORKER_CMD_CELL_DYN_MODEL,
            WORKER_CMD_CELL_SCENE_ENT,
        ]
        .into_iter()
        .zip(capacity)
        .any(|(kind, capacity)| queued(&world, kind) == capacity);
        assert!(full);
    }
}
