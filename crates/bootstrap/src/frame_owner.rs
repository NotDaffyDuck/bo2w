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
pub struct PerformanceCorePlan {
    /// Every performance core the chip has.
    pub performance: Vec<usize>,
    /// The cores the process could run on before.
    pub allowed: Vec<usize>,
    /// The cores it runs on now: the performance cores among the allowed.
    pub chosen: Vec<usize>,
}

/// Moves the calling thread, and the threads it starts later, onto the
/// performance cores it may use. `None` when the chip is not hybrid, nothing
/// would change, or the system refused.
pub fn prefer_performance_cores() -> Option<PerformanceCorePlan> {
    let plan = plan_for(
        &std::fs::read_to_string(PERFORMANCE_CPUS).ok()?,
        &affinity::current()?,
    )?;
    affinity::set(&plan.chosen)?;
    Some(plan)
}

/// The move that suits `allowed`, if there is one: some of the allowed cores
/// are performance cores, and some are not.
fn plan_for(performance_list: &str, allowed: &[usize]) -> Option<PerformanceCorePlan> {
    let performance = parse_cpu_list(performance_list);
    let chosen: Vec<usize> = allowed
        .iter()
        .copied()
        .filter(|cpu| performance.contains(cpu))
        .collect();
    if chosen.is_empty() || chosen.len() == allowed.len() {
        return None;
    }
    Some(PerformanceCorePlan {
        performance,
        allowed: allowed.to_vec(),
        chosen,
    })
}

/// The cores in a kernel cpu list such as `0-7,16,18-19`. A list that does
/// not parse is no cores at all.
fn parse_cpu_list(list: &str) -> Vec<usize> {
    let mut cpus = Vec::new();
    for item in list.trim().split(',').filter(|item| !item.is_empty()) {
        let (first, last) = match item.split_once('-') {
            Some((first, last)) => (first.parse::<usize>(), last.parse::<usize>()),
            None => (item.parse::<usize>(), item.parse::<usize>()),
        };
        match (first, last) {
            (Ok(first), Ok(last)) if first <= last => cpus.extend(first..=last),
            _ => return Vec::new(),
        }
    }
    cpus.sort_unstable();
    cpus.dedup();
    cpus
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
            (libc::sched_setaffinity(0, size, &raw const set) == 0).then_some(())
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
        assert_eq!(parse_cpu_list("0-3,8,10-11\n"), [0, 1, 2, 3, 8, 10, 11]);
        assert_eq!(parse_cpu_list("5"), [5]);
        assert_eq!(parse_cpu_list("4-4"), [4]);
    }

    #[test]
    fn a_cpu_list_comes_out_sorted_without_repeats() {
        assert_eq!(parse_cpu_list("8,0-2,1-3,8"), [0, 1, 2, 3, 8]);
        assert_eq!(parse_cpu_list("0-1,,2"), [0, 1, 2]);
    }

    #[test]
    fn a_cpu_list_that_does_not_parse_is_empty() {
        for list in ["", " ", "a", "1-", "-3", "1-2-3", "3-1", "0-3,x"] {
            assert_eq!(parse_cpu_list(list), Vec::<usize>::new(), "{list:?}");
        }
    }

    #[test]
    fn a_hybrid_chip_moves_to_its_performance_cores() {
        let plan = plan_for("0-7", &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]).unwrap();
        assert_eq!(plan.performance, (0..8).collect::<Vec<_>>());
        assert_eq!(plan.allowed.len(), 12);
        assert_eq!(plan.chosen, (0..8).collect::<Vec<_>>());
    }

    #[test]
    fn only_the_allowed_performance_cores_are_chosen() {
        let plan = plan_for("0-7", &[2, 3, 9, 10]).unwrap();
        assert_eq!(plan.chosen, [2, 3]);
        assert_eq!(plan.allowed, [2, 3, 9, 10]);
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
