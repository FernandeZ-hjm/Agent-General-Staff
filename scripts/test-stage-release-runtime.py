#!/usr/bin/env python3
"""Narrow regression tests for canonical release runtime staging."""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("stage-release-runtime.py")


class StageReleaseRuntimeTests(unittest.TestCase):
    @staticmethod
    def write_plan(root: Path, runtime_assets: list[str]) -> Path:
        plan = root / "plan.json"
        plan.write_text(
            json.dumps(
                {
                    "schema_version": "2.0-release",
                    "profile": "public-full",
                    "authority_errors": [],
                    "required_missing": [],
                    "extra_files": [],
                    "content_mismatches": [],
                    "forbidden_included": [],
                    "runtime_asset_files": runtime_assets,
                    "included_files": runtime_assets,
                }
            ),
            encoding="utf-8",
        )
        return plan

    def test_stages_only_runtime_assets_from_plan(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            source = root / "source"
            target = root / "target"
            for relative in ["manifests/a.yaml", "protocol/b.md"]:
                path = source / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(relative, encoding="utf-8")
            plan = self.write_plan(
                root,
                [
                    "manifests/a.yaml",
                    "protocol/b.md",
                ],
            )

            result = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT),
                    "--plan",
                    str(plan),
                    "--source",
                    str(source),
                    "--target",
                    str(target),
                ],
                check=False,
                capture_output=True,
                text=True,
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                (target / "manifests/a.yaml").read_text(encoding="utf-8"),
                "manifests/a.yaml",
            )
            self.assertEqual(
                (target / "protocol/b.md").read_text(encoding="utf-8"),
                "protocol/b.md",
            )

    def test_rejects_non_authority_plan(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            plan = self.write_plan(root, [])
            payload = json.loads(plan.read_text(encoding="utf-8"))
            payload["extra_files"] = ["private.txt"]
            plan.write_text(json.dumps(payload), encoding="utf-8")
            result = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT),
                    "--plan",
                    str(plan),
                    "--source",
                    str(root),
                    "--target",
                    str(root / "target"),
                ],
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("non-authority", result.stderr)

    def test_rejects_unapproved_content_drift(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            plan = self.write_plan(root, [])
            payload = json.loads(plan.read_text(encoding="utf-8"))
            payload["content_mismatches"] = ["README.md"]
            plan.write_text(json.dumps(payload), encoding="utf-8")

            result = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT),
                    "--plan",
                    str(plan),
                    "--source",
                    str(root),
                    "--target",
                    str(root / "target"),
                ],
                check=False,
                capture_output=True,
                text=True,
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("unapproved content drift", result.stderr)

    def test_rejects_runtime_asset_outside_included_payload(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            plan = self.write_plan(root, ["private.txt"])
            payload = json.loads(plan.read_text(encoding="utf-8"))
            payload["included_files"] = []
            plan.write_text(json.dumps(payload), encoding="utf-8")

            result = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT),
                    "--plan",
                    str(plan),
                    "--source",
                    str(root),
                    "--target",
                    str(root / "target"),
                ],
                check=False,
                capture_output=True,
                text=True,
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("outside the canonical included payload", result.stderr)

    def test_rejects_old_plan_without_content_contract(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            plan = self.write_plan(root, [])
            payload = json.loads(plan.read_text(encoding="utf-8"))
            del payload["content_mismatches"]
            plan.write_text(json.dumps(payload), encoding="utf-8")

            result = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT),
                    "--plan",
                    str(plan),
                    "--source",
                    str(root),
                    "--target",
                    str(root / "target"),
                ],
                check=False,
                capture_output=True,
                text=True,
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("must include content_mismatches", result.stderr)

    def test_rejects_nonempty_target(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            source = root / "source"
            source.mkdir()
            target = root / "target"
            target.mkdir()
            (target / "stale.txt").write_text("stale", encoding="utf-8")
            plan = self.write_plan(root, [])

            result = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT),
                    "--plan",
                    str(plan),
                    "--source",
                    str(source),
                    "--target",
                    str(target),
                ],
                check=False,
                capture_output=True,
                text=True,
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("must be empty", result.stderr)

    @unittest.skipIf(sys.platform == "win32", "symlink creation is privilege-dependent on Windows")
    def test_rejects_symlinked_plan(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            real_plan = self.write_plan(root, [])
            linked_plan = root / "linked-plan.json"
            linked_plan.symlink_to(real_plan)

            result = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT),
                    "--plan",
                    str(linked_plan),
                    "--source",
                    str(root),
                    "--target",
                    str(root / "target"),
                ],
                check=False,
                capture_output=True,
                text=True,
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("plan must not be a symlink", result.stderr)

    @unittest.skipIf(sys.platform == "win32", "symlink creation is privilege-dependent on Windows")
    def test_rejects_runtime_asset_symlink(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            source = root / "source"
            outside = root / "outside.yaml"
            outside.write_text("outside", encoding="utf-8")
            linked = source / "manifests/a.yaml"
            linked.parent.mkdir(parents=True)
            linked.symlink_to(outside)
            plan = self.write_plan(root, ["manifests/a.yaml"])
            result = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT),
                    "--plan",
                    str(plan),
                    "--source",
                    str(source),
                    "--target",
                    str(root / "target"),
                ],
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("must not contain a symlink", result.stderr)

    @unittest.skipIf(sys.platform == "win32", "symlink creation is privilege-dependent on Windows")
    def test_rejects_symlinked_source_parent(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            source = root / "source"
            outside = root / "outside"
            outside.mkdir()
            (outside / "a.yaml").write_text("outside", encoding="utf-8")
            source.mkdir()
            (source / "manifests").symlink_to(outside, target_is_directory=True)
            plan = self.write_plan(root, ["manifests/a.yaml"])

            result = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT),
                    "--plan",
                    str(plan),
                    "--source",
                    str(source),
                    "--target",
                    str(root / "target"),
                ],
                check=False,
                capture_output=True,
                text=True,
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("source path must not contain a symlink", result.stderr)

    @unittest.skipIf(sys.platform == "win32", "symlink creation is privilege-dependent on Windows")
    def test_rejects_symlinked_target_parent(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            source = root / "source"
            asset = source / "manifests/a.yaml"
            asset.parent.mkdir(parents=True)
            asset.write_text("inside", encoding="utf-8")
            outside = root / "outside"
            outside.mkdir()
            target = root / "target"
            target.mkdir()
            (target / "manifests").symlink_to(outside, target_is_directory=True)
            plan = self.write_plan(root, ["manifests/a.yaml"])

            result = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT),
                    "--plan",
                    str(plan),
                    "--source",
                    str(source),
                    "--target",
                    str(target),
                ],
                check=False,
                capture_output=True,
                text=True,
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("target path must not contain a symlink", result.stderr)

    @unittest.skipIf(sys.platform == "win32", "symlink creation is privilege-dependent on Windows")
    def test_rejects_symlinked_target_root(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            source = root / "source"
            asset = source / "manifests/a.yaml"
            asset.parent.mkdir(parents=True)
            asset.write_text("inside", encoding="utf-8")
            outside = root / "outside"
            outside.mkdir()
            target = root / "target"
            target.symlink_to(outside, target_is_directory=True)
            plan = self.write_plan(root, ["manifests/a.yaml"])

            result = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT),
                    "--plan",
                    str(plan),
                    "--source",
                    str(source),
                    "--target",
                    str(target),
                ],
                check=False,
                capture_output=True,
                text=True,
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("target root must not be a symlink", result.stderr)


if __name__ == "__main__":
    unittest.main()
