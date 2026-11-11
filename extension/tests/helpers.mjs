// Shared helpers for the Segmenta extension tests.
// Node standard library only: no npm dependencies.
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import path from 'node:path';

export const testsDir = fileURLToPath(new URL('.', import.meta.url));
export const extensionDir = path.resolve(testsDir, '..');
export const repoRoot = path.resolve(extensionDir, '..');

export function extFile(...parts) {
  return path.join(extensionDir, ...parts);
}

export function readExt(...parts) {
  return readFileSync(extFile(...parts), 'utf8');
}

export function readExtBuffer(...parts) {
  return readFileSync(extFile(...parts));
}

export function readJson(...parts) {
  return JSON.parse(readExt(...parts));
}

export function existsExt(...parts) {
  try {
    readFileSync(extFile(...parts));
    return true;
  } catch {
    return false;
  }
}

const sharedImports = new Map();

// The shared/ files are plain scripts (no ESM syntax) so the same bytes work as
// content scripts, service-worker imports and Node tests. They publish their API
// on globalThis, which is what this helper reads back.
export async function loadShared(globalName, file) {
  if (!sharedImports.has(file)) {
    sharedImports.set(file, import(new URL(`../shared/${file}`, import.meta.url).href));
  }
  await sharedImports.get(file);
  const api = globalThis[globalName];
  if (!api) {
    throw new Error(`shared/${file} did not publish globalThis.${globalName}`);
  }
  return api;
}

// Chromium derives the extension id from the DER public key: first 16 bytes of
// sha256, hex nibbles remapped onto a-p.
export function extensionIdFromKey(base64Key) {
  const der = Buffer.from(String(base64Key), 'base64');
  const digest = createHash('sha256').update(der).digest('hex').slice(0, 32);
  let id = '';
  for (const nibble of digest) {
    id += String.fromCharCode(97 + parseInt(nibble, 16));
  }
  return id;
}

export function matchAllKeys(source, patterns) {
  const found = new Set();
  for (const pattern of patterns) {
    const re = new RegExp(pattern, 'g');
    let match;
    while ((match = re.exec(source)) !== null) {
      found.add(match[1]);
    }
  }
  return found;
}

export function pngSize(buffer) {
  const signature = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
  if (buffer.length < 24 || !buffer.subarray(0, 8).equals(signature)) {
    return null;
  }
  if (buffer.subarray(12, 16).toString('ascii') !== 'IHDR') {
    return null;
  }
  return { width: buffer.readUInt32BE(16), height: buffer.readUInt32BE(20) };
}
