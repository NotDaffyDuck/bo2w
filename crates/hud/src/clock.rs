pub(crate) fn milliseconds() -> u32 {
    use std::sync::OnceLock;
    use std::time::Instant;
    static STARTED: OnceLock<Instant> = OnceLock::new();

    /// The clock starts a minute in so nothing sees a time of zero.
    const HEAD_START_MS: u32 = 60_000;
    let since_start = STARTED.get_or_init(Instant::now).elapsed();
    since_start.as_millis() as u32 + HEAD_START_MS
}
