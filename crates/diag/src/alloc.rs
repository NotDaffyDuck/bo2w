//! The process's allocator, and the allocation counts the frame and load
//! reports read. Counting is off unless `BO2MP_COUNTING_ALLOC` is set, and
//! the figures say so (`None`) rather than reading as zero.

use std::alloc::{GlobalAlloc, Layout};
use std::cell::Cell;
use std::sync::atomic::{AtomicU8, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// Threads share these counters, each thread keeping to one. A counter has
/// a cache line to itself, so threads counting at once do not slow each
/// other down.
const STRIPES: usize = 64;

#[repr(align(64))]
struct Stripe {
    allocations: AtomicU64,
    allocated: AtomicU64,
    freed: AtomicU64,
}

impl Stripe {
    const fn new() -> Self {
        Self {
            allocations: AtomicU64::new(0),
            allocated: AtomicU64::new(0),
            freed: AtomicU64::new(0),
        }
    }
}

static COUNTS: [Stripe; STRIPES] = [const { Stripe::new() }; STRIPES];
static NEXT_STRIPE: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static MY_STRIPE: Cell<Option<usize>> = const { Cell::new(None) };
}

fn stripe() -> &'static Stripe {
    let index = MY_STRIPE
        .try_with(|mine| {
            mine.get().unwrap_or_else(|| {
                let index = NEXT_STRIPE.fetch_add(1, Ordering::Relaxed) % STRIPES;
                mine.set(Some(index));
                index
            })
        })
        .unwrap_or(0);
    &COUNTS[index]
}

const UNKNOWN: u8 = 0;
const LOOKING: u8 = 1;
const OFF: u8 = 2;
const ON: u8 = 3;

static COUNTING: AtomicU8 = AtomicU8::new(UNKNOWN);

/// Whether allocations are being counted (`BO2MP_COUNTING_ALLOC` is set).
pub fn counting_enabled() -> bool {
    match COUNTING.load(Ordering::Relaxed) {
        ON => true,
        UNKNOWN => look_up_counting(),
        _ => false,
    }
}

#[cold]
fn look_up_counting() -> bool {
    if COUNTING
        .compare_exchange(UNKNOWN, LOOKING, Ordering::Relaxed, Ordering::Relaxed)
        .is_err()
    {
        return COUNTING.load(Ordering::Relaxed) == ON;
    }
    // Reading the environment allocates, which comes back through the
    // allocator; while LOOKING those allocations are not counted.
    let on = std::env::var_os("BO2MP_COUNTING_ALLOC").is_some();
    COUNTING.store(if on { ON } else { OFF }, Ordering::Relaxed);
    on
}

fn tally_allocation(bytes: usize) {
    let stripe = stripe();
    stripe.allocations.fetch_add(1, Ordering::Relaxed);
    stripe.allocated.fetch_add(bytes as u64, Ordering::Relaxed);
}

fn tally_free(bytes: usize) {
    stripe().freed.fetch_add(bytes as u64, Ordering::Relaxed);
}

#[cfg(windows)]
static BACKING: mimalloc::MiMalloc = mimalloc::MiMalloc;
#[cfg(not(windows))]
static BACKING: std::alloc::System = std::alloc::System;

/// The process's global allocator: mimalloc on Windows, the system heap
/// elsewhere, counting while `BO2MP_COUNTING_ALLOC` is set.
pub struct ProcessCountingAllocator;

// SAFETY: every call goes to the backing allocator with the caller's own
// arguments; the counting beside it touches no memory the caller owns.
unsafe impl GlobalAlloc for ProcessCountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if counting_enabled() {
            tally_allocation(layout.size());
        }
        // SAFETY: the caller's contract, passed on.
        unsafe { BACKING.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if counting_enabled() {
            tally_allocation(layout.size());
        }
        // SAFETY: the caller's contract, passed on.
        unsafe { BACKING.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if counting_enabled() {
            tally_free(layout.size());
        }
        // SAFETY: the caller's contract, passed on.
        unsafe { BACKING.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if counting_enabled() {
            tally_free(layout.size());
            tally_allocation(new_size);
        }
        // SAFETY: the caller's contract, passed on.
        unsafe { BACKING.realloc(ptr, layout, new_size) }
    }
}

/// The process's allocation totals; all zero while counting is off.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProcessAllocationStats {
    /// Allocations made; growing one counts as another.
    pub process_allocations: u64,
    /// Bytes those allocations asked for.
    pub process_allocation_bytes: u64,
    /// Bytes given back.
    pub process_freed_bytes: u64,
}

pub fn process_allocations() -> ProcessAllocationStats {
    COUNTS
        .iter()
        .fold(ProcessAllocationStats::default(), |mut total, stripe| {
            total.process_allocations += stripe.allocations.load(Ordering::Relaxed);
            total.process_allocation_bytes += stripe.allocated.load(Ordering::Relaxed);
            total.process_freed_bytes += stripe.freed.load(Ordering::Relaxed);
            total
        })
}

/// Bytes allocated and not yet given back, or None while counting is off.
pub fn process_live_heap_bytes() -> Option<u64> {
    counting_enabled().then(|| {
        let total = process_allocations();
        total
            .process_allocation_bytes
            .saturating_sub(total.process_freed_bytes)
    })
}

/// Hands freed heap pages back to the system after a big unload, and says
/// how long that took. Only glibc's heap needs asking; the others return
/// them on their own.
pub fn release_freed_heap() -> Duration {
    let started = Instant::now();
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    // SAFETY: malloc_trim only reorganises glibc's own free lists.
    unsafe {
        malloc_trim(0);
    }
    started.elapsed()
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
unsafe extern "C" {
    fn malloc_trim(pad: usize) -> std::ffi::c_int;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tallies_add_up_across_threads() {
        let before = process_allocations();
        let threads: Vec<_> = (0..4)
            .map(|_| {
                std::thread::spawn(|| {
                    tally_allocation(100);
                    tally_free(40);
                })
            })
            .collect();
        for thread in threads {
            thread.join().expect("tally thread");
        }
        let after = process_allocations();
        assert_eq!(after.process_allocations - before.process_allocations, 4);
        assert_eq!(
            after.process_allocation_bytes - before.process_allocation_bytes,
            400
        );
        assert_eq!(after.process_freed_bytes - before.process_freed_bytes, 160);
    }

    #[test]
    fn the_allocator_hands_out_usable_memory() {
        let layout = Layout::from_size_align(64, 16).expect("layout");
        // SAFETY: the layout is non-zero; the block is grown with the same
        // alignment and freed with its final size.
        unsafe {
            let block = ProcessCountingAllocator.alloc_zeroed(layout);
            assert!(!block.is_null());
            assert_eq!(*block.add(63), 0);
            *block = 7;
            let grown = ProcessCountingAllocator.realloc(block, layout, 256);
            assert_eq!(*grown, 7);
            ProcessCountingAllocator
                .dealloc(grown, Layout::from_size_align(256, 16).expect("layout"));
        }
    }
}
