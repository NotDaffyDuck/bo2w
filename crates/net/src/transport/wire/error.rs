use std::fmt;

/// Why a byte stream could not be turned into a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WireError {
    /// The stream ran out in the middle of a value.
    Truncated { needed: usize, available: usize },

    /// A field index that this build's registry does not have.
    UnknownField(usize),

    /// The bytes were all there but said something that cannot be true.
    Malformed(&'static str),
}

impl fmt::Display for WireError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Truncated { needed, available } => write!(
                out,
                "stream ended inside a value: needed {needed} bytes, {available} available"
            ),
            Self::UnknownField(index) => write!(
                out,
                "field index {index} is not in this build's playerState registry"
            ),
            Self::Malformed(what) => write!(out, "malformed frame: {what}"),
        }
    }
}

impl std::error::Error for WireError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_error_says_what_went_wrong() {
        assert_eq!(
            WireError::Truncated { needed: 4, available: 1 }.to_string(),
            "stream ended inside a value: needed 4 bytes, 1 available"
        );
        assert_eq!(
            WireError::UnknownField(9).to_string(),
            "field index 9 is not in this build's playerState registry"
        );
        assert_eq!(
            WireError::Malformed("bad count").to_string(),
            "malformed frame: bad count"
        );
    }

    #[test]
    fn it_can_travel_as_a_boxed_error() {
        let boxed: Box<dyn std::error::Error> = Box::new(WireError::UnknownField(1));
        assert!(boxed.to_string().contains("index 1"));
    }
}
