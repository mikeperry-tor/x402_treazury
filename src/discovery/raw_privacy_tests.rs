//! Real MCP framing/service logging, before manual header validation and after output.
use super::*;
use rmcp::ServiceExt;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

const SECRET: &str = "HEADER_PRIVACY_SECRET";

pub(super) fn trace_log() -> tempfile::NamedTempFile {
    let log = tempfile::NamedTempFile::new().unwrap();
    tracing::subscriber::set_global_default(
        tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(tracing::Level::TRACE)
            .with_writer(log.reopen().unwrap())
            .finish(),
    )
    .unwrap();
    log
}

fn arguments() -> Value {
    json!({"method":"POST","url":"https://api.example.com/api/v1/services","headers":{"Authorization":format!("Bearer {SECRET}"),"X-Label":SECRET,"Idempotency-Key":SECRET}})
}

pub(super) async fn exercise(manager: &Arc<Manager>, local: &reqwest::Client, base: &str) {
    for arguments in [
        arguments(),
        json!({"method":"POST","url":"https://api.example.com/api/v1/services","headers":{"X-Label":{"value":SECRET}}}),
    ] {
        let valid = arguments["headers"]["X-Label"].is_string();
        let result = local.post(format!("{base}/mcp"))
            .bearer_auth("test-token")
            .header("accept", "application/json, text/event-stream")
            .json(&json!({"jsonrpc":"2.0","id":700,"method":"tools/call","params":{"name":raw::NAME,"arguments":arguments}}))
            .send().await.unwrap().mcp_json::<Value>().await.unwrap();
        assert_eq!(
            result["result"]["isError"].as_bool().unwrap_or(false),
            !valid
        );
        assert_eq!(
            result.to_string().contains(SECRET),
            valid,
            "only upstream output may echo manual secrets"
        );
    }
    let malformed = local.post(format!("{base}/mcp"))
        .bearer_auth("test-token")
        .header("accept", "application/json, text/event-stream")
        .json(&json!({"jsonrpc":SECRET,"id":701,"method":"tools/call","params":{"name":raw::NAME,"arguments":arguments()}}))
        .send().await.unwrap();
    assert_eq!(
        malformed.status(),
        reqwest::StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
    assert!(!malformed.text().await.unwrap().contains(SECRET));
    // A body-reader error can itself carry arbitrary prose before JSON parsing.
    use tower::ServiceExt as _;
    let body = axum::body::Body::from_stream(futures_util::stream::once(async {
        Err::<axum::body::Bytes, _>(std::io::Error::other(SECRET))
    }));
    let request = axum::http::Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("host", "localhost")
        .header("authorization", "Bearer test-token")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .body(body)
        .unwrap();
    let response = crate::server::http_app(server(manager, "writer"), "test-token".into())
        .oneshot(request)
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        axum::http::StatusCode::INTERNAL_SERVER_ERROR
    );
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains(SECRET));
    use rmcp::transport::Transport;
    let mut failed_reader =
        rmcp::transport::async_rw::AsyncRwTransport::<rmcp::RoleServer, _, _>::new(
            PrivateReaderError,
            tokio::io::sink(),
        );
    assert!(failed_reader.receive().await.is_none());
    let (client_io, server_io) = tokio::io::duplex(65536);
    let endpoint = server(manager, "writer");
    let task = tokio::spawn(async move {
        endpoint
            .serve(server_io)
            .await
            .unwrap()
            .waiting()
            .await
            .unwrap();
    });
    let (read, mut write) = tokio::io::split(client_io);
    let mut read = tokio::io::BufReader::new(read);
    send(&mut write, json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":SECRET,"version":"1"}}})).await;
    let initialized = receive(&mut read).await;
    assert!(initialized["result"].is_object());
    send(
        &mut write,
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    )
    .await;
    send(&mut write, json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":raw::NAME,"arguments":arguments()}})).await;
    assert!(receive(&mut read).await.to_string().contains(SECRET));
    send(&mut write, json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":999,"reason":SECRET}})).await;
    // Syntax and data failures both occur before Server::invoke_output.
    write
        .write_all(format!("{{\"headers\":\"{SECRET}\", broken\n").as_bytes())
        .await
        .unwrap();
    send(&mut write, json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":raw::NAME,"arguments":SECRET}})).await;
    let invalid = receive(&mut read).await;
    assert!(invalid["error"].is_object());
    send(&mut write, json!({"jsonrpc":"2.0","id":4,"method":"ping"})).await;
    assert_eq!(receive(&mut read).await["id"], 4);
    drop(write);
    drop(read);
    tokio::time::timeout(std::time::Duration::from_secs(10), task)
        .await
        .unwrap()
        .unwrap();
}

async fn send(write: &mut (impl tokio::io::AsyncWrite + Unpin), value: Value) {
    write
        .write_all(format!("{value}\n").as_bytes())
        .await
        .unwrap();
}

async fn receive(read: &mut (impl tokio::io::AsyncBufRead + Unpin)) -> Value {
    let mut line = String::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        read.read_line(&mut line),
    )
    .await
    .unwrap()
    .unwrap();
    serde_json::from_str(&line).unwrap()
}

pub(super) fn assert_private(log: &tempfile::NamedTempFile) {
    let logs = std::fs::read_to_string(log.path()).unwrap();
    assert!(
        logs.contains("received request"),
        "the real SDK service was observed"
    );
    assert!(
        logs.contains("Sending MCP message"),
        "the real HTTP wire was observed"
    );
    assert!(
        logs.contains("MCP message parsing failed"),
        "the real stdio codec was observed"
    );
    assert!(
        !logs.contains(SECRET),
        "MCP diagnostics exposed manual input or upstream echo"
    );
}

struct PrivateReaderError;
impl tokio::io::AsyncRead for PrivateReaderError {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
        _: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Err(std::io::Error::other(SECRET)))
    }
}
