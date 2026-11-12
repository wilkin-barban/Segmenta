#!/usr/bin/env node
// Segmenta extension packer.
//
//   node extension/tools/pack.mjs [--check] [--out DIR]
//
// Chrome refuses to load an extension whose manifest declares `default_locale`
// without a `_locales` subtree, so the packer copies `extension/_locales` into each
// target and validates every `__MSG_*__` placeholder before writing the ZIP. The same
// copy makes `--load-extension=extension/chromium` work from the source tree.
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { deflateRawSync } from 'node:zlib';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
export const extensionDir = path.resolve(here, '..');
export const localesDir = path.join(extensionDir, '_locales');
export const targets = [
  { name: 'chromium', zip: 'segmenta-extension-0.1.1.zip' },
  { name: 'firefox', zip: 'segmenta-firefox-0.1.1.zip' },
];
export const languages = ['es', 'en', 'pt_BR'];

/**
 * Files every shipped target must contain. The capture confirmation window is part
 * of the payload: without it the extension can only choose between losing the
 * download and capturing it without asking.
 */
export const requiredFiles = {
  chromium: [
    'manifest.json',
    'background.js',
    'popup.html',
    'popup.js',
    'confirm.html',
    'confirm.js',
    'shared/browser-api.js',
    'shared/protocol.js',
    'shared/validate.js',
    'icons/icon-128.png',
  ],
  firefox: [
    'manifest.json',
    'background.js',
    'popup.html',
    'popup.js',
    'confirm.html',
    'confirm.js',
    'shared/browser-api.js',
    'shared/protocol.js',
    'shared/validate.js',
    'icons/icon-128.png',
  ],
};

/** Relative paths the target is missing from the staged file list. */
export function missingFiles(target, files) {
  const expected = requiredFiles[target] || [];
  const present = new Set(files.map((file) => file.split(path.sep).join('/')));
  return expected.filter((file) => !present.has(file));
}

/** Recursively list files relative to `root`. */
export function walk(root, prefix = '') {
  const out = [];
  for (const entry of readdirSync(path.join(root, prefix), { withFileTypes: true })) {
    const relative = path.join(prefix, entry.name);
    if (entry.isDirectory()) {
      out.push(...walk(root, relative));
    } else {
      out.push(relative);
    }
  }
  return out;
}

/** Message catalog for one locale. */
export function messages(language) {
  return JSON.parse(readFileSync(path.join(localesDir, language, 'messages.json'), 'utf8'));
}

/**
 * Validate that the locales are complete and cover every `__MSG_*__` placeholder in
 * `files`, which are relative to `root`.
 */
export function validateLocales(root, files) {
  const problems = [];
  const manifest = JSON.parse(readFileSync(path.join(root, 'manifest.json'), 'utf8'));
  const defaultLocale = manifest.default_locale;
  if (!defaultLocale) {
    problems.push('manifest has no default_locale');
  } else if (!languages.includes(defaultLocale)) {
    problems.push(`default_locale ${defaultLocale} has no _locales directory`);
  }
  for (const language of languages) {
    if (!existsSync(path.join(root, '_locales', language, 'messages.json'))) {
      problems.push(`missing _locales/${language}/messages.json in ${root}`);
    }
  }
  if (problems.length > 0) {
    return problems;
  }
  const catalogs = Object.fromEntries(languages.map((language) => [language, messages(language)]));
  const keys = Object.keys(catalogs[languages[0]]).sort();
  for (const language of languages.slice(1)) {
    const other = Object.keys(catalogs[language]).sort();
    if (other.join(',') !== keys.join(',')) {
      problems.push(`_locales/${language} has a different key set than ${languages[0]}`);
    }
  }
  const placeholders = new Set();
  for (const file of files) {
    if (!/\.(js|html|json)$/.test(file)) continue;
    const text = readFileSync(path.join(root, file), 'utf8');
    for (const match of text.matchAll(/__MSG_([A-Za-z0-9_]+)__/g)) {
      placeholders.add(match[1]);
    }
  }
  for (const key of placeholders) {
    for (const language of languages) {
      if (!catalogs[language][key]) {
        problems.push(`__MSG_${key}__ is missing from _locales/${language}`);
      }
    }
  }
  return problems;
}

/** Copy `extension/_locales` into a target so unpacked loading also works. */
export function syncLocales(target) {
  const destination = path.join(extensionDir, target, '_locales');
  for (const language of languages) {
    const from = path.join(localesDir, language, 'messages.json');
    const to = path.join(destination, language, 'messages.json');
    mkdirSync(path.dirname(to), { recursive: true });
    const source = readFileSync(from);
    if (!existsSync(to) || !readFileSync(to).equals(source)) {
      copyFileSync(from, to);
    }
  }
  return destination;
}

/** Materialise the exact directory that goes into the ZIP. */
export function stage(target, stagingRoot) {
  const root = path.join(extensionDir, target);
  syncLocales(target);
  rmSync(stagingRoot, { recursive: true, force: true });
  mkdirSync(stagingRoot, { recursive: true });
  const files = walk(root);
  for (const file of files) {
    const destination = path.join(stagingRoot, file);
    mkdirSync(path.dirname(destination), { recursive: true });
    copyFileSync(path.join(root, file), destination);
  }
  return { root, stagingRoot, files };
}

/** Minimal deterministic ZIP writer (deflate, no timestamps beyond a fixed date). */
export function writeZip(stagingRoot, files, output) {
  const chunks = [];
  const central = [];
  let offset = 0;
  const dosTime = 0;
  const dosDate = (1 << 5) | 1; // 1980-01-01
  for (const file of files) {
    const name = Buffer.from(file.split(path.sep).join('/'), 'utf8');
    const data = readFileSync(path.join(stagingRoot, file));
    const compressed = deflateRawSync(data, { level: 9 });
    const crc = crc32(data);
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(20, 4);
    local.writeUInt16LE(0x0800, 6);
    local.writeUInt16LE(8, 8);
    local.writeUInt16LE(dosTime, 10);
    local.writeUInt16LE(dosDate, 12);
    local.writeUInt32LE(crc, 14);
    local.writeUInt32LE(compressed.length, 18);
    local.writeUInt32LE(data.length, 22);
    local.writeUInt16LE(name.length, 26);
    local.writeUInt16LE(0, 28);
    chunks.push(local, name, compressed);

    const directory = Buffer.alloc(46);
    directory.writeUInt32LE(0x02014b50, 0);
    directory.writeUInt16LE(20, 4);
    directory.writeUInt16LE(20, 6);
    directory.writeUInt16LE(0x0800, 8);
    directory.writeUInt16LE(8, 10);
    directory.writeUInt16LE(dosTime, 12);
    directory.writeUInt16LE(dosDate, 14);
    directory.writeUInt32LE(crc, 16);
    directory.writeUInt32LE(compressed.length, 20);
    directory.writeUInt32LE(data.length, 24);
    directory.writeUInt16LE(name.length, 28);
    directory.writeUInt32LE(offset, 42);
    central.push(directory, name);
    offset += local.length + name.length + compressed.length;
  }
  const centralBuffer = Buffer.concat(central);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(files.length, 8);
  end.writeUInt16LE(files.length, 10);
  end.writeUInt32LE(centralBuffer.length, 12);
  end.writeUInt32LE(offset, 16);
  mkdirSync(path.dirname(output), { recursive: true });
  writeFileSync(output, Buffer.concat([...chunks, centralBuffer, end]));
  return statSync(output).size;
}

function crc32(buffer) {
  let crc = 0xffffffff;
  for (const byte of buffer) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit += 1) {
      crc = crc & 1 ? (crc >>> 1) ^ 0xedb88320 : crc >>> 1;
    }
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function main() {
  const check = process.argv.includes('--check');
  const outIndex = process.argv.indexOf('--out');
  const outDir = outIndex > 0 ? path.resolve(process.argv[outIndex + 1]) : path.resolve(extensionDir, '..', 'dist');
  let failed = false;
  for (const target of targets) {
    const root = path.join(extensionDir, target.name);
    syncLocales(target.name);
    const files = walk(root);
    const problems = validateLocales(root, files);
    for (const missing of missingFiles(target.name, files)) {
      problems.push(`missing required file ${missing}`);
    }
    if (problems.length > 0) {
      failed = true;
      for (const problem of problems) console.error(`pack: ${target.name}: ${problem}`);
      continue;
    }
    if (check) {
      console.log(`ok ${target.name} files=${files.length} locales=${languages.join(',')}`);
      continue;
    }
    const output = path.join(outDir, target.zip);
    const size = writeZip(root, files, output);
    console.log(`wrote ${output} (${size} bytes, ${files.length} files, locales ${languages.join(',')})`);
  }
  if (failed) process.exitCode = 1;
}

if (process.argv[1] && path.resolve(process.argv[1]) === path.resolve(fileURLToPath(import.meta.url))) {
  main();
}
