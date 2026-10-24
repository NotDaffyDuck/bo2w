//! A taped-together double magazine's reload times.

/// How long a double-mag gun's reloads take (ms): the plain and empty reloads, and what
/// each adds when the second mag is swapped in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DualMagTimes {
    pub reload_ms: i32,
    pub reload_empty_ms: i32,
    pub add_ms: i32,
    pub empty_add_ms: i32,
}
