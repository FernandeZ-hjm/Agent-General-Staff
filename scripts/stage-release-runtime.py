#!/usr/bin/env python3
"""Stage runtime support files from an AGS release package plan."""

from __future__ import annotations

import argparse
import json
import shutil
import sys
from pathlib import Path


def reject_symlink_components(root: Path, relative: Path, label: str) -> None:
    current = root
    if current.is_symlink():
        raise SystemExit(f"{label} root must not be a symlink: {root}")
    for component in relative.parts:
        current /= component
        if current.is_symlink():
            raise SystemExit(
                f"{label} path must not contain a symlink: {relative}"
            )


def ensure_target_parent(root: Path, relative: Path) -> Path:
    current = root
    for component in relative.parent.parts:
        current /= component
        if current.is_symlink():
            raise SystemExit(
                f"runtime target path must not contain a symlink: {relative}"
            )
        current.mkdir(exist_ok=True)
        if not current.is_dir():
            raise SystemExit(
                f"runtime target parent is not a directory: {relative}"
            )
    resolved = current.resolve(strict=True)
    try:
        resolved.relative_to(root.resolve(strict=True))
    except ValueError as error:
        raise SystemExit(
            f"runtime target escapes target root: {relative}"
        ) from error
    return current


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--plan", required=True, type=Path)
    parser.add_argument("--source", default=Path("."), type=Path)
    parser.add_argument("--target", required=True, type=Path)
    args = parser.parse_args()

    if args.plan.is_symlink():
        raise SystemExit(f"release plan must not be a symlink: {args.plan}")
    plan = json.loads(args.plan.read_text(encoding="utf-8"))
    if plan.get("schema_version") != "0.3.4-release-plan":
        raise SystemExit("release plan schema_version must be 0.3.4-release-plan")
    if plan.get("profile") != "public-full":
        raise SystemExit("release plan profile must be public-full")
    if plan.get("authority_errors"):
        raise SystemExit(
            "release payload authority errors: "
            + ", ".join(plan["authority_errors"])
        )
    if plan.get("required_missing"):
        raise SystemExit(
            "release payload missing required files: "
            + ", ".join(plan["required_missing"])
        )
    if plan.get("extra_files"):
        raise SystemExit(
            "release payload contains non-authority files: "
            + ", ".join(plan["extra_files"])
        )
    if "content_mismatches" not in plan:
        raise SystemExit("release plan must include content_mismatches")
    if plan.get("content_mismatches"):
        raise SystemExit(
            "release payload contains unapproved content drift: "
            + ", ".join(plan["content_mismatches"])
        )
    if plan.get("forbidden_included"):
        raise SystemExit(
            "release payload contains forbidden files: "
            + ", ".join(plan["forbidden_included"])
        )

    source_input = args.source.absolute()
    if source_input.is_symlink():
        raise SystemExit(f"runtime source root must not be a symlink: {args.source}")
    source = source_input.resolve(strict=True)
    target_input = args.target.absolute()
    if target_input.is_symlink():
        raise SystemExit(f"runtime target root must not be a symlink: {args.target}")
    target_input.mkdir(parents=True, exist_ok=True)
    target = target_input.resolve(strict=True)
    existing_target_entries = list(target.iterdir())
    if any(entry.is_symlink() for entry in existing_target_entries):
        raise SystemExit("runtime target path must not contain a symlink")
    if existing_target_entries:
        raise SystemExit("runtime target root must be empty before staging")
    runtime_assets = plan.get("runtime_asset_files")
    if not isinstance(runtime_assets, list) or not all(
        isinstance(relative, str) for relative in runtime_assets
    ):
        raise SystemExit("release plan runtime_asset_files must be a string array")
    if len(runtime_assets) != len(set(runtime_assets)):
        raise SystemExit("release plan runtime_asset_files must not contain duplicates")
    included_files = plan.get("included_files")
    if not isinstance(included_files, list) or not all(
        isinstance(relative, str) for relative in included_files
    ):
        raise SystemExit("release plan included_files must be a string array")
    if len(included_files) != len(set(included_files)):
        raise SystemExit("release plan included_files must not contain duplicates")
    outside_payload = sorted(set(runtime_assets).difference(included_files))
    if outside_payload:
        raise SystemExit(
            "runtime assets are outside the canonical included payload: "
            + ", ".join(outside_payload)
        )

    for relative in runtime_assets:
        relative_path = Path(relative)
        if (
            not relative
            or "\\" in relative
            or relative_path.is_absolute()
            or relative_path == Path(".")
            or ".." in relative_path.parts
        ):
            raise SystemExit(f"unsafe runtime asset path: {relative}")
        reject_symlink_components(source, relative_path, "runtime source")
        source_file = source / relative_path
        if not source_file.is_file():
            raise SystemExit(f"runtime asset missing: {relative}")
        resolved_source = source_file.resolve(strict=True)
        try:
            resolved_source.relative_to(source)
        except ValueError as error:
            raise SystemExit(
                f"runtime asset escapes source root: {relative}"
            ) from error
        ensure_target_parent(target, relative_path)
        target_file = target / relative_path
        if target_file.is_symlink():
            raise SystemExit(
                f"runtime target path must not contain a symlink: {relative}"
            )
        if target_file.exists() and not target_file.is_file():
            raise SystemExit(f"runtime target is not a regular file: {relative}")
        resolved_parent = target_file.parent.resolve(strict=True)
        try:
            resolved_parent.relative_to(target)
        except ValueError as error:
            raise SystemExit(
                f"runtime target escapes target root: {relative}"
            ) from error
        shutil.copy2(resolved_source, target_file)

    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"stage-release-runtime: {error}", file=sys.stderr)
        raise SystemExit(1) from error
