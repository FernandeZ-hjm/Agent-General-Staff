// Thin host adapter for the "omp" "host". Transport-only "lifecycle" bridge;
// all governance logic lives in the Rust ags-host executable.
import { spawnSync } from "node:child_process";

let startupContext;
const canonicalWorkspace = __AGS_CANONICAL_WORKSPACE__;

function lifecycle(event, ctx, payload = {}) {
  const contextSessionId = ctx.sessionManager?.getSessionId?.();
  const lifecyclePayload = {
    ...payload,
    session_id: contextSessionId || payload.session_id || "",
  };
  const result = spawnSync(
    "ags-host",
    [
      "lifecycle",
      "--event",
      event,
      "--host",
      "omp",
      "--workspace",
      canonicalWorkspace,
    ],
    {
      input: JSON.stringify(lifecyclePayload),
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
  });

  pi.on("session_shutdown", async (event, ctx) => {
    lifecycle("session-end", ctx, event);
  });
}
