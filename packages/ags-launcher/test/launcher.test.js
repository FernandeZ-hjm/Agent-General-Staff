import assert from "node:assert/strict";
import crypto from "node:crypto";
import { EventEmitter } from "node:events";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import zlib from "node:zlib";
import {
  MCP_ARGS,
  cachePaths,
  download,
  extractArchive,
  handleCoreMaintenanceCommand,
  applyUpdate,
  launch,
  maybeCheckForUpdate,
  planUpdate,
  releaseMetadata,
  sha256File,
  statusUpdate,
  verifyUpdate
} from "../src/launcher.js";

function temporaryRoot() {
  return fs.mkdtempSync(path.join(os.tmpdir(), "ags-launcher-test-"));
}

function removeTemporaryRoot(root) {
  fs.rmSync(root, { recursive: true, force: true });
}

function response(body, status = 200, headers = {}) {
  return {
    status,
    headers: new Headers(headers),
    arrayBuffer: async () => Buffer.from(body)
  };
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
    header.write(String(content.length.toString(8).padStart(11, "0")) + "\0", 124, 12, "ascii");
    header[156] = entry.type ? String(entry.type).charCodeAt(0) : 0;
    header.fill(0x20, 148, 156);
    const checksum = header.reduce((sum, byte) => sum + byte, 0);
    header.write(String(checksum.toString(8).padStart(6, "0")) + "\0 ", 148, 8, "ascii");
    blocks.push(header, content, Buffer.alloc((512 - (content.length % 512)) % 512));
  }
  blocks.push(Buffer.alloc(1024));
  return zlib.gzipSync(Buffer.concat(blocks));
}

function v3RuntimeEntries(label = "public v3 runtime") {
  return ["ags-agent", "ags-doctor", "ags-govern", "ags-init", "ags-setup"].flatMap((id) => [
    { name: `runtime/ags-skills/${id}/SKILL.md`, body: `---\nname: ${id}\ndescription: ${label}\n---\n` },
    { name: `runtime/ags-skills/${id}/agents/openai.yaml`, body: `interface:\n  display_name: ${id}\n` }
  ]);
}

function releaseArchive(binaryName = "ags") {
  const suffix = binaryName.endsWith(".exe") ? ".exe" : "";
  return tarGz([
    { name: binaryName, body: "#!/bin/sh\nexit 0\n" },
    { name: `ags-mcp${suffix}`, body: "#!/bin/sh\nexit 0\n" },
    { name: `ags-host${suffix}`, body: "#!/bin/sh\nexit 0\n" },
    { name: `ags-policy${suffix}`, body: "#!/bin/sh\nexit 0\n" },
    { name: `ags-release${suffix}`, body: "#!/bin/sh\nexit 0\n" },
    ...v3RuntimeEntries(),
    { name: "runtime/extra.txt", body: "content identity\n" }
  ]);
}

function releaseFetcher(metadata, archive, calls) {
  const checksum = sha256Buffer(archive);
  const index = Buffer.from(JSON.stringify({
    schema_version: "1.0-signed-release-index",
    version: metadata.version,
    channel: "stable",
    repository: "FernandeZ-hjm/Agent-General-Staff",
    tag: `v${metadata.version}`,
    commit: "a".repeat(40),
    assets: [{ name: metadata.assetName, sha256: checksum }]
  }));
  return async (url) => {
    calls.push(url);
    if (url.endsWith("/release-index.json")) return response(index);
    if (url.endsWith("/release-index.sig")) return response("test-signature");
    if (url.endsWith("/" + metadata.assetName)) return response(archive);
    throw new Error("unexpected test URL: " + url);
  };
}

function signedIndex(version = "0.4.20") {
  return Buffer.from(JSON.stringify({
    schema_version: "1.0-signed-release-index",
    version,
    channel: "stable",
    repository: "FernandeZ-hjm/Agent-General-Staff",
    tag: `v${version}`,
    commit: "a".repeat(40),
    assets: [{ name: `ags-v${version}-test.tar.gz`, sha256: "b".repeat(64) }]
  }));
}

function zipArchive(entries) {
  const locals = [];
  const centrals = [];
  let offset = 0;
  for (const entry of entries) {
    const body = Buffer.from(entry.body ?? "");
    const localName = Buffer.from(entry.localName ?? entry.name, "utf8");
    const centralName = Buffer.from(entry.name, "utf8");
    const local = Buffer.alloc(30 + localName.length + body.length);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(entry.flags ?? 0, 6);
    local.writeUInt16LE(entry.method ?? 0, 8);
    local.writeUInt32LE(body.length, 18);
    local.writeUInt32LE(body.length, 22);
    local.writeUInt16LE(localName.length, 26);
    localName.copy(local, 30);
    body.copy(local, 30 + localName.length);
    locals.push(local);

    const central = Buffer.alloc(46 + centralName.length);
    central.writeUInt32LE(0x02014b50, 0);
    central.writeUInt16LE(entry.madeBy ?? 20, 4);
    central.writeUInt16LE(entry.flags ?? 0, 8);
    central.writeUInt16LE(entry.method ?? 0, 10);
    central.writeUInt32LE(body.length, 20);
    central.writeUInt32LE(body.length, 24);
    central.writeUInt16LE(centralName.length, 28);
    central.writeUInt32LE(entry.externalAttributes ?? 0, 38);
    central.writeUInt32LE(offset, 42);
    centralName.copy(central, 46);
    centrals.push(central);
    offset += local.length;
  }
  const centralDirectory = Buffer.concat(centrals);
  const eocd = Buffer.alloc(22);
  eocd.writeUInt32LE(0x06054b50, 0);
  eocd.writeUInt16LE(entries.length, 8);
  eocd.writeUInt16LE(entries.length, 10);
  eocd.writeUInt32LE(centralDirectory.length, 12);
  eocd.writeUInt32LE(offset, 16);
  return Buffer.concat([...locals, centralDirectory, eocd]);
}

function sha256Buffer(value) {
  return crypto.createHash("sha256").update(value).digest("hex");
}

test("standalone adapters and CLI share one immutable verified download", async () => {
  const root = temporaryRoot();
  try {
    const metadata = releaseMetadata();
    const archive = releaseArchive(metadata.platform === "win32" ? "ags.exe" : "ags");
    const calls = [];
    const spawned = [];
    const fetchImpl = releaseFetcher(metadata, archive, calls);
    const spawnImpl = (file, args, options) => {
      spawned.push({ file, args, options });
      return fakeChild();
    };

    const verifyReleaseIndex = async () => true;
    const adapterName = metadata.platform === "win32" ? "ags-mcp.exe" : "ags-mcp";
    await launch({ cacheRoot: root, metadata, executableName: adapterName, fetchImpl, spawnImpl, verifyReleaseIndex, checkForUpdates: false });
    await launch({
      cacheRoot: root,
      metadata,
      args: ["--version", "--json"],
      fetchImpl,
      spawnImpl,
      verifyReleaseIndex,
      checkForUpdates: false
    });
    await launch({
      cacheRoot: root,
      metadata,
      args: ["setup"],
      fetchImpl,
      spawnImpl,
      verifyReleaseIndex,
      checkForUpdates: false
    });

    assert.equal(calls.length, 3, "signed index, signature and artifact are downloaded once");
    assert.deepEqual(spawned[0].args, MCP_ARGS);
    assert.equal(path.basename(spawned[0].file), adapterName);
    assert.deepEqual(spawned[1].args, ["--version", "--json"]);
    assert.deepEqual(spawned[2].args, [
      "setup",
      "--source-root",
      path.dirname(cachePaths(root, metadata).binaryPath)
    ]);
    assert.equal(spawned[1].options.env.AGS_SOURCE_ROOT, undefined);
    assert.equal(spawned[0].options.shell, false);
    const paths = cachePaths(root, metadata);
    assert.equal(paths.versionDir, path.join(root, "versions", metadata.version, metadata.triple));
    assert.ok(fs.existsSync(paths.binaryPath));
    assert.ok(fs.existsSync(paths.runtimeRoot));
    const current = JSON.parse(fs.readFileSync(paths.currentPath, "utf8"));
    assert.equal(current.version, metadata.version);
    assert.equal(current.asset_sha256, sha256Buffer(archive));
    assert.equal(current.binary_sha256, sha256File(paths.binaryPath));
    assert.match(current.release_index_sha256, /^[a-f0-9]{64}$/u);
    assert.match(fs.readFileSync(paths.markerPath, "utf8"), /\n[a-f0-9]{64}\n$/u);
    const marker = fs.readFileSync(paths.markerPath, "utf8").trim().split(/\r?\n/u);
    fs.writeFileSync(paths.currentPath, `${JSON.stringify({
      ...current,
      executables_sha256: marker[3],
      runtime_root: paths.runtimeRoot,
      activated_at_unix: 1_787_000_000
    }, null, 2)}\n`);
    await launch({
      cacheRoot: root,
      metadata,
      fetchImpl: () => { throw new Error("Rust pointer must reuse verified cache"); },
      spawnImpl,
      verifyReleaseIndex,
      checkForUpdates: false
    });
    assert.equal(calls.length, 3, "Rust pointer fields remain Node-compatible");
  } finally {
    removeTemporaryRoot(root);
  }
});

test("switching versions writes previous pointer and never overwrites old content", async () => {
  const root = temporaryRoot();
  try {
    const first = releaseMetadata({ version: "0.4.12" });
    const second = releaseMetadata({ version: "0.4.20" });
    const firstArchive = releaseArchive();
    const secondArchive = tarGz([
      { name: "ags", body: "second binary\n" },
      { name: "ags-mcp", body: "second mcp\n" },
      { name: "ags-host", body: "second host\n" },
      { name: "ags-policy", body: "second policy\n" },
      { name: "ags-release", body: "second release\n" },
      ...v3RuntimeEntries("second runtime")
    ]);
    const calls = [];
    const fetchImpl = async (url, options) => {
      const metadata = url.includes("v0.4.20") ? second : first;
      const archive = metadata === second ? secondArchive : firstArchive;
      return releaseFetcher(metadata, archive, calls)(url, options);
    };
    const spawnImpl = () => fakeChild();

    const verifyReleaseIndex = async () => true;
    await launch({ cacheRoot: root, metadata: first, fetchImpl, spawnImpl, verifyReleaseIndex, checkForUpdates: false });
    const firstPath = cachePaths(root, first);
    const firstHash = sha256File(firstPath.binaryPath);
    await launch({ cacheRoot: root, metadata: second, fetchImpl, spawnImpl, verifyReleaseIndex, checkForUpdates: false });

    const secondPath = cachePaths(root, second);
    const current = JSON.parse(fs.readFileSync(secondPath.currentPath, "utf8"));
    const previous = JSON.parse(fs.readFileSync(secondPath.previousPath, "utf8"));
    assert.equal(current.version, "0.4.20");
    assert.equal(previous.version, "0.4.12");
    assert.equal(sha256File(firstPath.binaryPath), firstHash);
    assert.notEqual(fs.realpathSync(firstPath.versionDir), fs.realpathSync(secondPath.versionDir));
    assert.equal(calls.length, 6);
  } finally {
    removeTemporaryRoot(root);
  }
});

test("tampered current pointer and marker fail closed", async () => {
  const root = temporaryRoot();
  try {
    const metadata = releaseMetadata();
    const archive = releaseArchive();
    const calls = [];
    const fetchImpl = releaseFetcher(metadata, archive, calls);
    const spawnImpl = () => fakeChild();
    const verifyReleaseIndex = async () => true;
    await launch({ cacheRoot: root, metadata, fetchImpl, spawnImpl, verifyReleaseIndex, checkForUpdates: false });
    const paths = cachePaths(root, metadata);

    const current = JSON.parse(fs.readFileSync(paths.currentPath, "utf8"));
    current.asset_name = "ags-v9.9.9-" + metadata.triple + "." + metadata.extension;
    fs.writeFileSync(paths.currentPath, JSON.stringify(current));
    await assert.rejects(
      () => launch({
        cacheRoot: root,
        metadata,
        fetchImpl: () => { throw new Error("must not fetch"); },
        spawnImpl,
        verifyReleaseIndex,
        checkForUpdates: false
      }),
      /current launcher pointer/u
    );

    current.asset_name = metadata.assetName;
    fs.writeFileSync(paths.currentPath, JSON.stringify(current));
    const mcpName = metadata.platform === "win32" ? "ags-mcp.exe" : "ags-mcp";
    const mcpBinary = path.join(paths.versionDir, mcpName);
    const mcpBytes = fs.readFileSync(mcpBinary);
    fs.appendFileSync(mcpBinary, "tampered");
    await assert.rejects(
      () => launch({
        cacheRoot: root,
        metadata,
        executableName: mcpName,
        fetchImpl: () => { throw new Error("must not fetch"); },
        spawnImpl,
        verifyReleaseIndex,
        checkForUpdates: false
      }),
      /current launcher pointer/u
    );
    fs.writeFileSync(mcpBinary, mcpBytes);
    const marker = fs.readFileSync(paths.markerPath, "utf8").split(/\r?\n/u);
    marker[2] = "0".repeat(64);
    fs.writeFileSync(paths.markerPath, marker.join("\n"));
    await assert.rejects(
      () => launch({
        cacheRoot: root,
        metadata,
        fetchImpl: () => { throw new Error("must not fetch"); },
        spawnImpl,
        verifyReleaseIndex,
        checkForUpdates: false
      }),
      /current launcher pointer/u
    );
    assert.equal(calls.length, 3);
  } finally {
    removeTemporaryRoot(root);
  }
});

test("archive traversal is rejected before extraction", async () => {
  const root = temporaryRoot();
  const archivePath = path.join(root, "bad.tar.gz");
  try {
    fs.writeFileSync(archivePath, tarGz([{ name: "runtime/../../escape", body: "nope" }]));
    await assert.rejects(
      () => extractArchive(archivePath, "tar.gz", root, "ags"),
      /unsafe archive path/u
    );
    assert.equal(fs.existsSync(path.join(root, "escape")), false);
  } finally {
    removeTemporaryRoot(root);
  }
});

test("release installation rejects an index that fails the pinned signature", async () => {
  const root = temporaryRoot();
  try {
    const metadata = releaseMetadata();
    const archive = releaseArchive();
    const calls = [];
    const fetchImpl = releaseFetcher(metadata, archive, calls);
    await assert.rejects(
      () => launch({
        cacheRoot: root,
        metadata,
        fetchImpl,
        spawnImpl: () => fakeChild(),
        checkForUpdates: false
      }),
      /release signature verification failed/u
    );
    const paths = cachePaths(root, metadata);
    assert.equal(fs.existsSync(paths.currentPath), false);
    assert.equal(fs.existsSync(paths.versionDir), false);
    await assert.rejects(
      () => launch({
        cacheRoot: root,
        metadata,
        fetchImpl,
        verifyReleaseIndex: async () => false,
        spawnImpl: () => fakeChild(),
        checkForUpdates: false
      }),
      /release signature verification failed/u
    );
  } finally {
    removeTemporaryRoot(root);
  }
});

test("explicitly recovers a verified previous pointer when current is invalid", async () => {
  const root = temporaryRoot();
  try {
    const first = releaseMetadata({ version: "0.4.12" });
    const second = releaseMetadata({ version: "0.4.20" });
    const firstArchive = releaseArchive();
    const secondArchive = tarGz([
      { name: "ags", body: "second binary\n" },
      { name: "ags-mcp", body: "second mcp\n" },
      { name: "ags-host", body: "second host\n" },
      { name: "ags-policy", body: "second policy\n" },
      { name: "ags-release", body: "second release\n" },
      ...v3RuntimeEntries("second runtime")
    ]);
    const calls = [];
    const fetchImpl = async (url, options) => {
      const metadata = url.includes("v0.4.20") ? second : first;
      const archive = metadata === second ? secondArchive : firstArchive;
      return releaseFetcher(metadata, archive, calls)(url, options);
    };
    const verifyReleaseIndex = async () => true;
    const spawned = [];
    const spawnImpl = (file) => {
      spawned.push(file);
      return fakeChild();
    };
    await launch({ cacheRoot: root, metadata: first, fetchImpl, spawnImpl, verifyReleaseIndex, checkForUpdates: false });
    await launch({ cacheRoot: root, metadata: second, fetchImpl, spawnImpl, verifyReleaseIndex, checkForUpdates: false });
    const paths = cachePaths(root, second);
    const current = JSON.parse(fs.readFileSync(paths.currentPath, "utf8"));
    current.binary_sha256 = "0".repeat(64);
    fs.writeFileSync(paths.currentPath, JSON.stringify(current));

    await assert.rejects(
      () => launch({
        cacheRoot: root,
        metadata: second,
        fetchImpl: () => { throw new Error("must not fetch an invalid current pointer"); },
        spawnImpl,
        verifyReleaseIndex,
        checkForUpdates: false
      }),
      /current launcher pointer/u
    );
    await launch({
      cacheRoot: root,
      metadata: second,
      fetchImpl: () => { throw new Error("must not fetch during explicit recovery"); },
      spawnImpl,
      verifyReleaseIndex,
      recoverFromPrevious: true,
      checkForUpdates: false
    });
    const recovered = JSON.parse(fs.readFileSync(paths.currentPath, "utf8"));
    assert.equal(recovered.version, first.version);
    assert.equal(spawned.at(-1), cachePaths(root, first).binaryPath);
    assert.equal(calls.length, 6);
  } finally {
    removeTemporaryRoot(root);
  }
});

test("older package invocation keeps a newer compatible current pointer", async () => {
  const root = temporaryRoot();
  try {
    const older = releaseMetadata({ version: "0.4.12" });
    const newer = releaseMetadata({ version: "0.4.20" });
    const olderArchive = releaseArchive();
    const newerArchive = tarGz([
      { name: "ags", body: "newer binary\n" },
      { name: "ags-mcp", body: "newer mcp\n" },
      { name: "ags-host", body: "newer host\n" },
      { name: "ags-policy", body: "newer policy\n" },
      { name: "ags-release", body: "newer release\n" },
      ...v3RuntimeEntries("newer runtime")
    ]);
    const calls = [];
    const fetchImpl = async (url, options) => {
      const metadata = url.includes("v0.4.20") ? newer : older;
      const archive = metadata === newer ? newerArchive : olderArchive;
      return releaseFetcher(metadata, archive, calls)(url, options);
    };
    const verifyReleaseIndex = async () => true;
    const spawned = [];
    const spawnImpl = (file) => {
      spawned.push(file);
      return fakeChild();
    };
    await launch({ cacheRoot: root, metadata: newer, fetchImpl, spawnImpl, verifyReleaseIndex, checkForUpdates: false });
    const currentBefore = fs.readFileSync(cachePaths(root, newer).currentPath, "utf8");
    await launch({
      cacheRoot: root,
      metadata: older,
      fetchImpl: () => { throw new Error("older package must not download"); },
      spawnImpl,
      checkForUpdates: false
    });
    assert.equal(spawned.at(-1), cachePaths(root, newer).binaryPath);
    assert.equal(fs.readFileSync(cachePaths(root, newer).currentPath, "utf8"), currentBefore);
    assert.equal(calls.length, 3);
  } finally {
    removeTemporaryRoot(root);
  }
});

test("launcher never intercepts core maintenance and legacy helpers hard-cut to Rust", async () => {
  assert.deepEqual(await handleCoreMaintenanceCommand(["update", "plan"]), { handled: false });
  await assert.rejects(() => planUpdate(), /moved to the Rust contract/u);
  await assert.rejects(() => applyUpdate(), /moved to the Rust contract/u);
  assert.throws(() => statusUpdate(), /moved to the Rust contract/u);
  await assert.rejects(() => verifyUpdate(), /moved to the Rust contract/u);
  assert.deepEqual(await maybeCheckForUpdate(), { checked: false, skipped: "rust-owned" });
});

test("tar and zip unsafe, symlink, special, and mismatched entries hard-fail", async () => {
  const root = temporaryRoot();
  try {
    const tarCases = [
      { name: "other/file", body: "outside" },
      { name: "runtime/link", body: "target", type: "2" },
      { name: "runtime/device", body: "", type: "3" }
    ];
    for (const [index, entry] of tarCases.entries()) {
      const archivePath = path.join(root, `bad-${index}.tar.gz`);
      fs.writeFileSync(archivePath, tarGz([entry]));
      await assert.rejects(
        () => extractArchive(archivePath, "tar.gz", path.join(root, `tar-${index}`), "ags"),
        /unsafe archive path|non-regular payload/u
      );
    }

    const zipCases = [
      {
        name: "../escape",
        body: "outside"
      },
      {
        name: "runtime/link",
        body: "target",
        madeBy: 0x0314,
        externalAttributes: (0o120777 << 16) >>> 0
      },
      {
        name: "runtime/device",
        body: "",
        madeBy: 0x0314,
        externalAttributes: (0o010644 << 16) >>> 0
      }
    ];
    for (const [index, entry] of zipCases.entries()) {
      const archivePath = path.join(root, `bad-${index}.zip`);
      fs.writeFileSync(archivePath, zipArchive([entry]));
      await assert.rejects(
        () => extractArchive(archivePath, "zip", path.join(root, `zip-${index}`), "ags"),
        /unsafe archive path|non-regular payload|special|symlink/u
      );
    }

    const mismatchPath = path.join(root, "mismatch.zip");
    fs.writeFileSync(mismatchPath, zipArchive([{ name: "ags", localName: "runtime/other", body: "bad" }]));
    await assert.rejects(
      () => extractArchive(mismatchPath, "zip", path.join(root, "mismatch"), "ags"),
      /local\/central entry identity mismatch/u
    );

    const validTarRoot = path.join(root, "valid-tar");
    const validTarPath = path.join(root, "valid.tar.gz");
    fs.writeFileSync(validTarPath, tarGz([
      { name: "ags", body: "#!/bin/sh\n" },
      { name: "runtime/", body: "", type: "5" },
      { name: "runtime/manifests/", body: "", type: "5" },
      { name: "runtime/manifests/suite.yaml", body: "schema_version: 2\n" }
    ]));
    await extractArchive(validTarPath, "tar.gz", validTarRoot, "ags");
    assert.equal(fs.readFileSync(path.join(validTarRoot, "ags"), "utf8"), "#!/bin/sh\n");
    assert.equal(fs.existsSync(path.join(validTarRoot, "runtime/manifests")), true);

    const validZipRoot = path.join(root, "valid-zip");
    const validZipPath = path.join(root, "valid.zip");
    fs.writeFileSync(validZipPath, zipArchive([
      { name: "ags", body: "#!/bin/sh\n" },
      { name: "runtime/", body: "" },
      { name: "runtime/manifests/suite.yaml", body: "schema_version: 2\n" },
      { name: "runtime/manifests/skills-registry.yaml", body: "schema_version: 1\n" },
      { name: "runtime/manifests/mcp-registry.yaml", body: "schema_version: 1\n" },
      { name: "runtime/protocol/agent-task-protocol.md", body: "# AGS\n" }
    ]));
    await extractArchive(validZipPath, "zip", validZipRoot, "ags");
    assert.equal(fs.readFileSync(path.join(validZipRoot, "ags"), "utf8"), "#!/bin/sh\n");
    assert.equal(fs.existsSync(path.join(validZipRoot, "runtime")), true);
  } finally {
    removeTemporaryRoot(root);
  }
});

test("download follows bounded redirects and enforces size", async () => {
  const root = temporaryRoot();
  try {
    const destination = path.join(root, "asset");
    let calls = 0;
    await download("https://github.com/example/repo/releases/download/v0.4.12/start", destination, {
      fetchImpl: async (url) => {
        calls += 1;
        return url.endsWith("/start")
          ? { status: 302, headers: new Headers({ location: "/asset" }) }
          : response("ok");
      }
    });
    assert.equal(calls, 2);
    assert.equal(fs.readFileSync(destination, "utf8"), "ok");
    await assert.rejects(
      () => download("https://github.com/example/repo/releases/download/v0.4.12/large", path.join(root, "large"), {
        maxBytes: 1,
        fetchImpl: async () => response("too large")
      }),
      /exceeds 1 bytes/u
    );
  } finally {
    removeTemporaryRoot(root);
  }
});

test("release URLs and redirects require HTTPS approved GitHub hosts", async () => {
  const root = temporaryRoot();
  try {
    const metadata = releaseMetadata();
    await assert.rejects(
      () => launch({
        cacheRoot: root,
        metadata: { ...metadata, releaseBase: metadata.releaseBase.replace("https://", "http://") },
        fetchImpl: () => { throw new Error("must not fetch"); },
        spawnImpl: () => fakeChild(),
        checkForUpdates: false
      }),
      /HTTPS|approved GitHub/u
    );
    await assert.rejects(
      () => launch({
        cacheRoot: root,
        metadata: { ...metadata, releaseBase: "https://evil.example/releases/download/v0.4.12" },
        fetchImpl: () => { throw new Error("must not fetch"); },
        spawnImpl: () => fakeChild(),
        checkForUpdates: false
      }),
      /approved GitHub/u
    );
    await assert.rejects(
      () => download("https://github.com/example/repo/releases/download/v0.4.12/start", path.join(root, "asset"), {
        fetchImpl: async () => ({
          status: 302,
          headers: new Headers({ location: "https://evil.example/asset" })
        })
      }),
      /approved GitHub/u
    );
    const update = await maybeCheckForUpdate();
    assert.deepEqual(update, { checked: false, skipped: "rust-owned" });
  } finally {
    removeTemporaryRoot(root);
  }
});
