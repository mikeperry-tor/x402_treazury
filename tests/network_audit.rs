//! Constructor inventory tripwire; dependency review and OS enforcement remain separate.
use std::path::Path;
fn walk(dir: &Path, files: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            walk(&path, files);
        } else if path.extension().is_some_and(|x| x == "rs") {
            files.push(path);
        }
    }
}
fn operational_sources(root: &Path) -> Vec<std::path::PathBuf> {
    let mut files = vec![
        root.join("tests/live/live_integration.rs"),
        root.join("tests/live/provider_preflight.rs"),
    ];
    for directory in [
        "src",
        "examples",
        "tests/live/runner",
        "tools/providers",
        "tools/diagnostics",
    ] {
        walk(&root.join(directory), &mut files);
    }
    files
}

#[test]
fn relocated_operational_sources_remain_in_constructor_inventory() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let files = operational_sources(root);
    for name in [
        "tests/live/live_integration.rs",
        "tests/live/provider_preflight.rs",
        "tests/live/runner/driver.rs",
        "tests/live/runner/tor/archive.rs",
        "tools/providers/snapshot_spec.rs",
        "tools/providers/snapshot_agentutility.rs",
        "tools/diagnostics/diagnose_credit.rs",
        "tools/diagnostics/quote_near.rs",
    ] {
        assert!(files.contains(&root.join(name)), "missing {name}");
    }
}

#[test]
fn runtime_egress_constructors_are_centralized() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for path in operational_sources(root) {
        let relative = path.strip_prefix(root).unwrap();
        // The consensus harness controls disposable nodes independently of the app.
        if relative == Path::new("src/network.rs")
            || relative == Path::new("src/treasury/regtest.rs")
        {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        for forbidden in [
            "reqwest::Client::new(",
            "reqwest::Client::builder(",
            "reqwest::get(",
            "GrpcIndexer::new(",
            "GrpcIndexer::new_lazy(",
            "CompactTxStreamerClient::new(",
            "CompactTxStreamerClient::connect(",
            "CompactTxStreamerClient::with_origin(",
            "CompactTxStreamerClient::with_interceptor(",
            "set_indexer_uri(",
            "Endpoint::from_shared(",
            "TcpStream::connect(",
            "UdpSocket::bind(",
            "lookup_host(",
        ] {
            assert!(
                !text.contains(forbidden),
                "{} bypasses network factory: {forbidden}",
                relative.display()
            );
        }
    }
}
