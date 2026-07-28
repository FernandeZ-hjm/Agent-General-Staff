#!/usr/bin/env python3
"""Prove that task-card release gates detect representative rule regressions."""

from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile


ROOT = Path(__file__).resolve().parents[1]
IGNORED_DIRS = {
    ".git",
    ".ags-local",
    ".codegraph",
    "node_modules",
    "target",
    "__pycache__",
}


def copy_workspace(target: Path) -> None:
    shutil.copytree(
        ROOT,
        target,
        ignore=lambda _directory, names: [
            name for name in names if name in IGNORED_DIRS
        ],
    )


def replace_once(path: Path, before: str, after: str) -> str:
    original = path.read_text(encoding="utf-8")
    count = original.count(before)
    if count != 1:
        raise RuntimeError(
            f"mutation anchor must occur exactly once in {path}, found {count}"
        )
    path.write_text(original.replace(before, after), encoding="utf-8")
    return original


def run(repo: Path, args: list[str]) -> subprocess.CompletedProcess[str]:
    environment = os.environ.copy()
    environment["CARGO_TARGET_DIR"] = str(ROOT / "target" / "mutation-guards")
    return subprocess.run(
        args,
        cwd=repo,
        env=environment,
        check=False,
        capture_output=True,
        text=True,
    )


def prove_semantic_contract(repo: Path) -> None:
    source = repo / "crates/ags-task-contract/src/validator/validate.rs"
    original = replace_once(
        source,
        "    check_execution_authority_gate(&fields, &mut errors);\n",
        "    // mutation: authorization gate omitted\n",
    )
    try:
        result = run(
            repo,
            [
                "cargo",
                "test",
                "-q",
                "-p",
                "ags-task-contract",
                "validator::tests::workflow_request_without_authority_has_stable_code",
                "--",
                "--exact",
            ],
        )
        if result.returncode == 0:
            raise RuntimeError(
                "semantic mutation survived: omitting the authorization gate did not fail its contract test"
            )
    finally:
        source.write_text(original, encoding="utf-8")


def prove_cli_fixture_gate(repo: Path) -> None:
    source = repo / "crates/ags-task-contract/src/validator/validate.rs"
    original = replace_once(
        source,
        'line.starts_with("AGENT_SUITE_COMPACT_TASK_CARD_V1")',
        'line.starts_with("MUTATION_DISABLED_COMPACT_DISCRIMINATOR")',
    )
    try:
        escaped = run(
            repo,
            [
                "cargo",
                "run",
                "-q",
                "-p",
                "ags-cli",
                "--",
                "task",
                "validate",
                "tests/fixtures/invalid-compact.md",
            ],
        )
        if escaped.returncode != 0:
            raise RuntimeError(
                "fixture mutation did not create the intended regression; validator still rejected compact input"
            )

        report = run(
            repo,
            [
                "cargo",
                "run",
                "-q",
                "-p",
                "ags-cli",
                "--",
                "verify",
                "--scope",
                "local",
                "--format",
                "json",
            ],
        )
        try:
            payload = json.loads(report.stdout)
        except json.JSONDecodeError as error:
            raise RuntimeError(
                f"mutated verifier did not produce JSON: {error}\n{report.stderr[-1000:]}"
            ) from error
        matching = [
            item
            for item in payload.get("items", [])
            if item.get("id") == "fixture-invalid-compact-rejected"
        ]
        if len(matching) != 1 or matching[0].get("status") != "fail":
            raise RuntimeError(
                "CLI fixture gate did not report the injected compact-format regression"
            )
    finally:
        source.write_text(original, encoding="utf-8")


def main() -> int:
    with tempfile.TemporaryDirectory(prefix="ags-validator-mutations-") as raw:
        repo = Path(raw) / "repo"
        copy_workspace(repo)
        prove_semantic_contract(repo)
        prove_cli_fixture_gate(repo)
    print(
        "PASS: semantic contract and CLI fixture gate detected representative injected regressions"
    )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except RuntimeError as error:
        print(f"FAIL: {error}", file=sys.stderr)
        raise SystemExit(1)
