//! Why a lobby link stopped: the step that broke, which side it ran on, the cause.

use std::fmt;

use tokio::sync::mpsc::{Sender, error::TrySendError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkFailureRecord {
    /// Short step name; sign-on reads `"admission"` to tell a refused entry from a broken link.
    pub step_name: &'static str,
    /// `"host"`, `"join"` or `"local"`.
    pub side: &'static str,
    pub source: String,
    /// What QUIC said when the connection was already closed.
    pub closed_by: Option<String>,
}

impl LinkFailureRecord {
    pub fn new<S>(step_name: &'static str, side: &'static str, source: S) -> Self
    where
        S: Into<String>,
    {
        let source = source.into();
        Self {
            step_name,
            side,
            source,
            closed_by: None,
        }
    }

    /// Like `new`, and records why the connection says it closed (when it has).
    pub(super) fn on_link(
        step_name: &'static str,
        side: &'static str,
        cause: impl fmt::Display,
        link: &quinn::Connection,
    ) -> Self {
        Self {
            closed_by: link.close_reason().as_ref().map(ToString::to_string),
            ..Self::new(step_name, side, cause.to_string())
        }
    }
}

impl fmt::Display for LinkFailureRecord {
    fn fmt(&self, sink: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(sink, "{} {}: {}", self.side, self.step_name, self.source)?;
        self.closed_by
            .as_deref()
            .map_or(Ok(()), |close| write!(sink, " (quic close: {close})"))
    }
}

/// The text for a bounded queue that would not take an item.
pub(super) fn refused_text(operation: &str, full: bool) -> String {
    let state = ["closed", "full"][usize::from(full)];
    format!("{operation} queue {state}")
}

/// Hands `item` to a bounded queue without waiting; a full or closed queue
/// becomes a fault named after `operation`.
pub(super) fn post<T>(
    queue: &Sender<T>,
    item: T,
    operation: &'static str,
    side: &'static str,
) -> Result<(), LinkFailureRecord> {
    let Err(refused) = queue.try_send(item) else {
        return Ok(());
    };
    let text = refused_text(operation, matches!(refused, TrySendError::Full(_)));
    Err(LinkFailureRecord::new(operation, side, text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_names_side_step_and_cause() {
        let fault = LinkFailureRecord::new("hello", "join", "stream reset");
        assert_eq!(fault.to_string(), "join hello: stream reset");
        let closed = LinkFailureRecord {
            closed_by: Some("idle".into()),
            ..fault
        };
        assert_eq!(
            closed.to_string(),
            "join hello: stream reset (quic close: idle)"
        );
    }

    #[test]
    fn a_full_queue_and_a_closed_queue_read_differently() {
        let (sender, receiver) = tokio::sync::mpsc::channel::<u8>(1);
        assert!(post(&sender, 1, "jobs", "host").is_ok());
        let full = post(&sender, 2, "jobs", "host").unwrap_err();
        assert_eq!(full.to_string(), "host jobs: jobs queue full");
        drop(receiver);
        let closed = post(&sender, 3, "jobs", "host").unwrap_err();
        assert_eq!(closed.source, "jobs queue closed");
    }
}
