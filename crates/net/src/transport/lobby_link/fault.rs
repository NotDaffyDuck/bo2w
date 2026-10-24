//! Why a lobby link stopped: the step that broke, which side it ran on, the cause.

use std::fmt;

use tokio::sync::mpsc::{Sender, error::TrySendError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransportFault {
    /// Short step name; sign-on reads `"admission"` to tell a refused entry from a broken link.
    pub operation: &'static str,
    /// `"host"`, `"join"` or `"local"`.
    pub role: &'static str,
    pub source: String,
    /// What QUIC said when the connection was already closed.
    pub close_reason: Option<String>,
}

impl TransportFault {
    pub fn new(operation: &'static str, role: &'static str, source: impl Into<String>) -> Self {
        Self {
            operation,
            role,
            source: source.into(),
            close_reason: None,
        }
    }

    /// Same, plus the close reason the connection holds (if it has closed).
    pub(super) fn on_link(
        operation: &'static str,
        role: &'static str,
        cause: impl fmt::Display,
        link: &quinn::Connection,
    ) -> Self {
        let mut fault = Self::new(operation, role, cause.to_string());
        fault.close_reason = link.close_reason().map(|why| why.to_string());
        fault
    }
}

impl fmt::Display for TransportFault {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(out, "{} {}: {}", self.role, self.operation, self.source)?;
        match &self.close_reason {
            Some(close) => write!(out, " (quic close: {close})"),
            None => Ok(()),
        }
    }
}

/// The text for a bounded queue that would not take an item.
pub(super) fn refused_text(operation: &str, full: bool) -> String {
    let why = if full { "full" } else { "closed" };
    format!("{operation} queue {why}")
}

/// Hands `item` to a bounded queue without waiting; a full or closed queue
/// becomes a fault named after `operation`.
pub(super) fn post<T>(
    queue: &Sender<T>,
    item: T,
    operation: &'static str,
    role: &'static str,
) -> Result<(), TransportFault> {
    queue.try_send(item).map_err(|refused| {
        let full = matches!(refused, TrySendError::Full(_));
        TransportFault::new(operation, role, refused_text(operation, full))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_names_side_step_and_cause() {
        let fault = TransportFault::new("hello", "join", "stream reset");
        assert_eq!(fault.to_string(), "join hello: stream reset");
        let closed = TransportFault {
            close_reason: Some("idle".into()),
            ..fault
        };
        assert_eq!(closed.to_string(), "join hello: stream reset (quic close: idle)");
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
