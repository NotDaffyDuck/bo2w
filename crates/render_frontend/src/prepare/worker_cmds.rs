//! The frontend's worker command queues: twenty-one fixed-record ring
//! buffers, one per command type, that the main thread fills while the
//! visibility and skinning work is decided and the workers drain.

pub const WORKER_CMD_COUNT: usize = 21;

pub const WORKER_CMD_CELL_DYN_BRUSH: u8 = 0;
pub const WORKER_CMD_CELL_DYN_MODEL: u8 = 1;
pub const WORKER_CMD_CELL_SCENE_ENT: u8 = 2;
pub const WORKER_CMD_DPVS_ENT: u8 = 3;
pub const WORKER_CMD_BOUND_ENT: u8 = 4;
pub const WORKER_CMD_SPOT_SHADOW_ENT: u8 = 5;
pub const WORKER_CMD_SMODELCACHE: u8 = 0x11;

pub const WORKER_CMD_STRIDE: usize = 0x80;

/// The largest record of any type.
pub const WORKER_CMD_MAX_DATA_SIZE: usize = 0x4c;

/// Entries still queued beyond which one `process` call takes a batch.
const BATCH_THRESHOLD: i32 = 0x13;
const BATCH_LEN: usize = 10;

/// Record size and ring capacity, both in bytes, of one command type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkerCmdInit {
    pub data_size: u32,
    pub buf_size: u32,
}

const fn init(data_size: u32, buf_size: u32) -> WorkerCmdInit {
    WorkerCmdInit {
        data_size,
        buf_size,
    }
}

pub const WORKER_CMD_INIT: [WorkerCmdInit; WORKER_CMD_COUNT] = [
    init(0x0c, 0x1800),
    init(0x0c, 0x1800),
    init(0x0c, 0x1800),
    init(0x10, 0x8000),
    init(0x04, 0x0400),
    init(0x08, 0x0800),
    init(0x0c, 0x0c),
    init(0x04, 0x04),
    init(0x0c, 0x0c),
    init(0x0c, 0x0c00),
    init(0x04, 0x04),
    init(0x18, 0x1800),
    init(0x10, 0x10),
    init(0x4c, 0x4c),
    init(0x48, 0x90),
    init(0x3c, 0x78),
    init(0x48, 0x90),
    init(0x04, 0x0800),
    init(0x24, 0x9000),
    init(0x04, 0x04),
    init(0x08, 0x18),
];

/// Types 0x0d to 0x10 are not run while the device is lost.
#[must_use]
pub fn worker_cmd_dispatch_skips_device_lost(typ: u8) -> bool {
    (0x0d..=0x10).contains(&typ)
}

/// What a command type has to wait for before its records may run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerCmdBusy {
    Never,
    RenderThread,
    RenderThreadOrQueued6Or7,
    Queued7,
    Queued11,
    Queued14,
    Queued11Or12OrEndFence,
    EndFence,
}

pub const WORKER_CMD_BUSY: [WorkerCmdBusy; WORKER_CMD_COUNT] = {
    use WorkerCmdBusy::*;
    [
        Never,
        Never,
        Never,
        RenderThread,
        RenderThread,
        RenderThread,
        Never,
        Never,
        RenderThreadOrQueued6Or7,
        Never,
        Never,
        Queued7,
        Queued11,
        Queued11Or12OrEndFence,
        EndFence,
        Queued14,
        EndFence,
        EndFence,
        EndFence,
        Queued7,
        Never,
    ]
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorkerCmdBusyInput {
    pub is_render_thread: bool,
    /// None when no end fence exists to wait for.
    pub end_fence_pending: Option<bool>,
}

fn le_u16(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn le_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SkinCachedStaticModelCmd {
    pub cache_index: u16,
    pub first_patch_vert: u16,
}

impl SkinCachedStaticModelCmd {
    #[must_use]
    pub fn to_bytes(self) -> [u8; 4] {
        let [a, b] = self.cache_index.to_le_bytes();
        let [c, d] = self.first_patch_vert.to_le_bytes();
        [a, b, c, d]
    }

    #[must_use]
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        let bytes: [u8; 4] = data.try_into().ok()?;
        Some(Self {
            cache_index: le_u16(&bytes, 0),
            first_patch_vert: le_u16(&bytes, 2),
        })
    }
}

/// One cell to cull the dynamic content of against a range of planes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellFrustumWorkerCmd {
    pub cell: u32,
    pub plane_count: u8,
    pub plane_begin: u8,
    pub view: u16,
}

impl CellFrustumWorkerCmd {
    /// The record is twelve bytes with its first four unused.
    #[must_use]
    pub fn to_bytes(self) -> [u8; 12] {
        let mut out = [0u8; 12];
        out[4..8].copy_from_slice(&self.cell.to_le_bytes());
        out[8] = self.plane_count;
        out[9] = self.plane_begin;
        out[10..].copy_from_slice(&self.view.to_le_bytes());
        out
    }

    #[must_use]
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        let bytes: [u8; 12] = data.try_into().ok()?;
        Some(Self {
            cell: le_u32(&bytes, 4),
            plane_count: bytes[8],
            plane_begin: bytes[9],
            view: le_u16(&bytes, 10),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DpvsEntWorkerCmd {
    pub scene_dobj: u32,
    pub plane_count: u16,
    pub cell: u16,
}

impl DpvsEntWorkerCmd {
    /// Sixteen bytes; the model object first, the planes and cell at 8.
    #[must_use]
    pub fn to_bytes(self) -> [u8; 16] {
        let mut out = [0u8; 16];
        out[..4].copy_from_slice(&self.scene_dobj.to_le_bytes());
        out[8..10].copy_from_slice(&self.plane_count.to_le_bytes());
        out[10..12].copy_from_slice(&self.cell.to_le_bytes());
        out
    }

    #[must_use]
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        let bytes: [u8; 16] = data.try_into().ok()?;
        Some(Self {
            scene_dobj: le_u32(&bytes, 0),
            plane_count: le_u16(&bytes, 8),
            cell: le_u16(&bytes, 10),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoundEntWorkerCmd {
    pub scene_dobj: u32,
}

impl BoundEntWorkerCmd {
    #[must_use]
    pub fn to_bytes(self) -> [u8; 4] {
        self.scene_dobj.to_le_bytes()
    }

    #[must_use]
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        let bytes: [u8; 4] = data.try_into().ok()?;
        Some(Self {
            scene_dobj: u32::from_le_bytes(bytes),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpotShadowEntWorkerCmd {
    pub scene_ent: u32,
    pub light: u32,
}

impl SpotShadowEntWorkerCmd {
    #[must_use]
    pub fn to_bytes(self) -> [u8; 8] {
        let mut out = [0u8; 8];
        out[..4].copy_from_slice(&self.scene_ent.to_le_bytes());
        out[4..].copy_from_slice(&self.light.to_le_bytes());
        out
    }

    #[must_use]
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        let bytes: [u8; 8] = data.try_into().ok()?;
        Some(Self {
            scene_ent: le_u32(&bytes, 0),
            light: le_u32(&bytes, 4),
        })
    }
}

/// Queues the cell for the brush, model and scene-entity workers alike.
pub fn enqueue_cell_frustum_cmds(
    queues: &mut WorkerCmdQueues,
    cmd: CellFrustumWorkerCmd,
) -> Result<[AddWorkerCmd; 3], WorkerCmdError> {
    let record = cmd.to_bytes();
    Ok([
        queues.add(WORKER_CMD_CELL_DYN_BRUSH, &record)?,
        queues.add(WORKER_CMD_CELL_DYN_MODEL, &record)?,
        queues.add(WORKER_CMD_CELL_SCENE_ENT, &record)?,
    ])
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddWorkerCmd {
    Queued,
    /// The ring was full: the caller does the work itself, now.
    OverflowInline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerCmdError {
    Type,
    DataSize,
}

/// One command type's ring of fixed-size records.
///
/// `in_size` counts the records added and not yet finished, `out_size` the
/// ones not yet taken by a worker; `in_size` is what bounds the ring.
struct Ring {
    record_len: usize,
    capacity: i32,
    records: Vec<u8>,
    /// Slot the next record is read from, and the one the next is written to.
    read_slot: i32,
    write_slot: i32,
    in_size: i32,
    out_size: i32,
}

impl Ring {
    fn new(init: WorkerCmdInit) -> Self {
        debug_assert!(init.data_size > 0);
        debug_assert!(init.buf_size % init.data_size == 0);
        debug_assert!(init.data_size as usize <= WORKER_CMD_MAX_DATA_SIZE);
        Self {
            record_len: init.data_size as usize,
            capacity: (init.buf_size / init.data_size) as i32,
            records: vec![0; init.buf_size as usize],
            read_slot: 0,
            write_slot: 0,
            in_size: 0,
            out_size: 0,
        }
    }

    fn reset(&mut self) {
        self.read_slot = 0;
        self.write_slot = 0;
        self.in_size = 0;
        self.out_size = 0;
    }

    fn step(&self, slot: i32) -> i32 {
        if slot + 1 == self.capacity { 0 } else { slot + 1 }
    }

    fn slot_bytes(&self, slot: i32) -> std::ops::Range<usize> {
        let start = slot as usize * self.record_len;
        start..start + self.record_len
    }

    fn push(&mut self, record: &[u8]) -> AddWorkerCmd {
        if self.in_size >= self.capacity {
            return AddWorkerCmd::OverflowInline;
        }
        let at = self.slot_bytes(self.write_slot);
        self.records[at].copy_from_slice(record);
        self.write_slot = self.step(self.write_slot);
        self.in_size += 1;
        self.out_size += 1;
        AddWorkerCmd::Queued
    }

    /// Runs `exec` on the next `take` records in order and retires them.
    fn run(&mut self, take: usize, exec: &mut impl FnMut(&mut [u8])) {
        let mut scratch = [0u8; WORKER_CMD_MAX_DATA_SIZE];
        let record = &mut scratch[..self.record_len];
        for _ in 0..take {
            record.copy_from_slice(&self.records[self.slot_bytes(self.read_slot)]);
            exec(record);
            self.read_slot = self.step(self.read_slot);
        }
        self.in_size -= take as i32;
    }
}

pub struct WorkerCmdQueues {
    rings: [Ring; WORKER_CMD_COUNT],
}

impl Default for WorkerCmdQueues {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkerCmdQueues {
    #[must_use]
    pub fn new() -> Self {
        Self {
            rings: std::array::from_fn(|typ| Ring::new(WORKER_CMD_INIT[typ])),
        }
    }

    fn ring(&self, typ: u8) -> Option<&Ring> {
        self.rings.get(usize::from(typ))
    }

    #[must_use]
    pub fn data_size(&self, typ: u8) -> Option<u32> {
        self.ring(typ).map(|ring| ring.record_len as u32)
    }

    #[must_use]
    pub fn buf_count(&self, typ: u8) -> Option<i32> {
        self.ring(typ).map(|ring| ring.capacity)
    }

    #[must_use]
    pub fn in_size(&self, typ: u8) -> Option<i32> {
        self.ring(typ).map(|ring| ring.in_size)
    }

    pub fn reset_type(&mut self, typ: u8) -> Result<(), WorkerCmdError> {
        self.rings
            .get_mut(usize::from(typ))
            .ok_or(WorkerCmdError::Type)?
            .reset();
        Ok(())
    }

    pub fn add(&mut self, typ: u8, data: &[u8]) -> Result<AddWorkerCmd, WorkerCmdError> {
        let ring = self
            .rings
            .get_mut(usize::from(typ))
            .ok_or(WorkerCmdError::Type)?;
        if data.len() != ring.record_len {
            return Err(WorkerCmdError::DataSize);
        }
        Ok(ring.push(data))
    }

    /// Whether the type has to wait now: None if it never waits, Some(true)
    /// for a type that does not exist.
    #[must_use]
    pub fn output_busy(&self, typ: u8, input: WorkerCmdBusyInput) -> Option<bool> {
        let Some(&rule) = WORKER_CMD_BUSY.get(usize::from(typ)) else {
            return Some(true);
        };
        busy_now(rule, |other| self.in_size(other).unwrap_or(0) > 0, input)
    }

    /// Runs the next record (or a batch, when many are queued) of the type
    /// unless its busy rule says to wait. True if something ran.
    pub fn process_type(
        &mut self,
        typ: u8,
        input: WorkerCmdBusyInput,
        exec: impl FnMut(&mut [u8]),
    ) -> Result<bool, WorkerCmdError> {
        match self.output_busy(typ, input) {
            None => self.process(typ, None, exec),
            Some(wait) => {
                let mut busy = move |_: &[u8]| wait;
                self.process(typ, Some(&mut busy), exec)
            }
        }
    }

    /// With `busy`, looks at the next record and runs it only if `busy`
    /// says not to wait; without, runs it unconditionally and, if more than
    /// a score are queued, nine more behind it.
    pub fn process(
        &mut self,
        typ: u8,
        busy: Option<&mut dyn FnMut(&[u8]) -> bool>,
        mut exec: impl FnMut(&mut [u8]),
    ) -> Result<bool, WorkerCmdError> {
        let ring = self
            .rings
            .get_mut(usize::from(typ))
            .ok_or(WorkerCmdError::Type)?;
        if ring.out_size <= 0 {
            return Ok(false);
        }
        let take = match busy {
            Some(busy) => {
                if busy(&ring.records[ring.slot_bytes(ring.read_slot)]) {
                    return Ok(false);
                }
                1
            }
            None if ring.out_size - 1 > BATCH_THRESHOLD => BATCH_LEN,
            None => 1,
        };
        ring.out_size -= take as i32;
        ring.run(take, &mut exec);
        Ok(true)
    }

    /// Drains the type, stopping early if it has to wait.
    pub fn wait_of_type(
        &mut self,
        typ: u8,
        input: WorkerCmdBusyInput,
        mut exec: impl FnMut(&mut [u8]),
    ) -> Result<(), WorkerCmdError> {
        while self.in_size(typ).unwrap_or(0) > 0 {
            if !self.process_type(typ, input, &mut exec)? {
                break;
            }
        }
        Ok(())
    }
}

fn busy_now(
    rule: WorkerCmdBusy,
    queued: impl Fn(u8) -> bool,
    input: WorkerCmdBusyInput,
) -> Option<bool> {
    let end_fence = input.end_fence_pending.unwrap_or(false);
    let wait = match rule {
        WorkerCmdBusy::Never => return None,
        WorkerCmdBusy::RenderThread => input.is_render_thread,
        WorkerCmdBusy::RenderThreadOrQueued6Or7 => {
            queued(6) || queued(7) || input.is_render_thread
        }
        WorkerCmdBusy::Queued7 => queued(7),
        WorkerCmdBusy::Queued11 => queued(11),
        WorkerCmdBusy::Queued14 => queued(14),
        WorkerCmdBusy::Queued11Or12OrEndFence => queued(11) || queued(12) || end_fence,
        WorkerCmdBusy::EndFence => end_fence,
    };
    Some(wait)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RENDER: WorkerCmdBusyInput = WorkerCmdBusyInput {
        is_render_thread: true,
        end_fence_pending: None,
    };
    const IDLE: WorkerCmdBusyInput = WorkerCmdBusyInput {
        is_render_thread: false,
        end_fence_pending: None,
    };

    fn drained(queues: &mut WorkerCmdQueues, typ: u8) -> Vec<Vec<u8>> {
        let mut seen = Vec::new();
        queues
            .wait_of_type(typ, IDLE, |record| seen.push(record.to_vec()))
            .unwrap();
        seen
    }

    #[test]
    fn every_record_size_fits_its_ring_and_the_scratch() {
        for (typ, init) in WORKER_CMD_INIT.iter().enumerate() {
            assert!(init.data_size > 0, "type {typ}");
            assert_eq!(init.buf_size % init.data_size, 0, "type {typ}");
            assert!(init.data_size as usize <= WORKER_CMD_MAX_DATA_SIZE, "type {typ}");
        }
        let queues = WorkerCmdQueues::new();
        assert_eq!(queues.buf_count(WORKER_CMD_CELL_DYN_BRUSH), Some(0x1800 / 0x0c));
        assert_eq!(queues.data_size(WORKER_CMD_DPVS_ENT), Some(0x10));
        assert_eq!(queues.buf_count(WORKER_CMD_COUNT as u8), None);
    }

    #[test]
    fn records_round_trip_through_their_bytes() {
        let skin = SkinCachedStaticModelCmd {
            cache_index: 0x1234,
            first_patch_vert: 0xbeef,
        };
        assert_eq!(skin.to_bytes(), [0x34, 0x12, 0xef, 0xbe]);
        assert_eq!(SkinCachedStaticModelCmd::from_bytes(&skin.to_bytes()), Some(skin));

        let cell = CellFrustumWorkerCmd {
            cell: 0x0102_0304,
            plane_count: 6,
            plane_begin: 2,
            view: 9,
        };
        let bytes = cell.to_bytes();
        assert_eq!(&bytes[..4], &[0; 4], "the first four bytes are unused");
        assert_eq!(&bytes[4..8], &[4, 3, 2, 1]);
        assert_eq!(bytes[8..], [6, 2, 9, 0]);
        assert_eq!(CellFrustumWorkerCmd::from_bytes(&bytes), Some(cell));

        let dpvs = DpvsEntWorkerCmd {
            scene_dobj: 77,
            plane_count: 5,
            cell: 300,
        };
        assert_eq!(DpvsEntWorkerCmd::from_bytes(&dpvs.to_bytes()), Some(dpvs));
        assert_eq!(&dpvs.to_bytes()[4..8], &[0; 4]);

        let bound = BoundEntWorkerCmd {
            scene_dobj: 0xdead_beef,
        };
        assert_eq!(BoundEntWorkerCmd::from_bytes(&bound.to_bytes()), Some(bound));

        let spot = SpotShadowEntWorkerCmd {
            scene_ent: 3,
            light: 4,
        };
        assert_eq!(spot.to_bytes(), [3, 0, 0, 0, 4, 0, 0, 0]);
        assert_eq!(SpotShadowEntWorkerCmd::from_bytes(&spot.to_bytes()), Some(spot));
    }

    #[test]
    fn a_record_of_the_wrong_length_does_not_decode() {
        assert_eq!(SkinCachedStaticModelCmd::from_bytes(&[0; 3]), None);
        assert_eq!(CellFrustumWorkerCmd::from_bytes(&[0; 11]), None);
        assert_eq!(DpvsEntWorkerCmd::from_bytes(&[0; 17]), None);
        assert_eq!(BoundEntWorkerCmd::from_bytes(&[]), None);
        assert_eq!(SpotShadowEntWorkerCmd::from_bytes(&[0; 4]), None);
    }

    #[test]
    fn add_checks_the_type_and_the_record_size() {
        let mut queues = WorkerCmdQueues::new();
        assert_eq!(
            queues.add(WORKER_CMD_COUNT as u8, &[0; 4]),
            Err(WorkerCmdError::Type)
        );
        assert_eq!(
            queues.add(WORKER_CMD_BOUND_ENT, &[0; 8]),
            Err(WorkerCmdError::DataSize)
        );
        assert_eq!(
            queues.add(WORKER_CMD_BOUND_ENT, &[1; 4]),
            Ok(AddWorkerCmd::Queued)
        );
        assert_eq!(queues.in_size(WORKER_CMD_BOUND_ENT), Some(1));
        assert_eq!(
            queues.reset_type(WORKER_CMD_COUNT as u8),
            Err(WorkerCmdError::Type)
        );
    }

    #[test]
    fn a_full_ring_hands_the_work_back_to_the_caller() {
        let mut queues = WorkerCmdQueues::new();
        // Type 6 holds exactly one twelve-byte record.
        assert_eq!(queues.buf_count(6), Some(1));
        assert_eq!(queues.add(6, &[1; 12]), Ok(AddWorkerCmd::Queued));
        assert_eq!(queues.add(6, &[2; 12]), Ok(AddWorkerCmd::OverflowInline));
        assert_eq!(queues.in_size(6), Some(1));
        assert_eq!(drained(&mut queues, 6), vec![vec![1; 12]]);
        assert_eq!(queues.add(6, &[3; 12]), Ok(AddWorkerCmd::Queued));
    }

    #[test]
    fn records_come_out_in_order_across_the_wrap() {
        let mut queues = WorkerCmdQueues::new();
        let cap = queues.buf_count(WORKER_CMD_BOUND_ENT).unwrap() as u32;
        let mut next = 0u32;
        for round in 0..3 {
            let count = if round == 0 { cap - 3 } else { 5 };
            let expected: Vec<Vec<u8>> = (next..next + count)
                .map(|n| n.to_le_bytes().to_vec())
                .collect();
            for n in next..next + count {
                assert_eq!(
                    queues.add(WORKER_CMD_BOUND_ENT, &n.to_le_bytes()),
                    Ok(AddWorkerCmd::Queued)
                );
            }
            next += count;
            let mut seen = Vec::new();
            while queues
                .process(WORKER_CMD_BOUND_ENT, None, |r| seen.push(r.to_vec()))
                .unwrap()
            {}
            assert_eq!(seen, expected, "round {round}");
        }
        assert_eq!(queues.in_size(WORKER_CMD_BOUND_ENT), Some(0));
    }

    #[test]
    fn reset_empties_a_ring_and_starts_it_over() {
        let mut queues = WorkerCmdQueues::new();
        queues.add(WORKER_CMD_BOUND_ENT, &[9; 4]).unwrap();
        queues.reset_type(WORKER_CMD_BOUND_ENT).unwrap();
        assert_eq!(queues.in_size(WORKER_CMD_BOUND_ENT), Some(0));
        assert_eq!(
            queues.process(WORKER_CMD_BOUND_ENT, None, |_| panic!("ran")),
            Ok(false)
        );
        queues.add(WORKER_CMD_BOUND_ENT, &[7; 4]).unwrap();
        assert_eq!(
            drained(&mut queues, WORKER_CMD_BOUND_ENT),
            vec![vec![7; 4]]
        );
    }

    #[test]
    fn many_queued_records_run_ten_at_a_time() {
        let after_one_process = |queued: u32| {
            let mut queues = WorkerCmdQueues::new();
            for n in 0..queued {
                queues.add(WORKER_CMD_BOUND_ENT, &n.to_le_bytes()).unwrap();
            }
            let mut ran = 0;
            queues
                .process(WORKER_CMD_BOUND_ENT, None, |_| ran += 1)
                .unwrap();
            (ran, queues.in_size(WORKER_CMD_BOUND_ENT).unwrap())
        };
        assert_eq!(after_one_process(1), (1, 0));
        assert_eq!(
            after_one_process(20),
            (1, 19),
            "twenty queued is not yet a batch"
        );
        assert_eq!(after_one_process(21), (10, 11));
        assert_eq!(after_one_process(40), (10, 30));
    }

    #[test]
    fn a_batch_runs_its_records_in_order() {
        let mut queues = WorkerCmdQueues::new();
        for n in 0..25u32 {
            queues.add(WORKER_CMD_BOUND_ENT, &n.to_le_bytes()).unwrap();
        }
        let mut seen = Vec::new();
        queues
            .process(WORKER_CMD_BOUND_ENT, None, |r| {
                seen.push(u32::from_le_bytes(r.try_into().unwrap()));
            })
            .unwrap();
        assert_eq!(seen, (0..10).collect::<Vec<_>>());
    }

    #[test]
    fn a_busy_callback_that_says_wait_leaves_the_record_queued() {
        let mut queues = WorkerCmdQueues::new();
        queues.add(WORKER_CMD_BOUND_ENT, &[5; 4]).unwrap();
        let mut seen_by_busy = Vec::new();
        let mut busy = |record: &[u8]| {
            seen_by_busy.push(record.to_vec());
            true
        };
        let ran = queues.process(WORKER_CMD_BOUND_ENT, Some(&mut busy), |_| panic!("ran"));
        assert_eq!(ran, Ok(false));
        assert_eq!(
            seen_by_busy,
            vec![vec![5; 4]],
            "busy sees the record it is deciding on"
        );
        assert_eq!(queues.in_size(WORKER_CMD_BOUND_ENT), Some(1));

        let mut go = |_: &[u8]| false;
        let ran = queues.process(WORKER_CMD_BOUND_ENT, Some(&mut go), |_| {});
        assert_eq!(ran, Ok(true));
        assert_eq!(queues.in_size(WORKER_CMD_BOUND_ENT), Some(0));
    }

    #[test]
    fn view_workers_wait_for_the_render_thread_to_be_off() {
        let queues = WorkerCmdQueues::new();
        assert_eq!(queues.output_busy(WORKER_CMD_DPVS_ENT, RENDER), Some(true));
        assert_eq!(queues.output_busy(WORKER_CMD_DPVS_ENT, IDLE), Some(false));
        assert_eq!(queues.output_busy(WORKER_CMD_CELL_DYN_MODEL, RENDER), None);
        assert_eq!(queues.output_busy(WORKER_CMD_COUNT as u8, IDLE), Some(true));
    }

    #[test]
    fn a_type_waits_while_the_types_it_depends_on_have_records() {
        let mut queues = WorkerCmdQueues::new();
        // Type 11 waits for 7, type 12 for 11, type 15 for 14.
        assert_eq!(queues.output_busy(11, IDLE), Some(false));
        queues.add(7, &[0; 4]).unwrap();
        assert_eq!(queues.output_busy(11, IDLE), Some(true));
        assert_eq!(queues.output_busy(19, IDLE), Some(true));
        queues.add(11, &[0; 0x18]).unwrap();
        assert_eq!(queues.output_busy(12, IDLE), Some(true));
        queues.add(14, &[0; 0x48]).unwrap();
        assert_eq!(queues.output_busy(15, IDLE), Some(true));
        // Type 8 waits for 6 or 7 or the render thread.
        assert_eq!(queues.output_busy(8, IDLE), Some(true));
    }

    #[test]
    fn end_fence_rules_wait_only_for_a_pending_fence() {
        let queues = WorkerCmdQueues::new();
        let pending = WorkerCmdBusyInput {
            is_render_thread: false,
            end_fence_pending: Some(true),
        };
        let done = WorkerCmdBusyInput {
            is_render_thread: false,
            end_fence_pending: Some(false),
        };
        for typ in [14u8, 16, 17, 18, 13] {
            assert_eq!(queues.output_busy(typ, pending), Some(true), "type {typ}");
            assert_eq!(queues.output_busy(typ, done), Some(false), "type {typ}");
            assert_eq!(queues.output_busy(typ, IDLE), Some(false), "type {typ}");
        }
    }

    #[test]
    fn draining_stops_at_a_type_that_has_to_wait() {
        let mut queues = WorkerCmdQueues::new();
        queues.add(WORKER_CMD_DPVS_ENT, &[0; 16]).unwrap();
        let mut ran = 0;
        queues
            .wait_of_type(WORKER_CMD_DPVS_ENT, RENDER, |_| ran += 1)
            .unwrap();
        assert_eq!(ran, 0);
        assert_eq!(queues.in_size(WORKER_CMD_DPVS_ENT), Some(1));
        queues
            .wait_of_type(WORKER_CMD_DPVS_ENT, IDLE, |_| ran += 1)
            .unwrap();
        assert_eq!(ran, 1);
        assert_eq!(queues.in_size(WORKER_CMD_DPVS_ENT), Some(0));
    }

    #[test]
    fn a_cell_is_queued_for_the_brush_model_and_scene_entity_workers() {
        let mut queues = WorkerCmdQueues::new();
        let cmd = CellFrustumWorkerCmd {
            cell: 12,
            plane_count: 4,
            plane_begin: 1,
            view: 0,
        };
        assert_eq!(
            enqueue_cell_frustum_cmds(&mut queues, cmd),
            Ok([AddWorkerCmd::Queued; 3])
        );
        for typ in [
            WORKER_CMD_CELL_DYN_BRUSH,
            WORKER_CMD_CELL_DYN_MODEL,
            WORKER_CMD_CELL_SCENE_ENT,
        ] {
            let seen = drained(&mut queues, typ);
            assert_eq!(seen.len(), 1, "type {typ}");
            assert_eq!(CellFrustumWorkerCmd::from_bytes(&seen[0]), Some(cmd));
        }
    }

    #[test]
    fn a_full_cell_ring_reports_the_overflow() {
        let mut queues = WorkerCmdQueues::new();
        let cap = queues.buf_count(WORKER_CMD_CELL_DYN_BRUSH).unwrap();
        let cmd = CellFrustumWorkerCmd {
            cell: 0,
            plane_count: 0,
            plane_begin: 0,
            view: 0,
        };
        for _ in 0..cap {
            enqueue_cell_frustum_cmds(&mut queues, cmd).unwrap();
        }
        let adds = enqueue_cell_frustum_cmds(&mut queues, cmd).unwrap();
        assert!(adds.contains(&AddWorkerCmd::OverflowInline));
    }

    #[test]
    fn only_four_types_skip_a_lost_device() {
        let skipped: Vec<u8> = (0..WORKER_CMD_COUNT as u8)
            .filter(|&typ| worker_cmd_dispatch_skips_device_lost(typ))
            .collect();
        assert_eq!(skipped, vec![0x0d, 0x0e, 0x0f, 0x10]);
    }
}
