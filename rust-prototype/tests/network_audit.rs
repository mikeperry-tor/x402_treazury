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
#[test]
fn runtime_egress_constructors_are_centralized() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = vec![];
    walk(&root.join("src"), &mut files);
    walk(&root.join("examples"), &mut files);
    for path in files {
        let relative = path.strip_prefix(root).unwrap().to_str().unwrap();
        // The consensus harness controls disposable nodes independently of the app.
        if matches!(relative, "src/network.rs" | "src/treasury/regtest.rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        for forbidden in [
            "reqwest::Client::new(",
            "reqwest::Client::builder(",
            "reqwest::get(",
            "GrpcIndexer::new(",
            "GrpcIndexer::new_lazy(",
            "set_indexer_uri(",
            "Endpoint::from_shared(",
            "TcpStream::connect(",
            "UdpSocket::bind(",
            "lookup_host(",
        ] {
            assert!(
                !text.contains(forbidden),
                "{relative} bypasses network factory: {forbidden}"
            );
        }
    }
}
