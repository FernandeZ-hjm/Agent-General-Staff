#!/usr/bin/env python3
"""Capture the visible AGS human CLI help tree as a deterministic contract."""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

VISIBLE_ROOTS = {
    "setup",
    "onboarding",
    "init",
    "doctor",
    "agents",
    "capability",
    "skill",
    "update",
}
MACHINE_PATHS = (
    ("task", "compile"),
    ("task", "validate"),
    ("run",),
    ("policy", "resolve"),
    ("project", "detect"),
    ("gate", "skill-tags"),
    ("skill", "adopt"),
    ("receipt", "verify"),
)


def help_text(binary: str, path: tuple[str, ...]) -> str:
    result = subprocess.run(
        [binary, *path, "--help"],
        check=True,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if result.stderr:
        raise SystemExit(
            f"unexpected stderr for {' '.join(path) or '<root>'}: {result.stderr}"
        )
    return result.stdout


def child_commands(text: str) -> list[str]:
    commands = []
    in_commands = False
    for line in text.splitlines():
        if line == "Commands:":
            in_commands = True
            continue
        if not in_commands:
            continue
        if not line.strip():
            if commands:
                break
            continue
        if not line.startswith("  "):
            break
        fields = line.strip().split(maxsplit=1)
        command = fields[0]
        if command != "help":
            commands.append(command)
    return commands


def main() -> None:
    if len(sys.argv) != 3:
        raise SystemExit(
            "usage: capture-human-cli-contract.py <ags-v0.3.0> <output.json>"
        )
    binary, destination = sys.argv[1:]
    version = subprocess.run(
        [binary, "--version"],
        check=True,
        text=True,
        stdout=subprocess.PIPE,
    ).stdout.strip()
    if version != "ags 0.3.0":
        raise SystemExit(f"baseline must be ags 0.3.0, got {version!r}")

    captured: dict[str, str] = {}
    pending = [()]
    while pending:
        path = pending.pop(0)
        text = help_text(binary, path)
        captured[" ".join(path)] = text
        children = child_commands(text)
        if not path:
            children = [child for child in children if child in VISIBLE_ROOTS]
        pending.extend((*path, child) for child in children)

    Path(destination).parent.mkdir(parents=True, exist_ok=True)
    Path(destination).write_text(
        json.dumps(
            {
                "schema_version": "ags-human-cli-contract/1",
                "baseline_product_version": "0.3.0",
                "allowed_version_change": "0.3.1",
                "help": captured,
                "machine_help": {
                    " ".join(path): help_text(binary, path) for path in MACHINE_PATHS
                },
            },
            ensure_ascii=False,
            indent=2,
            sort_keys=True,
        )
        + "\n",
        encoding="utf-8",
    )


if __name__ == "__main__":
    main()
