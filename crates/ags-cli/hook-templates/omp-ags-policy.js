import { spawnSync } from "node:child_process";

// AGS policy lifecycle extension for OMP (contract v3).
// OMP has no native tool-level hook events: tool-level policy rides the MCP
// ags_decide/ags_apply channel (degraded mode, per design §7.5). This
// extension only injects the policy summary at agent start.

let startupSummary;

function policySummary() {
  const result = spawnSync("ags-policy", ["--host", "omp", "--probe"], {
    encoding: "utf8",
    timeout: 3000,
  });
  if (result.status !== 0 || !result.stdout) return "";
  try {
    const probe = JSON.parse(result.stdout);
    if (!probe.ok) return "";
    const hosts = (probe.hosts || []).map((h) => `${h.host}:${h.mode}`).join(", ");
    return `AGS policy active (contract v3). Hooked hosts: ${hosts}. Tool-level policy on OMP rides MCP ags_decide.`;
  } catch {
    return "";
  }
}

export default function agsPolicyLifecycle(pi) {
  pi.on("before_agent_start", async () => {
    if (!startupSummary) {
      startupSummary = policySummary();
    }
    if (!startupSummary) return;
    return { systemPromptAppend: `\n\n${startupSummary}` };
  });
}
