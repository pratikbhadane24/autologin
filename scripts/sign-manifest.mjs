#!/usr/bin/env node
// Build and sign the broker manifest bundle published as release assets.
//
//   brokers-manifest.json      {"files": {"<name>.toml": "<contents>", ...}}
//   brokers-manifest.json.sig  base64url (no padding) ed25519 signature over
//                              the exact bytes of brokers-manifest.json
//
// Usage:
//   node scripts/sign-manifest.mjs [--dir <brokers dir>] [--out <dir>]
//       Sign. Reads the 32-byte hex seed from $MANIFEST_SIGNING_KEY.
//   node scripts/sign-manifest.mjs --verify <pubkeyhex> [--out <dir>] [--dir <brokers dir>]
//       Verify <out>/brokers-manifest.json(.sig). With --dir, also checks the
//       bundle matches the files on disk.
//   node scripts/sign-manifest.mjs --keygen <secret-key-file>   (refuses to overwrite)
//       Print a fresh hex seed (secret) and hex public key.
//   node scripts/sign-manifest.mjs --self-test
//       keygen -> sign the real brokers dir into a temp dir -> verify.
//
// Only Node built-ins are used (no install step needed in CI).

import { createPrivateKey, createPublicKey, generateKeyPairSync, sign, verify } from "node:crypto";
import { mkdtempSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync, mkdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const DEFAULT_BROKERS_DIR = join(REPO_ROOT, "app", "src-tauri", "brokers");
const DEFAULT_OUT_DIR = join(REPO_ROOT, "dist-manifest");
const MANIFEST_NAME = "brokers-manifest.json";
const SIG_NAME = `${MANIFEST_NAME}.sig`;
const KEY_BYTES = 32;
const HEX_KEY_RE = new RegExp(`^[0-9a-fA-F]{${KEY_BYTES * 2}}$`);
// ASN.1 prefixes that wrap a raw 32-byte ed25519 key (RFC 8410).
const PKCS8_PREFIX = Buffer.from("302e020100300506032b657004220420", "hex");
const SPKI_PREFIX = Buffer.from("302a300506032b6570032100", "hex");

function fail(message) {
  console.error(`sign-manifest: ${message}`);
  process.exit(1);
}

function parseHexKey(hex, label) {
  const trimmed = (hex ?? "").trim();
  if (!HEX_KEY_RE.test(trimmed)) {
    fail(`${label} must be ${KEY_BYTES * 2} hex characters (${KEY_BYTES} bytes)`);
  }
  return Buffer.from(trimmed, "hex");
}

function privateKeyFromSeed(seed) {
  return createPrivateKey({ key: Buffer.concat([PKCS8_PREFIX, seed]), format: "der", type: "pkcs8" });
}

function publicKeyFromRaw(raw) {
  return createPublicKey({ key: Buffer.concat([SPKI_PREFIX, raw]), format: "der", type: "spki" });
}

function rawPublicKeyHex(publicKey) {
  return publicKey.export({ format: "der", type: "spki" }).subarray(SPKI_PREFIX.length).toString("hex");
}

function rawSeedHex(privateKey) {
  return privateKey.export({ format: "der", type: "pkcs8" }).subarray(PKCS8_PREFIX.length).toString("hex");
}

/** Deterministic manifest bytes: .toml files only, sorted by name. */
function buildManifest(brokersDir) {
  let names;
  try {
    names = readdirSync(brokersDir)
      .filter((name) => name.endsWith(".toml") && statSync(join(brokersDir, name)).isFile())
      .sort();
  } catch (err) {
    fail(`cannot read brokers dir ${brokersDir}: ${err.message}`);
  }
  if (!names.includes("index.toml")) fail(`${brokersDir} has no index.toml`);
  const files = Object.fromEntries(names.map((name) => [name, readFileSync(join(brokersDir, name), "utf8")]));
  return Buffer.from(JSON.stringify({ files }), "utf8");
}

function manifestVersionOf(manifestBytes) {
  const index = JSON.parse(manifestBytes.toString("utf8")).files["index.toml"] ?? "";
  const match = index.match(/^\s*manifest_version\s*=\s*(\d+)/m);
  return match ? Number(match[1]) : null;
}

function signDir(brokersDir, outDir, seed) {
  const manifest = buildManifest(brokersDir);
  const signature = sign(null, manifest, privateKeyFromSeed(seed)).toString("base64url");
  mkdirSync(outDir, { recursive: true });
  writeFileSync(join(outDir, MANIFEST_NAME), manifest);
  writeFileSync(join(outDir, SIG_NAME), signature);
  return manifest;
}

/** Returns an error string, or null when the bundle is valid. */
function verifyDir(outDir, publicKeyRaw, brokersDir) {
  let manifest;
  let signatureText;
  try {
    manifest = readFileSync(join(outDir, MANIFEST_NAME));
    signatureText = readFileSync(join(outDir, SIG_NAME), "utf8").trim();
  } catch (err) {
    return `cannot read bundle in ${outDir}: ${err.message}`;
  }
  if (signatureText.includes("=")) return "signature must be unpadded base64url";
  const signature = Buffer.from(signatureText, "base64url");
  if (signature.length !== 64) return `signature is ${signature.length} bytes, expected 64`;
  if (!verify(null, manifest, publicKeyFromRaw(publicKeyRaw), signature)) return "signature does not verify";
  if (brokersDir && !buildManifest(brokersDir).equals(manifest)) {
    return `bundle does not match the files in ${brokersDir}`;
  }
  return null;
}

function keygen() {
  const { privateKey, publicKey } = generateKeyPairSync("ed25519");
  return { seedHex: rawSeedHex(privateKey), publicKeyHex: rawPublicKeyHex(publicKey) };
}

function selfTest(brokersDir) {
  const { seedHex, publicKeyHex } = keygen();
  const outDir = mkdtempSync(join(tmpdir(), "brokers-manifest-"));
  try {
    const manifest = signDir(brokersDir, outDir, Buffer.from(seedHex, "hex"));
    const goodError = verifyDir(outDir, Buffer.from(publicKeyHex, "hex"), brokersDir);
    if (goodError) fail(`self-test: valid bundle rejected: ${goodError}`);

    const otherKey = Buffer.from(keygen().publicKeyHex, "hex");
    if (!verifyDir(outDir, otherKey, null)) fail("self-test: wrong public key accepted");

    const tampered = Buffer.from(manifest);
    tampered[tampered.length - 2] ^= 1;
    writeFileSync(join(outDir, MANIFEST_NAME), tampered);
    if (!verifyDir(outDir, Buffer.from(publicKeyHex, "hex"), null)) fail("self-test: tampered bundle accepted");

    const count = Object.keys(JSON.parse(manifest.toString("utf8")).files).length;
    console.log(`self-test OK: ${count} files, manifest_version=${manifestVersionOf(manifest)}, ${manifest.length} bytes`);
  } finally {
    rmSync(outDir, { recursive: true, force: true });
  }
}

function parseArgs(argv) {
  const args = { dir: DEFAULT_BROKERS_DIR, out: DEFAULT_OUT_DIR, mode: "sign", pubkey: null };
  for (let i = 0; i < argv.length; i += 1) {
    const flag = argv[i];
    const takeValue = () => {
      const value = argv[i + 1];
      if (value === undefined || value.startsWith("--")) fail(`${flag} needs a value`);
      i += 1;
      return value;
    };
    if (flag === "--dir") args.dir = resolve(takeValue());
    else if (flag === "--out") args.out = resolve(takeValue());
    else if (flag === "--verify") Object.assign(args, { mode: "verify", pubkey: takeValue() });
    else if (flag === "--keygen") Object.assign(args, { mode: "keygen", keyFile: resolve(takeValue()) });
    else if (flag === "--self-test") args.mode = "self-test";
    else fail(`unknown argument ${flag}`);
  }
  return args;
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  if (args.mode === "keygen") {
    // The secret seed goes only to a private file, never to the screen/logs.
    const { seedHex, publicKeyHex } = keygen();
    writeFileSync(args.keyFile, `${seedHex}\n`, { mode: 0o600, flag: "wx" });
    console.log(`secret seed written to ${args.keyFile} (store it as the MANIFEST_SIGNING_KEY secret)`);
    console.log(`public key (embed in app): ${publicKeyHex}`);
    return;
  }
  if (args.mode === "self-test") {
    selfTest(args.dir);
    return;
  }
  if (args.mode === "verify") {
    const explicitDir = process.argv.includes("--dir") ? args.dir : null;
    const error = verifyDir(args.out, parseHexKey(args.pubkey, "public key"), explicitDir);
    if (error) fail(error);
    const version = manifestVersionOf(readFileSync(join(args.out, MANIFEST_NAME)));
    console.log(`verified ${join(args.out, MANIFEST_NAME)} (manifest_version=${version})`);
    return;
  }
  const seed = parseHexKey(process.env.MANIFEST_SIGNING_KEY, "MANIFEST_SIGNING_KEY");
  const manifest = signDir(args.dir, args.out, seed);
  const publicKeyHex = rawPublicKeyHex(createPublicKey(privateKeyFromSeed(seed)));
  console.log(`wrote ${join(args.out, MANIFEST_NAME)} and ${SIG_NAME}`);
  console.log(`manifest_version=${manifestVersionOf(manifest)} public key=${publicKeyHex}`);
}

main();
