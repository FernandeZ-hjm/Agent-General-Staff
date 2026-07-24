import assert from "node:assert/strict";
import test from "node:test";
import path from "node:path";
import { parseExpectedChecksum, releaseTarget, safeArchiveOutput } from "../src/launcher.js";

test("maps every supported release platform", () => {
  assert.equal(releaseTarget("darwin", "arm64").triple, "aarch64-apple-darwin");
  assert.equal(releaseTarget("darwin", "x64").triple, "x86_64-apple-darwin");
  assert.equal(releaseTarget("linux", "arm64").triple, "aarch64-unknown-linux-gnu");
  assert.equal(releaseTarget("linux", "x64").triple, "x86_64-unknown-linux-gnu");
  assert.equal(releaseTarget("win32", "x64").triple, "x86_64-pc-windows-msvc");
  assert.throws(() => releaseTarget("freebsd", "x64"), /unsupported platform/u);
});

test("selects only the exact checksum asset", () => {
  const digest = "a".repeat(64);
  assert.equal(parseExpectedChecksum(`${digest}  ags-v0.3.0-test.tar.gz\n`, "ags-v0.3.0-test.tar.gz"), digest);
  assert.throws(() => parseExpectedChecksum(`${digest}  another.tar.gz\n`, "wanted.tar.gz"), /no entry/u);
});

test("extractor accepts only the binary and runtime subtree", () => {
  const root = path.resolve("/tmp/ags-launcher-test");
  assert.equal(safeArchiveOutput(root, "ags", "ags"), path.join(root, "ags"));
  assert.equal(
    safeArchiveOutput(root, "runtime/manifests/mcp-registry.yaml", "ags"),
    path.join(root, "runtime/manifests/mcp-registry.yaml")
  );
  assert.equal(safeArchiveOutput(root, "../../escape", "ags"), null);
  assert.equal(safeArchiveOutput(root, "other/file", "ags"), null);
});
