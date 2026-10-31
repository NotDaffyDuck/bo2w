use std::fmt;

/// Why a byte stream could not be turned into a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PacketDecodeFault {
    /// The stream ran out in the middle of a value.
    ShortRead { wanted: usize, present: usize },

    /// A field index that this build's registry does not have.
    UnlistedField(usize),

    /// The bytes were all there but said something that cannot be true.
    GarbledRequest(&'static str),
}

impl fmt::Display for PacketDecodeFault {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::ShortRead { wanted, present } => write!(
                out,
                "byte stream cut off mid-value: wanted {wanted} bytes, only {present} present"
            ),
            Self::UnlistedField(index) => write!(
                out,
                "field number {index} has no entry in the playerState table of this build"
            ),
            Self::GarbledRequest(what) => write!(out, "garbled frame: {what}"),
        }
    }
}

impl std::error::Error for PacketDecodeFault {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_error_says_what_went_wrong() {
        assert_eq!(
            PacketDecodeFault::ShortRead { wanted: 4, present: 1 }.to_string(),
            "byte stream cut off mid-value: wanted 4 bytes, only 1 present"
        );
        assert_eq!(
            PacketDecodeFault::UnlistedField(9).to_string(),
            "field number 9 has no entry in the playerState table of this build"
        );
        assert_eq!(
            PacketDecodeFault::GarbledRequest("bad count").to_string(),
            "garbled frame: bad count"
        );
    }

    #[test]
    fn it_can_travel_as_a_boxed_error() {
        let boxed: Box<dyn std::error::Error> = Box::new(PacketDecodeFault::UnlistedField(1));
        assert!(boxed.to_string().contains("number 1"));
    }
}
