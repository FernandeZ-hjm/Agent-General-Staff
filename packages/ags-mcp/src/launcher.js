import {
  handleCoreMaintenanceCommand,
  launch as launchCore,
  mcpRuntimeArgs
} from "@agent-governance-suite/launcher";

export * from "@agent-governance-suite/launcher";

export async function launch(options = {}) {
  const args = options.args || process.argv.slice(2);
  const handled = await handleCoreMaintenanceCommand(args, options);
  if (handled.handled) {
    process.stdout.write(JSON.stringify(handled.result, null, 2) + "\n");
    return 0;
  }
  return launchCore({
    ...options,
    args: mcpRuntimeArgs(args),
    label: options.label || "ags-mcp"
  });
}
