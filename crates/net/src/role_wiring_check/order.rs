//! The authority and client sets run in the order their tables list them.

use bevy::prelude::*;

use super::probe::{
    authority_expected, bare_app, client_expected, probe_authority_sets, probe_client_sets,
    trace_of,
};
use crate::tick_sets::{
    SERVER_STAGES, CLIENT_STAGES, order_server_side_stages, order_player_side_stages,
};

const AUTHORITY_SETS: usize = 7;
const CLIENT_SETS: usize = 10;

pub fn host_stage_sequence_check() -> Result<(), String> {
    let mut harness = bare_app();
    order_server_side_stages(&mut harness);
    probe_authority_sets(&mut harness);
    harness.world_mut().run_schedule(FixedUpdate);
    let (ran, expected) = (trace_of(&harness).0, authority_expected());
    if ran == expected {
        Ok(())
    } else {
        Err(format!(
            "authority FixedUpdate order drifted from schedule.rs TOC: {ran:?} != {expected:?}"
        ))
    }
}

pub fn receiver_directory_sequence() -> Result<(), String> {
    let mut harness = bare_app();
    order_player_side_stages(&mut harness);
    probe_client_sets(&mut harness);
    harness.world_mut().run_schedule(Update);
    let (ran, expected) = (trace_of(&harness).0, client_expected());
    if ran == expected {
        Ok(())
    } else {
        Err(format!(
            "client Update order drifted from schedule.rs TOC: {ran:?} != {expected:?}"
        ))
    }
}

pub fn directory_sizes() -> Result<(), String> {
    if SERVER_STAGES.len() != AUTHORITY_SETS {
        return Err(format!(
            "AUTHORITY_TOC len {} (expected {AUTHORITY_SETS})",
            SERVER_STAGES.len()
        ));
    }
    if CLIENT_STAGES.len() != CLIENT_SETS {
        return Err(format!(
            "CLIENT_TOC len {} (expected {CLIENT_SETS})",
            CLIENT_STAGES.len()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sets_run_in_table_order() {
        assert_eq!(host_stage_sequence_check(), Ok(()));
        assert_eq!(receiver_directory_sequence(), Ok(()));
    }

    #[test]
    fn the_tables_have_their_lengths() {
        assert_eq!(directory_sizes(), Ok(()));
    }

    #[test]
    fn the_expected_labels_follow_the_tables() {
        assert_eq!(authority_expected().len(), AUTHORITY_SETS);
        assert_eq!(authority_expected()[0], "A:Advance");
        assert_eq!(client_expected().len(), CLIENT_SETS);
        assert_eq!(client_expected()[0], "C:Load");
    }
}
