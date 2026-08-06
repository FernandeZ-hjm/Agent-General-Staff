import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawn as nodeSpawn, spawnSync as nodeSpawnSync } from "node:child_process";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import zlib from "node:zlib";

const require = createRequire(import.meta.url);
const packageJson = require("../package.json");

export const RELEASE_REPOSITORY = "FernandeZ-hjm/Agent-General-Staff";
export const MAX_DOWNLOAD_BYTES = 128 * 1024 * 1024;
export const DOWNLOAD_TIMEOUT_MS = 30_000;
export const UPDATE_CHECK_INTERVAL_MS = 7 * 24 * 60 * 60 * 1000;
export const UPDATE_CHECK_STATE_SCHEMA = "0.4.13-update-check-state";
export const UPDATE_STATE_FILE = "update-check.json";
export const UPDATE_PLAN_SCHEMA = "1.1-launcher-update-plan";
export const UPDATE_RECEIPT_SCHEMA = "1.1-launcher-update-receipt";
export const VERIFIED_CATALOG_SCHEMA = "0.4.15-verified-catalog";
export const MCP_ARGS = Object.freeze(["mcp", "serve", "--transport", "stdio"]);
export function mcpRuntimeArgs(args) {
  if (args.length === 0) return [...MCP_ARGS];
  if (args[0] === "setup") return [...args];
  throw new Error(
    "supported commands are `ags-mcp setup ...` and `ags-mcp update ...`; run without arguments to start the MCP server"
  );
}
export const RELEASE_SIGNING_PUBLIC_KEY_PEM = fs.readFileSync(
  new URL("../release-signing-public.pem", import.meta.url),
  "utf8"
);

const VERSION_PATTERN = /^(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/u;
const TRIPLE_PATTERN = /^[A-Za-z0-9._-]+$/u;
const HASH_PATTERN = /^[a-f0-9]{64}$/u;
const CHANNELS = new Set(["stable", "beta", "nightly"]);
const APPROVED_RELEASE_HOSTS = new Set([
  "github.com",
  "api.github.com",
  "objects.githubusercontent.com",
  "github-releases.githubusercontent.com",
  "release-assets.githubusercontent.com"
]);
const REQUIRED_RUNTIME_FILES = Object.freeze([
  "manifests/suite.yaml",
  "manifests/skills-registry.yaml",
  "manifests/mcp-registry.yaml",
  "protocol/agent-task-protocol.md"
]);
const TAR_TYPE_REGULAR = "0".charCodeAt(0);
const TAR_TYPE_DIRECTORY = "5".charCodeAt(0);

const TARGETS = Object.freeze({
  "darwin-arm64": ["aarch64-apple-darwin", "tar.gz"],
  "darwin-x64": ["x86_64-apple-darwin", "tar.gz"],
  "linux-arm64": ["aarch64-unknown-linux-gnu", "tar.gz"],
  "linux-x64": ["x86_64-unknown-linux-gnu", "tar.gz"],
  "win32-x64": ["x86_64-pc-windows-msvc", "zip"]
});

export function releaseTarget(platform = process.platform, arch = process.arch) {
  const target = TARGETS[`${platform}-${arch}`];
  if (!target) {
    throw new Error(`unsupported platform: ${platform}-${arch}`);
  }
  return { triple: target[0], extension: target[1] };
}

export function releaseMetadata({
  version = packageJson.version,
  platform = process.platform,
  arch = process.arch,
  repository = RELEASE_REPOSITORY
} = {}) {
  assertVersion(version, "release version");
  const { triple, extension } = releaseTarget(platform, arch);
  const assetName = `ags-v${version}-${triple}.${extension}`;
  const metadata = {
    version,
    platform,
    arch,
    triple,
    extension,
    assetName,
    releaseBase: `https://github.com/${repository}/releases/download/v${version}`,
    releaseIndexEndpoint: `https://github.com/${repository}/releases/download/v${version}/release-index.json`,
    releaseSignatureEndpoint: `https://github.com/${repository}/releases/download/v${version}/release-index.sig`,
    updateEndpoint: `https://github.com/${repository}/releases/latest/download/release-index.json`,
    updateSignatureEndpoint: `https://github.com/${repository}/releases/latest/download/release-index.sig`
  };
  assertReleaseBaseUrl(metadata.releaseBase, version);
  assertVersionedReleaseIndexUrl(metadata.releaseIndexEndpoint, metadata.releaseBase, "release index");
  assertVersionedReleaseIndexUrl(metadata.releaseSignatureEndpoint, metadata.releaseBase, "release signature");
  assertUpdateEndpointUrl(metadata.updateEndpoint);
  assertUpdateEndpointUrl(metadata.updateSignatureEndpoint, "release signature");
  return Object.freeze(metadata);
}

export function sha256File(file) {
  const hash = crypto.createHash("sha256");
  hash.update(fs.readFileSync(file));
  return hash.digest("hex");
}

function hashCanonical(value) {
  return crypto
    .createHash("sha256")
    .update(JSON.stringify(sortCanonical(value)))
    .digest("hex");
}

function sortCanonical(value) {
  if (Array.isArray(value)) return value.map(sortCanonical);
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.keys(value)
        .sort()
        .map((key) => [key, sortCanonical(value[key])])
    );
  }
  return value;
}

export function sha256Directory(directory) {
  const records = [];
  collectDirectoryRecords(directory, "", records);
  records.sort((left, right) => left.relative.localeCompare(right.relative));
  const hash = crypto.createHash("sha256");
  for (const record of records) {
    hash.update(`${record.relative}\0${record.size}\0${record.sha256}\n`);
  }
  return hash.digest("hex");
}

export async function launch(options = {}) {
  const env = options.env || process.env;
  const metadata = normalizeMetadata(options.metadata);
  const paths = cachePaths(options.cacheRoot ?? env.AGS_CACHE_DIR, metadata, env);
  const artifact = await prepareArtifact(metadata, paths, options);

  if (options.checkForUpdates !== false) {
    await maybeCheckForUpdate({
      stateRoot: paths.stateRoot,
      currentVersion: artifact.version || metadata.version,
      channel: options.channel || "stable",
      enabled: options.updateChecksEnabled,
      fetchImpl: options.updateFetch || options.fetchImpl || globalThis.fetch,
      clock: options.clock,
      timeoutMs: options.updateTimeoutMs,
      endpoint: metadata.updateEndpoint,
      signatureEndpoint: metadata.updateSignatureEndpoint,
      verifyReleaseIndex: options.verifyReleaseIndex,
      onUpdate: options.onUpdate
    });
  }

  const args = options.args === undefined ? [...MCP_ARGS] : options.args;
  if (!Array.isArray(args) || args.some((arg) => typeof arg !== "string")) {
    throw new TypeError("launcher args must be an array of strings");
  }
  const runtimeHome =
    options.runtimeHome ||
    env.AGS_RUNTIME_HOME ||
    env.AGS_HOME ||
    path.join(paths.cacheRoot, "private-runtime");
  const child = (options.spawnImpl || nodeSpawn)(artifact.binaryPath, [...args], {
    stdio: options.stdio || "inherit",
    windowsHide: true,
    shell: false,
    env: {
      ...env,
      AGS_SOURCE_ROOT: artifact.runtimeRoot,
      AGS_RUNTIME_HOME: runtimeHome,
      AGS_HOME: runtimeHome
    }
  });
  return waitForChild(child);
}

/** Explicitly switch the shared launcher pointer back to the last verified
 * healthy version. This is invoked only by the user-facing `ags update
 * recover` command; ordinary startup recovery remains limited to an invalid
 * or missing current pointer. */
export async function recoverPrevious(options = {}) {
  const env = options.env || process.env;
  const metadata = normalizeMetadata(options.metadata);
  const paths = cachePaths(options.cacheRoot ?? env.AGS_CACHE_DIR, metadata, env);
  ensureCacheDirectories(paths);
  const current = readPointer(paths.currentPath);
  const previous = readPointer(paths.previousPath);
  if (!previous) throw new Error("previous launcher pointer is missing");
  const recovered = validatePointer(previous, paths);
  const active = current ? validatePointer(current, paths) : null;
  if (
    options.expectedPlanHash &&
    current?.update_plan_hash !== options.expectedPlanHash
  ) {
    throw new Error("active core pointer does not belong to the requested recovery plan");
  }
  const runtimeHome = resolveRuntimeHome(paths, env, options);
  let runtimeRecovery = { required: false, recovered: true };
  if (active && HASH_PATTERN.test(current.update_plan_hash || "")) {
    const receiptPath = path.join(
      paths.stateRoot,
      "update-receipts",
      `${current.update_plan_hash}.json`
    );
    const receipt = readJsonIfPresent(receiptPath);
    if (!receipt || receipt.schema_version !== UPDATE_RECEIPT_SCHEMA) {
      throw new Error("active core update receipt is missing; runtime recovery refused");
    }
    const { receipt_hash: storedHash, ...unsigned } = receipt;
    if (storedHash !== hashCanonical(unsigned) || receipt.plan_hash !== current.update_plan_hash) {
      throw new Error("active core update receipt identity mismatch");
    }
    if (receipt.runtime_setup?.required === true) {
      const result = await runRuntimeCommand(
        active,
        [
          "setup",
          "--recover-plan-hash",
          receipt.runtime_setup.plan_hash,
          "--target",
          runtimeHome,
          "--format",
          "json"
        ],
        runtimeHome,
        options
      );
      const document = parseRuntimeJson(result, "runtime setup recovery");
      if (document.phase !== "recover" || document.status !== "recovered") {
        throw new Error("runtime setup recovery did not return a recovered MaintenanceReceipt");
      }
      runtimeRecovery = {
        required: true,
        recovered: true,
        plan_hash: document.plan_hash,
        receipt_id: document.receipt_id
      };
    }
  }
  try {
    atomicWriteJson(paths.currentPath, previous);
  } catch (error) {
    if (runtimeRecovery.required && active) {
      await applyRuntimeSetup(active, runtimeHome, { required: true }, options).catch(() => undefined);
    }
    throw error;
  }
  if (current && !sameIdentity(current, previous)) {
    atomicWriteJson(paths.previousPath, current);
  }
  return {
    status: "recovered",
    version: recovered.version,
    triple: recovered.triple,
    binaryPath: recovered.binaryPath,
    runtimeRoot: recovered.runtimeRoot,
    runtime_recovery: runtimeRecovery
  };
}

/** Fetch, authenticate and persist one immutable core update plan. The plan is
 * bound to the current verified pointer and the exact signed index bytes. */
export async function planUpdate(options = {}) {
  const env = options.env || process.env;
  const metadata = normalizeMetadata(options.metadata);
  const paths = cachePaths(options.cacheRoot ?? env.AGS_CACHE_DIR, metadata, env);
  ensureCacheDirectories(paths);
  await prepareArtifact(metadata, paths, options);
  const current = readPointer(paths.currentPath);
  const active = validatePointer(current, paths);
  const channel = normalizeChannel(options.channel || "stable");
  if (channel !== "stable") {
    throw new Error(`signed release channel is not available: ${channel}`);
  }
  const fetchImpl = options.updateFetch || options.fetchImpl || globalThis.fetch;
  const timeoutMs = options.updateTimeoutMs || 2_000;
  const indexBytes = await fetchReleaseIndexBytes(metadata.updateEndpoint, {
    fetchImpl,
    timeoutMs,
    label: "release index"
  });
  const signatureBytes = await fetchReleaseIndexBytes(metadata.updateSignatureEndpoint, {
    fetchImpl,
    timeoutMs,
    label: "release signature"
  });
  const verified = typeof options.verifyReleaseIndex === "function"
    ? await options.verifyReleaseIndex(indexBytes, signatureBytes, {
      currentVersion: current.version,
      channel
    })
    : verifySignedReleaseIndex(indexBytes, signatureBytes);
  if (verified !== true && verified?.verified !== true) {
    throw new Error("release signature verification failed");
  }
  const index = parseSignedReleaseIndex(indexBytes, channel);
  if (compareVersions(parseVersion(index.version), parseVersion(current.version)) <= 0) {
    throw new Error("no_update_available");
  }
  const targetMetadata = releaseMetadata({
    version: index.version,
    platform: metadata.platform,
    arch: metadata.arch,
    repository: RELEASE_REPOSITORY
  });
  const asset = index.assets.find((candidate) => candidate.name === targetMetadata.assetName);
  if (!asset) {
    throw new Error(`signed release index has no entry for ${targetMetadata.assetName}`);
  }
  const targetPaths = cachePaths(paths.cacheRoot, targetMetadata, env);
  const targetIdentity = await installOrReuse(targetMetadata, targetPaths, {
    ...options,
    expectedReleaseIndexSha256: crypto.createHash("sha256").update(indexBytes).digest("hex")
  });
  if (targetIdentity.assetSha256 !== asset.sha256) {
    throw new Error("installed candidate does not match signed update plan source");
  }
  await verifyExecutable(targetIdentity, targetMetadata, options, "candidate AGS executable");
  const runtimeHome = resolveRuntimeHome(paths, env, options);
  const runtimeSetup = await planRuntimeSetup(targetIdentity, runtimeHome, options);
  const now = nowMillis(options.clock);
  const unsigned = {
    schema_version: UPDATE_PLAN_SCHEMA,
    current_pointer_hash: hashCanonical(current),
    current_version: active.version,
    target_version: index.version,
    channel,
    triple: metadata.triple,
    asset_name: asset.name,
    asset_sha256: asset.sha256,
    release_index_sha256: crypto.createHash("sha256").update(indexBytes).digest("hex"),
    release_index_url: targetMetadata.releaseIndexEndpoint,
    release_signature_url: targetMetadata.releaseSignatureEndpoint,
    candidate_binary_sha256: targetIdentity.binarySha256,
    candidate_runtime_sha256: targetIdentity.runtimeSha256,
    runtime_home_hash: ags_platformHashPath(runtimeHome),
    runtime_setup: runtimeSetup,
    created_at_unix: Math.floor(now / 1000),
    expires_at_unix: Math.floor(now / 1000) + (options.planTtlSeconds || 30 * 60)
  };
  const plan = { ...unsigned, plan_hash: hashCanonical(unsigned) };
  const planPath = path.join(paths.stateRoot, "update-plans", `${plan.plan_hash}.json`);
  fs.mkdirSync(path.dirname(planPath), { recursive: true, mode: 0o700 });
  assertDirectory(path.dirname(planPath), "update plan directory");
  atomicWriteJson(planPath, plan);
  return plan;
}

/** Apply an exact persisted plan, verify the new executable starts, then
 * atomically switch current/previous. Any source, pointer or time drift
 * requires a new plan. */
export async function applyUpdate(planHash, options = {}) {
  if (!HASH_PATTERN.test(planHash || "")) throw new Error("update plan hash is invalid");
  const env = options.env || process.env;
  const metadata = normalizeMetadata(options.metadata);
  const paths = cachePaths(options.cacheRoot ?? env.AGS_CACHE_DIR, metadata, env);
  ensureCacheDirectories(paths);
  const planPath = path.join(paths.stateRoot, "update-plans", `${planHash}.json`);
  const plan = readJsonIfPresent(planPath);
  if (!plan || plan.schema_version !== UPDATE_PLAN_SCHEMA) {
    throw new Error("update plan is missing or has an unsupported schema");
  }
  const { plan_hash: storedHash, ...unsigned } = plan;
  if (storedHash !== planHash || hashCanonical(unsigned) !== planHash) {
    throw new Error("update plan hash mismatch");
  }
  const nowUnix = Math.floor(nowMillis(options.clock) / 1000);
  if (plan.expires_at_unix < nowUnix) throw new Error("update plan expired");
  if (plan.triple !== metadata.triple || plan.channel !== "stable") {
    throw new Error("update plan platform or channel mismatch");
  }
  const current = readPointer(paths.currentPath);
  validatePointer(current, paths);
  if (hashCanonical(current) !== plan.current_pointer_hash) {
    throw new Error("update plan current pointer changed; re-plan");
  }
  const targetMetadata = releaseMetadata({
    version: plan.target_version,
    platform: metadata.platform,
    arch: metadata.arch,
    repository: RELEASE_REPOSITORY
  });
  if (
    plan.asset_name !== targetMetadata.assetName ||
    plan.release_index_url !== targetMetadata.releaseIndexEndpoint ||
    plan.release_signature_url !== targetMetadata.releaseSignatureEndpoint
  ) {
    throw new Error("update plan release identity mismatch");
  }
  const runtimeHome = resolveRuntimeHome(paths, env, options);
  if (plan.runtime_home_hash !== ags_platformHashPath(runtimeHome)) {
    throw new Error("update plan runtime home changed; re-plan");
  }
  const targetPaths = cachePaths(paths.cacheRoot, targetMetadata, env);
  const identity = await installOrReuse(targetMetadata, targetPaths, {
    ...options,
    expectedReleaseIndexSha256: plan.release_index_sha256
  });
  if (identity.assetSha256 !== plan.asset_sha256) {
    throw new Error("installed asset does not match approved update plan");
  }
  if (
    identity.binarySha256 !== plan.candidate_binary_sha256 ||
    identity.runtimeSha256 !== plan.candidate_runtime_sha256
  ) {
    throw new Error("installed candidate content differs from the approved update plan");
  }
  await verifyExecutable(identity, targetMetadata, options, "updated AGS executable");
  await verifyRuntimeSetupPlan(identity, runtimeHome, plan.runtime_setup, options);
  const observedCurrent = readPointer(paths.currentPath);
  if (hashCanonical(observedCurrent) !== plan.current_pointer_hash) {
    throw new Error("update plan current pointer changed during apply; activation refused");
  }
  const next = { ...pointerFor(targetMetadata, identity, options.clock), update_plan_hash: planHash };
  atomicWriteJson(paths.previousPath, observedCurrent);
  atomicWriteJson(paths.currentPath, next);
  let runtimeSetupReceipt;
  try {
    runtimeSetupReceipt = await applyRuntimeSetup(identity, runtimeHome, plan.runtime_setup, options);
  } catch (error) {
    atomicWriteJson(paths.currentPath, observedCurrent);
    throw new Error(`runtime setup activation failed; core pointer restored: ${error.message}`);
  }
  const receiptWithoutHash = {
    schema_version: UPDATE_RECEIPT_SCHEMA,
    plan_hash: planHash,
    previous_version: observedCurrent.version,
    active_version: next.version,
    triple: next.triple,
    asset_sha256: next.asset_sha256,
    release_index_sha256: next.release_index_sha256,
    runtime_setup: runtimeSetupReceipt,
    verified: true,
    applied_at_unix: nowUnix
  };
  const receipt = {
    ...receiptWithoutHash,
    receipt_hash: hashCanonical(receiptWithoutHash)
  };
  const receiptPath = path.join(paths.stateRoot, "update-receipts", `${planHash}.json`);
  fs.mkdirSync(path.dirname(receiptPath), { recursive: true, mode: 0o700 });
  assertDirectory(path.dirname(receiptPath), "update receipt directory");
  atomicWriteJson(receiptPath, receipt);
  return { ...receipt, receipt_path: receiptPath };
}

export function statusUpdate(planHash, options = {}) {
  if (!HASH_PATTERN.test(planHash || "")) throw new Error("update plan hash is invalid");
  const env = options.env || process.env;
  const metadata = normalizeMetadata(options.metadata);
  const paths = cachePaths(options.cacheRoot ?? env.AGS_CACHE_DIR, metadata, env);
  const planPath = path.join(paths.stateRoot, "update-plans", `${planHash}.json`);
  const receiptPath = path.join(paths.stateRoot, "update-receipts", `${planHash}.json`);
  const plan = readJsonIfPresent(planPath);
  if (!plan || plan.schema_version !== UPDATE_PLAN_SCHEMA || plan.plan_hash !== planHash) {
    throw new Error("update plan is missing or has an unsupported schema");
  }
  const { plan_hash: storedHash, ...unsigned } = plan;
  if (storedHash !== planHash || hashCanonical(unsigned) !== planHash) {
    throw new Error("update plan hash mismatch");
  }
  const receipt = readJsonIfPresent(receiptPath);
  return {
    schema_version: "0.4.13-core-update-status",
    plan,
    receipt,
    active: readJsonIfPresent(paths.currentPath)
  };
}

export async function verifyUpdate(planHash, options = {}) {
  const status = statusUpdate(planHash, options);
  const receipt = status.receipt;
  if (!receipt || receipt.schema_version !== UPDATE_RECEIPT_SCHEMA || receipt.plan_hash !== planHash) {
    throw new Error("core update has no applied receipt");
  }
  const { receipt_hash: storedHash, ...unsigned } = receipt;
  if (storedHash !== hashCanonical(unsigned)) throw new Error("core update receipt hash mismatch");
  const activePointer = status.active;
  if (
    !activePointer ||
    activePointer.version !== receipt.active_version ||
    activePointer.asset_sha256 !== receipt.asset_sha256 ||
    activePointer.release_index_sha256 !== receipt.release_index_sha256
  ) {
    throw new Error("core update active pointer does not match the receipt");
  }
  const env = options.env || process.env;
  const metadata = normalizeMetadata(options.metadata);
  const paths = cachePaths(options.cacheRoot ?? env.AGS_CACHE_DIR, metadata, env);
  const active = validatePointer(activePointer, paths);
  const activeMetadata = releaseMetadata({
    version: active.version,
    platform: metadata.platform,
    arch: metadata.arch,
    repository: RELEASE_REPOSITORY
  });
  await verifyExecutable(active, activeMetadata, options, "core update active binary");
  const runtimeHome = resolveRuntimeHome(paths, env, options);
  await verifyRuntimeSetup(active, runtimeHome, receipt.runtime_setup, options);
  return {
    schema_version: "0.4.13-core-update-verification",
    status: "verified",
    plan_hash: planHash,
    active_version: active.version,
    asset_sha256: active.asset_sha256,
    runtime_setup_verified: receipt.runtime_setup?.required !== true || receipt.runtime_setup?.verified === true,
    reconnect_required: true
  };
}

/** Shared transport adapter for CLI and MCP bins. The caller only renders the
 * structured result; all update planning and mutation remains in this module. */
export async function handleCoreMaintenanceCommand(args, options = {}) {
  if (args?.[0] !== "update") return { handled: false };
  const action = args[1];
  if (action === "check") {
    const metadata = normalizeMetadata(options.metadata);
    const env = options.env || process.env;
    const paths = cachePaths(options.cacheRoot ?? env.AGS_CACHE_DIR, metadata, env);
    return {
      handled: true,
      result: await maybeCheckForUpdate({
        stateRoot: paths.stateRoot,
        currentVersion: metadata.version,
        channel: options.channel || "stable",
        fetchImpl: options.updateFetch || options.fetchImpl || globalThis.fetch,
        clock: options.clock,
        timeoutMs: options.updateTimeoutMs,
        endpoint: metadata.updateEndpoint,
        signatureEndpoint: metadata.updateSignatureEndpoint,
        verifyReleaseIndex: options.verifyReleaseIndex,
        onUpdate: options.onUpdate,
        force: true
      })
    };
  }
  if (action === "config") {
    const metadata = normalizeMetadata(options.metadata);
    const env = options.env || process.env;
    const paths = cachePaths(options.cacheRoot ?? env.AGS_CACHE_DIR, metadata, env);
    let state = readUpdateState(paths.stateRoot);
    const enabled = optionValue(args, "--enabled");
    if (enabled !== undefined) {
      if (!["true", "false"].includes(enabled)) throw new Error("--enabled must be true or false");
      state = { ...state, enabled: enabled === "true" };
    }
    const ignored = optionValue(args, "--ignore-version");
    if (ignored !== undefined) {
      assertVersion(ignored, "ignored version");
      state = { ...state, ignored_versions: [...new Set([...state.ignored_versions, ignored])] };
    }
    const snoozed = optionValue(args, "--snooze-until-unix");
    if (snoozed !== undefined) {
      const until = Number(snoozed);
      if (!Number.isSafeInteger(until) || until < 0) throw new Error("--snooze-until-unix must be a non-negative integer");
      state = { ...state, snoozed_until_unix: until };
    }
    return { handled: true, result: writeUpdateState(paths.stateRoot, state) };
  }
  if (action === "plan") {
    return { handled: true, result: await planUpdate(options) };
  }
  if (["apply", "status", "verify"].includes(action)) {
    const planHash = optionValue(args, "--plan-hash");
    if (!planHash) throw new Error(`core update ${action} requires --plan-hash <hash>`);
    const result = action === "apply"
      ? await applyUpdate(planHash, options)
      : action === "status"
        ? statusUpdate(planHash, options)
        : await verifyUpdate(planHash, options);
    return { handled: true, result };
  }
  if (action === "recover") {
    const expectedPlanHash = optionValue(args, "--plan-hash");
    return {
      handled: true,
      result: await recoverPrevious({ ...options, expectedPlanHash })
    };
  }
  return { handled: false };
}

function optionValue(args, name) {
  const index = args.indexOf(name);
  return index >= 0 ? args[index + 1] : undefined;
}

async function verifyExecutable(identity, metadata, options, label) {
  const result = typeof options.verifyArtifact === "function"
    ? await options.verifyArtifact(identity, metadata)
    : nodeSpawnSync(identity.binaryPath, ["--version"], {
      encoding: "utf8",
      shell: false,
      windowsHide: true,
      timeout: options.verifyTimeoutMs || 10_000
    });
  if (
    result !== true &&
    result?.verified !== true &&
    (result?.status !== 0 || result?.error)
  ) {
    throw new Error(`${label} failed verification${result?.error ? `: ${result.error.message}` : ""}`);
  }
}

function resolveRuntimeHome(paths, env, options) {
  return path.resolve(
    options.runtimeHome ||
    env.AGS_RUNTIME_HOME ||
    env.AGS_HOME ||
    path.join(paths.cacheRoot, "private-runtime")
  );
}

function ags_platformHashPath(value) {
  return crypto.createHash("sha256").update(path.resolve(value)).digest("hex");
}

async function planRuntimeSetup(identity, runtimeHome, options) {
  const installManifest = path.join(runtimeHome, "install-manifest.json");
  if (!isRegularFile(installManifest)) {
    return Object.freeze({ required: false, reason: "runtime-not-initialized" });
  }
  const result = await runRuntimeCommand(
    identity,
    ["setup", "--dry-run", "--target", runtimeHome, "--format", "json"],
    runtimeHome,
    options
  );
  const document = parseRuntimeJson(result, "candidate runtime setup plan");
  return Object.freeze({
    required: true,
    preview_hash: hashCanonical(document),
    approved_lifecycle_hosts: document.lifecycle_approval?.approved_hosts || []
  });
}

async function verifyRuntimeSetupPlan(identity, runtimeHome, approved, options) {
  if (approved?.required !== true) return;
  const observed = await planRuntimeSetup(identity, runtimeHome, options);
  if (observed.required !== true || observed.preview_hash !== approved.preview_hash) {
    throw new Error("runtime setup facts changed after the approved update plan");
  }
}

async function applyRuntimeSetup(identity, runtimeHome, approved, options) {
  if (approved?.required !== true) return { required: false, verified: true };
  const result = await runRuntimeCommand(
    identity,
    ["setup", "--yes", "--force", "--target", runtimeHome, "--format", "json"],
    runtimeHome,
    options
  );
  const document = parseRuntimeJson(result, "candidate runtime setup apply");
  const closure = document.result;
  const verified = Array.isArray(closure?.maintenance_receipts) &&
    closure.maintenance_receipts.some((receipt) => receipt.phase === "verify" && receipt.status === "verified");
  if (!verified || !closure?.maintenance_plan?.plan_hash) {
    throw new Error("candidate runtime setup did not return a verified MaintenanceReceipt");
  }
  return {
    required: true,
    verified: true,
    plan_hash: closure.maintenance_plan.plan_hash,
    receipt_hash: hashCanonical(closure.maintenance_receipts),
    report_hash: hashCanonical(closure.report)
  };
}

async function verifyRuntimeSetup(identity, runtimeHome, applied, options) {
  if (applied?.required !== true) return;
  const result = await runRuntimeCommand(
    identity,
    ["doctor", "--format", "json"],
    runtimeHome,
    options
  );
  parseRuntimeJson(result, "updated AGS runtime verification");
}

async function runRuntimeCommand(identity, args, runtimeHome, options) {
  const env = {
    ...(options.env || process.env),
    AGS_SOURCE_ROOT: identity.runtimeRoot,
    AGS_RUNTIME_HOME: runtimeHome,
    AGS_HOME: runtimeHome
  };
  if (typeof options.runRuntimeCommand === "function") {
    return options.runRuntimeCommand(identity, args, { env, runtimeHome });
  }
  return nodeSpawnSync(identity.binaryPath, args, {
    encoding: "utf8",
    shell: false,
    windowsHide: true,
    env,
    timeout: options.runtimeSetupTimeoutMs || 120_000
  });
}

function parseRuntimeJson(result, label) {
  if (result?.status !== 0 || result?.error) {
    const detail = result?.stderr?.trim?.() || result?.error?.message || "unknown error";
    throw new Error(`${label} failed: ${detail}`);
  }
  try {
    return JSON.parse(result.stdout);
  } catch (error) {
    throw new Error(`${label} returned invalid JSON: ${error.message}`);
  }
}

export function cachePaths(cacheRoot, metadata = releaseMetadata(), env = process.env) {
  const root = path.resolve(
    cacheRoot || env.AGS_CACHE_DIR || path.join(os.homedir(), ".ags")
  );
  const stateRoot = path.join(root, "launcher-state");
  const versionRoot = path.join(root, "versions", metadata.version);
  const versionDir = path.join(versionRoot, metadata.triple);
  const binaryName = metadata.platform === "win32" ? "ags.exe" : "ags";
  return Object.freeze({
    cacheRoot: root,
    stateRoot,
    versionsRoot: path.join(root, "versions"),
    versionRoot,
    versionDir,
    binaryName,
    binaryPath: path.join(versionDir, binaryName),
    runtimeRoot: path.join(versionDir, "runtime"),
    markerPath: path.join(versionDir, ".verified-sha256"),
    currentPath: path.join(stateRoot, "current.json"),
    previousPath: path.join(stateRoot, "previous.json"),
    lockPath: path.join(stateRoot, ".install.lock"),
    triple: metadata.triple,
    extension: metadata.extension
  });
}

export function safeArchiveOutput(destination, entryName, binaryName) {
  return archiveOutputOrThrow(destination, entryName, binaryName);
}

function archiveOutputOrThrow(destination, entryName, binaryName) {
  if (typeof entryName !== "string") {
    throw new Error("unsafe archive path: entry name is not a string");
  }
  const normalized = entryName.replaceAll("\\", "/").replace(/^\.\/+/u, "");
  if (!normalized || normalized.includes("\0")) {
    throw new Error(`unsafe archive path: ${entryName}`);
  }
  const segments = normalized.split("/");
  if (
    path.posix.isAbsolute(normalized) ||
    /^[A-Za-z]:/u.test(normalized) ||
    segments.some((segment) => segment === ".." || segment === ".") ||
    segments.some((segment, index) => segment === "" && index !== segments.length - 1)
  ) {
    throw new Error(`unsafe archive path: ${entryName}`);
  }
  if (normalized === binaryName) return path.join(destination, binaryName);
  if (normalized !== "runtime" && !normalized.startsWith("runtime/")) {
    throw new Error(`unsafe archive path: ${entryName}`);
  }
  const output = path.resolve(destination, normalized);
  const destinationRoot = path.resolve(destination);
  const runtimeRoot = path.resolve(destination, "runtime");
  if (
    (output !== destinationRoot && !output.startsWith(`${destinationRoot}${path.sep}`)) ||
    (output !== runtimeRoot && !output.startsWith(`${runtimeRoot}${path.sep}`))
  ) {
    throw new Error(`unsafe archive path: ${entryName}`);
  }
  return output;
}

export async function download(
  url,
  destination,
  {
    fetchImpl = globalThis.fetch,
    maxBytes = MAX_DOWNLOAD_BYTES,
    timeoutMs = DOWNLOAD_TIMEOUT_MS,
    userAgent = `@agent-governance-suite/mcp/${packageJson.version}`
  } = {}
) {
  if (typeof fetchImpl !== "function") {
    throw new Error("fetch is unavailable; cannot download AGS release assets");
  }
  let currentUrl = approvedGitHubUrl(url, "release download URL").toString();
  for (let redirects = 0; redirects <= 5; redirects += 1) {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), timeoutMs);
    timer.unref?.();
    let response;
    try {
      response = await fetchImpl(currentUrl, {
        redirect: "manual",
        signal: controller.signal,
        headers: {
          "user-agent": userAgent,
          accept: "application/octet-stream"
        }
      });
    } catch (error) {
      if (controller.signal.aborted) {
        throw new Error(`download timed out: ${currentUrl}`);
      }
      throw error;
    } finally {
      clearTimeout(timer);
    }

    const status = Number(response?.status);
    if (status >= 300 && status < 400) {
      const location = response.headers?.get?.("location") || response.headers?.location;
      if (!location) {
        throw new Error(`download redirect has no location: ${currentUrl}`);
      }
      if (redirects === 5) {
        throw new Error(`too many redirects downloading ${url}`);
      }
      currentUrl = approvedGitHubUrl(
        new URL(location, currentUrl).toString(),
        "release redirect URL"
      ).toString();
      continue;
    }
    if (status !== 200) {
      throw new Error(`download failed (${status}): ${currentUrl}`);
    }
    const contentLength = Number(response.headers?.get?.("content-length"));
    if (Number.isFinite(contentLength) && contentLength > maxBytes) {
      throw new Error(`download exceeds ${maxBytes} bytes`);
    }
    const body = response.arrayBuffer
      ? Buffer.from(await response.arrayBuffer())
      : await readResponseBody(response);
    if (body.length > maxBytes) {
      throw new Error(`download exceeds ${maxBytes} bytes`);
    }
    fs.writeFileSync(destination, body, { flag: "wx", mode: 0o600 });
    return;
  }
  throw new Error(`too many redirects downloading ${url}`);
}

export async function maybeCheckForUpdate({
  stateRoot,
  cacheRoot,
  currentVersion = packageJson.version,
  channel = "stable",
  enabled,
  fetchImpl = globalThis.fetch,
  clock = Date.now,
  timeoutMs = 2_000,
  endpoint = releaseMetadata().updateEndpoint,
  signatureEndpoint = releaseMetadata().updateSignatureEndpoint,
  verifyReleaseIndex,
  onUpdate = defaultUpdateNotice,
  force = false
} = {}) {
  const resolvedStateRoot = stateRoot || path.join(resolveCacheRoot(cacheRoot), "launcher-state");
  const now = nowMillis(clock);
  let state = readUpdateState(resolvedStateRoot);
  const normalizedChannel = normalizeChannel(channel);

  if (typeof enabled === "boolean") state = { ...state, enabled };
  if (state.channel !== normalizedChannel) {
    state = { ...state, channel: normalizedChannel, last_checked_at_unix: null };
  }
  const nowUnix = Math.floor(now / 1000);
  if (state.snoozed_until_unix !== null && state.snoozed_until_unix <= nowUnix) {
    state = { ...state, snoozed_until_unix: null };
  }
  if (!force && !state.enabled) {
    writeUpdateStateBestEffort(resolvedStateRoot, state);
    return { checked: false, skipped: "disabled", state };
  }
  const snoozed = state.snoozed_until_unix !== null && state.snoozed_until_unix > nowUnix;
  const lastChecked =
    state.last_checked_at_unix === null ? Number.NaN : state.last_checked_at_unix * 1000;
  if (!force && snoozed) {
    writeUpdateStateBestEffort(resolvedStateRoot, state);
    return { checked: false, skipped: "snoozed", state };
  }
  if (!force && Number.isFinite(lastChecked) && now - lastChecked < UPDATE_CHECK_INTERVAL_MS) {
    writeUpdateStateBestEffort(resolvedStateRoot, state);
    return { checked: false, skipped: "fresh", state };
  }

  state = { ...state, last_checked_at_unix: nowUnix, last_error: null };
  writeUpdateStateBestEffort(resolvedStateRoot, state);
  try {
    if (normalizedChannel !== "stable") throw new Error(`signed release channel is not available: ${normalizedChannel}`);
    const indexBytes = await fetchReleaseIndexBytes(endpoint, { fetchImpl, timeoutMs, label: "release index" });
    const signatureBytes = await fetchReleaseIndexBytes(signatureEndpoint, {
      fetchImpl,
      timeoutMs,
      label: "release signature"
    });
    const releaseIndexHash = crypto.createHash("sha256").update(indexBytes).digest("hex");
    state = { ...state, release_index_hash: releaseIndexHash };
    let verified = false;
    try {
      verified = typeof verifyReleaseIndex === "function"
        ? await verifyReleaseIndex(indexBytes, signatureBytes, { currentVersion, channel: normalizedChannel })
        : verifySignedReleaseIndex(indexBytes, signatureBytes);
    } catch {
      verified = false;
    }
    if (!verified) {
      state = { ...state, latest_version: null, last_error: "unavailable" };
      writeUpdateStateBestEffort(resolvedStateRoot, state);
      return { checked: true, unavailable: true, available: null, state };
    }
    const payload = parseSignedReleaseIndex(indexBytes, normalizedChannel);
    const catalog = await refreshVerifiedCatalog({
      payload,
      indexEndpoint: endpoint,
      stateRoot: resolvedStateRoot,
      fetchImpl,
      timeoutMs
    });
    const latest = { version: payload.version, url: `https://github.com/${payload.repository}/releases/tag/${payload.tag}` };
    const available =
      latest && compareVersions(parseVersion(latest.version), parseVersion(currentVersion)) > 0
        ? latest
        : null;
    state = {
      ...state,
      latest_version: latest?.version || null,
      catalog_release: catalog?.release || state.catalog_release || null,
      catalog_hash: catalog?.content_hash?.replace(/^sha256:/u, "") || state.catalog_hash || null,
      last_error: null
    };
    writeUpdateStateBestEffort(resolvedStateRoot, state);
    if (!available || state.ignored_versions.includes(available.version)) {
      return { checked: true, available: null, state };
    }
    const update = {
      ...available,
      current_version: currentVersion,
      channel: normalizedChannel
    };
    try {
      onUpdate?.(update);
    } catch {
      // A notification hook is advisory and must not break launcher startup.
    }
    return { checked: true, available: update, state };
  } catch (error) {
    state = { ...state, latest_version: null, last_error: "offline" };
    writeUpdateStateBestEffort(resolvedStateRoot, state);
    return { checked: true, offline: true, error, available: null, state };
  }
}

export function readUpdateState(stateRootOrPath) {
  const statePath = updateStatePath(stateRootOrPath);
  const raw = readJsonIfPresent(statePath);
  return normalizeUpdateState(raw);
}

export function writeUpdateState(stateRootOrPath, state) {
  const statePath = updateStatePath(stateRootOrPath);
  const normalized = normalizeUpdateState(state);
  fs.mkdirSync(path.dirname(statePath), { recursive: true, mode: 0o700 });
  assertDirectory(path.dirname(statePath), "update state directory");
  atomicWriteJson(statePath, normalized);
  return normalized;
}

export function ignoreVersion(stateRootOrPath, version) {
  assertVersion(version, "ignored version");
  const state = readUpdateState(stateRootOrPath);
  return writeUpdateState(stateRootOrPath, {
    ...state,
    ignored_versions: [...new Set([...state.ignored_versions, version])]
  });
}

export function snoozeUpdates(stateRootOrPath, until) {
  const input = until instanceof Date ? until.getTime() : Number(until);
  const millis = input < 1_000_000_000_000 ? input * 1000 : input;
  if (!Number.isFinite(millis)) {
    throw new TypeError("snooze time must be a Date or millisecond timestamp");
  }
  return writeUpdateState(stateRootOrPath, {
    ...readUpdateState(stateRootOrPath),
    snoozed_until_unix: Math.floor(millis / 1000)
  });
}

export async function extractArchive(archivePath, extension, destination, binaryName) {
  if (extension === "zip") {
    extractStoredZip(archivePath, destination, binaryName);
    return;
  }
  if (extension !== "tar.gz") {
    throw new Error(`unsupported archive extension: ${extension}`);
  }
  const tar = zlib.gunzipSync(fs.readFileSync(archivePath), {
    maxOutputLength: MAX_DOWNLOAD_BYTES
  });
  let offset = 0;
  while (offset + 512 <= tar.length) {
    const header = tar.subarray(offset, offset + 512);
    if (header.every((byte) => byte === 0)) {
      return;
    }
    const name = header.subarray(0, 100).toString("utf8").replace(/\0.*$/u, "");
    const sizeText = header
      .subarray(124, 136)
      .toString("ascii")
      .replace(/\0.*$/u, "")
      .trim();
    if (!/^[0-7]+$/u.test(sizeText)) {
      throw new Error(`invalid tar entry size for ${name}`);
    }
    const size = Number.parseInt(sizeText || "0", 8);
    const type = header[156];
    offset += 512;
    if (!Number.isSafeInteger(size) || size > MAX_DOWNLOAD_BYTES || offset + size > tar.length) {
      throw new Error(`invalid tar entry size for ${name}`);
    }
    const output = archiveOutputOrThrow(destination, name, binaryName);
    if (type === 0 || type === TAR_TYPE_REGULAR) {
      if (output === path.resolve(destination, "runtime")) {
        throw new Error(`archive contains invalid regular directory payload: ${name}`);
      }
      writeArchiveFile(output, tar.subarray(offset, offset + size), binaryName);
    } else if (type === TAR_TYPE_DIRECTORY) {
      if (output === path.join(destination, binaryName)) {
        throw new Error(`archive contains invalid executable directory payload: ${name}`);
      }
      fs.mkdirSync(output, { recursive: true, mode: 0o700 });
    } else {
      throw new Error(`archive contains non-regular payload: ${name}`);
    }
    offset += Math.ceil(size / 512) * 512;
    if (offset > tar.length) {
      throw new Error(`truncated tar archive at ${name}`);
    }
  }
  throw new Error("truncated tar archive");
}

export const __filename = fileURLToPath(import.meta.url);

async function prepareArtifact(metadata, paths, options) {
  ensureCacheDirectories(paths);
  let current;
  try {
    current = readPointer(paths.currentPath);
  } catch (error) {
    if (!options.recoverFromPrevious) throw error;
    return recoverPreviousPointer(paths, error);
  }
  let active = null;
  if (current) {
    try {
      active = validatePointer(current, paths);
    } catch (error) {
      if (!options.recoverFromPrevious) throw error;
      return recoverPreviousPointer(paths, error);
    }
    if (
      current.version === metadata.version &&
      current.triple === metadata.triple
    ) {
      return active;
    }
    if (compareVersions(parseVersion(current.version), parseVersion(metadata.version)) > 0) {
      return active;
    }
  } else if (options.recoverFromPrevious) {
    const previous = readPointer(paths.previousPath);
    if (previous) return recoverPreviousPointer(paths, new Error("current launcher pointer is missing"));
  }

  const identity = await installOrReuse(metadata, paths, options);
  const pointer = pointerFor(metadata, identity, options.clock);
  if (!current || !sameIdentity(current, pointer)) {
    if (current) atomicWriteJson(paths.previousPath, current);
    atomicWriteJson(paths.currentPath, pointer);
  }
  return {
    ...identity,
    version: metadata.version,
    triple: metadata.triple,
    binaryPath: paths.binaryPath,
    runtimeRoot: paths.runtimeRoot
  };
}

function recoverPreviousPointer(paths, cause) {
  try {
    const previous = readPointer(paths.previousPath);
    if (!previous) throw new Error("previous launcher pointer is missing");
    const recovered = validatePointer(previous, paths);
    atomicWriteJson(paths.currentPath, previous);
    return recovered;
  } catch (error) {
    throw new Error(`${cause.message}; previous launcher pointer recovery failed: ${error.message}`);
  }
}

async function installOrReuse(metadata, paths, options) {
  const cached = inspectCache(paths.versionDir, {
    binaryName: paths.binaryName,
    assetName: metadata.assetName,
    version: metadata.version,
    assetSha256: undefined
  });
  if (cached) {
    return cached;
  }

  const versionDirectoryExists = pathExists(paths.versionDir);
  const release = await acquireInstallLock(paths.lockPath, {
    clock: options.clock,
    sleep: options.sleep
  });
  let stageRoot;
  try {
    const lockedCached = inspectCache(paths.versionDir, {
      binaryName: paths.binaryName,
      assetName: metadata.assetName,
      version: metadata.version
    });
    if (lockedCached) {
      return lockedCached;
    }
    if (versionDirectoryExists || pathExists(paths.versionDir)) {
      throw new Error(
        `immutable launcher cache entry is invalid: ${paths.versionDir}; refusing to overwrite it`
      );
    }

    stageRoot = fs.mkdtempSync(
      path.join(paths.versionsRoot, `.staged-${metadata.version}-${metadata.triple}-`)
    );
    const downloadRoot = path.join(stageRoot, ".download");
    fs.mkdirSync(downloadRoot, { recursive: true, mode: 0o700 });
    const assetPath = path.join(downloadRoot, metadata.assetName);
    const fetchImpl = options.fetchImpl || globalThis.fetch;
    const downloadImpl = options.downloadImpl || download;
    const signed = await acquireSignedReleaseIndex(metadata, downloadRoot, {
      ...options,
      fetchImpl,
      downloadImpl
    });
    const releaseIndexSha256 = signed.indexSha256;
    const expected = signed.asset.sha256;
    await downloadImpl(`${metadata.releaseBase}/${metadata.assetName}`, assetPath, {
      fetchImpl,
      userAgent: `@agent-governance-suite/launcher/${metadata.version}`
    });
    const actual = sha256File(assetPath);
    if (!timingSafeHexEqual(actual, expected)) {
      throw new Error(`checksum mismatch for ${metadata.assetName}`);
    }

    await extractArchive(assetPath, metadata.extension, stageRoot, paths.binaryName);
    const extractedBinary = path.join(stageRoot, paths.binaryName);
    const extractedRuntime = path.join(stageRoot, "runtime");
    assertReleasePayload(extractedBinary, extractedRuntime);
    if (metadata.platform !== "win32") {
      fs.chmodSync(extractedBinary, 0o755);
    }
    const identity = {
      assetName: metadata.assetName,
      assetSha256: expected,
      binarySha256: sha256File(extractedBinary),
      runtimeSha256: sha256Directory(extractedRuntime),
      releaseIndexSha256,
      version: metadata.version,
      triple: metadata.triple
    };
    fs.writeFileSync(
      path.join(stageRoot, ".verified-sha256"),
      `${identity.assetName}\n${identity.assetSha256}\n${identity.binarySha256}\n${identity.runtimeSha256}\n${identity.releaseIndexSha256}\n`,
      { mode: 0o600 }
    );

    fs.rmSync(downloadRoot, { recursive: true, force: true });
    fs.mkdirSync(paths.versionRoot, { recursive: true, mode: 0o700 });
    assertDirectory(paths.versionRoot, "version cache directory");
    if (pathExists(paths.versionDir)) {
      throw new Error(`launcher cache entry appeared during install: ${paths.versionDir}`);
    }
    fs.renameSync(stageRoot, paths.versionDir);
    stageRoot = undefined;
    const installed = inspectCache(paths.versionDir, {
      binaryName: paths.binaryName,
      assetName: metadata.assetName,
      assetSha256: expected,
      version: metadata.version
    });
    if (!installed) {
      throw new Error("installed launcher cache failed content verification");
    }
    return installed;
  } finally {
    if (stageRoot) fs.rmSync(stageRoot, { recursive: true, force: true });
    release();
  }
}

async function acquireSignedReleaseIndex(metadata, downloadRoot, options) {
  const indexPath = path.join(downloadRoot, "release-index.json");
  const signaturePath = path.join(downloadRoot, "release-index.sig");
  const downloadOptions = {
    fetchImpl: options.fetchImpl,
    maxBytes: 256 * 1024,
    userAgent: `@agent-governance-suite/launcher/${metadata.version}`
  };
  await options.downloadImpl(metadata.releaseIndexEndpoint, indexPath, downloadOptions);
  await options.downloadImpl(metadata.releaseSignatureEndpoint, signaturePath, downloadOptions);
  const indexBytes = fs.readFileSync(indexPath);
  const signatureBytes = fs.readFileSync(signaturePath);
  let verified;
  try {
    verified = typeof options.verifyReleaseIndex === "function"
      ? await options.verifyReleaseIndex(indexBytes, signatureBytes, {
        version: metadata.version,
        channel: "stable"
      })
      : verifySignedReleaseIndex(indexBytes, signatureBytes);
  } catch (error) {
    throw new Error(`release signature verification failed: ${error.message}`);
  }
  if (verified !== true && verified?.verified !== true) {
    throw new Error("release signature verification failed");
  }
  const index = parseSignedReleaseIndex(indexBytes, "stable");
  if (index.version !== metadata.version || index.tag !== `v${metadata.version}`) {
    throw new Error("signed release index does not match requested version");
  }
  const asset = index.assets.find((candidate) => candidate.name === metadata.assetName);
  if (!asset) {
    throw new Error(`signed release index has no entry for ${metadata.assetName}`);
  }
  const indexSha256 = crypto.createHash("sha256").update(indexBytes).digest("hex");
  if (
    options.expectedReleaseIndexSha256 &&
    !timingSafeHexEqual(indexSha256, options.expectedReleaseIndexSha256)
  ) {
    throw new Error("signed release index changed after the approved plan");
  }
  return { index, asset, indexBytes, signatureBytes, indexSha256 };
}

async function acquireInstallLock(lockPath, { clock = Date.now, sleep = defaultSleep } = {}) {
  const deadline = nowMillis(clock) + DOWNLOAD_TIMEOUT_MS;
  fs.mkdirSync(path.dirname(lockPath), { recursive: true, mode: 0o700 });
  while (true) {
    try {
      const descriptor = fs.openSync(lockPath, "wx", 0o600);
      return () => {
        fs.closeSync(descriptor);
        fs.rmSync(lockPath, { force: true });
      };
    } catch (error) {
      if (error.code !== "EEXIST" || nowMillis(clock) >= deadline) {
        throw new Error(`cannot acquire launcher cache lock: ${error.message}`);
      }
      await sleep(100);
    }
  }
}

function inspectCache(versionDir, { binaryName, assetName, assetSha256, version, triple } = {}) {
  if (!pathExists(versionDir)) return null;
  try {
    assertDirectory(versionDir, "version cache directory");
    const binaryPath = path.join(versionDir, binaryName);
    const runtimeRoot = path.join(versionDir, "runtime");
    const markerPath = path.join(versionDir, ".verified-sha256");
    if (!isRegularFile(binaryPath) || !isRegularFile(markerPath) || !isDirectory(runtimeRoot)) {
      return null;
    }
    const marker = parseMarker(fs.readFileSync(markerPath, "utf8"));
    if (assetName && marker.assetName !== assetName) return null;
    if (assetSha256 && marker.assetSha256 !== assetSha256) return null;
    assertReleasePayload(binaryPath, runtimeRoot);
    const binarySha256 = sha256File(binaryPath);
    const runtimeSha256 = sha256Directory(runtimeRoot);
    if (marker.binarySha256 !== binarySha256 || marker.runtimeSha256 !== runtimeSha256) {
      return null;
    }
    return {
      assetName: marker.assetName,
      assetSha256: marker.assetSha256,
      binarySha256,
      runtimeSha256,
      releaseIndexSha256: marker.releaseIndexSha256,
      version: version || null,
      triple: triple || null,
      binaryPath,
      runtimeRoot,
      markerPath
    };
  } catch {
    return null;
  }
}

function parseMarker(text) {
  const lines = text.split(/\r?\n/u);
  if (lines.at(-1) === "") lines.pop();
  if (lines.length !== 5 || !lines[0] || !lines.slice(1).every((line) => HASH_PATTERN.test(line))) {
    throw new Error("invalid launcher verification marker");
  }
  return {
    assetName: lines[0],
    assetSha256: lines[1],
    binarySha256: lines[2],
    runtimeSha256: lines[3],
    releaseIndexSha256: lines[4]
  };
}

function cacheIsVerified(binaryPath, runtimeRoot, markerPath, assetName) {
  const versionDir = path.dirname(binaryPath);
  const cached = inspectCache(versionDir, {
    binaryName: path.basename(binaryPath),
    assetName
  });
  return Boolean(
    cached &&
      cached.runtimeRoot === runtimeRoot &&
      cached.markerPath === markerPath
  );
}

function assertReleasePayload(binaryPath, runtimeRoot) {
  if (!isRegularFile(binaryPath)) {
    throw new Error("release archive does not contain the AGS binary");
  }
  if (!isDirectory(runtimeRoot)) {
    throw new Error("release archive does not contain the public AGS runtime profile");
  }
  for (const relative of REQUIRED_RUNTIME_FILES) {
    if (!isRegularFile(path.join(runtimeRoot, relative))) {
      throw new Error(`release archive is missing runtime/${relative}`);
    }
  }
}

function pointerFor(metadata, identity, clock) {
  return {
    schema_version: 1,
    version: metadata.version,
    triple: metadata.triple,
    binary_name: metadata.platform === "win32" ? "ags.exe" : "ags",
    asset_name: identity.assetName,
    asset_sha256: identity.assetSha256,
    binary_sha256: identity.binarySha256,
    runtime_sha256: identity.runtimeSha256,
    release_index_sha256: identity.releaseIndexSha256,
    activated_at: new Date(nowMillis(clock)).toISOString()
  };
}

function readPointer(file) {
  return readJsonIfPresent(file);
}

function validatePointer(pointer, paths) {
  if (
    !pointer ||
    pointer.schema_version !== 1 ||
    typeof pointer.version !== "string" ||
    typeof pointer.triple !== "string" ||
    typeof pointer.binary_name !== "string" ||
    typeof pointer.asset_name !== "string" ||
    !HASH_PATTERN.test(pointer.asset_sha256 || "") ||
    !HASH_PATTERN.test(pointer.binary_sha256 || "") ||
    !HASH_PATTERN.test(pointer.runtime_sha256 || "") ||
    !HASH_PATTERN.test(pointer.release_index_sha256 || "")
  ) {
    throw new Error("current launcher pointer is invalid");
  }
  assertVersion(pointer.version, "current launcher pointer version");
  if (!TRIPLE_PATTERN.test(pointer.triple) || pointer.triple !== paths.triple) {
    throw new Error("current launcher pointer targets an invalid platform");
  }
  if (pointer.binary_name !== paths.binaryPath.split(path.sep).at(-1)) {
    throw new Error("current launcher pointer targets an invalid binary");
  }
  const extension = paths.extension;
  const expectedAsset = `ags-v${pointer.version}-${pointer.triple}.${extension}`;
  if (pointer.asset_name !== expectedAsset) {
    throw new Error("current launcher pointer asset does not match its version");
  }
  const versionDir = path.join(paths.versionsRoot, pointer.version, pointer.triple);
  const identity = inspectCache(versionDir, {
    binaryName: pointer.binary_name,
    assetName: pointer.asset_name,
    assetSha256: pointer.asset_sha256,
    version: pointer.version,
    triple: pointer.triple
  });
  if (
    !identity ||
    identity.binarySha256 !== pointer.binary_sha256 ||
    identity.runtimeSha256 !== pointer.runtime_sha256 ||
    identity.releaseIndexSha256 !== pointer.release_index_sha256
  ) {
    throw new Error("current launcher pointer does not match verified cache content");
  }
  return {
    ...identity,
    version: pointer.version,
    triple: pointer.triple,
    binaryPath: path.join(versionDir, pointer.binary_name),
    runtimeRoot: path.join(versionDir, "runtime")
  };
}

function sameIdentity(left, right) {
  return (
    left.version === right.version &&
    left.triple === right.triple &&
    left.asset_name === right.asset_name &&
    left.asset_sha256 === right.asset_sha256 &&
    left.binary_sha256 === right.binary_sha256 &&
    left.runtime_sha256 === right.runtime_sha256 &&
    left.release_index_sha256 === right.release_index_sha256
  );
}

function ensureCacheDirectories(paths) {
  fs.mkdirSync(paths.cacheRoot, { recursive: true, mode: 0o700 });
  fs.mkdirSync(paths.stateRoot, { recursive: true, mode: 0o700 });
  fs.mkdirSync(paths.versionsRoot, { recursive: true, mode: 0o700 });
  assertDirectory(paths.cacheRoot, "AGS cache root");
  assertDirectory(paths.stateRoot, "launcher state directory");
  assertDirectory(paths.versionsRoot, "version cache root");
}

function normalizeMetadata(input) {
  const base = releaseMetadata({
    version: input?.version || packageJson.version,
    platform: input?.platform || process.platform,
    arch: input?.arch || process.arch,
    repository: input?.repository || RELEASE_REPOSITORY
  });
  const metadata = { ...base, ...(input || {}) };
  assertVersion(metadata.version, "release version");
  if (!TRIPLE_PATTERN.test(metadata.triple) || !/^(?:tar\.gz|zip)$/u.test(metadata.extension)) {
    throw new Error("invalid release metadata target");
  }
  const expectedAsset = `ags-v${metadata.version}-${metadata.triple}.${metadata.extension}`;
  if (metadata.assetName !== expectedAsset) {
    throw new Error("release metadata asset name does not match version and target");
  }
  const releaseBase = assertReleaseBaseUrl(metadata.releaseBase, metadata.version);
  const releaseIndexEndpoint = assertVersionedReleaseIndexUrl(
    metadata.releaseIndexEndpoint,
    releaseBase,
    "release index"
  );
  const releaseSignatureEndpoint = assertVersionedReleaseIndexUrl(
    metadata.releaseSignatureEndpoint,
    releaseBase,
    "release signature"
  );
  const updateEndpoint = assertUpdateEndpointUrl(metadata.updateEndpoint, "release index");
  const updateSignatureEndpoint = assertUpdateEndpointUrl(
    metadata.updateSignatureEndpoint,
    "release signature"
  );
  return Object.freeze({
    ...metadata,
    releaseBase,
    releaseIndexEndpoint,
    releaseSignatureEndpoint,
    updateEndpoint,
    updateSignatureEndpoint
  });
}

function approvedGitHubUrl(value, label) {
  let url;
  try {
    url = new URL(value);
  } catch {
    throw new Error(`invalid ${label}: URL is malformed`);
  }
  const hostname = url.hostname.toLowerCase();
  if (url.protocol !== "https:") {
    throw new Error(`${label} must use HTTPS`);
  }
  if (!APPROVED_RELEASE_HOSTS.has(hostname)) {
    throw new Error(`${label} must use an approved GitHub release host`);
  }
  if (url.username || url.password || (url.port && url.port !== "443")) {
    throw new Error(`${label} contains disallowed URL authority`);
  }
  if (url.hash) {
    throw new Error(`${label} must not contain a URL fragment`);
  }
  return url;
}

function assertReleaseBaseUrl(value, version) {
  const url = approvedGitHubUrl(value, "releaseBase");
  const parts = url.pathname.split("/").filter(Boolean);
  if (
    url.hostname.toLowerCase() !== "github.com" ||
    parts.length !== 5 ||
    parts[2] !== "releases" ||
    parts[3] !== "download" ||
    parts[4] !== `v${version}` ||
    url.search
  ) {
    throw new Error("releaseBase must be an approved GitHub release download URL");
  }
  return url.toString().replace(/\/$/u, "");
}

function assertVersionedReleaseIndexUrl(value, releaseBase, label) {
  const url = approvedGitHubUrl(value, label);
  const expected = `${releaseBase}/${label === "release signature" ? "release-index.sig" : "release-index.json"}`;
  if (url.toString() !== expected) {
    throw new Error(`${label} must belong to the exact versioned release`);
  }
  return url.toString();
}

function assertUpdateEndpointUrl(value, label = "release index") {
  const url = approvedGitHubUrl(value, label);
  const parts = url.pathname.split("/").filter(Boolean);
  const expectedFile = label === "release signature" ? "release-index.sig" : "release-index.json";
  if (
    url.hostname.toLowerCase() !== "github.com" ||
    parts.length !== 6 ||
    parts[0] !== "FernandeZ-hjm" ||
    parts[1] !== "Agent-General-Staff" ||
    parts[2] !== "releases" ||
    parts[3] !== "latest" ||
    parts[4] !== "download" ||
    parts[5] !== expectedFile ||
    url.search
  ) {
    throw new Error(`${label} must be the approved GitHub latest-release asset URL`);
  }
  return url.toString();
}

function assertVersion(value, label) {
  if (typeof value !== "string" || !VERSION_PATTERN.test(value)) {
    throw new Error(`${label} is invalid`);
  }
}

function resolveCacheRoot(cacheRoot) {
  return path.resolve(cacheRoot || path.join(os.homedir(), ".ags"));
}

function updateStatePath(stateRootOrPath) {
  const value = stateRootOrPath || path.join(os.homedir(), ".ags", "launcher-state");
  return [UPDATE_STATE_FILE, "update-state.json"].includes(path.basename(value))
    ? path.resolve(value)
    : path.join(path.resolve(value), UPDATE_STATE_FILE);
}

function normalizeUpdateState(raw) {
  const ignored = Array.isArray(raw?.ignored_versions)
    ? [...new Set(raw.ignored_versions.filter((version) => typeof version === "string" && VERSION_PATTERN.test(version)))]
    : [];
  const snoozed = unixSeconds(raw?.snoozed_until_unix);
  const lastChecked = unixSeconds(raw?.last_checked_at_unix);
  return {
    schema_version: UPDATE_CHECK_STATE_SCHEMA,
    enabled: typeof raw?.enabled === "boolean" ? raw.enabled : true,
    last_checked_at_unix: lastChecked,
    ignored_versions: ignored,
    snoozed_until_unix: snoozed,
    channel: normalizeChannel(raw?.channel),
    latest_version:
      typeof raw?.latest_version === "string" && VERSION_PATTERN.test(raw.latest_version)
        ? raw.latest_version
        : null,
    release_index_hash: HASH_PATTERN.test(raw?.release_index_hash || "")
      ? raw.release_index_hash
      : null,
    catalog_release:
      typeof raw?.catalog_release === "string" && VERSION_PATTERN.test(raw.catalog_release)
        ? raw.catalog_release
        : null,
    catalog_hash: HASH_PATTERN.test(raw?.catalog_hash || "") ? raw.catalog_hash : null,
    last_error: typeof raw?.last_error === "string" ? raw.last_error : null
  };
}

function normalizeChannel(channel) {
  return CHANNELS.has(channel) ? channel : "stable";
}

function unixSeconds(value) {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0) return null;
  return value;
}

function writeUpdateStateBestEffort(stateRoot, state) {
  try {
    writeUpdateState(stateRoot, state);
  } catch {
    // Update state is advisory. A read-only or full disk home must not stop AGS.
  }
}

async function fetchReleaseIndexBytes(url, { fetchImpl, timeoutMs, label }) {
  if (typeof fetchImpl !== "function") throw new Error("fetch is unavailable");
  let currentUrl = assertUpdateEndpointUrl(url, label);
  for (let redirects = 0; redirects <= 5; redirects += 1) {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), timeoutMs);
    timer.unref?.();
    try {
      const response = await fetchImpl(currentUrl, {
        redirect: "manual",
        headers: {
          accept: "application/octet-stream",
          "user-agent": `@agent-governance-suite/mcp/${packageJson.version}`
        },
        signal: controller.signal
      });
      const status = Number(response?.status);
      if (status >= 300 && status < 400) {
        const location = response.headers?.get?.("location") || response.headers?.location;
        if (!location) throw new Error(`update check redirect has no location: ${currentUrl}`);
        if (redirects === 5) throw new Error(`too many redirects checking ${url}`);
        currentUrl = approvedGitHubUrl(
          new URL(location, currentUrl).toString(),
          "update redirect URL"
        ).toString();
        continue;
      }
      if (status !== 200) {
        throw new Error(`update check failed (${status})`);
      }
      const body = response.arrayBuffer ? Buffer.from(await response.arrayBuffer()) : await readResponseBody(response);
      if (body.length > 256 * 1024) throw new Error(`${label} exceeds 256 KiB`);
      return body;
    } catch (error) {
      if (controller.signal.aborted) throw new Error("update check timed out");
      throw error;
    } finally {
      clearTimeout(timer);
    }
  }
  throw new Error(`too many redirects checking ${url}`);
}

async function fetchApprovedAssetBytes(url, { fetchImpl, timeoutMs, label, maxBytes }) {
  if (typeof fetchImpl !== "function") throw new Error("fetch is unavailable");
  let currentUrl = approvedGitHubUrl(url, `${label} URL`).toString();
  for (let redirects = 0; redirects <= 5; redirects += 1) {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), timeoutMs);
    timer.unref?.();
    try {
      const response = await fetchImpl(currentUrl, {
        redirect: "manual",
        headers: {
          accept: "application/octet-stream",
          "user-agent": `@agent-governance-suite/mcp/${packageJson.version}`
        },
        signal: controller.signal
      });
      const status = Number(response?.status);
      if (status >= 300 && status < 400) {
        const location = response.headers?.get?.("location") || response.headers?.location;
        if (!location) throw new Error(`${label} redirect has no location`);
        if (redirects === 5) throw new Error(`too many redirects fetching ${label}`);
        currentUrl = approvedGitHubUrl(
          new URL(location, currentUrl).toString(),
          `${label} redirect URL`
        ).toString();
        continue;
      }
      if (status !== 200) throw new Error(`${label} fetch failed (${status})`);
      const length = Number(response.headers?.get?.("content-length"));
      if (Number.isFinite(length) && length > maxBytes) {
        throw new Error(`${label} exceeds ${maxBytes} bytes`);
      }
      const body = response.arrayBuffer
        ? Buffer.from(await response.arrayBuffer())
        : await readResponseBody(response);
      if (body.length > maxBytes) throw new Error(`${label} exceeds ${maxBytes} bytes`);
      return body;
    } catch (error) {
      if (controller.signal.aborted) throw new Error(`${label} fetch timed out`);
      throw error;
    } finally {
      clearTimeout(timer);
    }
  }
  throw new Error(`too many redirects fetching ${label}`);
}

export function verifySignedReleaseIndex(indexBytes, signatureBytes) {
  const signatureText = Buffer.from(signatureBytes).toString("utf8").trim();
  if (!/^[A-Za-z0-9+/]{86}==$/u.test(signatureText)) return false;
  const signature = Buffer.from(signatureText, "base64");
  if (signature.length !== 64 || signature.toString("base64") !== signatureText) return false;
  return crypto.verify(null, Buffer.from(indexBytes), RELEASE_SIGNING_PUBLIC_KEY_PEM, signature);
}

export function parseSignedReleaseIndex(indexBytes, expectedChannel = "stable") {
  let index;
  try {
    index = JSON.parse(Buffer.from(indexBytes).toString("utf8"));
  } catch {
    throw new Error("signed release index is not valid JSON");
  }
  if (
    index?.schema_version !== "1.0-signed-release-index" ||
    !VERSION_PATTERN.test(index?.version || "") ||
    index?.channel !== expectedChannel ||
    index?.repository !== RELEASE_REPOSITORY ||
    index?.tag !== `v${index.version}` ||
    !/^[a-f0-9]{40}$/u.test(index?.commit || "") ||
    !Array.isArray(index?.assets) ||
    index.assets.length === 0
  ) {
    throw new Error("signed release index identity is invalid");
  }
  const names = new Set();
  for (const asset of index.assets) {
    if (
      typeof asset?.name !== "string" ||
      asset.name.length === 0 ||
      asset.name.includes("/") ||
      asset.name.includes("\\") ||
      names.has(asset.name) ||
      !HASH_PATTERN.test(asset?.sha256 || "")
    ) {
      throw new Error("signed release index asset identity is invalid");
    }
    names.add(asset.name);
  }
  if (index.catalog !== undefined) {
    const expectedCatalog = `ags-third-party-catalog-v${index.version}.yaml`;
    if (
      index.catalog?.name !== expectedCatalog ||
      index.catalog.name.includes("/") ||
      index.catalog.name.includes("\\") ||
      !HASH_PATTERN.test(index.catalog?.sha256 || "")
    ) {
      throw new Error("signed release index catalog identity is invalid");
    }
  }
  return index;
}

async function refreshVerifiedCatalog({ payload, indexEndpoint, stateRoot, fetchImpl, timeoutMs }) {
  if (!payload.catalog) return null;
  const catalogUrl = new URL(payload.catalog.name, indexEndpoint).toString();
  const bytes = await fetchApprovedAssetBytes(catalogUrl, {
    fetchImpl,
    timeoutMs,
    label: "signed catalog",
    maxBytes: 1024 * 1024
  });
  const observed = crypto.createHash("sha256").update(bytes).digest("hex");
  if (!timingSafeHexEqual(observed, payload.catalog.sha256)) {
    throw new Error("signed catalog hash mismatch");
  }
  const catalogRoot = path.join(stateRoot, "catalog");
  fs.mkdirSync(catalogRoot, { recursive: true, mode: 0o700 });
  assertDirectory(catalogRoot, "verified catalog cache");
  const catalogFile = `third-party-capabilities-${observed}.yaml`;
  const catalogPath = path.join(catalogRoot, catalogFile);
  atomicWriteBytes(catalogPath, bytes);
  const marker = {
    schema_version: VERIFIED_CATALOG_SCHEMA,
    release: payload.version,
    content_hash: `sha256:${observed}`,
    catalog_file: catalogFile
  };
  atomicWriteJson(path.join(catalogRoot, "current.json"), marker);
  return marker;
}

function parseVersion(version) {
  const [build, prereleaseText] = version.split("-", 2);
  const numbers = build.split(".").map((part) => Number.parseInt(part, 10));
  return { numbers, prerelease: prereleaseText ? prereleaseText.split(".") : [] };
}

function compareVersions(left, right) {
  for (let index = 0; index < 3; index += 1) {
    if (left.numbers[index] !== right.numbers[index]) return left.numbers[index] - right.numbers[index];
  }
  if (left.prerelease.length === 0 && right.prerelease.length > 0) return 1;
  if (left.prerelease.length > 0 && right.prerelease.length === 0) return -1;
  return left.prerelease.join(".").localeCompare(right.prerelease.join("."));
}

function defaultUpdateNotice(update) {
  const suffix = update.url ? `: ${update.url}` : "";
  process.stderr.write(
    `ags: update available ${update.current_version} -> ${update.version} (${update.channel})${suffix}\n`
  );
}

function collectDirectoryRecords(directory, relative, records) {
  if (!isDirectory(directory)) throw new Error(`runtime payload is not a directory: ${directory}`);
  const entries = fs.readdirSync(directory).sort((left, right) => left.localeCompare(right));
  for (const name of entries) {
    const absolute = path.join(directory, name);
    const childRelative = relative ? `${relative}/${name}` : name;
    const stats = fs.lstatSync(absolute);
    if (stats.isSymbolicLink()) throw new Error(`runtime payload contains symlink: ${childRelative}`);
    if (stats.isDirectory()) {
      collectDirectoryRecords(absolute, childRelative, records);
    } else if (stats.isFile()) {
      records.push({ relative: childRelative, size: stats.size, sha256: sha256File(absolute) });
    } else {
      throw new Error(`runtime payload contains unsupported entry: ${childRelative}`);
    }
  }
}

function isDirectory(file) {
  try {
    const stats = fs.lstatSync(file);
    return stats.isDirectory() && !stats.isSymbolicLink();
  } catch {
    return false;
  }
}

function isRegularFile(file) {
  try {
    const stats = fs.lstatSync(file);
    return stats.isFile() && !stats.isSymbolicLink();
  } catch {
    return false;
  }
}

function assertDirectory(directory, label) {
  let stats;
  try {
    stats = fs.lstatSync(directory);
  } catch (error) {
    throw new Error(`${label} is unavailable: ${error.message}`);
  }
  if (!stats.isDirectory() || stats.isSymbolicLink()) {
    throw new Error(`${label} must be a real directory`);
  }
}

function pathExists(file) {
  try {
    fs.lstatSync(file);
    return true;
  } catch (error) {
    if (error.code === "ENOENT") return false;
    throw error;
  }
}

function readJsonIfPresent(file) {
  try {
    const stats = fs.lstatSync(file);
    if (stats.isSymbolicLink() || !stats.isFile()) throw new Error(`state file is not a regular file: ${file}`);
    return JSON.parse(fs.readFileSync(file, "utf8"));
  } catch (error) {
    if (error.code === "ENOENT") return null;
    if (error.message?.startsWith("state file")) throw error;
    throw new Error(`invalid JSON state file ${file}: ${error.message}`);
  }
}

function atomicWriteJson(file, value) {
  const temporary = `${file}.tmp-${process.pid}-${crypto.randomUUID()}`;
  fs.writeFileSync(temporary, `${JSON.stringify(value, null, 2)}\n`, { mode: 0o600, flag: "wx" });
  const descriptor = fs.openSync(temporary, "r");
  try {
    fs.fsyncSync(descriptor);
  } finally {
    fs.closeSync(descriptor);
  }
  try {
    fs.renameSync(temporary, file);
  } catch (error) {
    if (process.platform !== "win32" || !["EEXIST", "EPERM"].includes(error.code)) throw error;
    fs.rmSync(file, { force: true });
    fs.renameSync(temporary, file);
  }
}

function atomicWriteBytes(file, bytes) {
  const temporary = `${file}.tmp-${process.pid}-${crypto.randomUUID()}`;
  fs.writeFileSync(temporary, bytes, { mode: 0o600, flag: "wx" });
  const descriptor = fs.openSync(temporary, "r");
  try {
    fs.fsyncSync(descriptor);
  } finally {
    fs.closeSync(descriptor);
  }
  try {
    fs.renameSync(temporary, file);
  } catch (error) {
    if (process.platform !== "win32" || !["EEXIST", "EPERM"].includes(error.code)) throw error;
    fs.rmSync(file, { force: true });
    fs.renameSync(temporary, file);
  }
}

function timingSafeHexEqual(left, right) {
  if (!HASH_PATTERN.test(left) || !HASH_PATTERN.test(right)) return false;
  return crypto.timingSafeEqual(Buffer.from(left, "hex"), Buffer.from(right, "hex"));
}

function writeArchiveFile(output, content, binaryName) {
  fs.mkdirSync(path.dirname(output), { recursive: true, mode: 0o700 });
  fs.writeFileSync(output, content, {
    flag: "wx",
    mode: path.basename(output) === binaryName ? 0o700 : 0o600
  });
}

function extractStoredZip(archivePath, destination, binaryName) {
  const data = fs.readFileSync(archivePath);
  const eocd = findZipEocd(data);
  const entries = data.readUInt16LE(eocd + 10);
  let offset = data.readUInt32LE(eocd + 16);
  if (offset + entries * 46 > eocd) throw new Error("invalid zip central directory");
  const seenNames = new Set();
  for (let index = 0; index < entries; index += 1) {
    if (offset + 46 > data.length || data.readUInt32LE(offset) !== 0x02014b50) {
      throw new Error("invalid zip central directory");
    }
    const centralEnd = offset + 46 + data.readUInt16LE(offset + 28) + data.readUInt16LE(offset + 30) + data.readUInt16LE(offset + 32);
    if (centralEnd > eocd) throw new Error("invalid zip central directory entry");
    const flags = data.readUInt16LE(offset + 8);
    const method = data.readUInt16LE(offset + 10);
    const compressedSize = data.readUInt32LE(offset + 20);
    const uncompressedSize = data.readUInt32LE(offset + 24);
    const nameLength = data.readUInt16LE(offset + 28);
    const extraLength = data.readUInt16LE(offset + 30);
    const commentLength = data.readUInt16LE(offset + 32);
    const localOffset = data.readUInt32LE(offset + 42);
    const name = data.subarray(offset + 46, offset + 46 + nameLength).toString("utf8");
    if (seenNames.has(name)) throw new Error(`duplicate zip entry: ${name}`);
    seenNames.add(name);
    if (
      (flags & 1) !== 0 ||
      compressedSize > MAX_DOWNLOAD_BYTES ||
      uncompressedSize > MAX_DOWNLOAD_BYTES
    ) {
      throw new Error(`invalid or oversized zip entry: ${name}`);
    }
    if (localOffset + 30 > data.length || data.readUInt32LE(localOffset) !== 0x04034b50) {
      throw new Error("invalid zip local header");
    }
    const localFlags = data.readUInt16LE(localOffset + 6);
    const localMethod = data.readUInt16LE(localOffset + 8);
    const localNameLength = data.readUInt16LE(localOffset + 26);
    const localExtraLength = data.readUInt16LE(localOffset + 28);
    const localNameStart = localOffset + 30;
    const localNameEnd = localNameStart + localNameLength;
    if (localNameEnd + localExtraLength > data.length) {
      throw new Error(`truncated zip local header: ${name}`);
    }
    const localName = data.subarray(localNameStart, localNameEnd).toString("utf8");
    if (localName !== name || localFlags !== flags || localMethod !== method) {
      throw new Error(`zip local/central entry identity mismatch: ${name}`);
    }
    if ((flags & 8) === 0) {
      const localCompressedSize = data.readUInt32LE(localOffset + 18);
      const localUncompressedSize = data.readUInt32LE(localOffset + 22);
      if (localCompressedSize !== compressedSize || localUncompressedSize !== uncompressedSize) {
        throw new Error(`zip local/central entry identity mismatch: ${name}`);
      }
    }
    const start = localOffset + 30 + localNameLength + localExtraLength;
    if (start + compressedSize > data.length) throw new Error(`truncated zip entry: ${name}`);
    const madeByPlatform = data.readUInt16LE(offset + 4) >>> 8;
    const externalAttributes = data.readUInt32LE(offset + 38);
    const unixMode = madeByPlatform === 3 ? (externalAttributes >>> 16) & 0xffff : 0;
    const unixType = unixMode & 0xf000;
    if (unixType === 0xa000) {
      throw new Error(`archive contains symlink payload: ${name}`);
    }
    if (unixType !== 0 && unixType !== 0x4000 && unixType !== 0x8000) {
      throw new Error(`archive contains special payload: ${name}`);
    }
    const directory =
      name.endsWith("/") ||
      unixType === 0x4000 ||
      (madeByPlatform !== 3 && (externalAttributes & 0x10) !== 0);
    const output = archiveOutputOrThrow(destination, name, binaryName);
    if (directory) {
      if (compressedSize !== 0 || uncompressedSize !== 0) {
        throw new Error(`zip directory has a payload: ${name}`);
      }
      if (output === path.join(destination, binaryName)) {
        throw new Error(`archive contains invalid executable directory payload: ${name}`);
      }
      fs.mkdirSync(output, { recursive: true, mode: 0o700 });
    } else {
      if (output === path.resolve(destination, "runtime")) {
        throw new Error(`archive contains invalid regular directory payload: ${name}`);
      }
      const body = data.subarray(start, start + compressedSize);
      const content =
        method === 0
          ? body
          : method === 8
            ? zlib.inflateRawSync(body, { maxOutputLength: MAX_DOWNLOAD_BYTES })
            : null;
      if (!content) throw new Error(`unsupported zip compression method: ${method}`);
      if (content.length !== uncompressedSize) {
        throw new Error(`zip size mismatch for ${name}`);
      }
      writeArchiveFile(output, content, binaryName);
    }
    offset += 46 + nameLength + extraLength + commentLength;
  }
}

function findZipEocd(data) {
  const minimum = Math.max(0, data.length - 65_557);
  for (let offset = data.length - 22; offset >= minimum; offset -= 1) {
    if (offset >= 0 && data.readUInt32LE(offset) === 0x06054b50) {
      return offset;
    }
  }
  throw new Error("zip end-of-central-directory record not found");
}

async function waitForChild(child) {
  return new Promise((resolve, reject) => {
    let settled = false;
    const signalHandlers = ["SIGINT", "SIGTERM"].map((signal) => {
      const handler = () => child.kill?.(signal);
      process.on(signal, handler);
      return [signal, handler];
    });
    const cleanup = () => {
      for (const [signal, handler] of signalHandlers) process.off(signal, handler);
    };
    const finish = (callback) => (value) => {
      if (settled) return;
      settled = true;
      cleanup();
      callback(value);
    };
    child.once("error", finish(reject));
    child.once(
      "exit",
      finish((code, signal) => resolve(signal ? 1 : code ?? 1))
    );
  });
}

async function readResponseBody(response) {
  if (response?.body?.[Symbol.asyncIterator]) {
    const chunks = [];
    for await (const chunk of response.body) chunks.push(Buffer.from(chunk));
    return Buffer.concat(chunks);
  }
  throw new Error("download response has no readable body");
}

function nowMillis(clock) {
  const value = typeof clock === "function" ? clock() : clock ?? Date.now();
  return value instanceof Date ? value.getTime() : Number(value);
}

function defaultSleep(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}
