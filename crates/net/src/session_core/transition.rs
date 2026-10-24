//! The one-line account of a session state change that goes into the logs.

use std::fmt::Write;

fn hex(bytes: &[u8; 16]) -> String {
    let mut out = String::with_capacity(32);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// A number, or a dash when there is none.
fn or_dash<T: std::fmt::Display>(value: Option<T>) -> String {
    match value {
        Some(value) => value.to_string(),
        None => "-".to_string(),
    }
}

/// Builds the transition line. The field order and names are what log readers grep for.
pub(crate) fn format_session_transition(
    session_id: [u8; 16],
    incarnation: Option<u64>,
    epoch: u32,
    local_load_id: Option<u64>,
    member_id: Option<[u8; 16]>,
    connection_id: Option<u64>,
    bootstrap_id: Option<u32>,
    stage_before: &str,
    event: &str,
    stage_after: &str,
    elapsed_ms: Option<u64>,
    failure_source: &str,
) -> String {
    let session = hex(&session_id);
    let incarnation = or_dash(incarnation);
    let local_load_id = or_dash(local_load_id);
    let member = member_id.map_or_else(|| "-".to_string(), |id| hex(&id));
    let connection = or_dash(connection_id);
    let bootstrap = or_dash(bootstrap_id);
    let elapsed = or_dash(elapsed_ms);
    format!(
        "session_id={session} incarnation={incarnation} epoch={epoch} local_load_id={local_load_id} member_id={member} connection_id={connection} bootstrap_id={bootstrap} stage_before={stage_before} event={event} stage_after={stage_after} elapsed_ms={elapsed} failure_source={failure_source}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_is_two_lowercase_digits_per_byte() {
        let mut id = [0u8; 16];
        id[0] = 0xab;
        id[15] = 0x05;
        assert_eq!(hex(&id), "ab000000000000000000000000000005");
    }

    #[test]
    fn a_missing_number_is_a_dash() {
        assert_eq!(or_dash::<u64>(None), "-");
        assert_eq!(or_dash(Some(17u32)), "17");
    }

    #[test]
    fn the_line_with_everything_missing() {
        let line = format_session_transition(
            [0; 16], None, 3, None, None, None, None, "a", "b", "c", None, "-",
        );
        assert_eq!(
            line,
            "session_id=00000000000000000000000000000000 incarnation=- epoch=3 local_load_id=- member_id=- connection_id=- bootstrap_id=- stage_before=a event=b stage_after=c elapsed_ms=- failure_source=-"
        );
    }

    #[test]
    fn the_line_with_everything_present() {
        let line = format_session_transition(
            [1; 16],
            Some(2),
            9,
            Some(40),
            Some([0xff; 16]),
            Some(7),
            Some(5),
            "x",
            "y",
            "z",
            Some(1500),
            "why",
        );
        assert_eq!(
            line,
            "session_id=01010101010101010101010101010101 incarnation=2 epoch=9 local_load_id=40 member_id=ffffffffffffffffffffffffffffffff connection_id=7 bootstrap_id=5 stage_before=x event=y stage_after=z elapsed_ms=1500 failure_source=why"
        );
    }
}
