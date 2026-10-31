//! The control envelope: `BO2M`, version (u16), kind (u8), request id (u64), body
//! length (u16), body. On a QUIC stream each envelope is preceded by its own length
//! as a big-endian u32.

use crate::body::{
    read_event, read_hello, read_request, read_response, read_view, write_event, write_hello,
    write_request, write_response, write_view,
};
use crate::bytes::{In, Out};
use crate::{
    ControlEnvelope, ControlQuery, ControlReply, ControlCodecFault, CONTROL_MESSAGE_SIZE_LIMIT,
    WIRE_REV,
};

const FORMAT_TAG: [u8; 4] = *b"BO2M";
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
pub fn encode_control_frame(frame: &ControlEnvelope) -> Result<Vec<u8>, ControlCodecFault> {
    let mut body = Out::default();
    let (code, rid) = match frame {
        ControlEnvelope::PassAlong(game_bytes) => {
            body.raw(game_bytes);
            (kind::TUNNEL, 0)
        }
        ControlEnvelope::Hello(hello) => {
            write_hello(&mut body, hello)?;
            (kind::HELLO, 0)
        }
        ControlEnvelope::Request(request) => {
            write_request(&mut body, &request.body)?;
            (kind::REQUEST, request.call_id)
        }
        ControlEnvelope::Response(response) => {
            write_response(&mut body, &response.body)?;
            (kind::RESPONSE, response.call_id)
        }
        ControlEnvelope::LobbyOverview(view) => {
            write_view(&mut body, view)?;
            (kind::VIEW, 0)
        }
        ControlEnvelope::Closed {
            lobby_number,
            epoch,
            reason,
        } => {
            body.put(*lobby_number).put(*epoch).put(*reason);
            (kind::CLOSED, 0)
        }
        ControlEnvelope::RoomMemberNotice(event) => {
            write_event(&mut body, event);
            (kind::EVENT, 0)
        }
    };
    seal(code, rid, &body.0)
}

/// Puts the envelope around a finished body.
fn seal(code: u8, rid: u64, body: &[u8]) -> Result<Vec<u8>, ControlCodecFault> {
    let declared = u16::try_from(body.len())
        .ok()
        .filter(|_| ENVELOPE + body.len() <= CONTROL_MESSAGE_SIZE_LIMIT)
        .ok_or(ControlCodecFault::SizeOverLimit(body.len()))?;
    let mut out = Out(Vec::with_capacity(ENVELOPE + body.len()));
    out.raw(&FORMAT_TAG)
        .put(WIRE_REV)
        .put(code)
        .put(rid)
        .put(declared)
        .raw(body);
    Ok(out.0)
}

/// Reads one complete envelope. Checks run size, magic, version, length, then body.
pub fn decode_control_frame(bytes: &[u8]) -> Result<ControlEnvelope, ControlCodecFault> {
    if bytes.len() > CONTROL_MESSAGE_SIZE_LIMIT {
        return Err(ControlCodecFault::SizeOverLimit(bytes.len()));
    }
    if bytes.len() < ENVELOPE {
        return Err(ControlCodecFault::Truncated);
    }
    let mut src = In::new(bytes);
    if src.array::<4>()? != FORMAT_TAG {
        return Err(ControlCodecFault::WrongSignature);
    }
    let remote = src.get::<u16>()?;
    if remote != WIRE_REV {
        return Err(ControlCodecFault::WireRevisionSkew {
            local: WIRE_REV,
            remote,
        });
    }
    let code = src.get::<u8>()?;
    let ticket = src.get::<u64>()?;
    let header = usize::from(src.get::<u16>()?);
    let present = src.rest().len();
    if header != present {
        return Err(ControlCodecFault::BodyLengthMismatch { header, present });
    }
    if code == kind::TUNNEL {
        return Ok(ControlEnvelope::PassAlong(src.rest().to_vec()));
    }
    let frame = match code {
        kind::HELLO => ControlEnvelope::Hello(read_hello(&mut src)?),
        kind::REQUEST => ControlEnvelope::Request(ControlQuery {
            call_id: ticket,
            body: read_request(&mut src)?,
        }),
        kind::RESPONSE => ControlEnvelope::Response(ControlReply {
            call_id: ticket,
            body: read_response(&mut src)?,
        }),
        kind::VIEW => ControlEnvelope::LobbyOverview(read_view(&mut src)?),
        kind::CLOSED => ControlEnvelope::Closed {
            lobby_number: src.get()?,
            epoch: src.get()?,
            reason: src.get()?,
        },
        kind::EVENT => ControlEnvelope::RoomMemberNotice(read_event(&mut src)?),
        other => return Err(ControlCodecFault::UnrecognizedKindByte(other)),
    };
    src.finish()?;
    Ok(frame)
}

/// An envelope with its u32 stream length in front.
pub fn write_stream_frame(frame: &ControlEnvelope) -> Result<Vec<u8>, ControlCodecFault> {
    let envelope = encode_control_frame(frame)?;
    let mut out = Out(Vec::with_capacity(4 + envelope.len()));
    out.put(envelope.len() as u32).raw(&envelope);
    Ok(out.0)
}

/// Reads the envelope that followed a stream length prefix.
pub fn read_stream_body(envelope: &[u8]) -> Result<ControlEnvelope, ControlCodecFault> {
    decode_control_frame(envelope)
}

pub fn stream_frame_len(len_bytes: [u8; 4]) -> Result<usize, ControlCodecFault> {
    let len = u32::from_be_bytes(len_bytes) as usize;
    if (1..=CONTROL_MESSAGE_SIZE_LIMIT).contains(&len) {
        Ok(len)
    } else {
        Err(ControlCodecFault::SizeOverLimit(len))
    }
}
