//! The worker-command table: its length, its names, its ordering rules, and where it sits in the
//! client's Present stage.

use bevy::prelude::*;

use super::probe::{bare_app, note, probe_client_sets, trace_of};
use crate::tick_sets::{
    WORKER_CMD_AFTER, JOB_QUEUE_TAIL_BARRIER, WORKER_CMD_RETAIL_NAMES, WORKER_CMD_TOC, JobQueueStage,
    order_player_side_stages, worker_cmd_name,
};

const WORKER_CMDS: usize = 21;

/// The sets that write vertices; the last commands must be exactly these.
const VERTEX_WRITERS: [JobQueueStage; 5] = [
    JobQueueStage::GlassVerts,
    JobQueueStage::MarkVerts,
    JobQueueStage::FxVerts,
    JobQueueStage::SmodelCache,
    JobQueueStage::SkinModel,
];

pub fn worker_command_layout() -> Result<(), String> {
    for (what, len) in [
        ("WORKER_CMD_TOC", WORKER_CMD_TOC.len()),
        ("WORKER_CMD_RETAIL_NAMES", WORKER_CMD_RETAIL_NAMES.len()),
    ] {
        if len != WORKER_CMDS {
            return Err(format!("{what} len {len} (expected {WORKER_CMDS})"));
        }
    }
    for (pos, set) in WORKER_CMD_TOC.iter().enumerate() {
        let number = *set as u8;
        if usize::from(number) != pos {
            return Err(format!(
                "WorkerCmdSet discriminant {number} at TOC[{pos}] (type number is the index)"
            ));
        }
        let (name, listed) = (worker_cmd_name(*set), WORKER_CMD_RETAIL_NAMES[pos]);
        if name != listed {
            return Err(format!(
                "name for {set:?} is {name:?} but table row {pos} says {listed:?}"
            ));
        }
    }
    // The two draw-list builders are never ordered against each other in either direction.
    for &(later, earlier) in WORKER_CMD_AFTER {
        let (a, b) = (JobQueueStage::SkinModel, JobQueueStage::CellStatic);
        if (later, earlier) == (a, b) {
            return Err(
                "WORKER_CMD_AFTER puts SkinModel after CellStatic, breaking the Present chain"
                    .into(),
            );
        }
        if (later, earlier) == (b, a) {
            return Err(
                "WORKER_CMD_AFTER puts CellStatic after SkinModel, breaking the Present chain"
                    .into(),
            );
        }
    }
    if JOB_QUEUE_TAIL_BARRIER != VERTEX_WRITERS.as_slice() {
        return Err(format!(
            "WORKER_CMD_END_FENCE {JOB_QUEUE_TAIL_BARRIER:?} != vertex writers {VERTEX_WRITERS:?}"
        ));
    }
    Ok(())
}

/// Put probes in two of the worker-command sets and check they run between the client's Send and
/// Ui stages, which is where Present sits.
pub fn worker_command_inside_display() -> Result<(), String> {
    let mut harness = bare_app();
    order_player_side_stages(&mut harness);
    probe_client_sets(&mut harness);
    harness.add_systems(
        Update,
        note("W:cell_static").in_set(JobQueueStage::CellStatic),
    );
    harness.add_systems(Update, note("W:skin_model").in_set(JobQueueStage::SkinModel));
    harness.world_mut().run_schedule(Update);
    let trace = trace_of(&harness);
    let (send, ui) = (trace.position("C:Send")?, trace.position("C:Ui")?);
    for label in ["W:cell_static", "W:skin_model"] {
        let at = trace.position(label)?;
        if !(send < at && at < ui) {
            let name = label.trim_start_matches("W:");
            return Err(format!(
                "{name} not inside Present (after Send, before Ui): {:?}",
                trace.0
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_table_holds() {
        assert_eq!(worker_command_layout(), Ok(()));
    }

    #[test]
    fn worker_commands_run_inside_present() {
        assert_eq!(worker_command_inside_display(), Ok(()));
    }

    #[test]
    fn the_vertex_writers_are_the_last_five_distinct_sets() {
        let mut seen = VERTEX_WRITERS.to_vec();
        seen.dedup();
        assert_eq!(seen.len(), VERTEX_WRITERS.len());
        assert_eq!(JOB_QUEUE_TAIL_BARRIER.len(), VERTEX_WRITERS.len());
    }
}
