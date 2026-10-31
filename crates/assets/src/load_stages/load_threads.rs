//! The one thread pool every load runs on, sized to the CPUs the process was
//! given.

use std::sync::OnceLock;

use bevy::tasks::{TaskPool, TaskPoolBuilder};

/// The CPUs the launcher handed this process, once somebody has said.
static ALLOWED_CPUS: OnceLock<Vec<usize>> = OnceLock::new();

/// Built on first use, then shared by every load.
static LOAD_POOL: OnceLock<TaskPool> = OnceLock::new();

/// CPUs left to the game's own threads when sizing the pool.
const KEPT_FOR_GAME: usize = 2;

/// Records which CPUs the process may use; the first call wins.
pub fn announce_worker_cores(cpus: Vec<usize>) {
    // A later list is dropped on purpose: workers may already be pinned.
    ALLOWED_CPUS.set(cpus).ok();
}

/// Worker threads for loading: the CPUs the process has, less two kept free
/// for the game itself, and never fewer than one.
pub fn spawn_reader_threads() -> usize {
    let machine = || std::thread::available_parallelism().map_or(1, usize::from);
    let usable = ALLOWED_CPUS.get().map(Vec::len).unwrap_or_else(machine);
    usable.saturating_sub(KEPT_FOR_GAME).max(1)
}

pub fn loader_pool() -> &'static TaskPool {
    LOAD_POOL.get_or_init(build_load_pool)
}

fn build_load_pool() -> TaskPool {
    let named = TaskPoolBuilder::new().thread_name(String::from("bo2mp load"));
    named.num_threads(spawn_reader_threads()).on_thread_spawn(pin_new_worker).build()
}

/// Runs on each worker as it starts: keeps it on the published CPUs.
fn pin_new_worker() {
    ALLOWED_CPUS.get().into_iter().for_each(|cpus| pin_thread_to(cpus));
}

/// Pins the calling thread to `cpus`. A list naming a CPU the kernel's set
/// cannot hold leaves the thread where it is.
#[cfg(target_os = "linux")]
fn pin_thread_to(cpus: &[usize]) {
    let capacity = libc::CPU_SETSIZE as usize;
    if cpus.iter().any(|&cpu| cpu >= capacity) {
        return;
    }
    // SAFETY: an all-zero cpu_set_t is the empty set, every index added is
    // below CPU_SETSIZE (checked above), and the set outlives the call that
    // reads it.
    unsafe {
        let mut mask = std::mem::zeroed::<libc::cpu_set_t>();
        for cpu in cpus.iter().copied() {
            libc::CPU_SET(cpu, &mut mask);
        }
        libc::sched_setaffinity(0, std::mem::size_of_val(&mask), &mask);
    }
}

#[cfg(not(target_os = "linux"))]
fn pin_thread_to(_cpus: &[usize]) {}

#[cfg(test)]
mod tests {
    #[test]
    fn the_pool_always_has_a_worker() {
        assert!(super::spawn_reader_threads() >= 1);
    }
}
