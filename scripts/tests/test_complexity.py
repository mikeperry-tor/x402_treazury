"""Offline report integrity tests; no analyzer installation or Cargo build needed."""
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch
from contextlib import redirect_stdout, redirect_stderr

spec = importlib.util.spec_from_file_location("complexity", Path(__file__).parents[1] / "complexity.py")
reporter = importlib.util.module_from_spec(spec)
spec.loader.exec_module(reporter)


def space(name="f", start=1, end=10, cognitive=3, cyclomatic=4, children=None):
    return {"name": name, "start_line": start, "end_line": end, "kind": "function", "spaces": children or [],
            "metrics": {"cognitive": {"sum": cognitive}, "cyclomatic": {"sum": cyclomatic}}}


def file_data():
    return {"path": "src/a.rs", "test_ranges": [], "macro_lines": [3], "cfg_attr_lines": [], "errors": [], "metrics": space()}


class ComplexityTests(unittest.TestCase):
    def test_nested_scores_and_inline_tests(self):
        f = file_data()
        f["metrics"]["spaces"] = [space("<anonymous>", 2, 4, 2, 2), space("check", 6, 8, 1, 1)]
        f["test_ranges"] = [(5, 9)]
        rows = reporter.flatten(f)
        self.assertEqual(rows[0]["own_cognitive"], 0)
        self.assertEqual(rows[0]["own_cyclomatic"], 1)
        self.assertTrue(rows[1]["closure"])
        self.assertEqual(rows[2]["category"], "tests")
        self.assertEqual(reporter.category("src/treasury/regtest.rs", 1, 2, []), "tests")
        self.assertEqual(reporter.category("examples/demo.rs", 1, 2, []), "examples")
        self.assertEqual(reporter.category("examples/demo.rs", 1, 2, [(1, 2)]), "tests")
        self.assertEqual(reporter.category("src/rotation/store/test_vfs.rs", 1, 2, []), "tests")
        self.assertEqual(reporter.category("src/rotation/test_capacity.rs", 1, 2, []), "tests")

    def test_relocated_operational_sources_retain_scrutiny_and_true_tests(self):
        for path in ["tests/live/live_integration.rs", "tests/live/provider_preflight.rs",
                     "tests/live/runner/registry.rs", "tests/live/runner/tor/archive.rs",
                     "tools/providers/snapshot_spec.rs", "tools/diagnostics/quote_near.rs"]:
            with self.subTest(path=path):
                self.assertEqual(reporter.category(path, 1, 2, []), "examples")
                self.assertEqual(reporter.category(path, 1, 2, [(1, 2)]), "tests")
        for path in ["tests/integration_preparation.rs", "tests/live/runner/tests.rs",
                     "tests/live/runner/registry_tests.rs", "tests/live/runner/test_fixture.rs"]:
            self.assertEqual(reporter.category(path, 1, 2, []), "tests")

    def test_advisory_thresholds_new_increased_unchanged_and_improved(self):
        def row(name, cognitive, cyclomatic, category="production", start=1):
            return dict(path="src/a.rs", name=name, cognitive=cognitive,
                        cyclomatic=cyclomatic, category=category, start=start)
        before = [row("unchanged", 30, 50), row("improved", 30, 50),
                  row("crossing", 20, 40), row("higher", 25, 45), row("below", 1, 2)]
        after = [row("unchanged", 30, 50, start=100), row("improved", 21, 41),
                 row("crossing", 21, 41), row("higher", 26, 44), row("below", 20, 40),
                 row("new", 0, 41, "examples"), row("boundary", 20, 40),
                 row("test", 100, 100, "tests")]
        warnings = reporter.regressions(after, before)
        self.assertEqual([w["name"] for w in warnings], ["crossing", "higher", "new"])
        self.assertEqual(set(warnings[0]["metrics"]), {"cognitive", "cyclomatic"})
        self.assertEqual(set(warnings[1]["metrics"]), {"cognitive"})
        self.assertEqual(warnings[2]["kind"], "new_or_unmatched")

    def test_ambiguous_names_and_renames_do_not_pair_by_line_number(self):
        rows = [dict(path="src/a.rs", name="f::<anonymous>", category="production",
                     start=n, cognitive=score, cyclomatic=1) for n, score in [(1, 25), (2, 26)]]
        self.assertEqual(reporter.regressions(list(reversed(rows)), rows), [])
        changed = [dict(rows[0], cognitive=27), rows[1]]
        warnings = reporter.regressions(changed, rows)
        self.assertEqual(len(warnings), 2)
        self.assertTrue(all(w["kind"] == "new_or_unmatched" for w in warnings))
        self.assertEqual(reporter.regressions([dict(rows[0], name="renamed")], rows)[0]["kind"], "new_or_unmatched")

    def test_region_union_excludes_expansion_and_non_code_regions(self):
        v = {"data": [{"functions": [
            {"filenames": ["/src/a.rs"], "regions": [[1, 1, 3, 2, 0, 0, 0, 0], [4, 1, 4, 2, 0, 0, 0, 0], [5, 1, 6, 2, 0, 0, 0, 2]]},
            {"filenames": ["/src/a.rs"], "regions": [[1, 1, 3, 2, 8, 0, 0, 0]]}]}]}
        regions = reporter.region_index(v)["/src/a.rs"]
        self.assertEqual(reporter.within(regions, 1, 4), {"covered": 1, "total": 2})
        self.assertIsNone(reporter.within(regions, 8, 10))

    def test_coverage_requires_unchanged_source_and_clean_provenance(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            provenance = "source_revision=abc\n--- source status ---\n?? user-file\n--- compiler ---\nrustc\n"
            (root / "provenance.txt").write_text(provenance)
            (root / "full.json").write_text(json.dumps({"data": [{"functions": [{"filenames": [str(root / "src/a.rs")], "regions": [[1, 1, 3, 2, 1, 0, 0, 0]]}]}]}))
            rows = reporter.flatten(file_data())
            with patch.object(reporter, "ROOT", root), patch.object(reporter.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, stdout=b"old")):
                reporter.attach_coverage(rows, root, {"src/a.rs": reporter.digest(b"new")})
                self.assertEqual(rows[0]["coverage_status"], "source changed since coverage")
                self.assertIsNone(rows[0]["coverage"])
                reporter.attach_coverage(rows, root, {"src/a.rs": reporter.digest(b"old")})
                self.assertEqual(rows[0]["coverage"], {"covered": 1, "total": 1})
                (root / "provenance.txt").write_text(provenance.replace("?? user-file", " M src/a.rs"))
                reporter.attach_coverage(rows, root, {"src/a.rs": reporter.digest(b"old")})
                self.assertEqual(rows[0]["coverage_status"], "unverifiable coverage provenance")
                self.assertIsNone(rows[0]["coverage"])

    def test_markdown_excerpt_is_explicit_and_does_not_drop_json_rows(self):
        rows = reporter.flatten(file_data()) * 3
        for row in rows:
            row.update(coverage=None, coverage_status="not supplied")
        report = {"status": "complete", "functions": rows, "files": {"src/a.rs": "hash"}, "excluded": []}
        md = reporter.markdown(report, 1)
        self.assertIn("Showing 1 of 3", md)
        self.assertIn("2 omitted here", md)
        self.assertEqual(len(report["functions"]), 3)

    def test_parser_failure_preserves_successful_publication_and_raw_evidence(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for p in ["src/a.rs", "tools/complexity/Cargo.lock", "target/tools/complexity/debug/treazury-complexity"]:
                path = root / p
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("fixture")
            data = {"version": reporter.VERSION, "files": [file_data()]}

            def fake_git(*args):
                return {"ls-files": b"src/a.rs\0", "rev-parse": b"abc", "status": b""}[args[0]]

            def fake_run(command, **kwargs):
                kwargs["stdout"].write(json.dumps(data))
                return subprocess.CompletedProcess(command, 0)

            output = io.StringIO()
            with patch.object(reporter, "ROOT", root), patch.object(reporter, "git", fake_git), patch.object(reporter.subprocess, "run", fake_run), patch("sys.argv", ["complexity.py", "--no-build", "--no-compare"]), redirect_stdout(output), redirect_stderr(output):
                self.assertEqual(reporter.main(), 0)
                previous = (root / "target/complexity").resolve()
                data["files"][0]["errors"] = ["injected parser failure"]
                self.assertEqual(reporter.main(), 1)
                self.assertEqual((root / "target/complexity").resolve(), previous)
                self.assertEqual((previous / "status").read_text(), "complete\n")
            runs = list((root / "target/complexity-runs").iterdir())
            self.assertEqual(len(runs), 2)
            failed = next(p for p in runs if p.resolve() != previous)
            self.assertTrue((failed / "raw.json").is_file())
            self.assertEqual(json.loads((failed / "report.json").read_text())["status"], "partial")
            self.assertIn("EXCLUDED src/a.rs", output.getvalue())

    def test_warnings_succeed_but_baseline_failure_preserves_previous_report(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for p in ["src/a.rs", "tools/complexity/Cargo.lock", "target/tools/complexity/debug/treazury-complexity"]:
                path = root / p
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("fixture")
            f = file_data()
            f["metrics"]["metrics"]["cognitive"]["sum"] = 21
            data = {"version": reporter.VERSION, "files": [f]}

            def fake_git(*args):
                return {"ls-files": b"src/a.rs\0", "rev-parse": b"abc", "status": b""}[args[0]]

            def fake_run(command, **kwargs):
                kwargs["stdout"].write(json.dumps(data))
                return subprocess.CompletedProcess(command, 0)

            output = io.StringIO()
            with patch.object(reporter, "ROOT", root), patch.object(reporter, "git", fake_git), patch.object(reporter.subprocess, "run", fake_run), patch.object(reporter, "baseline_report", return_value={"revision": "base", "functions": []}) as base, patch("sys.argv", ["complexity.py", "--no-build", "--base", "review-base"]), redirect_stdout(output), redirect_stderr(output):
                self.assertEqual(reporter.main(), 0)
                previous = (root / "target/complexity").resolve()
                report = json.loads((previous / "report.json").read_text())
                self.assertEqual(report["comparison"]["ref"], "review-base")
                self.assertEqual(len(report["comparison"]["warnings"]), 1)
                self.assertIn("COMPLEXITY REVIEW:", output.getvalue())
                self.assertIn("cognitive > 20", (previous / "report.md").read_text())
                base.side_effect = ValueError("incomplete baseline analysis")
                self.assertEqual(reporter.main(), 1)
                self.assertEqual((root / "target/complexity").resolve(), previous)


class BaselineTests(unittest.TestCase):
    def test_exact_git_blobs_and_same_analyzer_without_checkout(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)

            def git(*args):
                return subprocess.check_output(["git", *args], cwd=root, stderr=subprocess.DEVNULL)

            git("init", "-b", "main")
            git("config", "user.name", "Fixture")
            git("config", "user.email", "fixture@example.invalid")
            (root / "src").mkdir()
            original = b'fn f() { /* $Format:%H$ */ }\n'
            (root / "src/a.rs").write_bytes(original)
            inputs = {"src/a.rs": original}
            for name in ["tests/live/live_integration.rs", "tests/live/provider_preflight.rs",
                         "tests/live/runner/registry.rs", "tools/providers/snapshot_spec.rs",
                         "tools/diagnostics/quote_near.rs"]:
                inputs[name] = b"fn operational() {}\n"
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(inputs[name])
            excluded = root / "tools/complexity/src/main.rs"
            excluded.parent.mkdir(parents=True)
            excluded.write_text("fn analyzer() {}\n")
            new_tool = root / "tools/diagnostics/new.rs"
            (root / ".gitattributes").write_text("src/a.rs export-subst export-ignore\n")
            git("add", ".")
            git("-c", "commit.gpgsign=false", "commit", "-m", "base")
            revision = git("rev-parse", "HEAD").decode().strip()
            git("checkout", "-b", "topic")
            (root / "src/a.rs").write_text("fn changed() {}\n")
            git("-c", "commit.gpgsign=false", "commit", "-am", "change")
            head = git("rev-parse", "HEAD")
            binary = root / "same-analyzer"
            out = root / "report"
            out.mkdir()

            def analyze(actual, paths, cwd, output, prefix):
                self.assertEqual(actual, binary)
                self.assertEqual(paths, sorted(inputs))
                for name, raw in inputs.items():
                    self.assertEqual((cwd / name).read_bytes(), raw)
                self.assertEqual(prefix, "base-")
                return [], []

            with patch.object(reporter, "ROOT", root), patch.object(reporter, "analyze_sources", side_effect=analyze):
                new_tool.write_text("fn new_tool() {}\n")
                self.assertEqual(reporter.selected_paths(), sorted([*inputs, "tools/diagnostics/new.rs"]))
                report = reporter.baseline_report(binary, "main", out)
                self.assertEqual(report["revision"], revision)
                self.assertEqual(report["files"], {name: reporter.digest(raw) for name, raw in inputs.items()})
                with self.assertRaises(subprocess.CalledProcessError):
                    reporter.baseline_report(binary, "absent-ref", root / "absent")
            broken = root / "broken"
            broken.mkdir()
            with patch.object(reporter, "ROOT", root), patch.object(reporter, "analyze_sources", return_value=([], [{"path": "src/a.rs", "errors": ["grammar failure"]}])):
                with self.assertRaisesRegex(ValueError, "incomplete baseline analysis"):
                    reporter.baseline_report(binary, "main", broken)
                self.assertTrue(json.loads((broken / "base-report.json").read_text())["excluded"])
            self.assertEqual(git("rev-parse", "HEAD"), head)
            self.assertEqual((root / "src/a.rs").read_text(), "fn changed() {}\n")


if __name__ == "__main__":
    unittest.main()
