//! Newline-delimited JSON framing over any async stream. Transport-independent, so the
//! protocol is tested over in-memory duplex streams and runs over named pipes in production.

use crate::protocol::{Request, Response};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::io;
use std::sync::Arc;
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader,
};

/// Upper bound on one message. A profile or a status for many GPUs is a few KB.
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;

#[derive(Debug)]
pub enum Incoming<T> {
    Message(T),
    Malformed(String),
    Closed,
}

pub async fn read_message<T, R>(reader: &mut R) -> io::Result<Incoming<T>>
where
    T: DeserializeOwned,
    R: AsyncBufRead + Unpin,
{
    let mut buf = Vec::new();
    let n = reader.take(MAX_MESSAGE_BYTES as u64 + 1).read_until(b'\n', &mut buf).await?;
    if n == 0 {
        return Ok(Incoming::Closed);
    }
    if buf.len() > MAX_MESSAGE_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "message exceeds the size limit"));
    }
    match serde_json::from_slice(&buf) {
        Ok(v) => Ok(Incoming::Message(v)),
        Err(e) => Ok(Incoming::Malformed(e.to_string())),
    }
}

pub async fn write_message<T, W>(writer: &mut W, msg: &T) -> io::Result<()>
where
    T: Serialize,
    W: AsyncWrite + Unpin,
{
    let mut bytes = serde_json::to_vec(msg).map_err(io::Error::other)?;
    bytes.push(b'\n');
    writer.write_all(&bytes).await?;
    writer.flush().await
}

/// Handles one request. Runs on a blocking thread, so it may take locks and call drivers.
pub type Handler = Arc<dyn Fn(Request) -> Response + Send + Sync>;

/// Serve one connection until the client hangs up.
pub async fn serve_connection<S>(stream: S, handler: Handler) -> io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let (r, mut w) = tokio::io::split(stream);
    let mut r = BufReader::new(r);
    loop {
        let response = match read_message::<Request, _>(&mut r).await? {
            Incoming::Closed => return Ok(()),
            Incoming::Malformed(e) => Response::Error { message: format!("malformed request: {e}") },
            Incoming::Message(req) => {
                let h = Arc::clone(&handler);
                tokio::task::spawn_blocking(move || h(req))
                    .await
                    .unwrap_or_else(|e| Response::Error { message: format!("handler panicked: {e}") })
            }
        };
        write_message(&mut w, &response).await?;
    }
}

/// A request/response client over any stream.
#[derive(Debug)]
pub struct Client<S> {
    reader: BufReader<tokio::io::ReadHalf<S>>,
    writer: tokio::io::WriteHalf<S>,
}

impl<S: AsyncRead + AsyncWrite> Client<S> {
    pub fn new(stream: S) -> Self {
        let (r, w) = tokio::io::split(stream);
        Client { reader: BufReader::new(r), writer: w }
    }

    pub async fn call(&mut self, request: &Request) -> io::Result<Response> {
        write_message(&mut self.writer, request).await?;
        match read_message::<Response, _>(&mut self.reader).await? {
            Incoming::Message(r) => Ok(r),
            Incoming::Malformed(e) => Err(io::Error::new(io::ErrorKind::InvalidData, e)),
            Incoming::Closed => {
                Err(io::Error::new(io::ErrorKind::UnexpectedEof, "service closed the connection"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::PROTOCOL_VERSION;

    fn echo_handler() -> Handler {
        Arc::new(|req| match req {
            Request::Hello { protocol } => Response::Hello { protocol, service_version: "test".into() },
            other => Response::Ok { message: format!("{other:?}") },
        })
    }

    #[tokio::test]
    async fn request_response_over_duplex() {
        let (a, b) = tokio::io::duplex(4096);
        let server = tokio::spawn(serve_connection(b, echo_handler()));
        let mut client = Client::new(a);
        let r = client.call(&Request::Hello { protocol: PROTOCOL_VERSION }).await.unwrap();
        assert_eq!(r, Response::Hello { protocol: PROTOCOL_VERSION, service_version: "test".into() });
        let r = client.call(&Request::Status).await.unwrap();
        assert!(matches!(r, Response::Ok { .. }));
        drop(client);
        server.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn malformed_requests_get_an_error_and_the_connection_survives() {
        let (a, b) = tokio::io::duplex(4096);
        tokio::spawn(serve_connection(b, echo_handler()));
        let (r, mut w) = tokio::io::split(a);
        let mut r = BufReader::new(r);
        w.write_all(b"{not json}\n").await.unwrap();
        let Incoming::Message(Response::Error { message }) =
            read_message::<Response, _>(&mut r).await.unwrap()
        else {
            panic!("expected an error response");
        };
        assert!(message.contains("malformed"));
        write_message(&mut w, &Request::Status).await.unwrap();
        assert!(matches!(
            read_message::<Response, _>(&mut r).await.unwrap(),
            Incoming::Message(Response::Ok { .. })
        ));
    }

    #[tokio::test]
    async fn oversized_messages_are_refused() {
        let (a, b) = tokio::io::duplex(64 * 1024);
        let server = tokio::spawn(serve_connection(b, echo_handler()));
        let (_r, mut w) = tokio::io::split(a);
        let big = vec![b'x'; MAX_MESSAGE_BYTES + 10];
        // The server errors out mid-stream; the write may or may not complete first.
        let _ = w.write_all(&big).await;
        let result = server.await.unwrap();
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
    }
}
