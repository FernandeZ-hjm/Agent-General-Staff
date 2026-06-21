#!/usr/bin/env python3
"""Capture pasted Claude Code task-card runs into local task memory.

Claude Code command hooks receive JSON on stdin. This Stop hook looks for a
copy-pasted Agent Suite task card and the final delivery report in the current
transcript, builds a small receipt package, and delegates the actual memory
write to context-memory.sh.

It is intentionally conservative:
- no task card -> skip
- no delivery report -> skip
- duplicate report for the same transcript -> skip
- never writes context-capsule.md directly
"""

from __future__ import annotations

import datetime as _dt
import hashlib
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
from typing import Any


def now_stamp() -> str:
    return _dt.datetime.now().strftime("%Y%m%d-%H%M%S")


def log(message: str) -> None:
    log_dir = pathlib.Path(os.environ.get("CLAUDE_STOP_MEMORY_LOG_DIR", "~/.agents/logs")).expanduser()
    log_dir.mkdir(parents=True, exist_ok=True)
    log_file = log_dir / "claude-stop-memory-capture.log"
    line = f"{_dt.datetime.now().strftime('%Y-%m-%d %H:%M:%S')} {message}\n"
    with log_file.open("a", encoding="utf-8") as handle:
        handle.write(line)


def read_hook_input() -> dict[str, Any]:
    raw = sys.stdin.read()
    if not raw.strip():
        return {}
    try:
        data = json.loads(raw)
    except json.JSONDecodeError as exc:
        log(f"[SKIP] invalid hook JSON: {exc}")
        return {}
    return data if isinstance(data, dict) else {}


def text_from_content(content: Any) -> str:
    if isinstance(content, str):
        return content
    if not isinstance(content, list):
        return ""

    parts: list[str] = []
    for block in content:
        if not isinstance(block, dict):
            continue
        if block.get("type") == "text" and isinstance(block.get("text"), str):
            parts.append(block["text"])
    return "\n\n".join(part for part in parts if part)


def text_from_entry(entry: dict[str, Any]) -> tuple[str, str]:
    msg = entry.get("message")
    if not isinstance(msg, dict):
        return "", ""
    role = msg.get("role")
    text = text_from_content(msg.get("content"))
    return str(role or ""), text


def read_transcript(path: pathlib.Path) -> list[dict[str, Any]]:
    entries: list[dict[str, Any]] = []
    with path.open("r", encoding="utf-8") as handle:
        for line in handle:
            if not line.strip():
                continue
            try:
                item = json.loads(line)
            except json.JSONDecodeError:
                continue
            if isinstance(item, dict):
                entries.append(item)
    return entries


def looks_like_task_card(text: str) -> bool:
    if not text:
        return False
    # The compact task-card format is retired suite-wide; the canonical task
    # card is discriminated by the `## 任务卡` heading only.
    has_task_card_marker = "## 任务卡" in text
    has_runtime_fields = "Executor:" in text and "Runtime adapter:" in text
    has_task_body = "任务" in text and ("Verification gate:" in text or "验证" in text)
    return has_task_card_marker and has_runtime_fields and has_task_body


def looks_like_delivery_report(text: str) -> bool:
    if not text:
        return False
    has_report_marker = "任务交付报告" in text or "# Delivery Report" in text
    has_status = "## 任务状态" in text or "任务状态" in text
    has_conclusion = "一句话结论" in text or "Conclusion" in text
    return has_report_marker and has_status and has_conclusion


def find_task_card_and_report(entries: list[dict[str, Any]], hook_input: dict[str, Any]) -> tuple[str, str]:
    """Pair the latest task card with a delivery report that comes AFTER it.

    Scanning order matters: a delivery report only belongs to the most recent
    task card. When a newer task card appears, any earlier report is discarded
    so an old report can never be archived as the result of a new, unfinished
    task. Reports seen before any task card are ignored.
    """
    task_card = ""
    report = ""

    for entry in entries:
        role, text = text_from_entry(entry)
        if role == "user" and looks_like_task_card(text):
            task_card = text.strip()
            # A new task card invalidates any report collected for a prior card.
            report = ""
        elif role == "assistant" and task_card and looks_like_delivery_report(text):
            # Only reports that follow the current task card count; the latest
            # such report wins.
            report = text.strip()

    # The final assistant message (if a report) belongs to the latest card since
    # it is the most recent turn; only used when no in-transcript report paired.
    last_assistant = hook_input.get("last_assistant_message")
    if (
        task_card
        and not report
        and isinstance(last_assistant, str)
        and looks_like_delivery_report(last_assistant)
    ):
        report = last_assistant.strip()

    return task_card, report


def safe_slug(value: str) -> str:
    cleaned = re.sub(r"[^A-Za-z0-9._-]+", "-", value).strip("-")
    return cleaned or "project"


def resolve_repo_path(cwd: str) -> pathlib.Path:
    if not cwd:
        return pathlib.Path.cwd().resolve()

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


def find_context_memory_script() -> pathlib.Path | None:
    candidates = [
        os.environ.get("AGENT_CONTEXT_MEMORY_SH", ""),
        # Sibling next to this hook (e.g. ~/.agents/scripts/ after `ags setup`,
        # or scripts/ inside the suite). Keeps the bridge machine-independent.
        str(pathlib.Path(__file__).resolve().parent / "context-memory.sh"),
        "~/.agents/scripts/context-memory.sh",
    ]
    for candidate in candidates:
        if not candidate:
            continue
        path = pathlib.Path(candidate).expanduser()
        if path.is_file():
            return path
    return None


def fingerprint(transcript_path: pathlib.Path, task_card: str, report: str) -> str:
    payload = f"{transcript_path}\n---TASK---\n{task_card}\n---REPORT---\n{report}"
    return hashlib.sha256(payload.encode("utf-8")).hexdigest()


def state_marker(fp: str) -> pathlib.Path:
    state_dir = pathlib.Path(os.environ.get("CLAUDE_STOP_MEMORY_STATE_DIR", "~/.agents/state/claude-stop-memory-capture")).expanduser()
    state_dir.mkdir(parents=True, exist_ok=True)
    return state_dir / f"{fp}.done"


def already_captured(fp: str) -> bool:
    return state_marker(fp).exists()


def mark_captured(fp: str) -> None:
    state_marker(fp).write_text(now_stamp() + "\n", encoding="utf-8")


def write_receipt(repo_path: pathlib.Path, task_card: str, report: str, hook_input: dict[str, Any]) -> pathlib.Path:
    receipt_root = pathlib.Path(os.environ.get("CLAUDE_STOP_MEMORY_RECEIPT_ROOT", "~/.agents/task-receipts")).expanduser()
    repo_slug = safe_slug(repo_path.name)
    receipt_dir = receipt_root / repo_slug / f"{now_stamp()}-claude-stop-memory"
    receipt_dir.mkdir(parents=True, exist_ok=True)

    (receipt_dir / "task-card.md").write_text(task_card + "\n", encoding="utf-8")
    (receipt_dir / "delivery-report.md").write_text(report + "\n", encoding="utf-8")
    (receipt_dir / "hook-input.json").write_text(
        json.dumps(hook_input, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )
    (receipt_dir / "metadata.json").write_text(
        json.dumps(
            {
                "created_at": _dt.datetime.now().isoformat(timespec="seconds"),
                "repo_path": str(repo_path),
                "source": "claude-stop-memory-capture",
                "transcript_path": hook_input.get("transcript_path", ""),
                "session_id": hook_input.get("session_id", ""),
            },
            ensure_ascii=False,
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )

    for name, command in {
        "git-status.after.txt": ["git", "-C", str(repo_path), "status", "--short", "--untracked-files=all"],
        "diff-stat.txt": ["git", "-C", str(repo_path), "diff", "--stat"],
    }.items():
        try:
            result = subprocess.run(command, check=False, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
            (receipt_dir / name).write_text(result.stdout, encoding="utf-8")
        except OSError as exc:
            (receipt_dir / name).write_text(f"[SKIP] {exc}\n", encoding="utf-8")

    return receipt_dir


def capture_memory(receipt_dir: pathlib.Path, repo_path: pathlib.Path) -> bool:
    context_memory = find_context_memory_script()
    if context_memory is None:
        log("[SKIP] context-memory.sh not found")
        return False

    result = subprocess.run(
        ["bash", str(context_memory), "capture", str(receipt_dir), "--repo", str(repo_path)],
        check=False,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )
    capture_log = receipt_dir / "context-memory-capture.log"
    capture_log.write_text(result.stdout, encoding="utf-8")
    if result.returncode == 0:
        log(f"[OK] captured receipt={receipt_dir} repo={repo_path}")
        return True
    else:
        log(f"[SKIP] context-memory capture failed status={result.returncode} receipt={receipt_dir}")
        return False


def main() -> int:
    if os.environ.get("CLAUDE_STOP_MEMORY_CAPTURE", "1") != "1":
        return 0

    hook_input = read_hook_input()
    event = hook_input.get("hook_event_name")
    if event not in ("Stop", "SubagentStop", ""):
        return 0

    transcript = hook_input.get("transcript_path")
    if not isinstance(transcript, str) or not transcript:
        log("[SKIP] missing transcript_path")
        return 0

    transcript_path = pathlib.Path(transcript).expanduser()
    if not transcript_path.is_file():
        log(f"[SKIP] transcript not found: {transcript_path}")
        return 0

    entries = read_transcript(transcript_path)
    task_card, report = find_task_card_and_report(entries, hook_input)
    if not task_card:
        log(f"[SKIP] no task card in transcript: {transcript_path}")
        return 0
    if not report:
        log(f"[SKIP] no delivery report in transcript: {transcript_path}")
        return 0

    fp = fingerprint(transcript_path, task_card, report)
    if already_captured(fp):
        log(f"[OK] duplicate capture skipped transcript={transcript_path}")
        return 0

    repo_path = resolve_repo_path(str(hook_input.get("cwd") or ""))
    receipt_dir = write_receipt(repo_path, task_card, report, hook_input)
    if capture_memory(receipt_dir, repo_path):
        mark_captured(fp)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:  # Never block Claude from stopping because memory capture failed.
        log(f"[SKIP] unexpected error: {exc}")
        raise SystemExit(0)
