pub(crate) fn milliseconds() -> u32 {
    use std::sync::OnceLock;
    use std::time::Instant;
    static ORIGIN: OnceLock<Instant> = OnceLock::new();

    const UPTIME_BIAS_MS: u32 = 60_000;
    ORIGIN.get_or_init(Instant::now).elapsed().as_millis() as u32 + UPTIME_BIAS_MS
}
