//! Keeps the thread that owns the frame on a fast core.
//!
//! An Intel hybrid chip lists its performance cores in sysfs. Narrowing the
//! launching thread to them, before any other thread exists, makes every
//! thread the game starts inherit the narrower set, so none of them lands on
//! an efficiency core and drags a frame out. Only Linux is handled; elsewhere
//! the scheduler is left alone.

/// Where Linux lists the performance cores of a hybrid chip.
const PERFORMANCE_CPUS: &str = "/sys/devices/cpu_core/cpus";

/// The cores a launch moved onto.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FastCoreLayout {
    /// Every performance core the chip has.
    pub fast_cores: Vec<usize>,
    /// The cores the process could run on before.
    pub could_use: Vec<usize>,
    /// The cores it runs on now: the performance cores among those it could use.
    pub now_using: Vec<usize>,
}

/// Moves the calling thread, and the threads it starts later, onto the
/// performance cores it may use. `None` when the chip is not hybrid, nothing
/// would change, or the system refused.
pub fn prefer_performance_cores() -> Option<FastCoreLayout> {
    let layout = plan_for(
        &std::fs::read_to_string(PERFORMANCE_CPUS).ok()?,
        &affinity::current()?,
    )?;
    affinity::set(&layout.now_using)?;
    Some(layout)
}

/// The move that suits `usable`, if there is one: some of the usable cores
/// are performance cores, and some are not.
fn plan_for(performance_list: &str, usable: &[usize]) -> Option<FastCoreLayout> {
    let fast_cores = read_core_numbers(performance_list);
    let now_using: Vec<usize> = usable
        .iter()
        .copied()
        .filter(|core| fast_cores.contains(core))
        .collect();
    if now_using.is_empty() || now_using.len() == usable.len() {
        return None;
    }
    Some(FastCoreLayout {
        fast_cores,
        could_use: usable.to_vec(),
        now_using,
    })
}

/// The cores in a kernel cpu list such as `0-7,16,18-19`. A list that does
/// not parse is no cores at all.
fn read_core_numbers(list: &str) -> Vec<usize> {
    let mut found = Vec::new();
    for item in list.trim().split(',').filter(|item| !item.is_empty()) {
        let (low, high) = match item.split_once('-') {
            Some((low, high)) => (low.parse::<usize>(), high.parse::<usize>()),
            None => (item.parse::<usize>(), item.parse::<usize>()),
        };
        match (low, high) {
            (Ok(low), Ok(high)) if low <= high => found.extend(low..=high),
            _ => return Vec::new(),
        }
    }
    found.sort_unstable();
    found.dedup();
    found
}

#[cfg(target_os = "linux")]
mod affinity {
    use std::mem;

    /// The cores the calling thread may run on.
    pub(super) fn current() -> Option<Vec<usize>> {
        // SAFETY: `set` is a bitmask the kernel fills in, and `size` is its length.
        unsafe {
            let mut set: libc::cpu_set_t = mem::zeroed();
            let size = mem::size_of::<libc::cpu_set_t>();
            if libc::sched_getaffinity(0, size, &raw mut set) != 0 {
                return None;
            }
            let cpus: Vec<usize> = (0..libc::CPU_SETSIZE as usize)
                .filter(|&cpu| libc::CPU_ISSET(cpu, &set))
                .collect();
            (!cpus.is_empty()).then_some(cpus)
        }
    }

    /// Limits the calling thread to `cpus`; refuses a core the bitmask has no room for.
    pub(super) fn set(cpus: &[usize]) -> Option<()> {
        // SAFETY: `set` is a bitmask, and every core set in it is inside its size.
        unsafe {
            let mut set: libc::cpu_set_t = mem::zeroed();
            for &cpu in cpus {
                if cpu >= libc::CPU_SETSIZE as usize {
                    return None;
                }
                libc::CPU_SET(cpu, &mut set);
            }
            let size = mem::size_of::<libc::cpu_set_t>();
            let refused = libc::sched_setaffinity(0, size, &raw const set) != 0;
            (!refused).then_some(())
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod affinity {
    pub(super) fn current() -> Option<Vec<usize>> {
        None
    }

    pub(super) fn set(_cpus: &[usize]) -> Option<()> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cpu_list_has_singles_and_ranges() {
        assert_eq!(read_core_numbers("0-3,8,10-11\n"), [0, 1, 2, 3, 8, 10, 11]);
        assert_eq!(read_core_numbers("5"), [5]);
        assert_eq!(read_core_numbers("4-4"), [4]);
    }

    #[test]
    fn a_cpu_list_comes_out_sorted_without_repeats() {
        assert_eq!(read_core_numbers("8,0-2,1-3,8"), [0, 1, 2, 3, 8]);
        assert_eq!(read_core_numbers("0-1,,2"), [0, 1, 2]);
    }

    #[test]
    fn a_cpu_list_that_does_not_parse_is_empty() {
        for list in ["", " ", "a", "1-", "-3", "1-2-3", "3-1", "0-3,x"] {
            assert_eq!(read_core_numbers(list), Vec::<usize>::new(), "{list:?}");
        }
    }

    #[test]
    fn a_hybrid_chip_moves_to_its_performance_cores() {
        let layout = plan_for("0-7", &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]).unwrap();
        assert_eq!(layout.fast_cores, (0..8).collect::<Vec<_>>());
        assert_eq!(layout.could_use.len(), 12);
        assert_eq!(layout.now_using, (0..8).collect::<Vec<_>>());
    }

    #[test]
    fn only_the_allowed_performance_cores_are_chosen() {
        let layout = plan_for("0-7", &[2, 3, 9, 10]).unwrap();
        assert_eq!(layout.now_using, [2, 3]);
        assert_eq!(layout.could_use, [2, 3, 9, 10]);
    }

    #[test]
    fn nothing_changes_when_nothing_would() {
        // Every allowed core is a performance core.
        assert_eq!(plan_for("0-7", &[0, 1, 2]), None);
        // None of them is.
        assert_eq!(plan_for("0-7", &[8, 9]), None);
        // Not a hybrid chip, or an unreadable list.
        assert_eq!(plan_for("", &[0, 1, 2]), None);
        assert_eq!(plan_for("nonsense", &[0, 1, 2]), None);
    }
}
