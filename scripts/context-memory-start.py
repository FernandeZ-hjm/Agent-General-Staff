#!/usr/bin/env python3
"""Inject AGS project memory at host session start.

Native host adapters pass JSON on stdin. This start bridge resolves the current
repository, reads the AGS local project memory store, and returns a bounded
read-only context block. Claude Code and Codex consume
`hookSpecificOutput.additionalContext`; the OMP extension maps it to
`systemPromptAppend`.

It never writes memory files. Task-end capture remains owned by
claude-stop-memory-capture.py and context-memory.sh.
"""

from __future__ import annotations

import json
import os
import pathlib
import re
import subprocess
import sys
from typing import Any


DEFAULT_MAX_CAPSULE_CHARS = 12000
DEFAULT_MAX_TASK_MEMORY_CHARS = 8000


def read_hook_input() -> dict[str, Any]:
    raw = sys.stdin.read()
    if not raw.strip():
        return {}
    try:
        data = json.loads(raw)
    except json.JSONDecodeError:
        return {}
    return data if isinstance(data, dict) else {}


def int_env(name: str, default: int) -> int:
    raw = os.environ.get(name, "")
    try:
        value = int(raw)
    except ValueError:
        return default
    return value if value > 0 else default


def resolve_repo_path(cwd: str) -> pathlib.Path:
    if not cwd:
        cwd = os.getcwd()

    cwd_path = pathlib.Path(cwd).expanduser()
    try:
        result = subprocess.run(
            ["git", "-C", str(cwd_path), "rev-parse", "--show-toplevel"],
            check=False,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
        )
        if result.returncode == 0 and result.stdout.strip():
            return pathlib.Path(result.stdout.strip()).resolve()
    except OSError:
        pass

    try:
        return cwd_path.resolve()
    except OSError:
        return cwd_path


def safe_slug(value: str) -> str:
    cleaned = re.sub(r"[^A-Za-z0-9._-]+", "-", value).strip("-")
    return cleaned or "project"


def project_slug(repo_path: pathlib.Path) -> str:
    profile = repo_path / "config" / "agent-project-profile.yaml"
    try:
        raw = profile.read_text(encoding="utf-8")
    except OSError:
        return safe_slug(repo_path.name)

    in_project = False
    for line in raw.splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        if re.match(r"^project\s*:\s*$", stripped):
            in_project = True
            continue
        if in_project and re.match(r"^[A-Za-z0-9_-]+\s*:", stripped) and not line.startswith((" ", "\t")):
            in_project = False
        if in_project:
            match = re.match(r"^slug\s*:\s*['\"]?([^'\"#]+)['\"]?\s*(?:#.*)?$", stripped)
            if match:
                slug = match.group(1).strip()
                if slug:
                    return safe_slug(slug)

    return safe_slug(repo_path.name)


def memory_root() -> pathlib.Path:
    return pathlib.Path(os.environ.get("MEMORY_ROOT", "~/.agents/memory/projects")).expanduser()


def bounded_read(path: pathlib.Path, max_chars: int) -> str | None:
    try:
        text = path.read_text(encoding="utf-8")
    except OSError:
        return None
    if len(text) <= max_chars:
        return text.rstrip()
    return (
        text[:max_chars].rstrip()
        + f"\n\n[truncated by AGS context-memory-start.py at {max_chars} characters]"
    )


def build_context(repo_path: pathlib.Path) -> str | None:
    slug = project_slug(repo_path)
    project_dir = memory_root() / slug
    capsule = project_dir / "context-capsule.md"
    task_memory = project_dir / "task-memory.md"

    capsule_text = bounded_read(
        capsule,
        int_env("AGS_MEMORY_START_MAX_CAPSULE_CHARS", DEFAULT_MAX_CAPSULE_CHARS),
    )
    task_text = bounded_read(
        task_memory,
        int_env("AGS_MEMORY_START_MAX_TASK_CHARS", DEFAULT_MAX_TASK_MEMORY_CHARS),
    )

    if not capsule_text and not task_text:
        return None

    parts = [
        "## AGS Project Memory Context",
        "",
        "Read-only startup context injected by AGS. Do not write memory files from this hook.",
        f"Repository: {repo_path}",
        f"Memory store: {project_dir}",
    ]
    if capsule_text:
        parts.extend(["", "### context-capsule.md", "", capsule_text])
    if task_text:
        parts.extend(["", "### task-memory.md", "", task_text])
    return "\n".join(parts).rstrip()


def main() -> int:
    hook_input = read_hook_input()
    cwd = str(hook_input.get("cwd") or hook_input.get("workspace") or "")
    repo_path = resolve_repo_path(cwd)
    context = build_context(repo_path)
    if not context:
        return 0

    hook_event_name = str(
        hook_input.get("hook_event_name")
        or hook_input.get("hookEventName")
        or "SessionStart"
    )
    sys.stdout.write(
        json.dumps(
            {
                "suppressOutput": True,
                "hookSpecificOutput": {
                    "hookEventName": hook_event_name,
                    "additionalContext": context,
                },
            },
            ensure_ascii=False,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
