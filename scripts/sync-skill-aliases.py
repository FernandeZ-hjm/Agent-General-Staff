#!/usr/bin/env python3
"""Converge external skills into ~/.claude/skills as symlinks.

The script is intentionally conservative:
- create missing symlinks for valid skills from known source directories;
- remove broken symlinks;
- remove suite-declared obsolete skill symlinks;
- never delete real directories or files in ~/.claude/skills.
"""
from __future__ import annotations

from pathlib import Path

HOME = Path.home()
SKILLS_DIR = HOME / ".claude" / "skills"
SOURCE_DIRS = [
    HOME / ".agents" / "skills",
    HOME / ".codex" / "skills",
]
SKIP_SKILLS = {"writing-tools"}
OBSOLETE_SKILLS = {
    "graphify-project-map",
    "claude-execution-prompt-maker",
}


def read_name(skill_dir: Path) -> str | None:
    skill_file = skill_dir / "SKILL.md"
    try:
        text = skill_file.read_text(encoding="utf-8")
    except OSError:
        return None
    if not text.startswith("---"):
        return None
    for line in text.splitlines()[1:80]:
        if line.strip() == "---":
            break
        if line.startswith("name:"):
            return line.split(":", 1)[1].strip().strip("'\"")
    return None


def cleanup_stale_symlinks() -> None:
    if not SKILLS_DIR.is_dir():
        return
    for entry in SKILLS_DIR.iterdir():
        if not entry.is_symlink():
            continue
        if entry.name in OBSOLETE_SKILLS:
            entry.unlink()
            print(f"removed obsolete link: {entry.name}")
            continue
        if not entry.exists():
            entry.unlink()
            print(f"removed broken link: {entry.name}")


def iter_valid_source_skills():
    for source_dir in SOURCE_DIRS:
        if not source_dir.is_dir():
            continue
        for source_skill in source_dir.iterdir():
            if not source_skill.is_dir():
                continue
            name = read_name(source_skill)
            if not name or name != source_skill.name:
                continue
            if name in SKIP_SKILLS or name in OBSOLETE_SKILLS:
                continue
            yield name, source_skill


def main() -> None:
    SKILLS_DIR.mkdir(parents=True, exist_ok=True)
    cleanup_stale_symlinks()
    for name, source_skill in iter_valid_source_skills():
        destination = SKILLS_DIR / name
        if destination.exists() or destination.is_symlink():
            continue
        destination.symlink_to(source_skill)
        print(f"linked: {name}")


if __name__ == "__main__":
    main()
