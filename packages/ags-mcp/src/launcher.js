import {
  launch as launchCore,
  mcpRuntimeArgs
} from "@agent-governance-suite/launcher";

export * from "@agent-governance-suite/launcher";

export async function launch(options = {}) {
  const args = options.args || process.argv.slice(2);
  return launchCore({
    ...options,
    args: mcpRuntimeArgs(args),
    executableName: process.platform === "win32" ? "ags-mcp.exe" : "ags-mcp",
    label: options.label || "ags-mcp"
  });
}
