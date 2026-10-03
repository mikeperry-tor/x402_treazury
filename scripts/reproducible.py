#!/usr/bin/env python3
"""Compare two clean, network-denied macOS release builds of committed HEAD."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
SANDBOX = ["/usr/bin/sandbox-exec", "-p", "(version 1)(allow default)(deny network*)"]
# BLAKE2b-512 constants from locked zcash_proofs 0.30.0 src/lib.rs.
PARAMETERS = {
    "sapling-spend.params": (47958396, "8270785a1a0d0bc77196f000ee6d221c9c9894f55307bd9357c3f0105d31ca63991ab91324160d8f53e2bbd3c2633a6eb8bdf5205d822e7f3f73edac51b2b70c"),
    "sapling-output.params": (3592860, "657e3d38dbb5cb5e7dd2970e8b03d69b4787dd907285b5a7f0790dcc8072f60bf593b32cc2d1c030e00ff5ae64bf84c5c3beb84ddc841d48264b4a171744d028"),
}


def output(command, **kwargs):
    return subprocess.check_output(command, text=True, stderr=subprocess.STDOUT, **kwargs).strip()


def digest(path, algorithm="sha256"):
    with path.open("rb") as handle:
        return hashlib.file_digest(handle, algorithm).hexdigest()


def verify_parameters(directory):
    result = {}
    for name, (size, expected) in PARAMETERS.items():
        path = directory / name
        if not path.is_file() or path.stat().st_size != size or digest(path, "blake2b") != expected:
            raise RuntimeError(f"Missing or invalid {path}; prefetch public parameters with scripts/zcash.sh build")
        result[name] = {"size": size, "blake2b": expected}
    return result


def check_environment(expected):
    rust = dict(line.split(": ", 1) for line in output(["rustc", "-vV"]).splitlines()[1:] if ": " in line)
    actual = {
        "host": rust["host"], "rust_commit": rust["commit-hash"],
        "cargo_version": output(["cargo", "--version"]),
        "clang_version": output(["/usr/bin/clang", "--version"]).splitlines()[0],
        "linker_version": output(["/usr/bin/ld", "-v"]).splitlines()[0],
        "sdk_version": output(["xcrun", "--show-sdk-version"]),
    }
    for key, value in actual.items():
        if value != expected[key]:
            raise RuntimeError(f"Release environment mismatch: {key}: expected {expected[key]!r}, found {value!r}")
    return {**actual, "rust_verbose": output(["rustc", "-vV"]),
            "clang_verbose": output(["/usr/bin/clang", "--version"]),
            "linker_verbose": output(["/usr/bin/ld", "-v"]),
            "sdk_path": output(["xcrun", "--show-sdk-path"])}


def build_environment(work, source, cargo_cache, tool_paths, epoch, profile):
    home = work / "home"
    cargo_home = work / "cargo-home"
    tmp = work / "tmp"
    for path in (home, cargo_home, tmp):
        path.mkdir()
    # Share downloaded sources only, never Cargo configuration or credentials.
    for name in ("registry", "git"):
        cached = cargo_cache / name
        if cached.exists():
            (cargo_home / name).symlink_to(cached, target_is_directory=True)
    env = {
        "PATH": os.pathsep.join(tool_paths), "HOME": str(home), "TMPDIR": str(tmp),
        "LANG": "C", "LC_ALL": "C", "TZ": "UTC", "SOURCE_DATE_EPOCH": epoch,
        "CARGO_HOME": str(cargo_home), "CARGO_TARGET_DIR": str(work / "target"),
        "CARGO_NET_OFFLINE": "true", "CARGO_INCREMENTAL": "0",
        "GIT_CONFIG_NOSYSTEM": "1", "GIT_CEILING_DIRECTORIES": str(work),
        "CC": "/usr/bin/clang", "CXX": "/usr/bin/clang++", "AR": "/usr/bin/ar",
        "SDKROOT": profile["sdk_path"], "MACOSX_DEPLOYMENT_TARGET": profile["deployment_target"],
    }
    # Keep debug/assertion paths independent of both checkout and cache location.
    mappings = [(source, "/treazury/source"), (cargo_home, "/treazury/cargo"),
                (cargo_cache, "/treazury/cargo"), (work, "/treazury/build")]
    env["CARGO_ENCODED_RUSTFLAGS"] = "\x1f".join(f"--remap-path-prefix={a}={b}" for a, b in mappings)
    env["CFLAGS"] = env["CXXFLAGS"] = " ".join(f"-ffile-prefix-map={a}={b}" for a, b in mappings)
    # rustup proxies need their toolchain store even though HOME is isolated.
    env["RUSTUP_HOME"] = os.environ.get("RUSTUP_HOME", str(Path.home() / ".rustup"))
    return env


def run_logged(command, cwd, env, log):
    with log.open("ab") as stream:
        stream.write((json.dumps(command) + "\n").encode())
        stream.flush()
        subprocess.run(SANDBOX + command, cwd=cwd, env=env, stdout=stream, stderr=subprocess.STDOUT, check=True)


def compare(first, second):
    hashes = [digest(first), digest(second)]
    if hashes[0] != hashes[1]:
        raise RuntimeError(f"Release binaries differ: {hashes[0]} != {hashes[1]}; both binaries and full build logs are retained")
    return hashes[0]


def qualify(run, no_zcash):
    report = {"status": "incomplete", "features": "no-default-features" if no_zcash else "default", "artifacts": []}
    try:
        subprocess.run(["sh", str(ROOT / "scripts/check_toolchain.sh")], check=True)
        if output(["git", "status", "--porcelain", "--untracked-files=no"], cwd=ROOT):
            raise RuntimeError("Tracked changes present; commit the candidate before release qualification (untracked files are excluded)")
        profile = json.loads((ROOT / "scripts/release-environment.json").read_text())
        report["environment"] = check_environment(profile)
        report["revision"] = output(["git", "rev-parse", "HEAD"], cwd=ROOT)
        report["epoch"] = output(["git", "show", "-s", "--format=%ct", "HEAD"], cwd=ROOT)
        report["lock_sha256"] = digest(ROOT / "Cargo.lock")
        report["parameters"] = {} if no_zcash else verify_parameters(ROOT / "vendor/zingolib/zcash-params")
        archive = run / "source.tar"
        with archive.open("wb") as stream:
            subprocess.run(["git", "archive", "--format=tar", report["revision"]], cwd=ROOT, stdout=stream, check=True)
        report["archive_sha256"] = digest(archive)
        tool_paths = list(dict.fromkeys([str(Path(shutil.which(name)).parent) for name in ("cargo", "rustc", "python3")] + ["/usr/bin", "/bin"]))
        cargo_cache = Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo")).resolve()
        report["tools"] = {name: {"path": shutil.which(name), "sha256": digest(Path(shutil.which(name)).resolve())} for name in ("cargo", "rustc")}
        for label in ("first", "second-longer-path"):
            work = run / label
            source = work / "source"
            source.mkdir(parents=True)
            subprocess.run(["tar", "-xf", str(archive), "-C", str(source)], check=True)
            if not no_zcash:
                for name in PARAMETERS:
                    shutil.copyfile(ROOT / "vendor/zingolib/zcash-params" / name, source / "vendor/zingolib/zcash-params" / name)
                verify_parameters(source / "vendor/zingolib/zcash-params")
            env = build_environment(work, source, cargo_cache, tool_paths, report["epoch"], {**profile, **report["environment"]})
            log = run / f"{label}.log"
            print(f"Building {label}; full log: {log}", flush=True)
            # Prove that the sandbox is active: a refused connection is NOT enough.
            probe = "import socket\ns=socket.socket()\ntry: s.connect(('127.0.0.1',9))\nexcept PermissionError: pass\nelse: raise RuntimeError('network sandbox did not deny connect')"
            run_logged([sys.executable, "-c", probe], source, env, log)
            for verifier in ("verify.py", "verify_zingo.py"):
                run_logged([sys.executable, str(source / "vendor" / verifier)], source, env, log)
            # Build the locked protoc helper independently in each clean target.
            run_logged(["cargo", "build", "--frozen", "--manifest-path", "compat/protoc/Cargo.toml"], source, env, log)
            env["PROTOC"] = output(SANDBOX + [str(work / "target/debug/x402-protoc-path")], cwd=source, env=env)
            command = ["cargo", "build", "--frozen", "--release", "--bin", "treazury", "--target", profile["host"]]
            if no_zcash:
                command.append("--no-default-features")
            run_logged(command, source, env, log)
            binary = work / "target" / profile["host"] / "release/treazury"
            run_logged([str(binary), "--help"], source, env, log)
            report["artifacts"].append({"path": str(binary), "sha256": digest(binary), "bytes": binary.stat().st_size})
        report["matching_sha256"] = compare(*(Path(a["path"]) for a in report["artifacts"]))
        report["status"] = "passed"
    except Exception as error:
        report["status"] = "failed"
        report["error"] = str(error)
        raise
    finally:
        (run / "report.json").write_text(json.dumps(report, indent=2) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--no-default-features", action="store_true")
    args = parser.parse_args()
    runs = ROOT / "target/reproducibility-runs"
    runs.mkdir(parents=True, exist_ok=True)
    run = Path(tempfile.mkdtemp(prefix="release-", dir=runs)).resolve()
    print(f"Reproducibility evidence: {run}", flush=True)
    try:
        qualify(run, args.no_default_features)
    except Exception as error:
        print(f"Qualification failed: {error}\nEvidence retained: {run}", file=sys.stderr)
        return 1
    print(f"PASS: byte-identical release binaries; report: {run / 'report.json'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
