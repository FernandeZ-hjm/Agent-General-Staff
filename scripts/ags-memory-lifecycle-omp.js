import { spawnSync } from "node:child_process";
import { homedir } from "node:os";
import { join } from "node:path";

const startScript = join(homedir(), ".agents", "scripts", "context-memory-start.py");
const closeScript = join(homedir(), ".agents", "scripts", "claude-stop-memory-capture.py");
let startupContext;
let injectOnNextPrompt = true;

function run(script, payload, timeout) {
  return spawnSync("python3", [script], {
    input: JSON.stringify(payload),
    encoding: "utf8",
    timeout,
    env: { ...process.env, AGS_MEMORY_HOST: "omp" },
  });
}

function loadContext(cwd) {
  const result = run(
    startScript,
    { hook_event_name: "SessionStart", cwd, source_host: "omp" },
    2000,
  );
  if (result.status !== 0 || !result.stdout) return undefined;
  try {
    return JSON.parse(result.stdout)?.hookSpecificOutput?.additionalContext;
  } catch {
    return undefined;
  }
}

function compactMessages(messages) {
  if (!Array.isArray(messages)) return [];
  return messages.flatMap((message) => {
    if (!message || typeof message !== "object") return [];
    const content = Array.isArray(message.content)
      ? message.content.flatMap((block) =>
          block && block.type === "text" && typeof block.text === "string"
            ? [{ type: "text", text: block.text }]
            : [],
        )
      : typeof message.content === "string"
        ? message.content
        : [];
    return [{ role: message.role, content }];
  });
}

function sessionMessages(ctx) {
  const entries = ctx.sessionManager?.getBranch?.() ?? [];
  return entries.flatMap((entry) =>
    entry?.type === "message" && entry.message ? [entry.message] : [],
  );
}

function closeSession(eventName, ctx, reason) {
  run(
    closeScript,
    {
      hook_event_name: eventName,
      source_host: "omp",
      cwd: ctx.cwd,
      session_id: ctx.sessionManager?.getSessionId?.() ?? "",
      transcript_path: ctx.sessionManager?.getSessionFile?.() ?? "",
      messages: compactMessages(sessionMessages(ctx)),
      close_reason: reason,
    },
    2500,
  );
}

export default function agsMemoryLifecycle(pi) {
  const reload = (ctx) => {
    startupContext = loadContext(ctx.cwd);
    injectOnNextPrompt = true;
  };

  pi.on("session_start", async (_event, ctx) => reload(ctx));

  pi.on("before_agent_start", async () => {
    if (!injectOnNextPrompt || !startupContext) return;
    injectOnNextPrompt = false;
    return { systemPromptAppend: `\n\n${startupContext}` };
  });

  pi.on("agent_settled", async (_event, ctx) => {
    closeSession("agent_settled", ctx, "turn-settled");
  });

  pi.on("session_shutdown", async (event, ctx) => {
    closeSession(event.type, ctx, event.reason ?? "process-shutdown");
  });
}
