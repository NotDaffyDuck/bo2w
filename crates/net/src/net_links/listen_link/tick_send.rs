//! The sending half of the loopback: one frame per authority tick.

use super::tick_queue::LocalHostTickPipe;
use crate::client::run_ahead::InputCommandNumber;
use crate::net_links::tick_frames::{Transport, LinkIoFailure, frame_for_confirmed_tick};
use crate::{GuaranteedEnvelope, HostCardSlotMsg, HostGameAnnouncement, HostScreenFlash, HostMenuOpenMsg, HostSoundCue};
use sim::{ClientId, Snapshot, StepInputs};

impl LocalHostTickPipe {
    /// Put this tick's frame on the queue: the snapshot delta, and the server-to-client messages that ride with it.
    #[expect(clippy::too_many_arguments)]
    pub fn transmit_step(
        &mut self,
        tick_input: &StepInputs,
        world: &Snapshot,
        receipt_confirmed: Vec<(ClientId, InputCommandNumber)>,
        sounds: Vec<HostSoundCue>,
        scores: Option<String>,
        card_slots: Vec<HostCardSlotMsg>,
        open_menus: Vec<HostMenuOpenMsg>,
        hud_splashes: Vec<HostScreenFlash>,
        game_notifies: Vec<HostGameAnnouncement>,
        guaranteed: GuaranteedEnvelope,
    ) -> Result<(), LinkIoFailure> {
        let mut frame = frame_for_confirmed_tick(&mut self.encode, tick_input, world, receipt_confirmed);
        frame.host_sound_triggers = sounds;
        frame.board_text = scores;
        frame.host_nameplate_entries = card_slots;
        frame.host_menu_openings = open_menus;
        frame.flash_rows = hud_splashes;
        frame.alert_rows = game_notifies;
        frame.reliable = guaranteed;
        self.queue.send(&frame)?;
        self.dispatched += 1;
        Ok(())
    }
}
