export function cliArgs(argv = process.argv) {
  return argv.slice(2);
}

export async function launch(options = {}) {
  const { argv, coreModule: injectedModule, ...coreOptions } = options;
  const coreModule = injectedModule || await import("@agent-governance-suite/launcher");
  const args = coreOptions.args === undefined ? cliArgs(argv) : coreOptions.args;
  return coreModule.launch({
    ...coreOptions,
    args,
    label: options.label || "ags"
  });
}
