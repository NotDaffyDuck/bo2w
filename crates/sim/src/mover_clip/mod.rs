//! Keeping the prediction world's collision rows in step with the script movers that own them.
//!
//! `adopt_brush_movers` makes rows for brush movers that have none; `follow_movers` then moves every row to where
//! its mover is.

mod adopt;
mod follow;

pub(crate) use adopt::adopt_brush_movers;
pub(crate) use follow::follow_movers;

use crate::ent_slots::{MoverSlot, turn_path_of, move_path_of};
use crate::match_ids::ScriptedModel;
use entity_t6::evaluate_trajectory;
use entity_t6::missile::EF_NODRAW;
use std::collections::BTreeMap;

/// Where a mover is and how it is turned at one moment.
#[derive(Clone, Copy, PartialEq)]
struct Placement {
    at: [f32; 3],
    turn: [f32; 3],
}

impl Placement {
    fn of(mover: &MoverSlot, clock_stamp_ms: i32) -> Self {
        let state = &mover.state;
        Self {
            at: evaluate_trajectory(&move_path_of(state), clock_stamp_ms),
            turn: evaluate_trajectory(&turn_path_of(state), clock_stamp_ms),
        }
    }
}

/// Whether a mover is drawn.
fn is_hidden(mover: &MoverSlot) -> bool {
    mover.state.lerp.e_flags & EF_NODRAW != 0
}

/// The movers by the model id their rows carry.
fn movers_by_id(movers: &[MoverSlot]) -> BTreeMap<ScriptedModel, &MoverSlot> {
    movers.iter().map(|mover| (mover.id, mover)).collect()
}
