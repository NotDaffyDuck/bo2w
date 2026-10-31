//! Saying no to a match before any of it is published.

/// A refusal of the whole request: the loading screen shows `error`, the swap
/// completes on it, and nothing of the match has been published.
#[derive(Debug)]
pub(super) struct InstallRefusal {
    pub error: String,
    /// The gap the launch report records for the refusal, when it has one.
    pub stepping_lag: Option<&'static str>,
}

impl InstallRefusal {
    pub fn new(error: impl Into<String>) -> Self {
        Self {
            error: error.into(),
            stepping_lag: None,
        }
    }

    pub fn with_gap(error: impl Into<String>, gap: &'static str) -> Self {
        Self {
            stepping_lag: Some(gap),
            ..Self::new(error)
        }
    }
}

fn hex(raw: &[u8]) -> String {
    raw.iter().map(|octet| format!("{octet:02x}")).collect()
}

/// The first line of a fault, safe to put between double quotes in a report.
fn headline(fault: &str) -> String {
    fault.lines().next().unwrap_or("").replace('"', "'")
}

/// What the script boundary report says about a program that was compiled.
pub(super) fn install_facts(
    zone: &str,
    gametype: &str,
    program: &sim::gsc_host::Program,
    entries: usize,
) -> String {
    format!(
        " map={zone} gametype={gametype} fingerprint={} modules={} functions={} natives={} entries={entries}",
        hex(&program.fingerprint()),
        program.modules().len(),
        program.function_count(),
        program.native_count(),
    )
}

/// Refuse because the scripts would not compile or start, and say so at the
/// script boundary.
pub(super) fn script_refusal(
    zone: &str,
    gametype: &str,
    stage: &str,
    fault: &sim::gsc_host::Fault,
) -> InstallRefusal {
    let text = fault.to_string();
    diag::script_boundary(
        "refused",
        &format!(
            " map={zone} gametype={gametype} stage={stage} fault=\"{}\"",
            headline(&text)
        ),
    );
    InstallRefusal::new(format!("GSC {stage}: {text}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_refusal_has_no_gap() {
        let refusal = InstallRefusal::new("no");
        assert_eq!(refusal.error, "no");
        assert_eq!(refusal.stepping_lag, None);
    }

    #[test]
    fn a_refusal_with_a_gap_keeps_it() {
        let refusal = InstallRefusal::with_gap("no", "because");
        assert_eq!(refusal.error, "no");
        assert_eq!(refusal.stepping_lag, Some("because"));
    }

    #[test]
    fn bytes_print_as_two_hex_digits_each() {
        assert_eq!(hex(&[0, 1, 0xab, 0xff]), "0001abff");
        assert_eq!(hex(&[]), "");
    }

    #[test]
    fn the_headline_is_the_first_line_with_quotes_softened() {
        assert_eq!(headline("bad \"x\"\nmore detail"), "bad 'x'");
        assert_eq!(headline(""), "");
    }
}
