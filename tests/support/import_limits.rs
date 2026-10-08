use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn streamed_import_limits_partial_chunks_and_slow_download() {
    for mode in ["exact", "overflow", "interrupted", "slow"] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/spec", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(socket.read_u8().await.unwrap());
                assert!(request.len() <= 8192);
            }
            socket.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n4\r\nabcd\r\n").await.unwrap();
            match mode {
                "slow" => {
                    // Exceed the old total deadline with continuous progress.
                    for byte in b"efgh" {
                        tokio::time::sleep(Duration::from_millis(300)).await;
                        socket
                            .write_all(&[b'1', b'\r', b'\n', *byte, b'\r', b'\n'])
                            .await
                            .unwrap();
                    }
                    socket.write_all(b"0\r\n\r\n").await.unwrap();
                }
                "interrupted" => {
                    socket.write_all(b"4\r\ne").await.unwrap();
                }
                _ => {
                    socket.write_all(b"4\r\nefgh\r\n").await.unwrap();
                    if mode == "overflow" {
                        socket.write_all(b"1\r\ni\r\n").await.unwrap();
                    }
                    socket.write_all(b"0\r\n\r\n").await.unwrap();
                }
            }
        });
        let mut p = policy(None);
        p.max_spec_bytes = 8;
        p.read_timeout_seconds = Some(1);
        let result = tokio::time::timeout(Duration::from_secs(5), import::fixture_fetch(&p, &url))
            .await
            .expect("fixture completes");
        if mode == "exact" || mode == "slow" {
            assert_eq!(result.unwrap(), b"abcdefgh");
        } else {
            let error = format!("{:#}", result.unwrap_err());
            assert!(
                error.contains(match mode {
                    "overflow" => "spec_too_large",
                    "interrupted" => "source_read_failed",
                    _ => unreachable!(),
                }),
                "{mode}: {error}"
            );
        }
        server.abort();
    }
}

#[test]
fn allowed_origins_apply_to_spec_base_and_explicit_override() {
    let mut p = policy(None);
    p.allowed_origins = vec![
        "https://api.example.com".into(),
        "https://spec.example.com:8443".into(),
    ];
    for allowed in [
        "https://api.example.com:443/spec",
        "https://spec.example.com:8443/spec",
    ] {
        assert!(import::endpoint(&p, allowed).is_ok());
    }
    for denied in [
        "https://api.example.com:8443/spec",
        "https://spec.example.com/spec",
        "https://api.example.com.attacker.com/spec",
        "https://other.example.com/spec",
    ] {
        assert!(import::endpoint(&p, denied).is_err());
    }
    let mut c: Candidate =
        serde_json::from_value(candidate("origins", "process", "server")).unwrap();
    c.spec_url = "https://spec.example.com:8443/spec".into();
    let mut doc = spec();
    assert!(
        import::build(
            &p,
            &c,
            &Uuid::new_v4().to_string(),
            &serde_json::to_vec(&doc).unwrap()
        )
        .is_ok()
    );
    c.base_url = Some("https://other.example.com".into());
    assert!(
        import::build(
            &p,
            &c,
            &Uuid::new_v4().to_string(),
            &serde_json::to_vec(&doc).unwrap()
        )
        .is_err()
    );
    c.base_url = None;
    doc["servers"] = json!([{"url":"https://other.example.com"}]);
    assert!(
        import::build(
            &p,
            &c,
            &Uuid::new_v4().to_string(),
            &serde_json::to_vec(&doc).unwrap()
        )
        .is_err()
    );
    c.spec_url = "https://other.example.com/spec".into();
    doc = spec();
    assert!(
        import::build(
            &p,
            &c,
            &Uuid::new_v4().to_string(),
            &serde_json::to_vec(&doc).unwrap()
        )
        .is_err()
    );
}
