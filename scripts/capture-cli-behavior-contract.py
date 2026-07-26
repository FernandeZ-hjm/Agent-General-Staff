#!/usr/bin/env python3
"""Capture deterministic v0.3.0 Human and Machine CLI behavior contracts."""

from __future__ import annotations

import hashlib
import json
import os
import re
import stat
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parents[1]
BASELINE_RELEASE_COMMIT = "7d7e0477829a9288e97f3f2536a5ba6a8763cd58"
BASELINE_EXECUTABLE_SHA256 = (
    "af4aaf3f396bbb83c9f2bee3cac2c6352df412e4c6a2c9aade6a8417aeb2a7be"
)
INPUT_FIXTURE = Path("crates/ags-cli/tests/fixtures/cli-behavior-input-v0.3.0.json")
INPUT_FIXTURE_SHA256 = (
    "c0235bb37159d1788afc62086f299a6266c11102153ea69c888cfa01a4a463f1"
)
STDIN_FIXTURE_SHA256 = {
    "tests/fixtures/valid-full.md": (
        "e08f207b0ca39010f5a96c9ac4e0e4d52ec5b7b5e8f9a12ed8b379f59608b361"
    ),
    "tests/fixtures/invalid-ultracode-authority-abuse.md": (
        "07360be698269fdd4e68bbc10a7275850e5af10699608539bd4876b9607ce7e9"
    ),
    "tests/fixtures/receipt-valid.json": (
        "201116091de1d502e646aa888ca39abb30073d2e18ba8e9ed6bc3dfeb53e7157"
    ),
}


def human(
    case_id: str,
    root: str,
    args: list[str],
    *,
    sandbox: bool = False,
    output_policy: str = "exact",
    json_assertion_paths: tuple[str, ...] = (),
) -> dict[str, Any]:
    return {
        "id": case_id,
        "surface": "human",
        "human_root": root,
        "args": args,
        "stdin_fixture": None,
        "sandbox": sandbox,
        "output_policy": output_policy,
        "json_assertion_paths": json_assertion_paths,
    }


def machine(
    case_id: str,
    capability: str,
    args: list[str],
    stdin_fixture: str | None = None,
    *,
    argv_fixture: str | None = None,
) -> dict[str, Any]:
    return {
        "id": case_id,
        "surface": "machine",
        "machine_capability": capability,
        "args": args,
        "stdin_fixture": stdin_fixture,
        "argv_fixture": argv_fixture,
        "sandbox": True,
        "output_policy": "exact",
        "json_assertion_paths": (),
    }


CASES = (
    # Real Human execution paths against one immutable suite/project sandbox.
    # These are deliberately not `--help` probes: they lock plan, dry-run,
    # confirmed apply, refusal, JSON, stdout/stderr, exit status, and mutation
    # shape to the released v0.3.0 executable.
    human(
        "human_setup_yes_force_dry_run_json",
        "setup",
        [
            "setup",
            "--target",
            "{{runtime}}",
            "--yes",
            "--force",
            "--dry-run",
            "--format",
            "json",
        ],
        sandbox=True,
        output_policy="json-contract",
        json_assertion_paths=(
            "$.schema_version",
            "$.profile",
            "$.write_mode",
            "$.source_root",
            "$.target",
        ),
    ),
    human(
        "human_init_plan_json",
        "init",
        [
            "init",
            "--target",
            "{{project}}",
            "--dry-run",
            "--mode",
            "local",
            "--format",
            "json",
        ],
        sandbox=True,
        output_policy="json-contract",
        json_assertion_paths=(
            "$.schema_version",
            "$.dry_run",
            "$.target",
            "$.overlay.mode",
            "$.overlay.is_git_repo",
            "$.overlay.migrate",
        ),
    ),
    human(
        "human_init_apply_json",
        "init",
        [
            "init",
            "--target",
            "{{project}}",
            "--mode",
            "local",
            "--format",
            "json",
        ],
        sandbox=True,
        output_policy="json-contract",
        json_assertion_paths=(
            "$.schema_version",
            "$.plan.schema_version",
            "$.plan.dry_run",
            "$.plan.target",
            "$.overlay.mode",
            "$.overlay.is_git_repo",
            "$.overlay.migrate",
        ),
    ),
    human(
        "human_skill_adopt_apply_refusal_json",
        "skill",
        [
            "skill",
            "adopt",
            "missing-capability",
            "--apply",
            "--host",
            "codex",
            "--format",
            "json",
        ],
        sandbox=True,
    ),
    # Parser-level probes retain all eight visible roots and reject-path
    # details. Write flags are covered by the real Human cases above.
    human(
        "human_onboarding_missing_action",
        "onboarding",
        ["onboarding"],
    ),
    human(
        "human_doctor_invalid_format",
        "doctor",
        ["doctor", "--format", "yaml"],
    ),
    human(
        "human_agents_missing_action",
        "agents",
        ["agents"],
    ),
    human(
        "human_capability_missing_action",
        "capability",
        ["capability"],
    ),
    human(
        "human_update_missing_action",
        "update",
        ["update"],
    ),
    # Canonical Machine CLI capabilities. These preserve success, refusal,
    # missing arguments, invalid enums, JSON output, and exit codes.
    machine(
        "task_compile_rejects_unscoped_handoff_json",
        "TaskCompile",
        [
            "task",
            "compile",
            "-",
            "--format",
            "json",
            "--output",
            "report",
            "--task-card-requested",
            "--confirmed-handoff-contract",
        ],
        "tests/fixtures/valid-full.md",
    ),
    machine(
        "task_validate_success_text",
        "TaskValidate",
        ["task", "validate", "-"],
        "tests/fixtures/valid-full.md",
    ),
    machine(
        "task_validate_reject_text",
        "TaskValidate",
        ["task", "validate", "-"],
        "tests/fixtures/invalid-ultracode-authority-abuse.md",
    ),
    machine(
        "policy_resolve_missing_path",
        "PolicyResolve",
        ["policy", "resolve"],
    ),
    machine(
        "policy_resolve_invalid_format",
        "PolicyResolve",
        ["policy", "resolve", "-", "--format", "yaml"],
    ),
    machine(
        "policy_resolve_success_json",
        "PolicyResolve",
        ["policy", "resolve", "-", "--format", "json"],
        "tests/fixtures/valid-full.md",
    ),
    machine(
        "task_prepare_execution_check_only_json",
        "TaskPrepareExecution",
        ["run", "-", "--check-only", "--format", "json"],
        "tests/fixtures/valid-full.md",
    ),
    machine(
        "project_verify_invalid_scope",
        "ProjectVerify",
        [
            "verify",
            "--scope",
            "unsupported",
            "--format",
            "json",
            "--target",
            ".",
        ],
    ),
    machine(
        "skill_tags_verify_missing_path",
        "SkillTagsVerify",
        ["gate", "skill-tags"],
    ),
    machine(
        "skill_adopt_missing_skill_id",
        "SkillAdopt",
        ["skill", "adopt"],
    ),
    machine(
        "receipt_verify_success_json",
        "ReceiptVerify",
        [
            "receipt",
            "verify",
            "tests/fixtures/receipt-valid.json",
            "--format",
            "json",
        ],
        argv_fixture="tests/fixtures/receipt-valid.json",
    ),
)

RECEIPT_NAME = re.compile(r"ar-[a-z0-9-]+-\d{10}-[0-9a-f]{16}\.json")
RECEIPT_ID = re.compile(r"ar-[a-z0-9-]+-\d{10}-[0-9a-f]{16}")
UNIX_TIMESTAMP = re.compile(r"unix-\d{10}")


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def assert_sha(path: Path, expected: str, label: str) -> bytes:
    data = path.read_bytes()
    actual = sha256(data)
    if actual != expected:
        raise SystemExit(f"{label} SHA-256 mismatch: expected {expected}, got {actual}")
    return data


def load_input_fixture() -> dict[str, Any]:
    raw = assert_sha(
        REPO_ROOT / INPUT_FIXTURE,
        INPUT_FIXTURE_SHA256,
        "immutable Human input fixture",
    )
    document = json.loads(raw)
    if document.get("schema_version") != "ags-cli-behavior-input/1":
        raise SystemExit("unexpected Human input fixture schema")
    return document


def seed_sandbox(root: Path, input_document: dict[str, Any]) -> None:
    for relative, content in input_document["files"].items():
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")
        if relative.endswith(".sh"):
            path.chmod(0o755)
    for relative, content in input_document["stdin_fixtures"].items():
        path = root / "suite" / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")
    for directory in ("home", "runtime", "xdg"):
        (root / directory).mkdir(parents=True, exist_ok=True)


def expand_args(args: list[str], root: Path | None) -> list[str]:
    if root is None:
        return args
    replacements = {
        "{{root}}": str(root),
        "{{suite}}": str(root / "suite"),
        "{{project}}": str(root / "project"),
        "{{runtime}}": str(root / "runtime"),
        "{{home}}": str(root / "home"),
    }
    return [replacements.get(arg, arg) for arg in args]


def normalize(text: str, root: Path | None = None) -> str:
    normalized = text.replace("\r\n", "\n").replace("ags.exe", "ags")
    if root is not None:
        candidates = {
            str(root),
            str(root.resolve()),
            str(root / "home"),
            str((root / "home").resolve()),
        }
        for candidate in sorted(candidates, key=len, reverse=True):
            normalized = normalized.replace(candidate, "<CONTRACT_ROOT>")
            sanitized = candidate.strip("/").replace("/", "-").replace("\\", "-").replace(".", "-").strip("-")
            normalized = normalized.replace(
                sanitized, "<CONTRACT_ROOT_SANITIZED>"
            )
    normalized = RECEIPT_NAME.sub("<RECEIPT>.json", normalized)
    normalized = RECEIPT_ID.sub("<RECEIPT>", normalized)
    return UNIX_TIMESTAMP.sub("unix-<TIMESTAMP>", normalized)


def json_type(value: Any) -> str:
    if value is None:
        return "null"
    if isinstance(value, bool):
        return "boolean"
    if isinstance(value, str):
        return "string"
    if isinstance(value, list):
        return "array"
    if isinstance(value, dict):
        return "object"
    if isinstance(value, int):
        return "integer"
    return "number"


def collect_json_types(
    value: Any,
    path: str,
    output: set[tuple[str, str]],
) -> None:
    output.add((path, json_type(value)))
    if isinstance(value, dict):
        for key, child in value.items():
            collect_json_types(child, f"{path}.{key}", output)
    elif isinstance(value, list):
        for child in value:
            collect_json_types(child, f"{path}[]", output)


def json_value_at_path(value: Any, path: str) -> Any:
    if not path.startswith("$."):
        raise SystemExit(f"unsupported JSON assertion path: {path}")
    current = value
    for key in path[2:].split("."):
        if not isinstance(current, dict) or key not in current:
            raise SystemExit(f"baseline JSON does not contain assertion path: {path}")
        current = current[key]
    return current


def stdout_json_contract(stdout: str, assertion_paths: tuple[str, ...]) -> dict[str, Any]:
    document = json.loads(stdout)
    required_types: set[tuple[str, str]] = set()
    collect_json_types(document, "$", required_types)
    return {
        "baseline": stdout,
        "required_types": [
            {"path": path, "type": value_type}
            for path, value_type in sorted(required_types)
        ],
        "assertions": [
            {"path": path, "value": json_value_at_path(document, path)}
            for path in assertion_paths
        ],
        # Product-version exceptions must be named explicitly. Wire/schema
        # versions are compatibility identities and are never inferred here.
        "allowed_product_version_paths": [],
    }


def normalized_file_bytes(path: Path, root: Path) -> bytes:
    data = path.read_bytes()
    try:
        return normalize(data.decode("utf-8"), root).encode("utf-8")
    except UnicodeDecodeError:
        return data


def file_state(path: Path, root: Path) -> dict[str, Any]:
    metadata = path.lstat()
    mode = stat.S_IMODE(metadata.st_mode)
    if path.is_symlink():
        return {
            "kind": "symlink",
            "mode": f"{mode:o}",
            "target": normalize(os.readlink(path), root),
        }
    if path.is_dir():
        return {"kind": "directory", "mode": f"{mode:o}"}
    return {
        "kind": "file",
        "mode": f"{mode:o}",
        "sha256": "sha256:" + sha256(normalized_file_bytes(path, root)),
    }


def snapshot(root: Path) -> dict[str, dict[str, Any]]:
    result: dict[str, dict[str, Any]] = {}
    for path in sorted(root.rglob("*")):
        relative = path.relative_to(root).as_posix()
        result[RECEIPT_NAME.sub("<RECEIPT>.json", relative)] = file_state(path, root)
    return result


def filesystem_delta(
    before: dict[str, dict[str, Any]],
    after: dict[str, dict[str, Any]],
) -> dict[str, list[dict[str, Any]]]:
    created = [
        {"path": path, **after[path]} for path in sorted(after.keys() - before.keys())
    ]
    deleted = [
        {"path": path, **before[path]} for path in sorted(before.keys() - after.keys())
    ]
    modified = [
        {"path": path, "before": before[path], "after": after[path]}
        for path in sorted(before.keys() & after.keys())
        if before[path] != after[path]
    ]
    return {"created": created, "modified": modified, "deleted": deleted}


def stdin_for(
    case: dict[str, Any],
    input_document: dict[str, Any],
) -> tuple[str | None, str | None]:
    fixture = case["stdin_fixture"]
    if fixture is None:
        return None, None
    expected = STDIN_FIXTURE_SHA256[str(fixture)]
    raw = input_document["stdin_fixtures"][str(fixture)].encode("utf-8")
    actual = sha256(raw)
    if actual != expected:
        raise SystemExit(
            f"immutable stdin fixture {fixture} SHA-256 mismatch: "
            f"expected {expected}, got {actual}"
        )
    return raw.decode("utf-8"), "sha256:" + expected


def capture(
    binary: str,
    case: dict[str, Any],
    input_document: dict[str, Any],
) -> dict[str, Any]:
    stdin, stdin_sha256 = stdin_for(case, input_document)
    argv_fixture = case.get("argv_fixture")
    if argv_fixture is not None:
        expected = STDIN_FIXTURE_SHA256[str(argv_fixture)]
        actual = sha256(
            input_document["stdin_fixtures"][str(argv_fixture)].encode("utf-8")
        )
        if actual != expected:
            raise SystemExit(
                f"immutable argv fixture {argv_fixture} SHA-256 mismatch: "
                f"expected {expected}, got {actual}"
            )
    with tempfile.TemporaryDirectory(prefix="ags-cli-contract-") as temp:
        sandbox_root = Path(temp) if case["sandbox"] else None
        if sandbox_root is not None:
            seed_sandbox(sandbox_root, input_document)
            before = snapshot(sandbox_root)
            cwd = sandbox_root / "suite"
            environment = {
                **os.environ,
                "HOME": str(sandbox_root / "home"),
                "AGS_HOME": str(sandbox_root / "runtime"),
                "AGS_RUNTIME_HOME": str(sandbox_root / "runtime"),
                "AGS_THIRD_PARTY_MANIFEST_OFFLINE": "1",
                "XDG_CONFIG_HOME": str(sandbox_root / "xdg"),
                "PATH": "/usr/bin:/bin",
                "NO_COLOR": "1",
            }
        else:
            before = {}
            cwd = REPO_ROOT
            environment = os.environ.copy()

        result = subprocess.run(
            [binary, *expand_args(case["args"], sandbox_root)],
            cwd=cwd,
            env=environment,
            input=stdin,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        after = snapshot(sandbox_root) if sandbox_root is not None else {}

    captured = {
        key: value
        for key, value in case.items()
        if key not in {"sandbox", "json_assertion_paths"}
    }
    normalized_stdout = normalize(result.stdout, sandbox_root)
    captured.update(
        {
            "stdin_sha256": stdin_sha256,
            "argv_fixture_sha256": (
                "sha256:" + STDIN_FIXTURE_SHA256[str(argv_fixture)]
                if argv_fixture is not None
                else None
            ),
            "exit_code": result.returncode,
            "stdout": normalized_stdout,
            "stderr": normalize(result.stderr, sandbox_root),
            "filesystem_delta": filesystem_delta(before, after),
            "stdout_json_contract": (
                stdout_json_contract(
                    normalized_stdout,
                    tuple(case["json_assertion_paths"]),
                )
                if case["output_policy"] == "json-contract"
                else None
            ),
        }
    )
    return captured


def main() -> None:
    if len(sys.argv) != 3:
        raise SystemExit(
            "usage: capture-cli-behavior-contract.py <ags-v0.3.0> <output.json>"
        )
    binary, destination = sys.argv[1:]
    version = subprocess.run(
        [binary, "--version"],
        check=True,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if version.stderr:
        raise SystemExit(f"unexpected --version stderr: {version.stderr}")
    if version.stdout.strip() != "ags 0.3.0":
        raise SystemExit(
            f"baseline must be ags 0.3.0, got {version.stdout.strip()!r}"
        )
    executable_sha256 = sha256(Path(binary).read_bytes())
    if executable_sha256 != BASELINE_EXECUTABLE_SHA256:
        raise SystemExit(
            "baseline executable SHA-256 mismatch: "
            f"expected {BASELINE_EXECUTABLE_SHA256}, got {executable_sha256}"
        )

    input_document = load_input_fixture()
    document = {
        "schema_version": "ags-cli-behavior-contract/3",
        "baseline_product_version": "0.3.0",
        "baseline_release_tag": "v0.3.0",
        "baseline_release_commit": BASELINE_RELEASE_COMMIT,
        "baseline_executable_sha256": "sha256:" + executable_sha256,
        "input_fixture": INPUT_FIXTURE.as_posix(),
        "input_fixture_sha256": "sha256:" + INPUT_FIXTURE_SHA256,
        "filesystem_delta_policy": "exact-content-hash-unix-mode-symlink-target",
        "filesystem_content_change_allowlist": [
            "project/.gitignore",
            "project/AGENTS.md",
            "project/AGENT_SUITE_PROTOCOL.md",
            "project/CLAUDE.md",
            "runtime/managed-projects.yaml",
            "runtime/receipts/<RECEIPT>.json",
        ],
        "normalization": [
            "crlf-to-lf",
            "ags.exe-to-ags",
            "sandbox-root-to-contract-root",
            "sandbox-sanitized-root-to-placeholder",
            "receipt-name-to-placeholder",
        ],
        "cases": [capture(binary, case, input_document) for case in CASES],
    }
    output = Path(destination)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(
        json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


if __name__ == "__main__":
    main()
