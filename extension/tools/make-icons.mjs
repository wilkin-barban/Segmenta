#!/usr/bin/env node
// Segmenta icon generator.
//
//   node extension/tools/make-icons.mjs
//
// Rasterises packaging/io.github.wilkin.Segmenta.svg into
// extension/icons/icon-{16,32,48,128}.png.
//
// Preferred rasterisers, in order: rsvg-convert, then ffmpeg (librsvg decoder).
// When neither is available the script writes a flat accent-colour PNG built with
// node:zlib only, and says so. No npm dependencies, no network access.
import { execFileSync, spawnSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import zlib from 'node:zlib';

const here = path.dirname(fileURLToPath(import.meta.url));
const extensionDir = path.resolve(here, '..');
const repoRoot = path.resolve(extensionDir, '..');
const svgPath = path.join(repoRoot, 'packaging', 'io.github.wilkin.Segmenta.svg');
const outDir = path.join(extensionDir, 'icons');
const SIZES = [16, 32, 48, 128];
const ACCENT = [53, 132, 228]; // #3584e4, from the source SVG gradient

function haveCommand(command) {
  const result = spawnSync('bash', ['-lc', `command -v ${command}`], { encoding: 'utf8' });
  return result.status === 0 && result.stdout.trim() !== '';
}

const crcTable = (() => {
  const table = new Int32Array(256);
  for (let n = 0; n < 256; n += 1) {
    let c = n;
    for (let k = 0; k < 8; k += 1) {
      c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    }
    table[n] = c;
  }
  return table;
})();

function crc32(buffer) {
  let crc = -1;
  for (const byte of buffer) {
    crc = crcTable[(crc ^ byte) & 0xff] ^ (crc >>> 8);
  }
  return (crc ^ -1) >>> 0;
}

function chunk(type, data) {
  const length = Buffer.alloc(4);
  length.writeUInt32BE(data.length, 0);
  const typeBuffer = Buffer.from(type, 'ascii');
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(Buffer.concat([typeBuffer, data])), 0);
  return Buffer.concat([length, typeBuffer, data, crc]);
}

function placeholderPng(size) {
  const header = Buffer.alloc(13);
  header.writeUInt32BE(size, 0);
  header.writeUInt32BE(size, 4);
  header.writeUInt8(8, 8); // bit depth
  header.writeUInt8(6, 9); // colour type RGBA
  header.writeUInt8(0, 10);
  header.writeUInt8(0, 11);
  header.writeUInt8(0, 12);
  const raw = Buffer.alloc(size * (size * 4 + 1));
  for (let y = 0; y < size; y += 1) {
    const rowStart = y * (size * 4 + 1);
    raw[rowStart] = 0; // filter: none
    for (let x = 0; x < size; x += 1) {
      const offset = rowStart + 1 + x * 4;
      raw[offset] = ACCENT[0];
      raw[offset + 1] = ACCENT[1];
      raw[offset + 2] = ACCENT[2];
      raw[offset + 3] = 255;
    }
  }
  const signature = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
  return Buffer.concat([
    signature,
    chunk('IHDR', header),
    chunk('IDAT', zlib.deflateSync(raw, { level: 9 })),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}

function rasterise(size, target) {
  if (haveCommand('rsvg-convert')) {
    execFileSync('rsvg-convert', ['-w', String(size), '-h', String(size), '-o', target, svgPath], {
      stdio: 'pipe',
    });
    return 'rsvg-convert';
  }
  if (haveCommand('ffmpeg')) {
    execFileSync(
      'ffmpeg',
      ['-hide_banner', '-loglevel', 'error', '-y', '-i', svgPath, '-vf', `scale=${size}:${size}`, '-frames:v', '1', target],
      { stdio: 'pipe' },
    );
    return 'ffmpeg';
  }
  writeFileSync(target, placeholderPng(size));
  return 'placeholder';
}

mkdirSync(outDir, { recursive: true });

const used = new Set();
for (const size of SIZES) {
  const target = path.join(outDir, `icon-${size}.png`);
  used.add(rasterise(size, target));
  console.log(`icon-${size}.png written`);
}

if (used.has('placeholder')) {
  console.log('warning: neither rsvg-convert nor ffmpeg was found; a flat accent-colour PNG was written.');
} else {
  console.log(`rasterised ${svgPath} with ${[...used].join(', ')}`);
}
