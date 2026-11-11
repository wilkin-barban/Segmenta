#!/usr/bin/env node
// Segmenta shared-file sync.
//
//   node extension/tools/sync-shared.mjs [--check]
//
// The canonical sources live in extension/shared/ and extension/icons/. Browser
// manifests cannot reference parent directories, so each target directory gets a
// byte-identical copy. Run this after editing any shared file; --check only
// reports drift (the Node test suite asserts the copies stay in sync).
import { copyFileSync, mkdirSync, readFileSync, existsSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const extensionDir = path.resolve(here, '..');
const targets = ['chromium', 'firefox'];

const copies = [
  ['shared', 'browser-api.js'],
  ['shared', 'protocol.js'],
  ['shared', 'panel.js'],
  ['shared', 'validate.js'],
  ['icons', 'icon-16.png'],
  ['icons', 'icon-32.png'],
  ['icons', 'icon-48.png'],
  ['icons', 'icon-128.png'],
  // The browser targets share one implementation of these scripts.
  ['chromium', 'background.js'],
  ['chromium', 'content.js'],
  ['chromium', 'document.js'],
  ['chromium', 'popup.html'],
  ['chromium', 'popup.js'],
];

const checkOnly = process.argv.includes('--check');
let drifted = 0;
let written = 0;

for (const target of targets) {
  for (const parts of copies) {
    const from = path.join(extensionDir, ...parts);
    if (!existsSync(from)) {
      console.error(`sync-shared: missing source ${parts.join('/')}`);
      process.exitCode = 1;
      continue;
    }
    // Never copy a target file onto itself (chromium is its own source).
    let relative = parts;
    if (parts[0] === 'chromium') {
      if (target !== 'firefox') {
        continue;
      }
      relative = parts.slice(1);
    }
    const to = path.join(extensionDir, target, ...relative);
    const source = readFileSync(from);
    const current = existsSync(to) ? readFileSync(to) : null;
    if (current && current.equals(source)) {
      continue;
    }
    drifted += 1;
    if (checkOnly) {
      console.error(`sync-shared: out of date ${target}/${relative.join('/')}`);
      continue;
    }
    mkdirSync(path.dirname(to), { recursive: true });
    copyFileSync(from, to);
    written += 1;
    console.log(`synced ${target}/${relative.join('/')}`);
  }
}

if (checkOnly) {
  if (drifted === 0) {
    console.log('ok: shared copies are in sync');
  } else {
    console.error(`not ok: ${drifted} file(s) out of date`);
    process.exitCode = 1;
  }
} else {
  console.log(written === 0 ? 'ok: nothing to sync' : `ok: ${written} file(s) synced`);
}
