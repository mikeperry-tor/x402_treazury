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
            for p in ["src/a.rs", "tools/complexity/Cargo.lock", "target/tools/complexity/debug/treazure-complexity"]:
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
            with patch.object(reporter, "ROOT", root), patch.object(reporter, "git", fake_git), patch.object(reporter.subprocess, "run", fake_run), patch("sys.argv", ["complexity.py", "--no-build"]), redirect_stdout(output), redirect_stderr(output):
                self.assertEqual(reporter.main(), 0)
                previous = (root / "target/complexity").resolve()
                data["files"][0]["errors"] = ["injected parser failure"]
                self.assertEqual(reporter.main(), 1)
                self.assertEqual((root / "target/complexity").resolve(), previous)
                self.assertEqual((previous / "status").read_text(), "complete\n")
            runs = list((root / "target/complexity-runs").iterdir())
            self.assertEqual(len(runs), 2)
            failed = next(p for p in runs if p != previous)
            self.assertTrue((failed / "raw.json").is_file())
            self.assertEqual(json.loads((failed / "report.json").read_text())["status"], "partial")
            self.assertIn("EXCLUDED src/a.rs", output.getvalue())


if __name__ == "__main__":
    unittest.main()
