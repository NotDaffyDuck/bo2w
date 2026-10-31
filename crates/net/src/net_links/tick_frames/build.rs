use sim::{ClientId, Snapshot, StepInputs};

use super::payload::Frame;
use crate::FrameDiffWriter;
use crate::client::run_ahead::InputCommandNumber;
use crate::net_links::crossing_fields::players_hash;
use crate::net_links::once_queue::GuaranteedEnvelope;

/// The frame for a tick with no receipts_batch and nothing reliable owed.
pub fn frame_for_tick(
    coder: &mut FrameDiffWriter,
    step: &StepInputs,
    state: &Snapshot,
) -> Frame {
    frame_for_confirmed_tick(coder, step, state, Vec::new())
}

pub fn frame_for_confirmed_tick(
    coder: &mut FrameDiffWriter,
    step: &StepInputs,
    state: &Snapshot,
    receipts_batch: Vec<(ClientId, InputCommandNumber)>,
) -> Frame {
    frame_for_confirmed_tick_guaranteed(
        coder,
        step,
        state,
        receipts_batch,
        GuaranteedEnvelope::has_no_debt(),
    )
}

/// The frame for a tick: its snapshot delta against what `coder` last sent, the step's inputs, and the
/// receipts_batch and reliable rows given. The svc lists start empty; the caller fills the ones it has.
pub fn frame_for_confirmed_tick_guaranteed(
    coder: &mut FrameDiffWriter,
    step: &StepInputs,
    state: &Snapshot,
    receipts_batch: Vec<(ClientId, InputCommandNumber)>,
    reliable: GuaranteedEnvelope,
) -> Frame {
    let world_prop_bytes = coder
        .pack_scene_props(state.tick, &state.meta.world_objects)
        .to_vec();
    Frame {
        tick: state.tick,
        condition_checksum: players_hash(&state.players),
        inputs: step.cmds.clone(),
        actions: step.actions.clone(),
        receipts_batch,
        frame_diff_section: coder.pack(state),
        meta_rows: state.meta.clone(),
        world_prop_bytes,
        reliable,
        host_sound_triggers: Vec::new(),
        board_text: None,
        host_nameplate_entries: Vec::new(),
        host_menu_openings: Vec::new(),
        flash_rows: Vec::new(),
        alert_rows: Vec::new(),
    }
}

/// A 64-bit FNV-1a hash of the bytes of `snapshot` sent as a first frame with no input: the same
/// world gives the same number on any machine.
pub fn host_world_state_checksum(state: &Snapshot) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let frame = frame_for_tick(&mut FrameDiffWriter::new(), &StepInputs::default(), state);
    frame.wire_bytes().into_iter().fold(OFFSET_BASIS, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(PRIME)
    })
}

#[cfg(test)]
mod tests {
    use sim::Tick;

    use super::*;

    #[test]
    fn a_frame_takes_its_tick_from_the_snapshot_and_starts_with_no_svc_rows() {
        let frame = frame_for_tick(
            &mut FrameDiffWriter::new(),
            &StepInputs::default(),
            &Snapshot::unpublished(Tick(9)),
        );
        assert_eq!(frame.tick, Tick(9));
        assert!(frame.inputs.is_empty() && frame.receipts_batch.is_empty());
        assert_eq!(frame.reliable, GuaranteedEnvelope::has_no_debt());
        assert!(frame.host_sound_triggers.is_empty() && frame.board_text.is_none());
        assert!(frame.host_nameplate_entries.is_empty() && frame.host_menu_openings.is_empty());
        assert!(frame.flash_rows.is_empty() && frame.alert_rows.is_empty());
    }

    #[test]
    fn acks_are_looked_up_by_client() {
        let receipts_batch = vec![(ClientId(1), InputCommandNumber(4)), (ClientId(3), InputCommandNumber(9))];
        let frame = frame_for_confirmed_tick(
            &mut FrameDiffWriter::new(),
            &StepInputs::default(),
            &Snapshot::unpublished(Tick(1)),
            receipts_batch,
        );
        assert_eq!(frame.receipt_for(ClientId(3)), Some(InputCommandNumber(9)));
        assert_eq!(frame.receipt_for(ClientId(2)), None);
    }

    #[test]
    fn the_hash_is_steady_and_follows_the_world() {
        let one = Snapshot::unpublished(Tick(1));
        let two = Snapshot::unpublished(Tick(2));
        assert_eq!(host_world_state_checksum(&one), host_world_state_checksum(&one));
        assert_ne!(host_world_state_checksum(&one), host_world_state_checksum(&two));
    }

    #[test]
    fn the_section_sizes_add_up_to_the_encoded_length() {
        let frame = frame_for_tick(
            &mut FrameDiffWriter::new(),
            &StepInputs::default(),
            &Snapshot::unpublished(Tick(2)),
        );
        assert_eq!(frame.section_bytes().total_len, frame.wire_bytes().len());
    }
}
