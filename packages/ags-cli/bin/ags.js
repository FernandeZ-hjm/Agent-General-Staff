#!/usr/bin/env node

import { launch } from "../src/launcher.js";

launch()
  .then((exitCode) => {
    process.exitCode = exitCode;
  })
  .catch((error) => {
    process.stderr.write("ags: " + error.message + "\n");
    process.exitCode = 1;
  });
