import { spawnSync } from "node:child_process";

let startupContext;

function lifecycle(event, ctx, payload = {}) {
  const result = spawnSync(
    "ags",
    [
      "host",
      "lifecycle",
      "--event",
      event,
      "--host",
      "omp",
      "--target",
      ctx.cwd,
    ],
    {
      input: JSON.stringify(payload),
      encoding: "utf8",
      timeout: 3000,
    },
  );
  if (result.status !== 0 || !result.stdout) return undefined;
  try {
    return JSON.parse(result.stdout);
  } catch {
    return undefined;
  }
}

export default function agsMemoryLifecycle(pi) {
  pi.on("session_start", async (event, ctx) => {
    startupContext = lifecycle("session-start", ctx, event)
      ?.hookSpecificOutput?.additionalContext;
  });

  pi.on("before_agent_start", async () => {
    if (!startupContext) return;
    const context = startupContext;
    startupContext = undefined;
    return { systemPromptAppend: `\n\n${context}` };
  });

  pi.on("agent_settled", async (event, ctx) => {
    const guardContext = lifecycle("stop-guard", ctx, event)
      ?.hookSpecificOutput?.additionalContext;
    if (guardContext) startupContext = guardContext;
    lifecycle("session-end", ctx, {
      ...event,
      session_id: ctx.sessionManager?.getSessionId?.() ?? "",
    });
  });

  pi.on("session_shutdown", async (event, ctx) => {
    lifecycle("session-end", ctx, {
      ...event,
      session_id: ctx.sessionManager?.getSessionId?.() ?? "",
    });
  });
}
