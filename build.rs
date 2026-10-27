#[path = "src/build_identity/inputs.rs"]
mod inputs;
use std::{path::Path, process::Command};
fn main() {
    let root = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let root = Path::new(&root);
    for input in [
        "src",
        "tests/live/live_integration.rs",
        "tests/live/runner",
        "vendor",
        "Cargo.toml",
        "Cargo.lock",
        "tools/protoc/Cargo.toml",
        "tools/protoc/Cargo.lock",
        "tools/protoc/src",
        "build.rs",
        "rust-toolchain.toml",
        ".git/HEAD",
        ".git/index",
        ".git/refs",
    ] {
        println!("cargo:rerun-if-changed={input}");
    }
    for input in inputs::provider_files() {
        println!("cargo:rerun-if-changed={input}");
    }
    for (name, value) in [
        (
            "TREAZURY_SOURCE_HASH",
            inputs::source_hash(root).expect("fingerprint compilation inputs"),
        ),
        (
            "TREAZURY_LOCK_HASH",
            inputs::file_hash(&root.join("Cargo.lock")).unwrap(),
        ),
        ("TREAZURY_BUILD_TARGET", std::env::var("TARGET").unwrap()),
        ("TREAZURY_BUILD_PROFILE", std::env::var("PROFILE").unwrap()),
        (
            "TREAZURY_BUILD_RUSTC",
            output(&std::env::var("RUSTC").unwrap(), &["--version"], root),
        ),
        (
            "TREAZURY_BUILD_REVISION",
            git(&["rev-parse", "HEAD"], root).unwrap_or_else(|| "unavailable".into()),
        ),
        (
            "TREAZURY_BUILD_DIRTY",
            {
                let mut paths = vec![
                    "status",
                    "--porcelain",
                    "--untracked-files=all",
                    "--",
                    "src",
                    "tests/live/live_integration.rs",
                    "tests/live/runner",
                    "vendor",
                    "Cargo.toml",
                    "Cargo.lock",
                    "tools/protoc/Cargo.toml",
                    "tools/protoc/Cargo.lock",
                    "tools/protoc/src",
                    "build.rs",
                    "rust-toolchain.toml",
                ];
                paths.extend_from_slice(inputs::provider_files());
                git(&paths, root).is_none_or(|s| !s.is_empty())
            }
            .to_string(),
        ),
    ] {
        assert!(!value.contains(['\n', '\r']));
        println!("cargo:rustc-env={name}={value}");
    }
}
fn output(command: &str, args: &[&str], root: &Path) -> String {
    let out = Command::new(command)
        .args(args)
        .current_dir(root)
        .output()
        .expect("build metadata command");
    assert!(out.status.success(), "build metadata command failed");
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

fn git(args: &[&str], root: &Path) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8(out.stdout).ok())
        .flatten()
        .map(|s| s.trim().to_owned())
}
