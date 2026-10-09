use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use x402_treazury::mcp_wire::McpResponse;
use x402_treazury::output::{HttpOutput, ImageLimits, ResponseMapping};
const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aP8sAAAAASUVORK5CYII=";
fn response(bytes: Vec<u8>, mime: &str, paid: bool) -> HttpOutput {
    HttpOutput {
        advisories: vec![],
        bytes,
        mime_type: Some(mime.into()),
        paid_submission: paid,
    }
}
fn mapping(fields: Value) -> ResponseMapping {
    serde_json::from_value(json!({"images":fields})).unwrap()
}
#[test]
fn image_envelopes_preserve_metadata_and_text_remains_unchanged() {
    let fields = mapping(json!([
        {"pointer":"/data/0/b64_json","mime_pointer":"/data/0/media_type"},
        {"pointer":"/escaped~1key/~0image","encoding":"data_uri"}
    ]));
    let doc = json!({"data":[{"b64_json":PNG,"media_type":"image/png"}],"escaped/key":{"~image":format!("data:image/png;base64,{PNG}")},"usage":{"cost":0.05},"truncated":true});
    let result = response(serde_json::to_vec(&doc).unwrap(), "application/json", false)
        .render(Some(&fields), &Default::default())
        .unwrap();
    assert_eq!(result.images.len(), 2);
    assert!(
        result
            .images
            .iter()
            .all(|image| image.data == PNG && image.mime_type == "image/png")
    );
    assert!(!result.text.contains(PNG));
    let metadata: Value = serde_json::from_str(result.text.split_once('\n').unwrap().1).unwrap();
    assert_eq!(metadata["usage"], doc["usage"]);
    assert_eq!(metadata["truncated"], true);
    assert_eq!(metadata["escaped/key"]["~image"]["treazury_attachment"], 2);
    for mime in [
        "text/plain",
        "application/json",
        "application/problem+json",
        "application/xml",
        "application/atom+xml",
        "",
    ] {
        assert_eq!(
            response(b"{\"hello\":1}".to_vec(), mime, false)
                .render(None, &Default::default())
                .unwrap()
                .into_text()
                .unwrap(),
            "{\"hello\":1}"
        );
    }
    let raw = response(STANDARD.decode(PNG).unwrap(), "image/png", false)
        .render(Some(&fields), &Default::default())
        .unwrap();
    assert_eq!(raw.images[0].data, PNG); // Binary alternative bypasses JSON mapping.
}
#[test]
fn malformed_and_unsupported_media_fail_without_leaking_payloads() {
    let fields = mapping(json!([{"pointer":"/image","mime_type":"image/png"}]));
    for body in [
        json!({}),
        json!({"image":null}),
        json!({"image":"private-not-base64"}),
        json!({"image":STANDARD.encode(b"not png")}),
    ] {
        let err = response(serde_json::to_vec(&body).unwrap(), "application/json", true)
            .render(Some(&fields), &Default::default())
            .unwrap_err();
        let message = format!("{err:#}");
        assert!(message.contains("payment may already have settled"));
        assert!(!message.contains("private-not-base64"));
    }
    for (bytes, mime) in [
        (vec![255], "text/plain"),
        (b"%PDF-1".to_vec(), "application/pdf"),
        (STANDARD.decode(PNG).unwrap(), "image/jpeg"),
    ] {
        assert!(
            response(bytes, mime, false)
                .render(None, &Default::default())
                .is_err()
        );
    }
    for fields in [
        json!([]),
        json!([{"pointer":"","mime_type":"image/png"}]),
        json!([{"pointer":"/x~bad","mime_type":"image/png"}]),
        json!([{"pointer":"/x"}]),
        json!([{"pointer":"/x","mime_type":"image/png","mime_pointer":"/m"}]),
        json!([{"pointer":"/x","mime_type":"image/png"},{"pointer":"/x/y","mime_type":"image/png"}]),
    ] {
        assert!(mapping(fields).validate(&Default::default()).is_err());
    }
    let data_uri =
        mapping(json!([{"pointer":"/image","encoding":"data_uri","mime_type":"image/jpeg"}]));
    assert!(
        response(
            serde_json::to_vec(&json!({"image":format!("data:image/png;base64,{PNG}")})).unwrap(),
            "application/json",
            false
        )
        .render(Some(&data_uri), &Default::default())
        .is_err()
    );
}

#[derive(Clone)]
struct Capture(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
impl std::io::Write for Capture {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
#[test]
fn image_limits_reject_whole_results_and_log_each_cutoff() {
    let capture = Capture(Default::default());
    let writer = capture.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let bytes = STANDARD.decode(PNG).unwrap();
    let fields = mapping(
        json!([{"pointer":"/a","mime_type":"image/png"},{"pointer":"/b","mime_type":"image/png"}]),
    );
    let doc = serde_json::to_vec(&json!({"a":PNG,"b":PNG})).unwrap();
    for (limits, setting) in [
        (
            ImageLimits {
                max_image_bytes: bytes.len() - 1,
                ..Default::default()
            },
            "image_limits.max_image_bytes",
        ),
        (
            ImageLimits {
                max_total_bytes: bytes.len() * 2 - 1,
                ..Default::default()
            },
            "image_limits.max_total_bytes",
        ),
        (
            ImageLimits {
                max_images: 1,
                ..Default::default()
            },
            "image_limits.max_images",
        ),
    ] {
        let error = response(doc.clone(), "application/json", true)
            .render(Some(&fields), &limits)
            .unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains(setting));
        assert!(message.contains("no partial content"));
        assert!(
            String::from_utf8(capture.0.lock().unwrap().clone())
                .unwrap()
                .contains(setting)
        );
    }
    let exact = ImageLimits {
        max_image_bytes: bytes.len(),
        max_total_bytes: bytes.len() * 2,
        max_images: 2,
    };
    assert_eq!(
        response(doc, "application/json", false)
            .render(Some(&fields), &exact)
            .unwrap()
            .images
            .len(),
        2
    );
}

async fn serve(app: axum::Router) -> (String, tokio::task::JoinHandle<()>) {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    (
        url,
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() }),
    )
}
#[tokio::test]
async fn authenticated_mcp_returns_real_image_blocks_and_never_retries_conversion_errors() {
    use axum::{Router, routing::get};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use x402_treazury::{
        catalog::{Config, build_tools},
        payment::{PaidClient, Payer, SpendPolicy},
        server::{Server, http_app},
    };
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let (vendor, vendor_task) = serve(
        Router::new()
            .route(
                "/raw",
                get(|| async {
                    (
                        [("content-type", "image/png; ignored=parameter")],
                        STANDARD.decode(PNG).unwrap(),
                    )
                }),
            )
            .route(
                "/json",
                get(|| async { axum::Json(json!({"image":PNG,"model":"fixture"})) }),
            )
            .route(
                "/bad",
                get(move || {
                    let c = count.clone();
                    async move {
                        c.fetch_add(1, Ordering::SeqCst);
                        ([("content-type", "image/png")], "not an image")
                    }
                }),
            ),
    )
    .await;
    let cfg = Config {
        response_mappings: std::collections::BTreeMap::from([(
            "GET /json".into(),
            mapping(json!([{"pointer":"/image","mime_type":"image/png"}])),
        )]),
        ..Default::default()
    };
    let tools = build_tools(
        &cfg,
        &json!({"paths":{"/raw":{"get":{}},"/json":{"get":{}},"/bad":{"get":{}}}}),
        "media",
    )
    .unwrap();
    let client = PaidClient::new(
        Payer::new(&format!("{:064x}", 1), SpendPolicy::dollars("1").unwrap()).unwrap(),
    );
    let server = Server::new(tools, client, vendor, None, Some(8));
    let (base, task) = serve(http_app(server, "fixture".into())).await;
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    for name in ["media_raw", "media_json", "media_bad", "media_unknown"] {
        let result:Value=http.post(format!("{base}/mcp")).bearer_auth("fixture").header("accept","application/json, text/event-stream")
            .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":{}}}))
            .send().await.unwrap().mcp_json().await.unwrap();
        if name == "media_bad" || name == "media_unknown" {
            assert_eq!(result["result"]["isError"], true);
            continue;
        }
        assert_eq!(result["result"]["content"][1]["type"], "image");
        assert_eq!(result["result"]["content"][1]["data"], PNG);
        assert_eq!(result["result"]["content"][1]["mimeType"], "image/png");
        assert!(
            result["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("truncated by --max-response-chars")
        );
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    task.abort();
    vendor_task.abort();
}

#[tokio::test]
async fn paid_image_decode_failure_is_explicit_and_never_replays() {
    use axum::{
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::get,
    };
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use x402_treazury::{
        catalog::RoutedRequest,
        payment::{PaidClient, Payer, SpendPolicy, USDC},
    };
    let signed = Arc::new(AtomicUsize::new(0));
    let unsigned = Arc::new(AtomicUsize::new(0));
    let (s, u) = (signed.clone(), unsigned.clone());
    let (base,task)=serve(axum::Router::new().route("/image",get(move|headers:HeaderMap| {
        let(s,u)=(s.clone(),u.clone());async move {
            if headers.contains_key("payment-signature") {
                s.fetch_add(1,Ordering::SeqCst);
                return ([("content-type","image/png")],"bad image after settlement").into_response();
            }
            u.fetch_add(1,Ordering::SeqCst);
            let challenge=json!({"x402Version":2,"resource":{"url":"https://vendor.example/image","mimeType":"image/png"},"accepts":[{"scheme":"exact","network":"eip155:8453","asset":USDC,"amount":"1000","payTo":"0x0000000000000000000000000000000000000003","maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}}]});
            (StatusCode::PAYMENT_REQUIRED,[("payment-required",STANDARD.encode(serde_json::to_vec(&challenge).unwrap()))]).into_response()
        }
    }))).await;
    let client = PaidClient::new(
        Payer::new(&format!("{:064x}", 1), SpendPolicy::dollars("1").unwrap()).unwrap(),
    );
    let output = client
        .execute_response(RoutedRequest {
            method: "GET".into(),
            url: format!("{base}/image"),
            query: Default::default(),
            body: None,
        })
        .await
        .unwrap();
    let error = output.render(None, &Default::default()).unwrap_err();
    assert!(format!("{error:#}").contains("payment may already have settled"));
    assert_eq!(signed.load(Ordering::SeqCst), 1);
    assert_eq!(unsigned.load(Ordering::SeqCst), 1);
    task.abort();
}

#[tokio::test]
async fn toml_response_mappings_compose_and_invalid_unused_mappings_fail() {
    use x402_treazury::config;
    let dir = tempfile::tempdir().unwrap();
    let provider = dir.path().join("provider.toml");
    std::fs::write(
        &provider,
        r#"
spec="spec.json"
[image_limits]
max_image_bytes=512
max_total_bytes=1024
max_images=2
[response_mappings."POST /images"]
images=[{pointer="/data/0/image",mime_type="image/png"}]
"#,
    )
    .unwrap();
    let file = dir.path().join("source.toml");
    let resolved = config::resolve(toml::from_str("extends='provider.toml'").unwrap(), &file)
        .await
        .unwrap();
    assert_eq!(resolved.settings.image_limits.max_image_bytes, 512);
    assert!(
        resolved
            .settings
            .response_mappings
            .contains_key("POST /images")
    );
    assert!(resolved.origins["response_mappings"].ends_with("provider.toml"));
    let cleared = config::resolve(
        toml::from_str("extends='provider.toml'\nresponse_mappings={}").unwrap(),
        &file,
    )
    .await
    .unwrap();
    assert!(cleared.settings.response_mappings.is_empty());
    for invalid in [
        "spec='x'\n[image_limits]\nmax_images=0",
        "spec='x'\n[response_mappings.'GET /unused']\nimages=[{pointer='/x',mime_type='image/svg+xml'}]",
        "spec='x'\n[response_mappings.'bad']\nimages=[{pointer='/x',mime_type='image/png'}]",
    ] {
        assert!(
            config::resolve(toml::from_str(invalid).unwrap(), &file)
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn reviewed_provider_envelopes_render_without_guessing_fields() {
    let cfg = x402_treazury::config::load(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("providers/agent402.toml"),
    )
    .await
    .unwrap()
    .settings;
    for (operation, mapping) in &cfg.response_mappings {
        let body = if operation.ends_with("image-crop") {
            json!({"dataUri":format!("data:image/png;base64,{PNG}"),"width":1,"height":1})
        } else if operation.contains("/v1/images/") {
            json!({"data":[{"b64_json":PNG,"media_type":"image/png"}],"usage":{"prompt_tokens":14}})
        } else {
            json!({"image":PNG,"model":"fixture","revised_prompt":"retained"})
        };
        let output = response(
            serde_json::to_vec(&body).unwrap(),
            "application/json",
            false,
        )
        .render(Some(mapping), &cfg.image_limits)
        .unwrap();
        assert_eq!(output.images.len(), 1, "{operation}");
        assert_eq!(output.images[0].data, PNG, "{operation}");
        assert!(!output.text.contains(PNG), "{operation}");
    }
    assert_eq!(cfg.response_mappings.len(), 7);
    // Format identification only: Treazury does not decode raster pixels.
    for (mime, bytes) in [
        ("image/jpeg", b"\xff\xd8\xff\xe0".as_slice()),
        ("image/webp", b"RIFF\0\0\0\0WEBP".as_slice()),
    ] {
        let output = response(bytes.to_vec(), mime, false)
            .render(None, &Default::default())
            .unwrap();
        assert_eq!(output.images[0].mime_type, mime);
        assert_eq!(STANDARD.decode(&output.images[0].data).unwrap(), bytes);
    }
}
