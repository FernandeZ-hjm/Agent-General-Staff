#!/usr/bin/env node
// AGS Evolver Stop hook — Portable Template
//
// PUBLIC-SAFE: no real token, node_secret, API key, absolute $HOME path,
// task archive path, or memory capsule path.
//
// Install: copy to .claude/hooks/evolver-session-end.js, then configure
// the output path via EVOLVER_METHOD_LOG environment variable or edit the
// outputPath() function below.

const fs = require('fs');
const os = require('os');
const path = require('path');
const { spawnSync } = require('child_process');

const MAX_BUFFER = 5 * 1024 * 1024;

function readStdin(callback) {
  let input = '';
  process.stdin.setEncoding('utf8');
  process.stdin.on('data', (chunk) => {
    input += chunk;
  });
  process.stdin.on('end', () => callback(input));
}

function parseJson(text) {
  if (!text.trim()) return {};
  try {
    return JSON.parse(text);
  } catch {
    return {};
  }
}

function runGit(args, cwd) {
  const result = spawnSync('git', args, {
    cwd,
    encoding: 'utf8',
    timeout: 5000,
    maxBuffer: MAX_BUFFER,
    stdio: ['ignore', 'pipe', 'pipe'],
    shell: false,
  });

  return {
    ok: result.status === 0,
    out: typeof result.stdout === 'string' ? result.stdout.trim() : '',
  };
}

function resolveProjectDir(input) {
  const candidates = [
    input.cwd,
    input.workspace_dir,
    input.workspaceDir,
    process.env.AGS_PROJECT_DIR,
    process.cwd(),
  ];

  for (const candidate of candidates) {
    if (candidate && fs.existsSync(candidate)) {
      return path.resolve(candidate);
    }
  }

  return process.cwd();
}

function projectSlug(projectDir) {
  return path.basename(projectDir);
}

function defaultMemoryDir(projectDir) {
  // Portable: uses $HOME/.agents/memory/<project-slug> by default.
  // Override with AGS_MEMORY_DIR env var.
  const home = os.homedir();
  return path.join(home, '.agents', 'memory', 'projects', projectSlug(projectDir));
}

function readFileIfExists(filePath) {
  try {
    if (filePath && fs.existsSync(filePath) && fs.statSync(filePath).isFile()) {
      return fs.readFileSync(filePath, 'utf8');
    }
  } catch {
    return '';
  }
  return '';
}

function transcriptText(input) {
  return readFileIfExists(input.transcript_path || input.transcriptPath);
}

function latestFile(root, fileName) {
  const matches = [];

  function walk(dir, depth) {
    if (depth > 3) return;
    let entries = [];
    try {
      entries = fs.readdirSync(dir, { withFileTypes: true });
    } catch {
      return;
    }

    for (const entry of entries) {
      const fullPath = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        walk(fullPath, depth + 1);
      } else if (entry.isFile() && entry.name === fileName) {
        try {
          matches.push({ path: fullPath, mtimeMs: fs.statSync(fullPath).mtimeMs });
        } catch {
          // Ignore unreadable candidates.
        }
      }
    }
  }

  walk(root, 0);
  matches.sort((a, b) => b.mtimeMs - a.mtimeMs);
  return matches.length > 0 ? matches[0].path : '';
}

// Resolve a task identifier from hook input.  Evidence from the task
// archive is ONLY used when an explicit task_id (or compatible session
// id) is provided.  Without it, the hook cannot safely bind archive
// evidence to the current run and falls back to git-diff-only capture.
//
// task_id minimum length: 8 chars (short ids are too collision-prone).
// Format: alphanumeric + hyphen/underscore.
function resolveTaskId(input) {
  const raw = (
    input.task_id
    || input.taskId
    || input.archive_session_id
    || input.archiveSessionId
    || ''
  ).trim();
  // Reject ids that are too short or contain suspicious characters.
  if (raw.length < 8) return '';
  if (!/^[a-zA-Z0-9][a-zA-Z0-9_-]{6,}[a-zA-Z0-9]$/.test(raw)) return '';
  return raw;
}

function taskArchiveEvidence(projectDir, taskId) {
  if (!taskId) return null;

  const memoryDir = process.env.AGS_MEMORY_DIR || defaultMemoryDir(projectDir);
  // Look for evidence files matching the explicit task id prefix.
  // This avoids picking up another task's evidence under concurrent runs.
  const archiveRoot = path.join(memoryDir, 'task-archive');
  const candidates = [];

  function walk(dir, depth) {
    if (depth > 3) return;
    let entries = [];
    try {
      entries = fs.readdirSync(dir, { withFileTypes: true });
    } catch { return; }
    for (const entry of entries) {
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        // STRICT match only: directory name must equal taskId exactly,
        // or start with `${taskId}-` (taskId + hyphen suffix).  Weak
        // prefix/includes matching would risk picking up another task's
        // archive under concurrent runs.
        if (entry.name === taskId || entry.name.startsWith(taskId + '-')) {
          walk(full, depth + 1);
        }
      } else if (entry.isFile()) {
        candidates.push({ path: full, name: entry.name,
          mtimeMs: (() => { try { return fs.statSync(full).mtimeMs; } catch { return 0; }})() });
      }
    }
  }
  walk(archiveRoot, 0);
  if (candidates.length === 0) return null;

  candidates.sort((a, b) => b.mtimeMs - a.mtimeMs);

  const delivery = candidates.find(c => c.name === 'delivery-report.md');
  if (delivery) return { tier: 'delivery_report',
    task_id: taskId, text: readFileIfExists(delivery.path) };

  const receipt = candidates.find(c => c.name === 'receipt.json');
  if (receipt) return { tier: 'receipt',
    task_id: taskId, text: readFileIfExists(receipt.path) };

  const verification = candidates.find(c => c.name === 'verification.log');
  if (verification) return { tier: 'verification_result',
    task_id: taskId, text: readFileIfExists(verification.path) };

  return null;
}

function transcriptEvidence(input) {
  const text = transcriptText(input);
  if (!text) return null;

  // Do NOT store the absolute transcript_path in the evidence struct —
  // it exposes local filesystem layout to Evolver method events.
  if (text.includes('# 任务交付报告') || text.includes('## 任务状态')) {
    return { tier: 'delivery_report', task_id: resolveTaskId(input) || 'transcript', text };
  }
  if (/receipt/i.test(text) && /verification/i.test(text)) {
    return { tier: 'receipt', task_id: resolveTaskId(input) || 'transcript', text };
  }
  if (/verification|验证结果|已运行/i.test(text)) {
    return { tier: 'verification_result', task_id: resolveTaskId(input) || 'transcript', text };
  }

  return null;
}

function gitDiffEvidence(projectDir) {
  const stat = runGit(['diff', '--stat'], projectDir);
  const diff = runGit(['diff', '--no-color'], projectDir);
  const isRepo = runGit(['rev-parse', '--is-inside-work-tree'], projectDir).out === 'true';

  if (!isRepo || !stat.out) {
    return {
      tier: 'fallback_observation',
      status: 'observed',
      summary: isRepo ? 'no git diff signal' : 'not a git workspace',
      signals: [],
    };
  }

  return {
    tier: 'git_diff_signal',
    status: 'observed',
    summary: summarizeDiffStat(stat.out),
    signals: detectMethodSignals(diff.out),
  };
}

function summarizeDiffStat(stat) {
  const files = (stat.match(/\d+ files? changed/) || ['changes observed'])[0];
  const insertions = (stat.match(/(\d+) insertions?/) || [null, '0'])[1];
  const deletions = (stat.match(/(\d+) deletions?/) || [null, '0'])[1];
  return `${files}, +${insertions}/-${deletions}`;
}

function reportStatus(text) {
  const match = text.match(/##\s*任务状态\s*\n+([^\n]+)/);
  if (!match) return 'reported';
  const line = match[1].trim();
  if (line.includes('未完成')) return 'incomplete';
  if (line.includes('部分完成')) return 'partial';
  if (line.includes('完成')) return 'completed';
  return 'reported';
}

function detectMethodSignals(text) {
  const signals = [];
  const lower = text.toLowerCase();

  if (/protocol|协议/.test(lower)) signals.push('protocol-boundary-update');
  if (/memory|capsule|archive|记忆|胶囊|归档/.test(lower)) signals.push('memory-boundary-update');
  if (/hook|stop|session-end/.test(lower)) signals.push('hook-outcome-hardening');
  if (/verification|verify|测试|验证/.test(lower)) signals.push('verification-evidence');
  if (/permission|risk|gate|权限|门禁|风险/.test(lower)) signals.push('authority-gate-protection');
  if (/fallback|observed|advisory/.test(lower)) signals.push('advisory-fallback');

  return [...new Set(signals)];
}

function methodEventFromEvidence(evidence, projectDir) {
  const text = evidence.text || '';
  const signals = detectMethodSignals(text);
  const status = evidence.tier === 'delivery_report' ? reportStatus(text) : 'reported';

  // NEVER persist absolute filesystem paths in Evolver events.
  // Store only evidence_tier plus an opaque task_id / session reference;
  // project facts and file paths remain in AGS memory.
  return {
    schema_version: 'ags-evolution-memory/1',
    timestamp: new Date().toISOString(),
    project_slug: projectSlug(projectDir),
    source: 'hook:evolver-session-end',
    evidence_tier: evidence.tier,
    reference_id: evidence.task_id || '',
    evidence_path: '',  // intentionally empty — paths stay in AGS memory
    outcome: {
      status,
      note: 'method capture derived from AGS evidence; project facts remain in AGS memory',
    },
    method: {
      signals,
      reusable_note: signals.length > 0
        ? `Reusable method signals: ${signals.join(', ')}`
        : 'Reusable method capture requires human or Evolver review.',
    },
  };
}

function methodEventFromGit(evidence, projectDir) {
  return {
    schema_version: 'ags-evolution-memory/1',
    timestamp: new Date().toISOString(),
    project_slug: projectSlug(projectDir),
    source: 'hook:evolver-session-end',
    evidence_tier: evidence.tier,
    evidence_path: '',
    outcome: {
      status: 'observed',
      note: evidence.summary,
    },
    method: {
      signals: evidence.signals,
      reusable_note: 'Observed only; no delivery report, receipt, or verification result was available.',
    },
  };
}

function outputPath() {
  // Portable default: if EVOLVER_METHOD_LOG is set, use it.
  // Otherwise log to a file under the project's .claude/ directory.
  // Override this in your installed copy to match your EvoMap setup.
  if (process.env.EVOLVER_METHOD_LOG) {
    return process.env.EVOLVER_METHOD_LOG;
  }
  return path.join(
    os.homedir(),
    '.evolver', 'logs', 'ags-method-events.jsonl'
  );
}

function appendEvent(event) {
  const target = outputPath();
  fs.mkdirSync(path.dirname(target), { recursive: true });
  fs.appendFileSync(target, `${JSON.stringify(event)}\n`, 'utf8');
  return target;
}

function finish(payload) {
  process.stdout.write(JSON.stringify(payload || {}));
}

readStdin((rawInput) => {
  const input = parseJson(rawInput);
  const projectDir = resolveProjectDir(input);
  const taskId = resolveTaskId(input);

  try {
    // Evidence priority: transcript (if classifiable) → task-specific
    // archive (only with explicit task_id) → git diff.
    // Without an explicit task_id, archive evidence is skipped entirely
    // to prevent recording another task's outcome as this task's evidence.
    const evidence = transcriptEvidence(input)
      || (taskId ? taskArchiveEvidence(projectDir, taskId) : null);
    const event = evidence
      ? methodEventFromEvidence(evidence, projectDir)
      : methodEventFromGit(gitDiffEvidence(projectDir), projectDir);
    const target = appendEvent(event);

    finish({
      systemMessage:
        `[Evolution] Method capture recorded (${event.evidence_tier}, ${event.outcome.status}) to ${target}.`,
    });
  } catch {
    finish({});
  }
});
