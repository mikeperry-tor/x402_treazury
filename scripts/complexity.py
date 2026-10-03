#!/usr/bin/env python3
"""Reporting-only Rust complexity audit. Build inputs are separate from the app."""
import argparse
import csv
import datetime
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import uuid

ROOT = Path(__file__).resolve().parents[1]
VERSION = "0.0.25"


def digest(data):
    return hashlib.sha256(data).hexdigest()


def git(*args):
    return subprocess.check_output(["git", *args], cwd=ROOT)


def category(path, start, end, ranges):
    p = Path(path)
    if p.parts[0] == "tests" or p.name in {"tests.rs", "regtest.rs"} or p.name.endswith("_tests.rs"):
        return "tests"
    if p.parts[0] == "examples":
        return "examples"
    if any(a <= start and end <= b for a, b in ranges):
        return "tests"
    return "production"


def flatten(file):
    rows = []

    def walk(space, parents):
        name = space["name"] or "<unnamed>"
        chain = parents + [name]
        if space["kind"] == "function":
            metrics = space["metrics"]
            own = {}
            for metric in ("cognitive", "cyclomatic"):
                total = metrics[metric]["sum"]
                children = sum(c["metrics"][metric]["sum"] for c in space["spaces"])
                own[metric] = total - children
                if own[metric] < 0:
                    raise ValueError(f"inconsistent {metric} aggregation in {file['path']}")
            start, end = space["start_line"], space["end_line"]
            rows.append({"path": file["path"], "name": "::".join(chain), "start": start, "end": end,
                         "category": category(file["path"], start, end, file["test_ranges"]),
                         "closure": name == "<anonymous>", "cognitive": metrics["cognitive"]["sum"],
                         "cyclomatic": metrics["cyclomatic"]["sum"], "own_cognitive": own["cognitive"],
                         "own_cyclomatic": own["cyclomatic"], "lines": end - start + 1,
                         "test_only_attributes": sum(start <= line <= end for line in file.get("test_attribute_lines", [])),
                         "macros": sum(start <= line <= end for line in file["macro_lines"])})
        for child in space["spaces"]:
            walk(child, chain if space["kind"] != "unit" else [])

    walk(file["metrics"], [])
    return rows


def region_index(document):
    """Deduplicate LLVM code regions across instantiations; any execution covers it."""
    result = {}
    for data in document["data"]:
        for function in data["functions"]:
            for region in function["regions"]:
                if region[7] != 0:
                    continue
                path = str(Path(function["filenames"][region[5]]).resolve())
                key = tuple(region[:4])
                regions = result.setdefault(path, {})
                regions[key] = max(regions.get(key, 0), region[4])
    return result


def within(regions, start, end):
    values = [count for (a, _, b, _), count in regions.items() if start <= a and b <= end]
    return None if not values else {"covered": sum(v > 0 for v in values), "total": len(values)}


def coverage_status(provenance):
    revision = None
    clean = False
    in_status = False
    for line in provenance.splitlines():
        if line.startswith("source_revision="):
            revision = line.split("=", 1)[1]
        if line.startswith("--- source status"):
            in_status = True
            clean = True
        elif in_status and line.startswith("---"):
            in_status = False
        elif in_status and line.strip() and not line.startswith("?? "):
            clean = False
    return revision, clean


def attach_coverage(rows, directory, hashes):
    if directory is None or not directory.exists():
        for row in rows:
            row.update(coverage_status="not supplied", coverage=None)
        return {"status": "not supplied"}
    provenance = (directory / "provenance.txt").read_text()
    revision, clean = coverage_status(provenance)
    full = directory / "full.json"
    raw = full.read_bytes()
    regions = region_index(json.loads(raw))
    states = {}
    for path in hashes:
        if not revision or not clean:
            states[path] = "unverifiable coverage provenance"
            continue
        previous = subprocess.run(["git", "show", f"{revision}:{path}"], cwd=ROOT, capture_output=True)
        states[path] = ("source matched" if previous.returncode == 0 and digest(previous.stdout) == hashes[path]
                        else "source changed since coverage")
    for row in rows:
        status = states[row["path"]]
        observed = within(regions.get(str((ROOT / row["path"]).resolve()), {}), row["start"], row["end"]) if status == "source matched" else None
        if status == "source matched" and observed is None:
            status = "not instrumented in this report"
        row.update(coverage_status=status, coverage=observed)
    return {"directory": str(directory.resolve()), "revision": revision, "clean_tracked_tree": clean,
            "full_json_sha256": digest(raw), "provenance": provenance,
            "note": "Code regions contained within each inclusive line span; deduplicated across instantiations. Not LLVM's official per-function percentage. Source equality does not prove unchanged dependencies or test coverage."}


def markdown(report, top):
    rows = report["functions"]
    lines = ["# Rust complexity report", "", f"Status: **{report['status']}**. Analyzer: rust-code-analysis {VERSION}.",
             "", "Scores include nested closures/functions; own scores subtract child spaces. Do not sum inclusive scores across rows.",
             "Macro invocations are counted but are not expanded. All source feature branches are parsed; this is not a compiled-feature report.",
             "Test-only callables are separated. Production callables can still contain test-only branches; the Test attrs column flags those inclusive scores.",
             "Coverage is a source-matched, deduplicated LLVM code-region count inside the inclusive line span, including nested closures.",
             "Changed/unverifiable source and uninstrumented spans have no coverage value; they are never presented as zero coverage.",
             "", f"Analyzed {len(report['files'])} files. Parser/classification exclusions: {len(report['excluded'])}.", ""]
    for excluded in report["excluded"]:
        lines.append(f"- EXCLUDED `{excluded['path']}`: {'; '.join(excluded['errors'])}")
    for group in ("production", "tests", "examples"):
        selected = sorted((r for r in rows if r["category"] == group), key=lambda r: (-r["cognitive"], -r["cyclomatic"], r["path"], r["start"]))
        lines += ["", f"## {group.title()}", "", f"Showing {min(top,len(selected))} of {len(selected)} callable spaces; {max(0,len(selected)-top)} omitted here. Complete results are in functions.csv and report.json.", "",
                  "| Location | Callable | Cognitive (own) | Cyclomatic (own) | Lines | Macros | Test attrs | Covered/total regions |",
                  "| --- | --- | ---: | ---: | ---: | ---: | ---: | --- |"]
        for r in selected[:top]:
            c = r["coverage"]
            coverage = f"{c['covered']}/{c['total']}" if c else r["coverage_status"]
            name = r["name"].replace("|", "\\|")
            lines.append(f"| {r['path']}:{r['start']} | `{name}` | {r['cognitive']:g} ({r['own_cognitive']:g}) | {r['cyclomatic']:g} ({r['own_cyclomatic']:g}) | {r['lines']} | {r['macros']} | {r['test_only_attributes']} | {coverage} |")
    lines += ["", "Parser failures and unsupported cfg_attr classification exclude the whole file from trusted rankings; raw.json retains all analyzer output.",
              "A complexity score is a review aid, not a correctness, concurrency-safety, or maintainability guarantee. No score thresholds are enforced.", ""]
    return "\n".join(lines)


def selected_paths():
    return sorted(set(p for p in git("ls-files", "--cached", "--others", "--exclude-standard", "-z", "--", "src", "tests", "examples").decode().split("\0") if p.endswith(".rs") and (ROOT / p).is_file()))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--no-build", action="store_true", help="reuse the locally built helper")
    parser.add_argument("--coverage", type=Path, default=ROOT / "target/coverage", help="existing coverage report directory")
    parser.add_argument("--top", type=int, default=20, help="rows per Markdown category; complete JSON/CSV are always retained")
    args = parser.parse_args()
    if args.top < 1:
        parser.error("--top must be positive")
    tag = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ-") + uuid.uuid4().hex[:8]
    out = ROOT / "target/complexity-runs" / tag
    out.mkdir(parents=True)
    print(f"Complexity artifacts: {out}", flush=True)
    try:
        binary = ROOT / "target/tools/complexity/debug/treazure-complexity"
        if not args.no_build:
            with (out / "build.log").open("w") as log:
                subprocess.run(["cargo", "build", "--locked", "--manifest-path", "tools/complexity/Cargo.toml", "--target-dir", "target/tools/complexity"], cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, check=True)
        paths = selected_paths()
        if not paths:
            raise ValueError("no Rust inputs selected")
        hashes = {p: digest((ROOT / p).read_bytes()) for p in paths}
        command = [str(binary), *paths]
        (out / "command.json").write_text(json.dumps(command, indent=2) + "\n")
        with (out / "raw.json").open("w") as raw, (out / "analyzer.log").open("w") as log:
            subprocess.run(command, cwd=ROOT, stdout=raw, stderr=log, check=True)
        data = json.loads((out / "raw.json").read_text())
        if data["version"] != VERSION or sorted(f["path"] for f in data["files"]) != paths:
            raise ValueError("wrong analyzer version or incomplete/duplicate file inventory")
        if paths != selected_paths() or any(digest((ROOT / p).read_bytes()) != h for p, h in hashes.items()):
            raise ValueError("source changed during analysis; rerun from a stable tree")
        rows, excluded = [], []
        for f in data["files"]:
            errors = list(f["errors"])
            if f["cfg_attr_lines"]:
                errors.append(f"cfg_attr needs classification review at lines {f['cfg_attr_lines']}")
            if errors:
                excluded.append({"path": f["path"], "errors": errors})
            else:
                rows.extend(flatten(f))
        coverage = attach_coverage(rows, args.coverage, hashes)
        report = {"status": "partial" if excluded else "complete", "source_revision": git("rev-parse", "HEAD").decode().strip(),
                  "source_status": git("status", "--short").decode(), "analyzer": VERSION,
                  "analyzer_binary_sha256": digest(binary.read_bytes()), "tool_lock_sha256": digest((ROOT / "tools/complexity/Cargo.lock").read_bytes()),
                  "files": hashes, "excluded": excluded, "coverage": coverage, "functions": rows}
        (out / "report.json").write_text(json.dumps(report, indent=2) + "\n")
        with (out / "functions.csv").open("w", newline="") as csvfile:
            writer = csv.DictWriter(csvfile, fieldnames=list(rows[0]) if rows else ["path"])
            writer.writeheader()
            writer.writerows(rows)
        (out / "report.md").write_text(markdown(report, args.top))
        if excluded:
            for item in excluded:
                print(f"EXCLUDED {item['path']}: {'; '.join(item['errors'])}", file=sys.stderr)
            raise ValueError(f"partial report: {len(excluded)} files excluded; previous successful report preserved")
        latest = ROOT / "target/complexity"
        link = latest.with_name(".complexity-" + uuid.uuid4().hex)
        link.symlink_to(out.relative_to(latest.parent))
        try:
            os.replace(link, latest)
        finally:
            link.unlink(missing_ok=True)
        (out / "status").write_text("complete\n")
        print(f"Complete: {len(paths)} files, {len(rows)} callable spaces. Markdown shows at most {args.top} per category; JSON/CSV contain every row.")
        print(f"Report: {latest / 'report.md'}")
    except Exception as error:
        (out / "status").write_text(f"failed: {error}\n")
        print(f"Complexity report failed: {error}; evidence: {out}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
