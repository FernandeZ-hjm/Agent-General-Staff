import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import zlib from "node:zlib";
import { cliArgs, launch } from "../src/launcher.js";
import {
  launch as sharedLaunch,
  releaseMetadata,
  sha256File
} from "../../ags-launcher/src/launcher.js";

function temporaryRoot() {
  return fs.mkdtempSync(path.join(os.tmpdir(), "ags-cli-test-"));
}

function removeTemporaryRoot(root) {
  fs.rmSync(root, { recursive: true, force: true });
}

function fakeChild(exitCode = 0) {
  const child = new EventEmitter();
  child.kill = () => true;
  queueMicrotask(() => child.emit("exit", exitCode, null));
  return child;
}

function tarGz(entries) {
  const blocks = [];
  for (const entry of entries) {
    const content = Buffer.from(entry.body);
    const header = Buffer.alloc(512);
    header.write(entry.name, 0, 100, "utf8");
    header.write(`${content.length.toString(8).padStart(11, "0")}\0`, 124, 12, "ascii");
    header[156] = 0;
    header.fill(0x20, 148, 156);
    const checksum = header.reduce((sum, byte) => sum + byte, 0);
    header.write(`${checksum.toString(8).padStart(6, "0")}\0 `, 148, 8, "ascii");
    blocks.push(header, content, Buffer.alloc((512 - (content.length % 512)) % 512));
  }
  blocks.push(Buffer.alloc(1024));
  return zlib.gzipSync(Buffer.concat(blocks));
}

test("publishes the ags bin and exact shared launcher dependency version", () => {
  const packageJson = JSON.parse(
    fs.readFileSync(new URL("../package.json", import.meta.url), "utf8")
  );
  assert.equal(packageJson.version, "0.5.0");
  assert.equal(packageJson.bin.ags, "bin/ags.js");
  assert.equal(packageJson.dependencies["@agent-governance-suite/launcher"], packageJson.version);
  assert.match(
    fs.readFileSync(new URL("../bin/ags.js", import.meta.url), "utf8"),
    /^#!\/usr\/bin\/env node\n/u
  );
});

test("CLI forwards process arguments to the shared launcher core", async () => {
  let received;
  const exitCode = await launch({
    argv: ["node", "ags", "--version", "--json"],
    coreModule: {
      handleCoreMaintenanceCommand: async () => ({ handled: false }),
      launch: async (options) => {
        received = options;
        return 7;
      }
    }
  });
  assert.equal(exitCode, 7);
  assert.deepEqual(received.args, ["--version", "--json"]);
  assert.equal(received.label, "ags");
  assert.deepEqual(cliArgs(["node", "ags", "mcp", "serve"]), ["mcp", "serve"]);
});

test("CLI update recover switches through the shared recovery core without launching Rust", async () => {
  let launched = false;
  const originalWrite = process.stdout.write;
  let output = "";
  process.stdout.write = (chunk) => {
    output += String(chunk);
    return true;
  };
  try {
    const exitCode = await launch({
      argv: ["node", "ags", "update", "recover"],
      coreModule: {
        handleCoreMaintenanceCommand: async () => ({
          handled: true,
          result: { status: "recovered", version: "0.4.11" }
        }),
        launch: async () => {
          launched = true;
          return 9;
        }
      }
    });
    assert.equal(exitCode, 0);
    assert.equal(launched, false);
    assert.equal(JSON.parse(output).status, "recovered");
  } finally {
    process.stdout.write = originalWrite;
  }
});

test("CLI core update plan and apply stay in the shared launcher transaction", async () => {
  let launched = false;
  let appliedHash;
  const coreModule = {
    handleCoreMaintenanceCommand: async (args) => {
      if (args[1] === "plan") {
        return {
          handled: true,
          result: { plan_hash: "a".repeat(64), target_version: "0.4.13" }
        };
      }
      appliedHash = args[args.indexOf("--plan-hash") + 1];
      return { handled: true, result: { verified: true, active_version: "0.4.13" } };
    },
    launch: async () => {
      launched = true;
      return 9;
    }
  };
  const originalWrite = process.stdout.write;
  let output = "";
  process.stdout.write = (chunk) => {
    output += String(chunk);
    return true;
  };
  try {
    const planExit = await launch({
      argv: ["node", "ags", "update", "plan"],
      coreModule
    });
    assert.equal(planExit, 0);
    assert.equal(JSON.parse(output).target_version, "0.4.13");
    output = "";
    const applyExit = await launch({
      argv: ["node", "ags", "update", "apply", "--plan-hash", "a".repeat(64)],
      coreModule
    });
    assert.equal(applyExit, 0);
    assert.equal(appliedHash, "a".repeat(64));
    assert.equal(JSON.parse(output).verified, true);
    assert.equal(launched, false);
  } finally {
    process.stdout.write = originalWrite;
  }
});

test("MCP and CLI entrances share one verified installer and cache", async () => {
  const root = temporaryRoot();
  try {
    const metadata = releaseMetadata();
    const archive = tarGz([
      { name: "ags", body: "#!/bin/sh\n" },
      { name: "runtime/manifests/suite.yaml", body: "schema_version: 2\n" },
      { name: "runtime/manifests/skills-registry.yaml", body: "schema_version: 1\n" },
      { name: "runtime/manifests/mcp-registry.yaml", body: "schema_version: 1\n" },
      { name: "runtime/protocol/agent-task-protocol.md", body: "# AGS\n" }
    ]);
    const archivePath = path.join(root, "artifact.tar.gz");
    fs.writeFileSync(archivePath, archive);
    const archiveHash = sha256File(archivePath);
    const releaseIndex = Buffer.from(JSON.stringify({
      schema_version: "1.0-signed-release-index",
      version: metadata.version,
      channel: "stable",
      repository: "FernandeZ-hjm/Agent-General-Staff",
      tag: `v${metadata.version}`,
      commit: "a".repeat(40),
      assets: [{ name: metadata.assetName, sha256: archiveHash }]
    }));
    const calls = [];
    const fetchImpl = async (url) => {
      calls.push(url);
      if (url.endsWith("/release-index.json")) {
        return {
          status: 200,
          headers: new Headers(),
          arrayBuffer: async () => releaseIndex
        };
      }
      if (url.endsWith("/release-index.sig")) {
        return { status: 200, headers: new Headers(), arrayBuffer: async () => Buffer.from("test-signature") };
      }
      if (url.endsWith(`/${metadata.assetName}`)) {
        return { status: 200, headers: new Headers(), arrayBuffer: async () => archive };
      }
      throw new Error(`unexpected URL: ${url}`);
    };
    const spawned = [];
    const spawnImpl = (file, args) => {
      spawned.push({ file, args });
      return fakeChild();
    };
    const verifyReleaseIndex = async () => true;

    await sharedLaunch({ cacheRoot: root, metadata, fetchImpl, spawnImpl, verifyReleaseIndex, checkForUpdates: false });
    await launch({
      argv: ["node", "ags", "--version"],
      coreModule: {
        handleCoreMaintenanceCommand: async () => ({ handled: false }),
        launch: sharedLaunch
      },
      cacheRoot: root,
      metadata,
      fetchImpl,
      spawnImpl,
      verifyReleaseIndex,
      checkForUpdates: false
    });

    assert.equal(calls.length, 3);
    assert.deepEqual(spawned[1].args, ["--version"]);
  } finally {
    removeTemporaryRoot(root);
  }
});
