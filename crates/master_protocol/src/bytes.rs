//! Field writer and reader shared by every layout. Integers are big-endian, a bool is
//! one byte (0 or 1), text is a length byte followed by UTF-8.

use crate::{
    ListingId, AssetCoverageBits, PeerKindTag, RoomOccupantId, ControlCodecFault, LobbyStage, RoomShutdownCause,
};

/// A fixed-layout value: written without failing, read with a check.
pub(crate) trait Scalar: Copy {
    fn emit(self, out: &mut Out);
    fn parse(src: &mut In<'_>) -> Result<Self, ControlCodecFault>;
}

/// Output buffer with chained writes.
#[derive(Default)]
pub(crate) struct Out(pub(crate) Vec<u8>);

impl Out {
    pub(crate) fn put<T: Scalar>(&mut self, value: T) -> &mut Self {
        value.emit(self);
        self
    }

    pub(crate) fn raw(&mut self, bytes: &[u8]) -> &mut Self {
        self.0.extend_from_slice(bytes);
        self
    }

    /// Text over `cap` bytes (or over 255) is refused with its length.
    pub(crate) fn put_text(&mut self, text: &str, cap: usize) -> Result<&mut Self, ControlCodecFault> {
        let len = u8::try_from(text.len())
            .ok()
            .filter(|&len| usize::from(len) <= cap)
            .ok_or(ControlCodecFault::SizeOverLimit(text.len()))?;
        Ok(self.put(len).raw(text.as_bytes()))
    }
}

/// Read cursor over one body.
pub(crate) struct In<'a> {
    left: &'a [u8],
}

impl<'a> In<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self { left: bytes }
    }

    pub(crate) fn get<T: Scalar>(&mut self) -> Result<T, ControlCodecFault> {
        T::parse(self)
    }

    pub(crate) fn chunk(&mut self, len: usize) -> Result<&'a [u8], ControlCodecFault> {
        if self.left.len() < len {
            return Err(ControlCodecFault::Truncated);
        }
        let (head, tail) = self.left.split_at(len);
        self.left = tail;
        Ok(head)
    }

    pub(crate) fn array<const N: usize>(&mut self) -> Result<[u8; N], ControlCodecFault> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.chunk(N)?);
        Ok(out)
    }

    /// A length over `cap` is refused before the text itself is read.
    pub(crate) fn text(&mut self, cap: usize) -> Result<String, ControlCodecFault> {
        let len = usize::from(self.get::<u8>()?);
        if len > cap {
            return Err(ControlCodecFault::SizeOverLimit(len));
        }
        let raw = self.chunk(len)?;
        core::str::from_utf8(raw)
            .map(str::to_owned)
            .map_err(|_| ControlCodecFault::TextNotUtf8)
    }

    /// Everything not read yet.
    pub(crate) fn rest(&self) -> &'a [u8] {
        self.left
    }

    /// A body must be used up exactly; spare bytes make it invalid.
    pub(crate) fn finish(self) -> Result<(), ControlCodecFault> {
        if self.left.is_empty() {
            Ok(())
        } else {
            Err(ControlCodecFault::MalformedContent)
        }
    }
}

macro_rules! big_endian {
    ($($int:ty),*) => {$(
        impl Scalar for $int {
            fn emit(self, out: &mut Out) {
                out.raw(&self.to_be_bytes());
            }

            fn parse(src: &mut In<'_>) -> Result<Self, ControlCodecFault> {
                src.array().map(<$int>::from_be_bytes)
            }
        }
    )*};
}

big_endian!(u8, u16, u32, u64);

impl Scalar for bool {
    fn emit(self, out: &mut Out) {
        out.put(u8::from(self));
    }

    fn parse(src: &mut In<'_>) -> Result<Self, ControlCodecFault> {
        match src.get::<u8>()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(ControlCodecFault::MalformedContent),
        }
    }
}

impl Scalar for ListingId {
    fn emit(self, out: &mut Out) {
        out.raw(&self.0);
    }

    fn parse(src: &mut In<'_>) -> Result<Self, ControlCodecFault> {
        src.array().map(Self)
    }
}

impl Scalar for RoomOccupantId {
    fn emit(self, out: &mut Out) {
        out.raw(&self.0);
    }

    fn parse(src: &mut In<'_>) -> Result<Self, ControlCodecFault> {
        src.array().map(Self)
    }
}

impl Scalar for AssetCoverageBits {
    fn emit(self, out: &mut Out) {
        out.put(self.0);
    }

    fn parse(src: &mut In<'_>) -> Result<Self, ControlCodecFault> {
        src.get().map(Self)
    }
}

/// A one-byte enum and its wire codes. Unknown codes read as `MalformedContent`.
macro_rules! byte_enum {
    ($ty:ident { $($variant:ident = $code:literal),* $(,)? }) => {
        impl Scalar for $ty {
            fn emit(self, out: &mut Out) {
                let code: u8 = match self {
                    $($ty::$variant => $code,)*
                };
                out.put(code);
            }

            fn parse(src: &mut In<'_>) -> Result<Self, ControlCodecFault> {
                match src.get::<u8>()? {
                    $($code => Ok($ty::$variant),)*
                    _ => Err(ControlCodecFault::MalformedContent),
                }
            }
        }
    };
}

byte_enum!(PeerKindTag { Cli = 0, Host = 1, Join = 2 });
byte_enum!(LobbyStage { Gathering = 0, Loading = 1, Running = 2 });
byte_enum!(RoomShutdownCause { HostLeft = 0, TenancyLapsed = 1, Overload = 2, Kicked = 3 });
