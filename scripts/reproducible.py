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
# BLAKE2b-512 constants from locked zcash_proofs 0.31.0-pre.1 src/lib.rs.
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


def parameter_directory():
    # Match zcash_proofs' macOS XDG data-home lookup, including relative overrides.
    data_home = Path(os.environ.get("XDG_DATA_HOME", ""))
    if not data_home.is_absolute():
        data_home = Path.home() / ".local/share"
    return data_home / "ZcashParams"


def stage_parameters(original, env):
    directory = Path(env["XDG_DATA_HOME"]) / "ZcashParams"
    directory.mkdir(parents=True)
    for name in PARAMETERS:
        shutil.copyfile(original / name, directory / name)
    verify_parameters(directory)


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
        "XDG_DATA_HOME": str(home / ".local/share"),
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


def capture_logged(command, cwd, env, log):
    # Cargo metadata is JSON on stdout; warnings on stderr must not corrupt it.
    with log.open("ab") as stream:
        stream.write((json.dumps(command) + "\n").encode())
        stream.flush()
        result = subprocess.run(SANDBOX + command, cwd=cwd, env=env,
                                stdout=subprocess.PIPE, stderr=stream)
        stream.write(result.stdout)
        result.check_returncode()
    return result.stdout.decode()


def zingolib_inputs(metadata, env):
    packages = [package for package in metadata["packages"] if package["name"] == "zingolib"]
    if len(packages) != 1:
        raise RuntimeError(f"Expected one resolved zingolib package, found {len(packages)}")
    package = packages[0]
    directory = Path(package["manifest_path"]).parent
    observed = {}
    for name, arguments in (
        ("revision", ["rev-parse", "HEAD"]),
        ("tags", ["tag", "--points-at", "HEAD", "--list", "zingolib_v*", "--sort=-version:refname"]),
        ("diff_index", ["diff-index", "--quiet", "HEAD", "--"]),
    ):
        result = subprocess.run(SANDBOX + ["git", *arguments], cwd=directory, env=env,
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        # In particular, failed diff-index is not evidence of a clean tree.
        # Preserve its status instead of predicting the upstream descriptor.
        observed[name] = {"returncode": result.returncode, "stdout": result.stdout.rstrip(),
                          "stderr": result.stderr.rstrip()}
    return {key: package[key] for key in ("id", "source", "version", "manifest_path")} | {"git": observed}


def generated_zingolib_descriptor(target):
    # The runner uses a clean target for one release binary and one SDK version.
    paths = list(target.glob("**/build/zingolib-*/out/git_description.txt"))
    if len(paths) != 1:
        raise RuntimeError(f"Expected one generated zingolib descriptor, found {len(paths)}")
    return {"path": str(paths[0]), "text": paths[0].read_text()}


def compare(first, second):
    hashes = [digest(first), digest(second)]
    if hashes[0] != hashes[1]:
        raise RuntimeError(f"Release binaries differ: {hashes[0]} != {hashes[1]}; both binaries and full build logs are retained")
    return hashes[0]


def qualify(run):
    report = {"status": "incomplete", "features": "default", "artifacts": [], "builds": []}
    try:
        subprocess.run(["sh", str(ROOT / "scripts/check_toolchain.sh")], check=True)
        if output(["git", "status", "--porcelain", "--untracked-files=no"], cwd=ROOT):
            raise RuntimeError("Tracked changes present; commit the candidate before release qualification (untracked files are excluded)")
        profile = json.loads((ROOT / "scripts/release-environment.json").read_text())
        report["environment"] = check_environment(profile)
        report["revision"] = output(["git", "rev-parse", "HEAD"], cwd=ROOT)
        report["epoch"] = output(["git", "show", "-s", "--format=%ct", "HEAD"], cwd=ROOT)
        report["lock_sha256"] = digest(ROOT / "Cargo.lock")
        parameters = parameter_directory()
        report["parameters"] = verify_parameters(parameters)
        archive = run / "source.tar"
        with archive.open("wb") as stream:
            subprocess.run(["git", "archive", "--format=tar", report["revision"]], cwd=ROOT, stdout=stream, check=True)
        report["archive_sha256"] = digest(archive)
        tool_paths = list(dict.fromkeys([str(Path(shutil.which(name)).parent) for name in ("cargo", "rustc", "python3")] + ["/usr/bin", "/bin"]))
        cargo_cache = Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo")).resolve()
        report["tools"] = {name: {"path": shutil.which(name), "sha256": digest(Path(shutil.which(name)).resolve())} for name in ("cargo", "rustc")}
        for label in ("first", "second-longer-path"):
            build = {"label": label, "status": "incomplete"}
            report["builds"].append(build)
            work = run / label
            source = work / "source"
            source.mkdir(parents=True)
            subprocess.run(["tar", "-xf", str(archive), "-C", str(source)], check=True)
            env = build_environment(work, source, cargo_cache, tool_paths, report["epoch"], {**profile, **report["environment"]})
            stage_parameters(parameters, env)
            log = run / f"{label}.log"
            print(f"Building {label}; full log: {log}", flush=True)
            # Prove that the sandbox is active: a refused connection is NOT enough.
            probe = "import socket\ns=socket.socket()\ntry: s.connect(('127.0.0.1',9))\nexcept PermissionError: pass\nelse: raise RuntimeError('network sandbox did not deny connect')"
            run_logged([sys.executable, "-c", probe], source, env, log)
            for verifier in ("verify.py", "verify_zingo.py"):
                run_logged([sys.executable, str(source / "vendor" / verifier)], source, env, log)
            metadata = json.loads(capture_logged(
                ["cargo", "metadata", "--frozen", "--format-version", "1", "--manifest-path", "Cargo.toml",
                 "--filter-platform", profile["host"]],
                source, env, log,
            ))
            build["zingolib"] = zingolib_inputs(metadata, env)
            # Build the locked protoc helper independently in each clean target.
            run_logged(["cargo", "build", "--frozen", "--manifest-path", "compat/protoc/Cargo.toml"], source, env, log)
            env["PROTOC"] = output(SANDBOX + [str(work / "target/debug/x402-protoc-path")], cwd=source, env=env)
            command = ["cargo", "build", "--frozen", "--release", "--bin", "x402_treazury", "--target", profile["host"]]
            run_logged(command, source, env, log)
            build["zingolib"]["descriptor"] = generated_zingolib_descriptor(Path(env["CARGO_TARGET_DIR"]))
            binary = work / "target" / profile["host"] / "release/x402_treazury"
            run_logged([str(binary), "--help"], source, env, log)
            report["artifacts"].append({"path": str(binary), "sha256": digest(binary), "bytes": binary.stat().st_size})
            build["status"] = "passed"
        report["matching_sha256"] = compare(*(Path(a["path"]) for a in report["artifacts"]))
        report["status"] = "passed"
    except Exception as error:
        report["status"] = "failed"
        report["error"] = str(error)
        if report["builds"] and report["builds"][-1]["status"] == "incomplete":
            report["builds"][-1].update(status="failed", error=str(error))
        raise
    finally:
        (run / "report.json").write_text(json.dumps(report, indent=2) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.parse_args()
    runs = ROOT / "target/reproducibility-runs"
    runs.mkdir(parents=True, exist_ok=True)
    run = Path(tempfile.mkdtemp(prefix="release-", dir=runs)).resolve()
    print(f"Reproducibility evidence: {run}", flush=True)
    try:
        qualify(run)
    except Exception as error:
        print(f"Qualification failed: {error}\nEvidence retained: {run}", file=sys.stderr)
        return 1
    print(f"PASS: byte-identical release binaries; report: {run / 'report.json'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
