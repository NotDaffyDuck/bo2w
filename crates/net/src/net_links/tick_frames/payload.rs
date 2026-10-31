use playerstate_t6::game::InputCommand;
use sim::{PlayerIntent, ClientId, SnapshotMeta, Tick, StepInputs};

use crate::client::run_ahead::InputCommandNumber;
use crate::net_links::snapshot_diff::{FrameDiff, unpack_input_record, pack_input_record};
use crate::net_links::once_queue::{GuaranteedEnvelope, unpack_guaranteed_body};
use crate::net_links::once_queue::pack_guaranteed_body;
use crate::net_links::snapshot_wire::{read_intent_list, parse_state_header};
use crate::net_links::snapshot_wire::{write_intent_list, pack_state_header_parts};
use crate::net_links::snapshot_wire::{MetaSectionSizes, WorldPropStreamDecoder};
use crate::net_links::byte_codec::{PacketDecodeFault, ByteCursorReader, ByteSinkWriter};

/// Everything the server says about one tick.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub tick: Tick,
    /// A hash of the players' state, for a client to check its own copy against.
    pub condition_checksum: u32,
    pub inputs: Vec<(ClientId, InputCommand)>,
    pub actions: Vec<(ClientId, PlayerIntent)>,
    /// The last command of each client the server has applied.
    pub receipts_batch: Vec<(ClientId, InputCommandNumber)>,
    pub frame_diff_section: FrameDiff,
    pub meta_rows: SnapshotMeta,
    pub world_prop_bytes: Vec<u8>,
    pub reliable: GuaranteedEnvelope,
    pub host_sound_triggers: Vec<crate::HostSoundCue>,
    pub board_text: Option<String>,
    pub host_nameplate_entries: Vec<crate::HostCardSlotMsg>,
    pub host_menu_openings: Vec<crate::HostMenuOpenMsg>,
    pub flash_rows: Vec<crate::HostScreenFlash>,
    pub alert_rows: Vec<crate::HostGameAnnouncement>,
}

/// How many bytes each part of an encoded frame took.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameChunkLengths {
    pub header_len: usize,
    pub frame_diff_section: usize,
    pub meta: MetaSectionSizes,
    pub reliable_len: usize,
    pub svc_len: usize,
    pub total_len: usize,
}

impl FrameChunkLengths {
    /// The parts added up; equal to `total` when no byte went unaccounted for.
    pub fn labeled_total(self) -> usize {
        self.header_len + self.frame_diff_section + self.meta.sum_bytes() + self.reliable_len + self.svc_len
    }
}

/// The server-to-client extras that follow the reliable rows, as decoded.
struct HostExtras {
    sounds: Vec<crate::HostSoundCue>,
    scores: Option<String>,
    cards: Vec<crate::HostCardSlotMsg>,
    menus: Vec<crate::HostMenuOpenMsg>,
    splashes: Vec<crate::HostScreenFlash>,
    notices: Vec<crate::HostGameAnnouncement>,
}

/// The most entries a count-prefixed list is given room for before its bytes have been seen, so a
/// count lying about itself cannot make the reader reserve a huge buffer.
const PREALLOCATED_ENTRIES: usize = 64;

impl Frame {
    /// Appends the encoded frame to dst.
    pub fn pack(&self, dst: &mut ByteSinkWriter) {
        let _sizes = self.write_sections(dst);
    }

    /// Encode into a scratch buffer and report the size of each part.
    pub fn section_bytes(&self) -> FrameChunkLengths {
        self.write_sections(&mut ByteSinkWriter::new())
    }

    fn write_sections(&self, dst: &mut ByteSinkWriter) -> FrameChunkLengths {
        let start = dst.len();
        let header = Self::measured(dst, |out| self.write_header(out));
        let frame_diff_section = Self::measured(dst, |out| self.frame_diff_section.pack(out));
        let meta = pack_state_header_parts(dst, &self.meta_rows, &self.world_prop_bytes);
        let reliable = Self::measured(dst, |out| {
            let rel = &self.reliable;
            pack_guaranteed_body(out, rel.confirmed_up_to, &rel.entries, rel.evicted_stale_count);
        });
        let svc = Self::measured(dst, |out| self.write_svc(out));
        FrameChunkLengths {
            header_len: header,
            frame_diff_section,
            meta,
            reliable_len: reliable,
            svc_len: svc,
            total_len: dst.len() - start,
        }
    }

    /// Runs `write` and returns how many bytes it appended to `dst`.
    fn measured(dst: &mut ByteSinkWriter, write: impl FnOnce(&mut ByteSinkWriter)) -> usize {
        let before = dst.len();
        write(dst);
        dst.len() - before
    }

    /// The tick, the state hash, the commands, the actions and the receipts_batch.
    fn write_header(&self, dst: &mut ByteSinkWriter) {
        [self.tick.0, self.condition_checksum]
            .into_iter()
            .for_each(|word| dst.put_u32(word));
        Self::put_count(dst, self.inputs.len(), "too many commands for the wire");
        for (who, cmd) in &self.inputs {
            dst.put_u32(who.0);
            pack_input_record(dst, cmd);
        }
        write_intent_list(dst, &self.actions);
        Self::put_count(dst, self.receipts_batch.len(), "too many acks for the wire");
        for (who, seq) in &self.receipts_batch {
            [who.0, seq.0].into_iter().for_each(|word| dst.put_u32(word));
        }
    }

    /// The server-to-client extras that follow the reliable rows.
    fn write_svc(&self, dst: &mut ByteSinkWriter) {
        use crate::{notice_msgs, card_msgs, board_msgs, local_sound_msgs};
        local_sound_msgs::pack_host_sound_triggers(dst, &self.host_sound_triggers);
        board_msgs::encode_svc_scores(dst, self.board_text.as_deref());
        card_msgs::pack_host_nameplates(dst, &self.host_nameplate_entries);
        card_msgs::pack_host_menu_openings(dst, &self.host_menu_openings);
        card_msgs::pack_host_overlay_flashes(dst, &self.flash_rows);
        notice_msgs::pack_host_game_alerts(dst, &self.alert_rows);
    }

    /// A list length goes out as a u16.
    fn put_count(
        dst: &mut ByteSinkWriter,
        len: usize,
        complaint: &str,
    ) {
        debug_assert!(len <= usize::from(u16::MAX), "{complaint}");
        dst.put_u16(len as u16);
    }

    /// Reads a u16 count and then that many `(client, value)` rows, reserving room only up to
    /// `PREALLOCATED_ENTRIES` before any row has been seen.
    fn read_rows<T>(
        src: &mut ByteCursorReader<'_>,
        mut row: impl FnMut(&mut ByteCursorReader<'_>, ClientId) -> Result<T, PacketDecodeFault>,
    ) -> Result<Vec<(ClientId, T)>, PacketDecodeFault> {
        let count = usize::from(src.get_u16()?);
        let mut rows = Vec::with_capacity(count.min(PREALLOCATED_ENTRIES));
        for _ in 0..count {
            let who = ClientId(src.read_word32()?);
            rows.push((who, row(src, who)?));
        }
        Ok(rows)
    }

    pub fn decode(
        src: &mut ByteCursorReader<'_>,
        objects: &mut WorldPropStreamDecoder,
    ) -> Result<Self, PacketDecodeFault> {
        let [tick, condition_checksum] = [src.read_word32()?, src.read_word32()?];
        let cmds = Self::read_rows(src, |src, _| unpack_input_record(src))?;
        let actions = read_intent_list(src)?;
        let receipts_batch = Self::read_rows(src, |src, _| Ok(InputCommandNumber(src.read_word32()?)))?;
        let frame_diff_section = FrameDiff::decode(src)?;
        let (meta_rows, world_prop_bytes) = parse_state_header(src, objects)?;
        let reliable = unpack_guaranteed_body(src)?;
        let extras = Self::read_svc(src)?;
        Ok(Self {
            tick: Tick(tick),
            condition_checksum,
            inputs: cmds,
            actions,
            receipts_batch,
            frame_diff_section,
            meta_rows,
            world_prop_bytes,
            reliable,
            host_sound_triggers: extras.sounds,
            board_text: extras.scores,
            host_nameplate_entries: extras.cards,
            host_menu_openings: extras.menus,
            flash_rows: extras.splashes,
            alert_rows: extras.notices,
        })
    }

    /// The server-to-client extras, in the order `write_svc` put them.
    fn read_svc(src: &mut ByteCursorReader<'_>) -> Result<HostExtras, PacketDecodeFault> {
        use crate::{notice_msgs, card_msgs, board_msgs, local_sound_msgs};
        // Field order is read order.
        Ok(HostExtras {
            sounds: local_sound_msgs::decode_svc_sounds(src)?,
            scores: board_msgs::parse_scoreboard_message(src)?,
            cards: card_msgs::decode_svc_card_slots(src)?,
            menus: card_msgs::decode_svc_open_menus(src)?,
            splashes: card_msgs::decode_svc_hud_splashes(src)?,
            notices: notice_msgs::parse_host_game_alerts(src)?,
        })
    }

    /// The last command of `peer` the server had applied by this frame.
    pub fn receipt_for(
        &self,
        peer: ClientId,
    ) -> Option<InputCommandNumber> {
        self.receipts_batch
            .iter()
            .find_map(|&(id, seq)| (id == peer).then_some(seq))
    }

    /// The whole encoded frame.
    pub fn wire_bytes(&self) -> Vec<u8> {
        let mut sink = ByteSinkWriter::new();
        self.write_sections(&mut sink);
        sink.into_bytes()
    }

    /// The input this frame carries, as the simulation takes it.
    pub fn tick_input(&self) -> StepInputs {
        let (commands, intents) = (self.inputs.to_vec(), self.actions.to_vec());
        StepInputs { cmds: commands, actions: intents }
    }
}
