export function cliArgs(argv = process.argv) {
  return argv.slice(2);
}

export async function launch(options = {}) {
  const { argv, coreModule: injectedModule, ...coreOptions } = options;
  const coreModule = injectedModule || await import("@agent-governance-suite/launcher");
  const args = coreOptions.args === undefined ? cliArgs(argv) : coreOptions.args;
  const handled = await coreModule.handleCoreMaintenanceCommand(args, coreOptions);
  if (handled.handled) {
    process.stdout.write(JSON.stringify(handled.result, null, 2) + "\n");
    return 0;
  }
  return coreModule.launch({
    ...coreOptions,
    args,
    label: options.label || "ags"
  });
}
