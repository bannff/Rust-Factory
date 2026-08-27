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
use tokio_util::codec::{FramedRead, FramedWrite};

pub(crate) const MAX_MCP_STDIO_FRAME_BYTES: usize = 64 * 1024;

type Reader<R> = FramedRead<R, JsonRpcMessageCodec<RxJsonRpcMessage<RoleServer>>>;
type Writer<W> = FramedWrite<W, JsonRpcMessageCodec<TxJsonRpcMessage<RoleServer>>>;

/// A bounded newline-delimited JSON-RPC stdio transport for the MCP server role.
pub(crate) struct BoundedStdioTransport<R, W> {
    reader: Reader<R>,
    writer: Arc<Mutex<Option<Writer<W>>>>,
    closed: bool,
}

impl<R, W> BoundedStdioTransport<R, W>
where
    R: AsyncRead + Send + Unpin,
    W: AsyncWrite + Send + Unpin + 'static,
{
    pub(crate) fn new(reader: R, writer: W) -> Self {
        Self {
            reader: FramedRead::new(
                reader,
                JsonRpcMessageCodec::new_with_max_length(MAX_MCP_STDIO_FRAME_BYTES),
            ),
            writer: Arc::new(Mutex::new(Some(FramedWrite::new(
                writer,
                JsonRpcMessageCodec::new_with_max_length(MAX_MCP_STDIO_FRAME_BYTES),
            )))),
            closed: false,
        }
    }

    async fn close_writer(writer: Arc<Mutex<Option<Writer<W>>>>) {
        writer.lock().await.take();
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

    use rmcp::transport::Transport;
    use tokio::{
        io::{AsyncWriteExt, split},
        time::timeout,
    };

    use super::{BoundedStdioTransport, MAX_MCP_STDIO_FRAME_BYTES};

    #[tokio::test]
    async fn receives_bounded_lf_and_crlf_frames() {
        for suffix in [b"\n".as_slice(), b"\r\n"] {
            let (server, client) = tokio::io::duplex(4_096);
            let (server_reader, server_writer) = split(server);
            let (_, mut client_writer) = split(client);
            let mut transport = BoundedStdioTransport::new(server_reader, server_writer);

            client_writer
                .write_all(
                    [
                        br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
                        suffix,
                    ]
                    .concat()
                    .as_slice(),
                )
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
    async fn rejects_fragmented_oversized_frame_before_newline_and_stays_closed() {
        let (server, client) = tokio::io::duplex(MAX_MCP_STDIO_FRAME_BYTES * 2);
        let (server_reader, server_writer) = split(server);
        let (_, mut client_writer) = split(client);
        let mut transport = BoundedStdioTransport::new(server_reader, server_writer);

        client_writer
            .write_all(&vec![b'x'; MAX_MCP_STDIO_FRAME_BYTES])
            .await
            .expect("write first fragment");
        assert!(
            timeout(Duration::from_millis(10), transport.receive())
                .await
                .is_err(),
            "the first fragment has no delimiter and must remain pending"
        );
        client_writer
            .write_all(b"x\n{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n")
            .await
            .expect("write oversized frame and valid successor");

        assert!(transport.receive().await.is_none());
        assert!(transport.receive().await.is_none());
    }

    #[tokio::test]
    async fn retains_in_limit_fragment_across_cancelled_receive() {
        let (server, client) = tokio::io::duplex(4_096);
        let (server_reader, server_writer) = split(server);
        let (_, mut client_writer) = split(client);
        let mut transport = BoundedStdioTransport::new(server_reader, server_writer);

        client_writer
            .write_all(br#"{"jsonrpc":"2.0","method":"notifications/"#)
            .await
            .expect("write partial frame");
        assert!(
            timeout(Duration::from_millis(10), transport.receive())
                .await
                .is_err()
        );

        client_writer
            .write_all(b"initialized\"}\n")
            .await
            .expect("complete frame");
        let message = transport.receive().await.expect("reassembled frame");
        assert_eq!(
            serde_json::to_value(message).expect("serialize message")["method"],
            "notifications/initialized"
        );
    }
}
