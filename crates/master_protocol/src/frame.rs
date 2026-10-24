//! The control envelope: `BO2M`, version (u16), kind (u8), request id (u64), body
//! length (u16), body. On a QUIC stream each envelope is preceded by its own length
//! as a big-endian u32.

use crate::body::{
    read_event, read_hello, read_request, read_response, read_view, write_event, write_hello,
    write_request, write_response, write_view,
};
use crate::bytes::{In, Out};
use crate::{
    ControlFrame, ControlRequest, ControlResponse, ProtocolError, MAX_CONTROL_BYTES,
    PROTOCOL_VERSION,
};

const MAGIC: [u8; 4] = *b"BO2M";
/// Magic, version, kind, request id and body length.
const ENVELOPE: usize = 4 + 2 + 1 + 8 + 2;

/// Frame kinds, the byte after the version.
mod kind {
    pub const HELLO: u8 = 0;
    pub const REQUEST: u8 = 1;
    pub const RESPONSE: u8 = 2;
    pub const VIEW: u8 = 3;
    pub const CLOSED: u8 = 4;
    pub const EVENT: u8 = 5;
    pub const TUNNEL: u8 = 6;
}

/// Writes one complete envelope.
pub fn encode_control_frame(frame: &ControlFrame) -> Result<Vec<u8>, ProtocolError> {
    let mut body = Out::default();
    let (code, request_id) = match frame {
        ControlFrame::Relay(game_bytes) => {
            body.raw(game_bytes);
            (kind::TUNNEL, 0)
        }
        ControlFrame::Hello(hello) => {
            write_hello(&mut body, hello)?;
            (kind::HELLO, 0)
        }
        ControlFrame::Request(request) => {
            write_request(&mut body, &request.body)?;
            (kind::REQUEST, request.request_id)
        }
        ControlFrame::Response(response) => {
            write_response(&mut body, &response.body)?;
            (kind::RESPONSE, response.request_id)
        }
        ControlFrame::RoomView(view) => {
            write_view(&mut body, view)?;
            (kind::VIEW, 0)
        }
        ControlFrame::Closed {
            room_id,
            epoch,
            reason,
        } => {
            body.put(*room_id).put(*epoch).put(*reason);
            (kind::CLOSED, 0)
        }
        ControlFrame::PeerEvent(event) => {
            write_event(&mut body, event);
            (kind::EVENT, 0)
        }
    };
    seal(code, request_id, &body.0)
}

/// Puts the envelope around a finished body.
fn seal(code: u8, request_id: u64, body: &[u8]) -> Result<Vec<u8>, ProtocolError> {
    let declared = u16::try_from(body.len())
        .ok()
        .filter(|_| ENVELOPE + body.len() <= MAX_CONTROL_BYTES)
        .ok_or(ProtocolError::LengthExceeded(body.len()))?;
    let mut out = Out(Vec::with_capacity(ENVELOPE + body.len()));
    out.raw(&MAGIC)
        .put(PROTOCOL_VERSION)
        .put(code)
        .put(request_id)
        .put(declared)
        .raw(body);
    Ok(out.0)
}

/// Reads one complete envelope. Checks run size, magic, version, length, then body.
pub fn decode_control_frame(bytes: &[u8]) -> Result<ControlFrame, ProtocolError> {
    if bytes.len() > MAX_CONTROL_BYTES {
        return Err(ProtocolError::LengthExceeded(bytes.len()));
    }
    if bytes.len() < ENVELOPE {
        return Err(ProtocolError::Truncated);
    }
    let mut src = In::new(bytes);
    if src.array::<4>()? != MAGIC {
        return Err(ProtocolError::BadMagic);
    }
    let theirs = src.get::<u16>()?;
    if theirs != PROTOCOL_VERSION {
        return Err(ProtocolError::VersionMismatch {
            ours: PROTOCOL_VERSION,
            theirs,
        });
    }
    let code = src.get::<u8>()?;
    let request_id = src.get::<u64>()?;
    let declared = usize::from(src.get::<u16>()?);
    let actual = src.rest().len();
    if declared != actual {
        return Err(ProtocolError::LengthMismatch { declared, actual });
    }
    if code == kind::TUNNEL {
        return Ok(ControlFrame::Relay(src.rest().to_vec()));
    }
    let frame = match code {
        kind::HELLO => ControlFrame::Hello(read_hello(&mut src)?),
        kind::REQUEST => ControlFrame::Request(ControlRequest {
            request_id,
            body: read_request(&mut src)?,
        }),
        kind::RESPONSE => ControlFrame::Response(ControlResponse {
            request_id,
            body: read_response(&mut src)?,
        }),
        kind::VIEW => ControlFrame::RoomView(read_view(&mut src)?),
        kind::CLOSED => ControlFrame::Closed {
            room_id: src.get()?,
            epoch: src.get()?,
            reason: src.get()?,
        },
        kind::EVENT => ControlFrame::PeerEvent(read_event(&mut src)?),
        other => return Err(ProtocolError::UnknownTag(other)),
    };
    src.finish()?;
    Ok(frame)
}

/// An envelope with its u32 stream length in front.
pub fn encode_stream_frame(frame: &ControlFrame) -> Result<Vec<u8>, ProtocolError> {
    let envelope = encode_control_frame(frame)?;
    let mut out = Out(Vec::with_capacity(4 + envelope.len()));
    out.put(envelope.len() as u32).raw(&envelope);
    Ok(out.0)
}

/// Reads the envelope that followed a stream length prefix.
pub fn decode_stream_payload(envelope: &[u8]) -> Result<ControlFrame, ProtocolError> {
    decode_control_frame(envelope)
}

/// The envelope size a stream length prefix announces; zero or over the cap is refused.
pub fn stream_frame_len(prefix: [u8; 4]) -> Result<usize, ProtocolError> {
    let len = u32::from_be_bytes(prefix) as usize;
    if (1..=MAX_CONTROL_BYTES).contains(&len) {
        Ok(len)
    } else {
        Err(ProtocolError::LengthExceeded(len))
    }
}
