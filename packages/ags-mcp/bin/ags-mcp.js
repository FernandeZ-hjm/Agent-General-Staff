#!/usr/bin/env node

import { launch } from "../src/launcher.js";
import { fileURLToPath } from "node:url";

process.env.AGS_MAINTENANCE_LAUNCHER = fileURLToPath(import.meta.url);

launch({ args: process.argv.slice(2), label: "ags-mcp" })
  .then((exitCode) => {
    process.exitCode = exitCode;
  })
  .catch((error) => {
    process.stderr.write(`ags-mcp: ${error.message}\n`);
    process.exitCode = 1;
  });
