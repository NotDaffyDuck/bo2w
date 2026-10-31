//! Queues a worker command for every cell the visibility walk admitted, so
//! the cell's dynamic models, brushes and scene entities get culled against
//! the cell's clip planes.

use bevy::prelude::*;
use crate::{
    AddTaskOrder, RoomViewVolumeTaskOrder, TASK_ORDER_ROOM_DEBRIS_SOLID, TASK_ORDER_ROOM_DEBRIS_MODEL,
    TASK_ORDER_ROOM_STAGE_ACTOR, enqueue_room_view_volume_orders,
};

use super::cull::{SightFrameStats, apply_sight_hide};
use crate::prepare::scene::prop_geom_memo::FrontendTaskOrders;

/// The cells whose bit is set in `words` (32 cells a word, lowest bit first),
/// leaving out bits past `cell_count`.
#[must_use]
pub fn admitted_room_indices(words: &[u32], cells_n: usize) -> Vec<u32> {
    let set_bits = words.iter().enumerate().flat_map(|(word_at, &word)| {
        (0..u32::BITS)
            .filter(move |bit| word >> bit & 1 != 0)
            .map(move |bit| word_at * u32::BITS as usize + bit as usize)
    });
    set_bits
        .filter(|&cell| cell < cells_n)
        .map(|cell| cell as u32)
        .collect()
}

pub fn register_room_view_volume_orders(app: &mut App) {
    app.add_systems(
        Update,
        enqueue_admitted_cells
            .after(apply_sight_hide)
            .in_set(frame::JobQueueStage::CellStatic),
    );
}

/// The plane range a cell is culled against: its own clip if it has one,
/// otherwise the view frustum's planes.
fn plane_range(stats: &SightFrameStats, cell: u32) -> (u8, u8) {
    match stats.room_clips.get(cell as usize) {
        Some(clip) if clip.plane_tally > 0 => (clip.plane_tally, clip.view_wall_total),
        _ => {
            let frustum = u8::try_from(stats.view_volume_planes.len()).unwrap_or(u8::MAX);
            (frustum, frustum)
        }
    }
}

fn enqueue_admitted_cells(
    stats: Option<Res<SightFrameStats>>,
    mut task_orders: ResMut<FrontendTaskOrders>,
) {
    for kind in [
        TASK_ORDER_ROOM_DEBRIS_SOLID,
        TASK_ORDER_ROOM_DEBRIS_MODEL,
        TASK_ORDER_ROOM_STAGE_ACTOR,
    ] {
        let _ = task_orders.queues.reset_kind(kind);
    }
    let Some(tally_stats) = stats.as_deref() else {
        return;
    };
    for cell in admitted_room_indices(&tally_stats.room_vis, tally_stats.room_vis_tally) {
        let (plane_tally, plane_first) = plane_range(tally_stats, cell);
        let cmd = RoomViewVolumeTaskOrder {
            cell,
            plane_tally,
            plane_first,
            view_pos: 0,
        };
        // A full queue ends the frame's enqueueing for good.
        match enqueue_room_view_volume_orders(&mut task_orders.queues, cmd) {
            Ok(adds) if !adds.contains(&AddTaskOrder::SpillInline) => {}
            _ => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::ecs::system::RunSystemOnce;
    use dpvs_t6::CellClip;

    use super::*;

    fn clip(plane_tally: u8, view_wall_total: u8) -> CellClip {
        CellClip {
            plane_tally,
            view_wall_total,
            ..CellClip::BLANK
        }
    }

    fn stats(words: &[u32], cell_count: usize) -> SightFrameStats {
        SightFrameStats {
            room_vis: words.to_vec(),
            room_vis_tally: cell_count,
            ..SightFrameStats::default()
        }
    }

    fn queued(world: &World, kind: u8) -> i32 {
        world
            .resource::<FrontendTaskOrders>()
            .queues
            .input_size(kind)
            .unwrap()
    }

    #[test]
    fn the_admitted_cells_are_the_set_bits_lowest_first() {
        assert_eq!(admitted_room_indices(&[0b1011], 4), [0, 1, 3]);
        assert_eq!(admitted_room_indices(&[0], 32), Vec::<u32>::new());
        assert_eq!(admitted_room_indices(&[], 32), Vec::<u32>::new());
    }

    #[test]
    fn the_second_word_holds_cells_thirty_two_and_up() {
        assert_eq!(admitted_room_indices(&[1 << 31, 0b101], 40), [31, 32, 34]);
        assert_eq!(admitted_room_indices(&[0, 1 << 5], 64), [37]);
    }

    #[test]
    fn bits_past_the_cell_count_are_left_out() {
        assert_eq!(admitted_room_indices(&[u32::MAX], 3), [0, 1, 2]);
        assert_eq!(admitted_room_indices(&[u32::MAX, u32::MAX], 33), (0..33).collect::<Vec<_>>());
        assert!(admitted_room_indices(&[u32::MAX], 0).is_empty());
    }

    #[test]
    fn a_cell_with_its_own_clip_is_culled_against_that_clip() {
        let mut frame = stats(&[], 0);
        frame.room_clips = vec![clip(0, 0), clip(7, 4)];
        frame.view_volume_planes = vec![[0.0; 4]; 5];
        assert_eq!(plane_range(&frame, 1), (7, 4));
    }

    #[test]
    fn a_cell_without_a_clip_is_culled_against_the_view_frustum() {
        let mut frame = stats(&[], 0);
        frame.room_clips = vec![clip(0, 0)];
        frame.view_volume_planes = vec![[0.0; 4]; 5];
        // An empty clip, and a cell past the clips.
        assert_eq!(plane_range(&frame, 0), (5, 5));
        assert_eq!(plane_range(&frame, 9), (5, 5));
    }

    #[test]
    fn a_view_with_too_many_planes_reports_the_most_a_byte_holds() {
        let mut frame = stats(&[], 0);
        frame.view_volume_planes = vec![[0.0; 4]; 300];
        assert_eq!(plane_range(&frame, 0), (u8::MAX, u8::MAX));
    }

    fn enqueue(stats: Option<SightFrameStats>, world: Option<World>) -> World {
        let mut world = world.unwrap_or_default();
        world.init_resource::<FrontendTaskOrders>();
        match stats {
            Some(stats) => world.insert_resource(stats),
            None => {
                world.remove_resource::<SightFrameStats>();
            }
        }
        world.run_system_once(enqueue_admitted_cells).unwrap();
        world
    }

    #[test]
    fn each_admitted_cell_is_queued_for_the_brush_model_and_scene_entity_workers() {
        let world = enqueue(Some(stats(&[0b1011], 4)), None);
        for kind in [
            TASK_ORDER_ROOM_DEBRIS_SOLID,
            TASK_ORDER_ROOM_DEBRIS_MODEL,
            TASK_ORDER_ROOM_STAGE_ACTOR,
        ] {
            assert_eq!(queued(&world, kind), 3);
        }
    }

    #[test]
    fn each_frame_starts_the_cell_queues_afresh() {
        let world = enqueue(Some(stats(&[0b1011], 4)), None);
        let world = enqueue(Some(stats(&[0b1], 4)), Some(world));
        assert_eq!(queued(&world, TASK_ORDER_ROOM_DEBRIS_MODEL), 1);
        // And a frame with no statistics at all leaves them empty.
        let world = enqueue(None, Some(world));
        assert_eq!(queued(&world, TASK_ORDER_ROOM_DEBRIS_SOLID), 0);
        assert_eq!(queued(&world, TASK_ORDER_ROOM_DEBRIS_MODEL), 0);
        assert_eq!(queued(&world, TASK_ORDER_ROOM_STAGE_ACTOR), 0);
    }

    #[test]
    fn a_full_queue_ends_the_frames_enqueueing() {
        let capacity = {
            let queues = FrontendTaskOrders::default().queues;
            [
                TASK_ORDER_ROOM_DEBRIS_SOLID,
                TASK_ORDER_ROOM_DEBRIS_MODEL,
                TASK_ORDER_ROOM_STAGE_ACTOR,
            ]
            .map(|kind| queues.buf_tally(kind).unwrap())
        };
        let cells = capacity.into_iter().max().unwrap() as usize + 50;
        let words = vec![u32::MAX; cells.div_ceil(32)];
        let world = enqueue(Some(stats(&words, cells)), None);
        for (kind, capacity) in [
            TASK_ORDER_ROOM_DEBRIS_SOLID,
            TASK_ORDER_ROOM_DEBRIS_MODEL,
            TASK_ORDER_ROOM_STAGE_ACTOR,
        ]
        .into_iter()
        .zip(capacity)
        {
            assert!(queued(&world, kind) <= capacity);
        }
        // The smallest ring filled up, which is what stopped the frame.
        let full = [
            TASK_ORDER_ROOM_DEBRIS_SOLID,
            TASK_ORDER_ROOM_DEBRIS_MODEL,
            TASK_ORDER_ROOM_STAGE_ACTOR,
        ]
        .into_iter()
        .zip(capacity)
        .any(|(kind, capacity)| queued(&world, kind) == capacity);
        assert!(full);
    }
}
