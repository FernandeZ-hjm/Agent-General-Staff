#!/usr/bin/env node

import { launch } from "../src/launcher.js";

launch().catch((error) => {
  process.stderr.write(`ags-mcp: ${error.message}\n`);
  process.exitCode = 1;
});
