import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";

const version = process.env.AGS_RELEASE_VERSION;
const commit = process.env.AGS_RELEASE_COMMIT;
const repository = process.env.AGS_RELEASE_REPOSITORY;
const privateKey = process.env.AGS_RELEASE_SIGNING_PRIVATE_KEY;
const directory = path.resolve(process.argv[2] || "dist");

if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/u.test(version || "")) {
  throw new Error("AGS_RELEASE_VERSION is invalid");
}
if (!/^[a-f0-9]{40}$/u.test(commit || "")) {
  throw new Error("AGS_RELEASE_COMMIT must be the exact 40-character tag commit");
}
if (repository !== "FernandeZ-hjm/Agent-General-Staff") {
  throw new Error("AGS_RELEASE_REPOSITORY is not the public AGS repository");
}
if (!privateKey?.includes("BEGIN PRIVATE KEY")) {
  throw new Error("AGS_RELEASE_SIGNING_PRIVATE_KEY is unavailable or malformed");
}

const assetPattern = new RegExp(
  `^ags-v${version.replaceAll(".", "\\.")}-(?:aarch64-apple-darwin|x86_64-apple-darwin|aarch64-unknown-linux-gnu|x86_64-unknown-linux-gnu)\\.tar\\.gz$|^ags-v${version.replaceAll(".", "\\.")}-x86_64-pc-windows-msvc\\.zip$`,
  "u"
);
const names = fs.readdirSync(directory).filter((name) => assetPattern.test(name)).sort();
if (names.length !== 5) throw new Error(`expected 5 platform assets, observed ${names.length}`);
const assets = names.map((name) => ({
  name,
  sha256: crypto.createHash("sha256").update(fs.readFileSync(path.join(directory, name))).digest("hex")
}));
const catalogName = `ags-third-party-catalog-v${version}.yaml`;
const catalogPath = path.join(directory, catalogName);
if (!fs.statSync(catalogPath).isFile()) {
  throw new Error(`signed catalog asset is missing: ${catalogName}`);
}
const catalog = {
  name: catalogName,
  sha256: crypto.createHash("sha256").update(fs.readFileSync(catalogPath)).digest("hex")
};
const index = {
  schema_version: "1.0-signed-release-index",
  version,
  channel: "stable",
  repository,
  tag: `v${version}`,
  commit,
  assets,
  catalog
};
const bytes = Buffer.from(`${JSON.stringify(index, null, 2)}\n`);
const signature = crypto.sign(null, bytes, crypto.createPrivateKey(privateKey));
const publicKey = fs.readFileSync(
  new URL("../packages/ags-launcher/release-signing-public.pem", import.meta.url),
  "utf8"
);
if (!crypto.verify(null, bytes, publicKey, signature)) {
  throw new Error("release index signature does not match the pinned public key");
}
fs.writeFileSync(path.join(directory, "release-index.json"), bytes, { flag: "wx", mode: 0o600 });
fs.writeFileSync(path.join(directory, "release-index.sig"), `${signature.toString("base64")}\n`, {
  flag: "wx",
  mode: 0o600
});
