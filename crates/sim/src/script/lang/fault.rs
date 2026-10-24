//! What a script failure looks like, and where in the script it happened.

use std::fmt;

/// A spot in a script. Engine-side spots (`<runtime>`, `<scheduler>`) have no
/// line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    pub module: String,
    pub function: String,
    pub line: usize,
    pub column: usize,
}

impl Location {
    /// A spot that belongs to the engine rather than to script text.
    pub fn engine(module: &str, function: &str) -> Self {
        Self {
            module: module.to_owned(),
            function: function.to_owned(),
            line: 0,
            column: 0,
        }
    }
}

/// A script failure: the spot, the reason and the call sites that led to it
/// (oldest first).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fault {
    pub location: Location,
    pub message: String,
    pub callers: Vec<Location>,
}

impl Fault {
    pub fn at(location: &Location, message: impl Into<String>) -> Self {
        Self {
            location: location.clone(),
            message: message.into(),
            callers: Vec::new(),
        }
    }
}

fn describe(spot: &Location) -> String {
    format!(
        "{}:{}:{} in {}",
        spot.module, spot.line, spot.column, spot.function
    )
}

impl fmt::Display for Fault {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(out, "{}: {}", describe(&self.location), self.message)?;
        self.callers
            .iter()
            .rev()
            .try_for_each(|spot| write!(out, "\n  called from {}", describe(spot)))
    }
}

impl std::error::Error for Fault {}
