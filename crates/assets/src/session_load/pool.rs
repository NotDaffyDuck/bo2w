//! The one thread pool every load runs on, sized to the CPUs the process was
//! given.

use bevy::tasks::{TaskPool, TaskPoolBuilder};

static PROCESS_CPUS: std::sync::OnceLock<Vec<usize>> = std::sync::OnceLock::new();

/// Records which CPUs the process may use; the first call wins.
pub fn publish_process_cpus(cpus: Vec<usize>) {
    let _ = PROCESS_CPUS.set(cpus);
}

/// Worker threads for loading: the CPUs the process has, less two kept free
/// for the game itself, and never fewer than one.
pub fn load_workers() -> usize {
    let cpus = match PROCESS_CPUS.get() {
        Some(cpus) => cpus.len(),
        None => std::thread::available_parallelism().map_or(1, usize::from),
    };
    cpus.saturating_sub(2).max(1)
}

pub fn load_pool() -> &'static TaskPool {
    static POOL: std::sync::OnceLock<TaskPool> = std::sync::OnceLock::new();
    POOL.get_or_init(|| {
        TaskPoolBuilder::new()
            .num_threads(load_workers())
            .thread_name("bo2mp load".to_owned())
            .on_thread_spawn(|| {
                if let Some(cpus) = PROCESS_CPUS.get() {
                    pin_thread_to(cpus);
                }
            })
            .build()
    })
}

#[cfg(target_os = "linux")]
fn pin_thread_to(cpus: &[usize]) {
    // SAFETY: an all-zero cpu_set_t is the empty set, and only indices below
    // CPU_SETSIZE are added to it.
    unsafe {
        let mut set: libc::cpu_set_t = std::mem::zeroed();
        for &cpu in cpus {
            if cpu >= libc::CPU_SETSIZE as usize {
                return;
            }
            libc::CPU_SET(cpu, &mut set);
        }
        libc::sched_setaffinity(0, size_of::<libc::cpu_set_t>(), &raw const set);
    }
}

#[cfg(not(target_os = "linux"))]
fn pin_thread_to(_cpus: &[usize]) {}
