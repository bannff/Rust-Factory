#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]
#![allow(clippy::missing_errors_doc)]

//! Bounded server-side newline-delimited MCP stdio transport.

use std::sync::Arc;

use futures::{SinkExt, StreamExt};
use rmcp::{
    ErrorData, RoleServer,
    service::{RxJsonRpcMessage, TxJsonRpcMessage},
    transport::{
        Transport,
        async_rw::{JsonRpcMessageCodec, JsonRpcMessageCodecError},
    },
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::Mutex,
};
use tokio_util::{
    bytes::BytesMut,
    codec::{Decoder, FramedRead, FramedWrite},
};

/// Maximum JSON-RPC payload bytes in a newline-delimited MCP stdio message.
pub const MAX_MCP_STDIO_FRAME_BYTES: usize = 64 * 1024;
const MAX_MCP_STDIO_CODEC_BYTES: usize = MAX_MCP_STDIO_FRAME_BYTES + 1;

type Reader<R> = FramedRead<R, BoundedJsonRpcMessageCodec<RxJsonRpcMessage<RoleServer>>>;
type Writer<W> = FramedWrite<W, JsonRpcMessageCodec<TxJsonRpcMessage<RoleServer>>>;

/// Preserves the payload limit while allowing an optional CR immediately before LF.
struct BoundedJsonRpcMessageCodec<T> {
    inner: JsonRpcMessageCodec<T>,
}

impl<T> BoundedJsonRpcMessageCodec<T> {
    fn new() -> Self {
        Self {
            inner: JsonRpcMessageCodec::new_with_max_length(MAX_MCP_STDIO_CODEC_BYTES),
        }
    }

    fn payload_is_oversized(buf: &[u8]) -> bool {
        let payload = buf.strip_suffix(b"\r").unwrap_or(buf);
        payload.len() > MAX_MCP_STDIO_FRAME_BYTES
    }
}

impl<T> Decoder for BoundedJsonRpcMessageCodec<T>
where
    T: serde::de::DeserializeOwned,
{
    type Item = T;
    type Error = JsonRpcMessageCodecError;

    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        let newline = buf.iter().position(|byte| *byte == b'\n');
        if let Some(newline) = newline {
            if Self::payload_is_oversized(&buf[..newline]) {
                return Err(JsonRpcMessageCodecError::MaxLineLengthExceeded);
            }
        } else if buf.len() > MAX_MCP_STDIO_FRAME_BYTES
            && !(buf.len() == MAX_MCP_STDIO_CODEC_BYTES && buf.ends_with(b"\r"))
        {
            return Err(JsonRpcMessageCodecError::MaxLineLengthExceeded);
        }
        match self.inner.decode(buf) {
            Err(JsonRpcMessageCodecError::Serde(error))
                if matches!(
                    error.classify(),
                    serde_json::error::Category::Syntax | serde_json::error::Category::Eof
                ) =>
            {
                self.decode(buf)
            }
            result => result,
        }
    }

    fn decode_eof(&mut self, buf: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if Self::payload_is_oversized(buf) {
            return Err(JsonRpcMessageCodecError::MaxLineLengthExceeded);
        }
        match self.inner.decode_eof(buf) {
            Err(JsonRpcMessageCodecError::Serde(error))
                if matches!(
                    error.classify(),
                    serde_json::error::Category::Syntax | serde_json::error::Category::Eof
                ) =>
            {
                Ok(None)
            }
            result => result,
        }
    }
}

/// A bounded newline-delimited JSON-RPC transport for the MCP server role.
pub struct BoundedStdioTransport<R, W> {
    reader: Reader<R>,
    writer: Arc<Mutex<Option<Writer<W>>>>,
    closed: bool,
}

impl<R, W> BoundedStdioTransport<R, W>
where
    R: AsyncRead + Send + Unpin,
    W: AsyncWrite + Send + Unpin + 'static,
{
    /// Creates a server-role transport over the supplied asynchronous reader and writer.
    #[must_use]
    pub fn new(reader: R, writer: W) -> Self {
        Self {
            reader: FramedRead::new(reader, BoundedJsonRpcMessageCodec::new()),
            writer: Arc::new(Mutex::new(Some(FramedWrite::new(
                writer,
                JsonRpcMessageCodec::new_with_max_length(MAX_MCP_STDIO_FRAME_BYTES),
            )))),
            closed: false,
        }
    }

    async fn close_writer(writer: Arc<Mutex<Option<Writer<W>>>>) {
        if let Some(mut writer) = writer.lock().await.take() {
            let _ = writer.close().await;
        }
    }

    async fn close_after_framing_error(&mut self) {
        self.closed = true;
        Self::close_writer(Arc::clone(&self.writer)).await;
    }
}

impl<R, W> Transport<RoleServer> for BoundedStdioTransport<R, W>
where
    R: AsyncRead + Send + Unpin,
    W: AsyncWrite + Send + Unpin + 'static,
{
    type Error = std::io::Error;

    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleServer>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        let writer = Arc::clone(&self.writer);
        async move {
            let mut writer = writer.lock().await;
            let writer = writer.as_mut().ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotConnected, "transport is closed")
            })?;
            writer.send(item).await.map_err(Into::into)
        }
    }

    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleServer>> {
        if self.closed {
            return None;
        }
        loop {
            match self.reader.next().await {
                Some(Ok(message)) => return Some(message),
                Some(Err(JsonRpcMessageCodecError::Serde(error))) => match error.classify() {
                    serde_json::error::Category::Syntax | serde_json::error::Category::Eof => {}
                    serde_json::error::Category::Data | serde_json::error::Category::Io => {
                        let response = TxJsonRpcMessage::<RoleServer>::error(
                            ErrorData::invalid_request("Invalid request", None),
                            None,
                        );
                        if self.send(response).await.is_err() {
                            self.close_after_framing_error().await;
                            return None;
                        }
                    }
                },
                Some(Err(_)) => {
                    self.close_after_framing_error().await;
                    return None;
                }
                None => {
                    self.closed = true;
                    Self::close_writer(Arc::clone(&self.writer)).await;
                    return None;
                }
            }
        }
    }

    async fn close(&mut self) -> Result<(), Self::Error> {
        self.closed = true;
        Self::close_writer(Arc::clone(&self.writer)).await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use rmcp::{ErrorData, RoleServer, service::TxJsonRpcMessage, transport::Transport};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt, split},
        time::timeout,
    };

    use super::{BoundedStdioTransport, MAX_MCP_STDIO_FRAME_BYTES};

    const NOTIFICATION: &[u8] = br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;

    fn frame(suffix: &[u8]) -> Vec<u8> {
        [NOTIFICATION, suffix].concat()
    }

    #[tokio::test]
    async fn receives_valid_lf_and_crlf_frames() {
        for suffix in [b"\n".as_slice(), b"\r\n"] {
            let (server, client) = tokio::io::duplex(4_096);
            let (server_reader, server_writer) = split(server);
            let (_, mut client_writer) = split(client);
            let mut transport = BoundedStdioTransport::new(server_reader, server_writer);
            client_writer
                .write_all(&frame(suffix))
                .await
                .expect("write frame");
            let message = transport.receive().await.expect("bounded frame");
            assert_eq!(
                serde_json::to_value(message).expect("serialize message")["method"],
                "notifications/initialized"
            );
        }
    }

    #[tokio::test]
    async fn accepts_exact_limit_lf_and_crlf_payloads() {
        for suffix in [b"\n".as_slice(), b"\r\n"] {
            let (server, client) = tokio::io::duplex(MAX_MCP_STDIO_FRAME_BYTES * 2);
            let (server_reader, server_writer) = split(server);
            let (_, mut client_writer) = split(client);
            let mut transport = BoundedStdioTransport::new(server_reader, server_writer);
            let mut payload = NOTIFICATION.to_vec();
            payload.resize(MAX_MCP_STDIO_FRAME_BYTES, b' ');
            payload.extend_from_slice(suffix);
            client_writer
                .write_all(&payload)
                .await
                .expect("write exact-limit frame");
            assert!(transport.receive().await.is_some());
        }
    }

    #[tokio::test]
    async fn framing_overflow_is_terminal_and_writes_nothing() {
        for suffix in [b"\n".as_slice(), b"\r\n"] {
            let (server, client) = tokio::io::duplex(MAX_MCP_STDIO_FRAME_BYTES * 2);
            let (server_reader, server_writer) = split(server);
            let (mut client_reader, mut client_writer) = split(client);
            let mut transport = BoundedStdioTransport::new(server_reader, server_writer);
            let mut overflow = vec![b'x'; MAX_MCP_STDIO_FRAME_BYTES + 1];
            overflow.extend_from_slice(suffix);
            overflow.extend_from_slice(&frame(b"\n"));
            client_writer
                .write_all(&overflow)
                .await
                .expect("write overflow and successor");
            assert!(transport.receive().await.is_none());
            assert!(transport.receive().await.is_none());
            let mut output = [0_u8; 1];
            assert_eq!(
                timeout(Duration::from_millis(10), client_reader.read(&mut output))
                    .await
                    .expect("writer closed")
                    .expect("read output"),
                0
            );
        }
    }

    #[tokio::test]
    async fn rejects_non_cr_partial_payload_above_limit() {
        let (server, client) = tokio::io::duplex(MAX_MCP_STDIO_FRAME_BYTES * 2);
        let (server_reader, server_writer) = split(server);
        let (_, mut client_writer) = split(client);
        let mut transport = BoundedStdioTransport::new(server_reader, server_writer);
        client_writer
            .write_all(&vec![b'x'; MAX_MCP_STDIO_FRAME_BYTES + 1])
            .await
            .expect("write oversized partial payload");
        assert!(
            timeout(Duration::from_millis(10), transport.receive())
                .await
                .expect("overflow rejects incrementally")
                .is_none()
        );
    }

    #[tokio::test]
    async fn retains_valid_and_pending_crlf_partials_across_cancelled_receive() {
        let (server, client) = tokio::io::duplex(MAX_MCP_STDIO_FRAME_BYTES * 2);
        let (server_reader, server_writer) = split(server);
        let (_, mut client_writer) = split(client);
        let mut transport = BoundedStdioTransport::new(server_reader, server_writer);
        client_writer
            .write_all(&NOTIFICATION[..20])
            .await
            .expect("write valid partial");
        assert!(
            timeout(Duration::from_millis(10), transport.receive())
                .await
                .is_err()
        );
        client_writer
            .write_all(&[&NOTIFICATION[20..], b"\n"].concat())
            .await
            .expect("complete valid partial");
        assert!(transport.receive().await.is_some());

        let mut payload = NOTIFICATION.to_vec();
        payload.resize(MAX_MCP_STDIO_FRAME_BYTES, b' ');
        payload.push(b'\r');
        client_writer
            .write_all(&payload)
            .await
            .expect("write pending CRLF partial");
        assert!(
            timeout(Duration::from_millis(10), transport.receive())
                .await
                .is_err()
        );
        client_writer.write_all(b"\n").await.expect("complete CRLF");
        assert!(transport.receive().await.is_some());
    }

    #[tokio::test]
    async fn ignores_syntax_and_eof_json_errors_before_valid_message() {
        let (server, client) = tokio::io::duplex(4_096);
        let (server_reader, server_writer) = split(server);
        let (_, mut client_writer) = split(client);
        let mut transport = BoundedStdioTransport::new(server_reader, server_writer);
        client_writer
            .write_all(b"{bad}\n{\"jsonrpc\":\n")
            .await
            .expect("write malformed frames");
        client_writer
            .write_all(&frame(b"\n"))
            .await
            .expect("write valid frame");
        assert!(transport.receive().await.is_some());
    }

    #[tokio::test]
    async fn data_shape_errors_emit_generic_invalid_request() {
        let (server, client) = tokio::io::duplex(4_096);
        let (server_reader, server_writer) = split(server);
        let (mut client_reader, mut client_writer) = split(client);
        let mut transport = BoundedStdioTransport::new(server_reader, server_writer);
        client_writer
            .write_all(b"[]\n")
            .await
            .expect("write shape-invalid message");
        client_writer
            .write_all(&frame(b"\n"))
            .await
            .expect("write valid successor");
        assert!(transport.receive().await.is_none());
        let mut output = vec![0_u8; 512];
        let read = timeout(Duration::from_millis(100), client_reader.read(&mut output))
            .await
            .expect("invalid-request response")
            .expect("read response");
        let response = std::str::from_utf8(&output[..read]).expect("UTF-8 response");
        assert!(response.contains("Invalid request"));
        assert!(!response.contains("[]"));
    }

    #[tokio::test]
    async fn concurrent_sends_are_complete_and_non_interleaved() {
        let (server, client) = tokio::io::duplex(4_096);
        let (server_reader, server_writer) = split(server);
        let (mut client_reader, _) = split(client);
        let mut transport = BoundedStdioTransport::new(server_reader, server_writer);
        let first = transport.send(TxJsonRpcMessage::<RoleServer>::error(
            ErrorData::invalid_request("first", None),
            None,
        ));
        let second = transport.send(TxJsonRpcMessage::<RoleServer>::error(
            ErrorData::invalid_request("second", None),
            None,
        ));
        tokio::try_join!(first, second).expect("serialized sends");
        let mut output = vec![0_u8; 512];
        let read = timeout(Duration::from_millis(100), client_reader.read(&mut output))
            .await
            .expect("responses")
            .expect("read responses");
        let lines = std::str::from_utf8(&output[..read])
            .expect("UTF-8 responses")
            .lines()
            .collect::<Vec<_>>();
        assert_eq!(lines.len(), 2);
        assert!(
            lines
                .iter()
                .all(|line| serde_json::from_str::<serde_json::Value>(line).is_ok())
        );
    }

    #[tokio::test]
    async fn send_fails_after_terminal_framing_error() {
        let (server, client) = tokio::io::duplex(MAX_MCP_STDIO_FRAME_BYTES * 2);
        let (server_reader, server_writer) = split(server);
        let (_, mut client_writer) = split(client);
        let mut transport = BoundedStdioTransport::new(server_reader, server_writer);
        client_writer
            .write_all(&vec![b'x'; MAX_MCP_STDIO_FRAME_BYTES + 1])
            .await
            .expect("write overflow");
        assert!(transport.receive().await.is_none());
        assert!(
            transport
                .send(TxJsonRpcMessage::<RoleServer>::error(
                    ErrorData::invalid_request("request", None),
                    None
                ))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn eof_closes_writer_and_rejects_later_sends() {
        let (server, client) = tokio::io::duplex(4_096);
        let (server_reader, server_writer) = split(server);
        let (mut client_reader, mut client_writer) = split(client);
        let mut transport = BoundedStdioTransport::new(server_reader, server_writer);
        client_writer.shutdown().await.expect("close input");

        assert!(
            timeout(Duration::from_millis(100), transport.receive())
                .await
                .expect("EOF must terminate receive")
                .is_none()
        );
        assert!(
            transport
                .send(TxJsonRpcMessage::<RoleServer>::error(
                    ErrorData::invalid_request("request", None),
                    None
                ))
                .await
                .is_err(),
            "a terminal EOF must drop the writer before later sends"
        );
        let mut output = [0_u8; 1];
        assert_eq!(
            timeout(Duration::from_millis(100), client_reader.read(&mut output))
                .await
                .expect("writer must close")
                .expect("read closed writer"),
            0,
            "a terminal EOF must close the output side"
        );
    }
}
