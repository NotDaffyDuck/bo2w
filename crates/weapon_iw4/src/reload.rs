
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DualMagTimes {
    pub reload_ms: i32,
    pub reload_empty_ms: i32,
    pub add_ms: i32,
    pub empty_add_ms: i32,
}
