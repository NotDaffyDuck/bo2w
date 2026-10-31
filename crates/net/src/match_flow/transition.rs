//! The one-line account of a session state change that goes into the logs.

/// Two lowercase hex digits per byte, 32 in all.
fn hex_text(raw: &[u8; 16]) -> String {
    raw.iter().map(|b| format!("{b:02x}")).collect()
}

/// A number, or a dash when there is none.
fn or_dash<T: std::fmt::Display>(item: Option<T>) -> String {
    match item {
        Some(value) => value.to_string(),
        None => "-".to_string(),
    }
}

/// Builds the transition line. The field order and names are what log readers grep for.
pub(crate) fn describe_session_change(
    sid: [u8; 16], inc_raw: Option<u64>, epoch: u32, load_id: Option<u64>,
    member_raw: Option<[u8; 16]>, link_raw: Option<u64>, setup_raw: Option<u32>,
    stage_from: &str, what: &str, stage_to: &str, spent_ms: Option<u64>, fail_note: &str,
) -> String {
    let session = hex_text(&sid);
    let incarnation = or_dash(inc_raw);
    let load_no = or_dash(load_id);
    let member = member_raw.map_or_else(|| "-".to_string(), |id| hex_text(&id));
    let connection = or_dash(link_raw);
    let bootstrap = or_dash(setup_raw);
    let elapsed = or_dash(spent_ms);
    format!(
        "session_id={session} incarnation={incarnation} epoch={epoch} local_load_id={load_no} member_id={member} connection_id={connection} bootstrap_id={bootstrap} stage_before={stage_from} event={what} stage_after={stage_to} elapsed_ms={elapsed} failure_source={fail_note}"
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
        assert_eq!(hex_text(&id), "ab000000000000000000000000000005");
    }

    #[test]
    fn a_missing_number_is_a_dash() {
        assert_eq!(or_dash::<u64>(None), "-");
        assert_eq!(or_dash(Some(17u32)), "17");
    }

    #[test]
    fn the_line_with_everything_missing() {
        let line = describe_session_change(
            [0; 16], None, 3, None, None, None, None, "a", "b", "c", None, "-",
        );
        assert_eq!(
            line,
            "session_id=00000000000000000000000000000000 incarnation=- epoch=3 local_load_id=- member_id=- connection_id=- bootstrap_id=- stage_before=a event=b stage_after=c elapsed_ms=- failure_source=-"
        );
    }

    #[test]
    fn the_line_with_everything_present() {
        let line = describe_session_change(
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
