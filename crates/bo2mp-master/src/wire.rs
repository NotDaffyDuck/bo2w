//! Control frames on a QUIC stream: a four-byte length, then the encoded frame.

use master_protocol as proto;
use quinn::{RecvStream, SendStream};

use crate::Error;

pub async fn receive(stream: &mut RecvStream) -> Result<proto::ControlEnvelope, Error> {
    let mut prefix = [0; 4];
    stream.read_exact(&mut prefix).await?;
    let mut payload = vec![0; proto::stream_frame_len(prefix)?];
    stream.read_exact(&mut payload).await?;
    Ok(proto::read_stream_body(&payload)?)
}

/// `receive` that hands the stream back, so a half-read frame survives a
/// `select!` that picked another branch: keep the future, not the stream.
pub async fn receive_owned(mut stream: RecvStream) -> (RecvStream, Result<proto::ControlEnvelope, Error>) {
    let frame = receive(&mut stream).await;
    (stream, frame)
}

pub async fn send(stream: &mut SendStream, frame: &proto::ControlEnvelope) -> Result<(), Error> {
    let bytes = proto::write_stream_frame(frame)?;
    stream.write_all(&bytes).await?;
    Ok(())
}
