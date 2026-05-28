#!/usr/bin/env node
// configure-review-hooks.mjs
// Normalizes Claude/Codex runtime hook configs.
//
// Active policy:
//   Claude Code:
//     - UserPromptSubmit: sync skill aliases
//     - UserPromptSubmit: read local project memory capsule/index
//     - PreToolUse(Bash): RTK command rewrite
//   Codex:
//     - UserPromptSubmit: sync skill aliases
//     - UserPromptSubmit: read local project memory capsule/index
//
// Old review hooks are removed from config instead of reinstalled.

import fs from "node:fs";
import os from "node:os";
import path from "node:path";

let targetHome = process.env.TARGET_HOME || os.homedir();
let dryRun = true;

for (let i = 2; i < process.argv.length; i += 1) {
  const arg = process.argv[i];
  if (arg === "--target-home") {
    targetHome = process.argv[++i];
  } else if (arg === "--apply") {
    dryRun = false;
  } else if (arg === "--dry-run") {
    dryRun = true;
  } else if (arg === "--help" || arg === "-h") {
    console.log("Usage: configure-review-hooks.mjs [--target-home PATH] [--dry-run|--apply]");
    process.exit(0);
  } else {
    throw new Error(`Unknown option: ${arg}`);
  }
}

const CONFIGS = [
  {
    file: path.join(targetHome, ".claude/settings.json"),
    required: [
      {
        event: "UserPromptSubmit",
        command: "python3 ~/.claude/sync-skill-aliases.py",
      },
      {
        event: "UserPromptSubmit",
        command: "bash ~/.agents/scripts/memory-start-context.sh",
      },
      {
        event: "PreToolUse",
        matcher: "Bash",
        command: "rtk hook claude",
      },
    ],
  },
  {
    file: path.join(targetHome, ".codex/hooks.json"),
    required: [
      {
        event: "UserPromptSubmit",
        command: "python3 ~/.claude/sync-skill-aliases.py",
      },
      {
        event: "UserPromptSubmit",
        command: "bash ~/.agents/scripts/memory-start-context.sh",
      },
    ],
  },
];

const FORBIDDEN_COMMANDS = new Set([
  "node ~/.claude/hooks/review-baseline-snapshot.mjs",
  "node ~/.claude/hooks/leveled-review-gate.mjs",
  "node ~/.claude/hooks/codex-stop-review-adapter.mjs",
]);

function readJson(file) {
  try {
    return JSON.parse(fs.readFileSync(file, "utf8"));
  } catch {
    return {};
  }
}

function matcherMatches(entry, matcher) {
  if (matcher === undefined) return true;
  return entry?.matcher === matcher;
}

function hasCommand(entries, command, matcher) {
  return entries.some((entry) =>
    matcherMatches(entry, matcher) &&
    Array.isArray(entry.hooks) &&
    entry.hooks.some((hook) => hook?.type === "command" && hook?.command === command)
  );
}

function ensureCommand(config, event, command, matcher) {
  config.hooks ||= {};
  config.hooks[event] ||= [];
  if (hasCommand(config.hooks[event], command, matcher)) return false;
  const entry = {
    hooks: [
      {
        type: "command",
        command,
      },
    ],
  };
  if (matcher !== undefined) entry.matcher = matcher;
  config.hooks[event].push(entry);
  return true;
}

function removeForbiddenCommands(config) {
  const changes = [];
  if (!config.hooks || typeof config.hooks !== "object") return changes;

  for (const [event, entries] of Object.entries(config.hooks)) {
    if (!Array.isArray(entries)) continue;
    const nextEntries = [];
    for (const entry of entries) {
      if (!Array.isArray(entry?.hooks)) {
        nextEntries.push(entry);
        continue;
      }
      const nextHooks = [];
      for (const hook of entry.hooks) {
        if (hook?.type === "command" && FORBIDDEN_COMMANDS.has(hook?.command)) {
          changes.push(`${event}: removed ${hook.command}`);
          continue;
        }
        nextHooks.push(hook);
      }
      if (nextHooks.length > 0) {
        nextEntries.push({ ...entry, hooks: nextHooks });
      }
    }
    if (nextEntries.length > 0) {
      config.hooks[event] = nextEntries;
    } else {
      delete config.hooks[event];
    }
  }

  return changes;
}

let changedTotal = 0;

for (const { file, required } of CONFIGS) {
  const config = readJson(file);
  const changes = [];
  changes.push(...removeForbiddenCommands(config));

  for (const { event, command, matcher } of required) {
    if (ensureCommand(config, event, command, matcher)) {
      const matcherLabel = matcher === undefined ? "" : `(${matcher})`;
      changes.push(`${event}${matcherLabel}: added ${command}`);
    }
  }

  if (changes.length === 0) {
    console.log(`[OK] ${file}: hook config already normalized`);
    continue;
  }

  changedTotal += changes.length;
  if (dryRun) {
    console.log(`[DRY-RUN] ${file}: would apply ${changes.length} hook config change(s)`);
    for (const change of changes) console.log(`  - ${change}`);
  } else {
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, `${JSON.stringify(config, null, 2)}\n`);
    console.log(`[APPLY] ${file}: applied ${changes.length} hook config change(s)`);
  }
}

if (dryRun && changedTotal > 0) {
  console.log("Run with --apply to update hook configs.");
}
