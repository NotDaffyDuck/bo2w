//! The frontend's worker command queues: twenty-one fixed-record ring
//! buffers, one per command type, that the main thread fills while the
//! visibility and skinning work is decided and the workers drain.

pub const TASK_ORDER_TALLY: usize = 21;

pub const TASK_ORDER_ROOM_DEBRIS_SOLID: u8 = 0;
pub const TASK_ORDER_ROOM_DEBRIS_MODEL: u8 = 1;
pub const TASK_ORDER_ROOM_STAGE_ACTOR: u8 = 2;
pub const TASK_ORDER_SIGHT_ACTOR: u8 = 3;
pub const TASK_ORDER_BOUND_ACTOR: u8 = 4;
pub const TASK_ORDER_CONE_SHADOW_ACTOR: u8 = 5;
pub const TASK_ORDER_SMODELCACHE: u8 = 0x11;

/// Room for the largest record of any type.
const SCRATCH_LEN: usize = 0x4c;

/// With more than this many records waiting after the next one, a `process`
/// call takes a whole batch.
const BATCH_AFTER: usize = 0x13;
const BATCH_LEN: usize = 10;

/// What a type's records wait for before they may run; any one holding is
/// enough.
#[derive(Clone, Copy, Debug)]
struct WaitRule {
    render_thread: bool,
    /// Types that must have no record left.
    queued: &'static [u8],
    end_fence: bool,
}

impl WaitRule {
    const fn on(render_thread: bool, queued: &'static [u8], end_fence: bool) -> Option<Self> {
        Some(Self {
            render_thread,
            queued,
            end_fence,
        })
    }

    fn holds(self, has_records: impl Fn(u8) -> bool, input: TaskOrderBusyInput) -> bool {
        (self.render_thread && input.on_render_thread)
            || (self.end_fence && input.end_fence_queued == Some(true))
            || self.queued.iter().any(|&other| has_records(other))
    }
}

const FREE: Option<WaitRule> = None;
const RENDER_THREAD: Option<WaitRule> = WaitRule::on(true, &[], false);
const FENCE: Option<WaitRule> = WaitRule::on(false, &[], true);

/// One command type: record length and ring size in bytes, and what it waits
/// for.
#[derive(Clone, Copy, Debug)]
struct TypeSpec {
    record: u32,
    ring: u32,
    wait: Option<WaitRule>,
}

const fn spec(record: u32, ring: u32, wait: Option<WaitRule>) -> TypeSpec {
    TypeSpec { record, ring, wait }
}

/// Every command type, by number.
const TYPE_SPECS: [TypeSpec; TASK_ORDER_TALLY] = [
    spec(0x0c, 0x1800, FREE),
    spec(0x0c, 0x1800, FREE),
    spec(0x0c, 0x1800, FREE),
    spec(0x10, 0x8000, RENDER_THREAD),
    spec(0x04, 0x0400, RENDER_THREAD),
    spec(0x08, 0x0800, RENDER_THREAD),
    spec(0x0c, 0x0c, FREE),
    spec(0x04, 0x04, FREE),
    spec(0x0c, 0x0c, WaitRule::on(true, &[6, 7], false)),
    spec(0x0c, 0x0c00, FREE),
    spec(0x04, 0x04, FREE),
    spec(0x18, 0x1800, WaitRule::on(false, &[7], false)),
    spec(0x10, 0x10, WaitRule::on(false, &[11], false)),
    spec(0x4c, 0x4c, WaitRule::on(false, &[11, 12], true)),
    spec(0x48, 0x90, FENCE),
    spec(0x3c, 0x78, WaitRule::on(false, &[14], false)),
    spec(0x48, 0x90, FENCE),
    spec(0x04, 0x0800, FENCE),
    spec(0x24, 0x9000, FENCE),
    spec(0x04, 0x04, WaitRule::on(false, &[7], false)),
    spec(0x08, 0x18, FREE),
];

// Every ring holds whole records, each of which fits the scratch record.
const fn specs_fit(specs: &[TypeSpec]) -> bool {
    let mut i = 0;
    while i < specs.len() {
        let TypeSpec { record, ring, .. } = specs[i];
        if record == 0 || ring % record != 0 || record as usize > SCRATCH_LEN {
            return false;
        }
        i += 1;
    }
    true
}
const _: () = assert!(specs_fit(&TYPE_SPECS));

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TaskOrderBusyInput {
    pub on_render_thread: bool,
    /// None when no end fence exists to wait for.
    pub end_fence_queued: Option<bool>,
}

/// A record being laid out: little-endian fields at their offsets, the rest
/// zero.
struct Record<const N: usize>([u8; N]);

impl<const N: usize> Record<N> {
    fn zeroed() -> Self {
        Self([0; N])
    }

    fn put(mut self, at: usize, bytes: &[u8]) -> Self {
        self.0[at..at + bytes.len()].copy_from_slice(bytes);
        self
    }
}

/// The `N` bytes at `at` of a record already checked for length.
fn bytes_at<const N: usize>(record: &[u8], at: usize) -> [u8; N] {
    std::array::from_fn(|i| record[at + i])
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SkinCachedPropOrder {
    pub cache_slot_no: u16,
    pub first_patch_vertex: u16,
}

impl SkinCachedPropOrder {
    /// Four bytes: the cache entry, then the first patch vertex.
    #[must_use]
    pub fn as_bytes_le(self) -> [u8; 4] {
        Record::zeroed()
            .put(0, &self.cache_slot_no.to_le_bytes())
            .put(2, &self.first_patch_vertex.to_le_bytes())
            .0
    }

    #[must_use]
    pub fn parse_bytes(data: &[u8]) -> Option<Self> {
        (data.len() == 4).then(|| Self {
            cache_slot_no: u16::from_le_bytes(bytes_at(data, 0)),
            first_patch_vertex: u16::from_le_bytes(bytes_at(data, 2)),
        })
    }
}

/// One cell to cull the dynamic content of against a range of planes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RoomViewVolumeTaskOrder {
    pub cell: u32,
    pub plane_tally: u8,
    pub plane_first: u8,
    pub view_pos: u16,
}

impl RoomViewVolumeTaskOrder {
    /// The record is twelve bytes with its first four unused.
    #[must_use]
    pub fn record_bytes(self) -> [u8; 12] {
        Record::zeroed()
            .put(4, &self.cell.to_le_bytes())
            .put(8, &[self.plane_tally, self.plane_first])
            .put(10, &self.view_pos.to_le_bytes())
            .0
    }

    #[must_use]
    pub fn from_le(data: &[u8]) -> Option<Self> {
        if data.len() != 12 {
            return None;
        }
        let [plane_tally, plane_first] = bytes_at(data, 8);
        Some(Self {
            cell: u32::from_le_bytes(bytes_at(data, 4)),
            plane_tally,
            plane_first,
            view_pos: u16::from_le_bytes(bytes_at(data, 10)),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SightActorTaskOrder {
    pub stage_puppet: u32,
    pub plane_tally: u16,
    pub cell: u16,
}

impl SightActorTaskOrder {
    /// Sixteen bytes; the model object first, the planes and cell at 8.
    #[must_use]
    pub fn record_bytes(self) -> [u8; 16] {
        Record::zeroed()
            .put(0, &self.stage_puppet.to_le_bytes())
            .put(8, &self.plane_tally.to_le_bytes())
            .put(10, &self.cell.to_le_bytes())
            .0
    }

    #[must_use]
    pub fn read_record(data: &[u8]) -> Option<Self> {
        (data.len() == 16).then(|| Self {
            stage_puppet: u32::from_le_bytes(bytes_at(data, 0)),
            plane_tally: u16::from_le_bytes(bytes_at(data, 8)),
            cell: u16::from_le_bytes(bytes_at(data, 10)),
        })
    }
}

/// The three workers a cell goes to.
const CELL_WORKERS: [u8; 3] = [
    TASK_ORDER_ROOM_DEBRIS_SOLID,
    TASK_ORDER_ROOM_DEBRIS_MODEL,
    TASK_ORDER_ROOM_STAGE_ACTOR,
];

/// Queues the cell for the brush, model and scene-entity workers alike.
pub fn enqueue_room_view_volume_orders(
    queues: &mut TaskOrderQueues,
    cmd: RoomViewVolumeTaskOrder,
) -> Result<[AddTaskOrder; 3], TaskOrderError> {
    let bytes = cmd.record_bytes();
    let mut outcomes = [AddTaskOrder::Queued; 3];
    for (outcome, kind) in outcomes.iter_mut().zip(CELL_WORKERS) {
        *outcome = queues.queue_record(kind, &bytes)?;
    }
    Ok(outcomes)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddTaskOrder {
    Queued,
    /// The ring was full: the caller does the work itself, now.
    SpillInline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskOrderError {
    Type,
    DataSize,
}

/// Where a ring stands. `unfinished` counts the records added and not yet
/// run, `untaken` the ones no `process` call has claimed yet; `unfinished`
/// is what bounds the ring.
#[derive(Clone, Copy, Debug, Default)]
struct Cursor {
    next_read: usize,
    next_write: usize,
    unfinished: usize,
    untaken: usize,
}

/// One command type's ring of fixed-size records.
struct Ring {
    record_len: usize,
    capacity: usize,
    bytes: Vec<u8>,
    at: Cursor,
}

impl Ring {
    fn for_spec(spec: TypeSpec) -> Self {
        Self {
            record_len: spec.record as usize,
            capacity: (spec.ring / spec.record) as usize,
            bytes: vec![0; spec.ring as usize],
            at: Cursor::default(),
        }
    }

    /// The bytes of slot `slot`.
    fn record_at(&self, slot: usize) -> &[u8] {
        &self.bytes[slot * self.record_len..][..self.record_len]
    }

    fn after(&self, slot: usize) -> usize {
        (slot + 1) % self.capacity
    }

    fn store_record(&mut self, data: &[u8]) -> AddTaskOrder {
        if self.at.unfinished >= self.capacity {
            return AddTaskOrder::SpillInline;
        }
        let from = self.at.next_write * self.record_len;
        self.bytes[from..from + self.record_len].copy_from_slice(data);
        self.at.next_write = self.after(self.at.next_write);
        self.at.unfinished += 1;
        self.at.untaken += 1;
        AddTaskOrder::Queued
    }

    /// Runs `run_one` on a copy of each of the next `batch` records, in order,
    /// and retires them.
    fn drain(&mut self, batch: usize, run_one: &mut impl FnMut(&mut [u8])) {
        let mut work_buf = [0u8; SCRATCH_LEN];
        let copy = &mut work_buf[..self.record_len];
        for _ in 0..batch {
            copy.copy_from_slice(self.record_at(self.at.next_read));
            run_one(copy);
            self.at.next_read = self.after(self.at.next_read);
        }
        self.at.unfinished -= batch;
    }
}

/// What a queue call hands back.

pub struct TaskOrderQueues {
    rings: Vec<Ring>,
}

impl Default for TaskOrderQueues {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskOrderQueues {
    #[must_use]
    pub fn new() -> Self {
        Self {
            rings: TYPE_SPECS.iter().copied().map(Ring::for_spec).collect(),
        }
    }

    fn ring(&self, kind: u8) -> Option<&Ring> {
        self.rings.get(usize::from(kind))
    }

    fn ring_mut(&mut self, kind: u8) -> Result<&mut Ring, TaskOrderError> {
        self.rings.get_mut(usize::from(kind)).ok_or(TaskOrderError::Type)
    }

    /// Bytes per record of the type.
    #[must_use]
    pub fn payload_size(&self, kind: u8) -> Option<u32> {
        Some(self.ring(kind)?.record_len as u32)
    }

    /// Records the type's ring holds.
    #[must_use]
    pub fn buf_tally(&self, kind: u8) -> Option<i32> {
        Some(self.ring(kind)?.capacity as i32)
    }

    /// Records of the type added and not yet run.
    #[must_use]
    pub fn input_size(&self, kind: u8) -> Option<i32> {
        Some(self.ring(kind)?.at.unfinished as i32)
    }

    /// Empties the type's ring.
    pub fn reset_kind(&mut self, kind: u8) -> Result<(), TaskOrderError> {
        self.ring_mut(kind)?.at = Cursor::default();
        Ok(())
    }

    /// Queues one record of the type, or says the caller must run it.
    pub fn queue_record(&mut self, kind: u8, data: &[u8]) -> Result<AddTaskOrder, TaskOrderError> {
        let ring = self.ring_mut(kind)?;
        match data.len() == ring.record_len {
            true => Ok(ring.store_record(data)),
            false => Err(TaskOrderError::DataSize),
        }
    }

    /// Whether the type has to wait now: None if it never waits; a type that
    /// does not exist always waits.
    #[must_use]
    pub fn out_busy(&self, kind: u8, input: TaskOrderBusyInput) -> Option<bool> {
        match TYPE_SPECS.get(usize::from(kind)) {
            None => Some(true),
            Some(spec) => {
                let has_records = |other: u8| self.input_size(other).is_some_and(|n| n > 0);
                spec.wait.map(|rule| rule.holds(has_records, input))
            }
        }
    }

    /// Runs the next record (or a batch, when many are queued) of the type
    /// unless its busy rule says to wait. True if something ran.
    pub fn handle_kind(
        &mut self,
        kind: u8,
        input: TaskOrderBusyInput,
        run_one: impl FnMut(&mut [u8]),
    ) -> Result<bool, TaskOrderError> {
        let Some(wait) = self.out_busy(kind, input) else {
            return self.process(kind, None, run_one);
        };
        let mut verdict = move |_: &[u8]| wait;
        self.process(kind, Some(&mut verdict), run_one)
    }

    /// With `busy`, looks at the next record and runs it only if `busy`
    /// says not to wait; without, runs it unconditionally and, if more than
    /// a score are queued, nine more behind it.
    pub fn process(
        &mut self,
        kind: u8,
        busy: Option<&mut dyn FnMut(&[u8]) -> bool>,
        mut run_one: impl FnMut(&mut [u8]),
    ) -> Result<bool, TaskOrderError> {
        let ring = self.ring_mut(kind)?;
        let waiting = ring.at.untaken;
        if waiting == 0 {
            return Ok(false);
        }
        let count = if let Some(busy) = busy {
            if busy(ring.record_at(ring.at.next_read)) {
                return Ok(false);
            }
            1
        } else if waiting - 1 > BATCH_AFTER {
            BATCH_LEN
        } else {
            1
        };
        ring.at.untaken -= count;
        ring.drain(count, &mut run_one);
        Ok(true)
    }

    /// Drains the type, stopping early if it has to wait.
    pub fn wait_for_type(
        &mut self,
        kind: u8,
        input: TaskOrderBusyInput,
        mut run_one: impl FnMut(&mut [u8]),
    ) -> Result<(), TaskOrderError> {
        loop {
            let pending = self.input_size(kind).unwrap_or(0) > 0;
            if !pending || !self.handle_kind(kind, input, &mut run_one)? {
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RENDER: TaskOrderBusyInput = TaskOrderBusyInput {
        on_render_thread: true,
        end_fence_queued: None,
    };
    const IDLE: TaskOrderBusyInput = TaskOrderBusyInput {
        on_render_thread: false,
        end_fence_queued: None,
    };
    const BOUND: u8 = TASK_ORDER_BOUND_ACTOR;
    const WAITS: Option<bool> = Some(true);
    const FREE: Option<bool> = Some(false);

    fn drained(queues: &mut TaskOrderQueues, kind: u8) -> Vec<Vec<u8>> {
        let mut seen = Vec::new();
        let done = queues.wait_for_type(kind, IDLE, |r| seen.push(r.to_vec()));
        assert_eq!(done, Ok(()));
        seen
    }

    fn fill(queues: &mut TaskOrderQueues, numbers: std::ops::Range<u32>) {
        for n in numbers {
            assert_eq!(queues.queue_record(BOUND, &n.to_le_bytes()), Ok(AddTaskOrder::Queued));
        }
    }

    #[test]
    fn the_rings_have_their_sizes() {
        let queues = TaskOrderQueues::new();
        assert_eq!(queues.buf_tally(TASK_ORDER_ROOM_DEBRIS_SOLID), Some(0x1800 / 0x0c));
        assert_eq!(queues.payload_size(TASK_ORDER_SIGHT_ACTOR), Some(0x10));
        assert_eq!(queues.buf_tally(TASK_ORDER_TALLY as u8), None);
        let largest = TYPE_SPECS.iter().map(|s| s.record as usize).max();
        assert_eq!(largest, Some(SCRATCH_LEN));
    }

    #[test]
    fn records_round_trip_through_their_bytes() {
        let skin = SkinCachedPropOrder {
            cache_slot_no: 0x1234,
            first_patch_vertex: 0xbeef,
        };
        assert_eq!(skin.as_bytes_le(), [0x34, 0x12, 0xef, 0xbe]);
        assert_eq!(SkinCachedPropOrder::parse_bytes(&skin.as_bytes_le()), Some(skin));

        let cell = RoomViewVolumeTaskOrder {
            cell: 0x0102_0304,
            plane_tally: 6,
            plane_first: 2,
            view_pos: 9,
        };
        // The first four bytes are unused.
        assert_eq!(cell.record_bytes(), [0, 0, 0, 0, 4, 3, 2, 1, 6, 2, 9, 0]);
        assert_eq!(RoomViewVolumeTaskOrder::from_le(&cell.record_bytes()), Some(cell));

        let dpvs = SightActorTaskOrder {
            stage_puppet: 77,
            plane_tally: 5,
            cell: 300,
        };
        assert_eq!(dpvs.record_bytes(), [77, 0, 0, 0, 0, 0, 0, 0, 5, 0, 44, 1, 0, 0, 0, 0]);
        assert_eq!(SightActorTaskOrder::read_record(&dpvs.record_bytes()), Some(dpvs));
    }

    #[test]
    fn a_record_of_the_wrong_length_does_not_decode() {
        assert_eq!(SkinCachedPropOrder::parse_bytes(&[0; 3]), None);
        assert_eq!(RoomViewVolumeTaskOrder::from_le(&[0; 11]), None);
        assert_eq!(RoomViewVolumeTaskOrder::from_le(&[0; 13]), None);
        assert_eq!(SightActorTaskOrder::read_record(&[0; 17]), None);
    }

    #[test]
    fn add_checks_the_type_and_the_record_size() {
        let mut queues = TaskOrderQueues::new();
        let no_type = TASK_ORDER_TALLY as u8;
        assert_eq!(queues.queue_record(no_type, &[0; 4]), Err(TaskOrderError::Type));
        assert_eq!(queues.queue_record(BOUND, &[0; 8]), Err(TaskOrderError::DataSize));
        assert_eq!(queues.queue_record(BOUND, &[1; 4]), Ok(AddTaskOrder::Queued));
        assert_eq!(queues.input_size(BOUND), Some(1));
        assert_eq!(queues.reset_kind(no_type), Err(TaskOrderError::Type));
    }

    #[test]
    fn a_full_ring_hands_the_work_back_to_the_caller() {
        let mut queues = TaskOrderQueues::new();
        // Type 6 holds exactly one twelve-byte record.
        assert_eq!(queues.buf_tally(6), Some(1));
        assert_eq!(queues.queue_record(6, &[1; 12]), Ok(AddTaskOrder::Queued));
        assert_eq!(queues.queue_record(6, &[2; 12]), Ok(AddTaskOrder::SpillInline));
        assert_eq!(queues.input_size(6), Some(1));
        assert_eq!(drained(&mut queues, 6), [vec![1; 12]]);
        assert_eq!(queues.queue_record(6, &[3; 12]), Ok(AddTaskOrder::Queued));
    }

    #[test]
    fn records_come_out_in_order_across_the_wrap() {
        let mut queues = TaskOrderQueues::new();
        let cap = queues.buf_tally(BOUND).map_or(0, |n| n as u32);
        let mut next = 0u32;
        for (round, count) in [cap - 3, 5, 5].into_iter().enumerate() {
            let span = next..next + count;
            next = span.end;
            let expected: Vec<Vec<u8>> = span.clone().map(|n| n.to_le_bytes().to_vec()).collect();
            fill(&mut queues, span);
            let mut seen = Vec::new();
            while queues.process(BOUND, None, |r| seen.push(r.to_vec())) == Ok(true) {}
            assert_eq!(seen, expected, "round {round}");
        }
        assert_eq!(queues.input_size(BOUND), Some(0));
    }

    #[test]
    fn reset_empties_a_ring_and_starts_it_over() {
        let mut queues = TaskOrderQueues::new();
        assert_eq!(queues.queue_record(BOUND, &[9; 4]), Ok(AddTaskOrder::Queued));
        assert_eq!(queues.reset_kind(BOUND), Ok(()));
        assert_eq!(queues.input_size(BOUND), Some(0));
        assert_eq!(queues.process(BOUND, None, |_| panic!("ran")), Ok(false));
        assert_eq!(queues.queue_record(BOUND, &[7; 4]), Ok(AddTaskOrder::Queued));
        assert_eq!(drained(&mut queues, BOUND), [vec![7; 4]]);
    }

    #[test]
    fn many_queued_records_run_ten_at_a_time() {
        let after_one_process = |queued: u32| {
            let mut queues = TaskOrderQueues::new();
            fill(&mut queues, 0..queued);
            let mut ran = 0;
            assert_eq!(queues.process(BOUND, None, |_| ran += 1), Ok(true));
            (ran, queues.input_size(BOUND).unwrap_or(-1))
        };
        assert_eq!(after_one_process(1), (1, 0));
        assert_eq!(after_one_process(20), (1, 19), "twenty queued is not yet a batch");
        assert_eq!(after_one_process(21), (10, 11));
        assert_eq!(after_one_process(40), (10, 30));
    }

    #[test]
    fn a_batch_runs_its_records_in_order() {
        let mut queues = TaskOrderQueues::new();
        fill(&mut queues, 0..25);
        let mut seen = Vec::new();
        let ran = queues.process(BOUND, None, |r| seen.push(u32::from_le_bytes(bytes_at(r, 0))));
        assert_eq!(ran, Ok(true));
        assert_eq!(seen, (0..10).collect::<Vec<u32>>());
    }

    #[test]
    fn a_busy_callback_that_says_wait_leaves_the_record_queued() {
        let mut queues = TaskOrderQueues::new();
        assert_eq!(queues.queue_record(BOUND, &[5; 4]), Ok(AddTaskOrder::Queued));
        let mut seen_by_busy = Vec::new();
        let mut busy = |r: &[u8]| {
            seen_by_busy.push(r.to_vec());
            true
        };
        let ran = queues.process(BOUND, Some(&mut busy), |_| panic!("ran"));
        assert_eq!(ran, Ok(false));
        // Busy sees the record it is deciding on.
        assert_eq!(seen_by_busy, [vec![5; 4]]);
        assert_eq!(queues.input_size(BOUND), Some(1));

        let mut go = |_: &[u8]| false;
        assert_eq!(queues.process(BOUND, Some(&mut go), |_| {}), Ok(true));
        assert_eq!(queues.input_size(BOUND), Some(0));
    }

    #[test]
    fn view_workers_wait_for_the_render_thread_to_be_off() {
        let queues = TaskOrderQueues::new();
        let cases = [
            (TASK_ORDER_SIGHT_ACTOR, RENDER, WAITS),
            (TASK_ORDER_SIGHT_ACTOR, IDLE, FREE),
            (TASK_ORDER_ROOM_DEBRIS_MODEL, RENDER, None),
            (TASK_ORDER_TALLY as u8, IDLE, WAITS),
        ];
        for (kind, input, busy) in cases {
            assert_eq!(queues.out_busy(kind, input), busy, "type {kind}");
        }
    }

    #[test]
    fn a_type_waits_while_the_types_it_depends_on_have_records() {
        let mut queues = TaskOrderQueues::new();
        // Type 11 waits for 7, type 12 for 11, type 15 for 14.
        assert_eq!(queues.out_busy(11, IDLE), FREE);
        let added = [(7, 0x04), (11, 0x18), (14, 0x48)].map(|(kind, len)| queues.queue_record(kind, &vec![0; len]));
        assert!(added.iter().all(|a| *a == Ok(AddTaskOrder::Queued)));
        for kind in [11, 19, 12, 15] {
            assert_eq!(queues.out_busy(kind, IDLE), WAITS, "type {kind}");
        }
        // Type 8 waits for 6 or 7 or the render thread.
        assert_eq!(queues.out_busy(8, IDLE), WAITS);
    }

    #[test]
    fn end_fence_rules_wait_only_for_a_pending_fence() {
        let queues = TaskOrderQueues::new();
        let fence = |pending| TaskOrderBusyInput {
            on_render_thread: false,
            end_fence_queued: Some(pending),
        };
        for kind in [14u8, 16, 17, 18, 13] {
            assert_eq!(queues.out_busy(kind, fence(true)), WAITS, "type {kind}");
            assert_eq!(queues.out_busy(kind, fence(false)), FREE, "type {kind}");
            assert_eq!(queues.out_busy(kind, IDLE), FREE, "type {kind}");
        }
    }

    #[test]
    fn draining_stops_at_a_type_that_has_to_wait() {
        let mut queues = TaskOrderQueues::new();
        assert_eq!(queues.queue_record(TASK_ORDER_SIGHT_ACTOR, &[0; 16]), Ok(AddTaskOrder::Queued));
        let mut ran = 0;
        let mut drain = |queues: &mut TaskOrderQueues, input| {
            assert_eq!(queues.wait_for_type(TASK_ORDER_SIGHT_ACTOR, input, |_| ran += 1), Ok(()));
            queues.input_size(TASK_ORDER_SIGHT_ACTOR)
        };
        assert_eq!(drain(&mut queues, RENDER), Some(1));
        assert_eq!(drain(&mut queues, IDLE), Some(0));
        assert_eq!(ran, 1);
    }

    #[test]
    fn a_cell_is_queued_for_the_brush_model_and_scene_entity_workers() {
        let mut queues = TaskOrderQueues::new();
        let cmd = RoomViewVolumeTaskOrder {
            cell: 12,
            plane_tally: 4,
            plane_first: 1,
            view_pos: 0,
        };
        assert_eq!(enqueue_room_view_volume_orders(&mut queues, cmd), Ok([AddTaskOrder::Queued; 3]));
        for kind in CELL_WORKERS {
            let seen = drained(&mut queues, kind);
            let decoded: Vec<_> = seen.iter().map(|r| RoomViewVolumeTaskOrder::from_le(r)).collect();
            assert_eq!(decoded, [Some(cmd)], "type {kind}");
        }
    }

    #[test]
    fn a_full_cell_ring_reports_the_overflow() {
        let mut queues = TaskOrderQueues::new();
        let cap = queues.buf_tally(TASK_ORDER_ROOM_DEBRIS_SOLID).unwrap_or(0);
        let cmd = RoomViewVolumeTaskOrder {
            cell: 0,
            plane_tally: 0,
            plane_first: 0,
            view_pos: 0,
        };
        for _ in 0..cap {
            assert!(enqueue_room_view_volume_orders(&mut queues, cmd).is_ok());
        }
        let adds = enqueue_room_view_volume_orders(&mut queues, cmd);
        assert!(adds.is_ok_and(|adds| adds.contains(&AddTaskOrder::SpillInline)));
    }
}
