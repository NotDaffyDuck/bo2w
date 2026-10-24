//! Control frames on a QUIC stream: a four-byte length, then the encoded frame.

use master_protocol::{decode_stream_payload, encode_stream_frame, stream_frame_len, ControlFrame};
use quinn::{RecvStream, SendStream};

use crate::Error;

pub async fn receive(stream: &mut RecvStream) -> Result<ControlFrame, Error> {
    let mut prefix = [0; 4];
    stream.read_exact(&mut prefix).await?;
    let mut payload = vec![0; stream_frame_len(prefix)?];
    stream.read_exact(&mut payload).await?;
    Ok(decode_stream_payload(&payload)?)
}

/// `receive` that hands the stream back, so a half-read frame survives a
/// `select!` that picked another branch: keep the future, not the stream.
pub async fn receive_owned(mut stream: RecvStream) -> (RecvStream, Result<ControlFrame, Error>) {
    let frame = receive(&mut stream).await;
    (stream, frame)
}

pub async fn send(stream: &mut SendStream, frame: &ControlFrame) -> Result<(), Error> {
    let bytes = encode_stream_frame(frame)?;
    stream.write_all(&bytes).await?;
    Ok(())
}
