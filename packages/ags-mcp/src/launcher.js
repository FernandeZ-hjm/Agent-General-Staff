import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";
import http from "node:http";
import https from "node:https";
import zlib from "node:zlib";

const require = createRequire(import.meta.url);
const packageJson = require("../package.json");
const RELEASE_REPOSITORY = "FernandeZ-hjm/Agent-General-Staff";
const MAX_DOWNLOAD_BYTES = 128 * 1024 * 1024;

export function releaseTarget(platform = process.platform, arch = process.arch) {
  const key = `${platform}-${arch}`;
  const targets = {
    "darwin-arm64": ["aarch64-apple-darwin", "tar.gz"],
    "darwin-x64": ["x86_64-apple-darwin", "tar.gz"],
    "linux-arm64": ["aarch64-unknown-linux-gnu", "tar.gz"],
    "linux-x64": ["x86_64-unknown-linux-gnu", "tar.gz"],
    "win32-x64": ["x86_64-pc-windows-msvc", "zip"]
  };
  const target = targets[key];
  if (!target) {
    throw new Error(`unsupported platform: ${key}`);
  }
  return { triple: target[0], extension: target[1] };
}

export function parseExpectedChecksum(text, assetName) {
  for (const line of text.split(/\r?\n/u)) {
    const match = line.trim().match(/^([a-fA-F0-9]{64})\s+\*?(.+)$/u);
    if (match && match[2] === assetName) {
      return match[1].toLowerCase();
    }
  }
  throw new Error(`SHA256SUMS has no entry for ${assetName}`);
}

export function sha256File(file) {
  const hash = crypto.createHash("sha256");
  hash.update(fs.readFileSync(file));
  return hash.digest("hex");
}

export async function launch() {
  const version = packageJson.version;
  const { triple, extension } = releaseTarget();
  const assetName = `ags-v${version}-${triple}.${extension}`;
  const releaseBase = `https://github.com/${RELEASE_REPOSITORY}/releases/download/v${version}`;
  const cacheRoot =
    process.env.AGS_CACHE_DIR ||
    path.join(
      process.env.XDG_CACHE_HOME || path.join(os.homedir(), ".cache"),
      "ags",
      "mcp"
    );
  const versionDir = path.join(cacheRoot, version, triple);
  const binaryName = process.platform === "win32" ? "ags.exe" : "ags";
  const binaryPath = path.join(versionDir, binaryName);
  const sourceRoot = path.join(versionDir, "runtime");
  const runtimeHome =
    process.env.AGS_RUNTIME_HOME ||
    process.env.AGS_HOME ||
    path.join(os.homedir(), ".ags", "runtime");
  const verifiedMarker = path.join(versionDir, ".verified-sha256");

  if (!cacheIsVerified(binaryPath, sourceRoot, verifiedMarker, assetName)) {
    fs.mkdirSync(versionDir, { recursive: true, mode: 0o700 });
    const releaseLock = await acquireInstallLock(path.join(versionDir, ".install.lock"));
    try {
      if (!cacheIsVerified(binaryPath, sourceRoot, verifiedMarker, assetName)) {
        const workDir = fs.mkdtempSync(path.join(versionDir, ".download-"));
        try {
          const sumsPath = path.join(workDir, "SHA256SUMS");
          const assetPath = path.join(workDir, assetName);
          await download(`${releaseBase}/SHA256SUMS`, sumsPath);
          await download(`${releaseBase}/${assetName}`, assetPath);
          const expected = parseExpectedChecksum(fs.readFileSync(sumsPath, "utf8"), assetName);
          const actual = sha256File(assetPath);
          if (!crypto.timingSafeEqual(Buffer.from(actual), Buffer.from(expected))) {
            throw new Error(`checksum mismatch for ${assetName}`);
          }
          extractArchive(assetPath, extension, workDir, binaryName);
          const extracted = path.join(workDir, binaryName);
          if (!fs.existsSync(extracted)) {
            throw new Error(`release archive does not contain ${binaryName}`);
          }
          const extractedRuntime = path.join(workDir, "runtime");
          if (
            !fs.existsSync(path.join(extractedRuntime, "manifests", "skills-registry.yaml")) ||
            !fs.existsSync(path.join(extractedRuntime, "manifests", "mcp-registry.yaml")) ||
            !fs.existsSync(path.join(extractedRuntime, "protocol", "agent-task-protocol.md"))
          ) {
            throw new Error("release archive does not contain the public AGS runtime profile");
          }
          if (process.platform !== "win32") {
            fs.chmodSync(extracted, 0o755);
          }
          atomicReplace(extracted, binaryPath);
          if (fs.existsSync(sourceRoot)) {
            fs.rmSync(sourceRoot, { recursive: true, force: true });
          }
          fs.renameSync(extractedRuntime, sourceRoot);
          fs.writeFileSync(
            verifiedMarker,
            `${assetName}\n${expected}\n${sha256File(binaryPath)}\n`,
            { mode: 0o600 }
          );
        } finally {
          fs.rmSync(workDir, { recursive: true, force: true });
        }
      }
    } finally {
      releaseLock();
    }
  }

  const child = spawn(binaryPath, ["mcp", "serve", "--transport", "stdio"], {
    stdio: "inherit",
    windowsHide: true,
    shell: false,
    env: {
      ...process.env,
      AGS_SOURCE_ROOT: sourceRoot,
      AGS_RUNTIME_HOME: runtimeHome,
      AGS_HOME: runtimeHome
    }
  });
  child.on("error", (error) => {
    throw error;
  });
  const signalHandlers = ["SIGINT", "SIGTERM"].map((signal) => {
    const handler = () => child.kill(signal);
    process.on(signal, handler);
    return [signal, handler];
  });
  const exitCode = await new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("exit", (code, signal) => {
      if (signal) {
        resolve(1);
      } else {
        resolve(code ?? 1);
      }
    });
  });
  for (const [signal, handler] of signalHandlers) {
    process.off(signal, handler);
  }
  process.exitCode = exitCode;
}

async function acquireInstallLock(lockPath) {
  const deadline = Date.now() + 30_000;
  while (true) {
    try {
      const descriptor = fs.openSync(lockPath, "wx", 0o600);
      return () => {
        fs.closeSync(descriptor);
        fs.rmSync(lockPath, { force: true });
      };
    } catch (error) {
      if (error.code !== "EEXIST" || Date.now() >= deadline) {
        throw new Error(`cannot acquire launcher cache lock: ${error.message}`);
      }
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
  }
}

function cacheIsVerified(binaryPath, sourceRoot, markerPath, assetName) {
  if (
    !fs.existsSync(binaryPath) ||
    !fs.existsSync(markerPath) ||
    !fs.existsSync(path.join(sourceRoot, "manifests", "skills-registry.yaml")) ||
    !fs.existsSync(path.join(sourceRoot, "manifests", "mcp-registry.yaml")) ||
    !fs.existsSync(path.join(sourceRoot, "protocol", "agent-task-protocol.md"))
  ) {
    return false;
  }
  const lines = fs.readFileSync(markerPath, "utf8").trim().split(/\r?\n/u);
  return (
    lines.length === 3 &&
    lines[0] === assetName &&
    /^[a-f0-9]{64}$/u.test(lines[1]) &&
    /^[a-f0-9]{64}$/u.test(lines[2]) &&
    sha256File(binaryPath) === lines[2]
  );
}

function atomicReplace(source, destination) {
  const temporary = `${destination}.tmp-${process.pid}`;
  fs.copyFileSync(source, temporary, fs.constants.COPYFILE_EXCL);
  if (process.platform !== "win32") {
    fs.chmodSync(temporary, 0o755);
  }
  if (fs.existsSync(destination)) {
    fs.rmSync(destination, { force: true });
  }
  fs.renameSync(temporary, destination);
}

function download(url, destination, redirects = 0) {
  if (redirects > 5) {
    return Promise.reject(new Error(`too many redirects downloading ${url}`));
  }
  const client = url.startsWith("https:") ? https : http;
  return new Promise((resolve, reject) => {
    const request = client.get(
      url,
      {
        headers: {
          "user-agent": `@agent-governance-suite/mcp/${packageJson.version}`,
          accept: "application/octet-stream"
        }
      },
      (response) => {
        if (
          response.statusCode >= 300 &&
          response.statusCode < 400 &&
          response.headers.location
        ) {
          response.resume();
          const next = new URL(response.headers.location, url).toString();
          resolve(download(next, destination, redirects + 1));
          return;
        }
        if (response.statusCode !== 200) {
          response.resume();
          reject(new Error(`download failed (${response.statusCode}): ${url}`));
          return;
        }
        let received = 0;
        const output = fs.createWriteStream(destination, { flags: "wx", mode: 0o600 });
        response.on("data", (chunk) => {
          received += chunk.length;
          if (received > MAX_DOWNLOAD_BYTES) {
            request.destroy(new Error(`download exceeds ${MAX_DOWNLOAD_BYTES} bytes`));
          }
        });
        response.pipe(output);
        output.once("finish", () => output.close(resolve));
        output.once("error", reject);
      }
    );
    request.setTimeout(30_000, () => request.destroy(new Error(`download timed out: ${url}`)));
    request.once("error", reject);
  });
}

function extractArchive(archivePath, extension, destination, binaryName) {
  if (extension === "zip") {
    extractStoredZip(archivePath, destination, binaryName);
    return;
  }
  const tar = zlib.gunzipSync(fs.readFileSync(archivePath), {
    maxOutputLength: MAX_DOWNLOAD_BYTES
  });
  let offset = 0;
  while (offset + 512 <= tar.length) {
    const header = tar.subarray(offset, offset + 512);
    if (header.every((byte) => byte === 0)) break;
    const name = header.subarray(0, 100).toString("utf8").replace(/\0.*$/u, "");
    const sizeText = header.subarray(124, 136).toString("ascii").replace(/\0.*$/u, "").trim();
    const size = Number.parseInt(sizeText || "0", 8);
    const type = header[156];
    offset += 512;
    if (type === 0 || type === 48) {
      const output = safeArchiveOutput(destination, name, binaryName);
      if (output) {
        fs.mkdirSync(path.dirname(output), { recursive: true, mode: 0o700 });
        fs.writeFileSync(output, tar.subarray(offset, offset + size), {
          mode: path.basename(name) === binaryName ? 0o700 : 0o600
        });
      }
    }
    offset += Math.ceil(size / 512) * 512;
  }
}

function extractStoredZip(archivePath, destination, binaryName) {
  const data = fs.readFileSync(archivePath);
  const eocd = findZipEocd(data);
  const entries = data.readUInt16LE(eocd + 10);
  let offset = data.readUInt32LE(eocd + 16);
  for (let index = 0; index < entries; index += 1) {
    if (offset + 46 > data.length || data.readUInt32LE(offset) !== 0x02014b50) {
      throw new Error("invalid zip central directory");
    }
    const method = data.readUInt16LE(offset + 10);
    const compressedSize = data.readUInt32LE(offset + 20);
    const uncompressedSize = data.readUInt32LE(offset + 24);
    const nameLength = data.readUInt16LE(offset + 28);
    const extraLength = data.readUInt16LE(offset + 30);
    const commentLength = data.readUInt16LE(offset + 32);
    const localOffset = data.readUInt32LE(offset + 42);
    const name = data.subarray(offset + 46, offset + 46 + nameLength).toString("utf8");
    if (uncompressedSize > MAX_DOWNLOAD_BYTES) {
      throw new Error(`zip entry exceeds ${MAX_DOWNLOAD_BYTES} bytes`);
    }
    if (localOffset + 30 > data.length || data.readUInt32LE(localOffset) !== 0x04034b50) {
      throw new Error("invalid zip local header");
    }
    const localNameLength = data.readUInt16LE(localOffset + 26);
    const localExtraLength = data.readUInt16LE(localOffset + 28);
    const start = localOffset + 30 + localNameLength + localExtraLength;
    const output = safeArchiveOutput(destination, name, binaryName);
    if (output) {
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
      fs.mkdirSync(path.dirname(output), { recursive: true, mode: 0o700 });
      fs.writeFileSync(output, content, {
        mode: path.basename(name) === binaryName ? 0o700 : 0o600
      });
    }
    offset += 46 + nameLength + extraLength + commentLength;
  }
}

function findZipEocd(data) {
  const minimum = Math.max(0, data.length - 65_557);
  for (let offset = data.length - 22; offset >= minimum; offset -= 1) {
    if (data.readUInt32LE(offset) === 0x06054b50) {
      return offset;
    }
  }
  throw new Error("zip end-of-central-directory record not found");
}

export function safeArchiveOutput(destination, entryName, binaryName) {
  const normalized = entryName.replaceAll("\\", "/").replace(/^\.\/+/u, "");
  if (normalized === binaryName) {
    return path.join(destination, binaryName);
  }
  if (
    !normalized.startsWith("runtime/") ||
    normalized.includes("../") ||
    path.isAbsolute(normalized)
  ) {
    return null;
  }
  const output = path.resolve(destination, normalized);
  const runtimeRoot = path.resolve(destination, "runtime");
  if (output !== runtimeRoot && !output.startsWith(`${runtimeRoot}${path.sep}`)) {
    throw new Error(`unsafe archive path: ${entryName}`);
  }
  return output;
}

export const __filename = fileURLToPath(import.meta.url);
