
/// How many numbers each random stream has handed out, kept so two machines can be compared when they disagree.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DiceTrace {
    pub root_seed: u64,
    pub scheme: u32,
    pub spawn_draws: u64,
    pub combat_draws: u64,
    pub bot_draws: u64,
}
/// Older spelling RngDebugMeta still calls; the rename sweep moves it over.
pub use self::DiceTrace as RngDebugMeta;
