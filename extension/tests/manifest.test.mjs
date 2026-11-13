import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import {
  extensionIdFromKey,
  extFile,
  existsExt,
  pngSize,
  readExt,
  readExtBuffer,
  readJson,
} from './helpers.mjs';

const ICONS = [16, 32, 48, 128];

export default function suite({ test }) {
  test('chromium manifest is a minimal MV3 manifest', async () => {
    const manifest = readJson('chromium', 'manifest.json');
    assert.equal(manifest.manifest_version, 3);
    assert.equal(manifest.version, '0.2.1');
    assert.equal(manifest.default_locale, 'es');
    assert.equal(typeof manifest.name, 'string');
    assert.equal(typeof manifest.description, 'string');
    assert.equal(typeof manifest.key, 'string');
    assert.equal(manifest.key.length > 100, true);
    assert.deepEqual(manifest.permissions.slice().sort(), [
      'activeTab',
      'alarms',
      'contextMenus',
      'downloads',
      'nativeMessaging',
      'notifications',
      'scripting',
      'storage',
    ]);
    assert.deepEqual(manifest.host_permissions, ['https://www.youtube.com/*']);
    assert.equal(Array.isArray(manifest.optional_host_permissions), true);
    assert.equal(manifest.optional_host_permissions.includes('https://*/*'), true);
    assert.equal(manifest.background.service_worker, 'background.js');
    assert.equal(manifest.action.default_popup, 'popup.html');
    assert.equal(manifest.action.default_title.startsWith('__MSG_'), true);
  });

  test('chromium content scripts split MAIN and ISOLATED worlds', async () => {
    const manifest = readJson('chromium', 'manifest.json');
    const scripts = manifest.content_scripts;
    assert.equal(Array.isArray(scripts), true);

    const main = scripts.find((entry) => entry.world === 'MAIN');
    const isolated = scripts.find((entry) => entry.world === 'ISOLATED');
    assert.ok(main, 'missing MAIN world content script');
    assert.ok(isolated, 'missing ISOLATED world content script');
    assert.deepEqual(main.js, ['document.js']);
    assert.equal(main.run_at, 'document_start');
    assert.equal(isolated.js[isolated.js.length - 1], 'content.js');
    assert.equal(isolated.run_at, 'document_start');
    for (const entry of scripts) {
      assert.deepEqual(entry.matches, ['https://www.youtube.com/*']);
      for (const file of entry.js) {
        assert.equal(existsExt('chromium', file), true, `missing chromium/${file}`);
      }
    }
  });

  test('chromium manifest points at real icons', async () => {
    const manifest = readJson('chromium', 'manifest.json');
    for (const size of ICONS) {
      assert.equal(manifest.icons[String(size)], `icons/icon-${size}.png`);
      assert.equal(existsExt('chromium', `icons/icon-${size}.png`), true, `missing icon ${size}`);
    }
  });

  test('extension id derived from the key matches CHROMIUM-ID.txt', async () => {
    const manifest = readJson('chromium', 'manifest.json');
    const computed = extensionIdFromKey(manifest.key);
    assert.match(computed, /^[a-p]{32}$/);
    assert.equal(existsExt('CHROMIUM-ID.txt'), true, 'missing extension/CHROMIUM-ID.txt');
    assert.equal(readExt('CHROMIUM-ID.txt').trim(), computed);
  });

  test('install-native.sh authorizes the computed chromium id', async () => {
    const manifest = readJson('chromium', 'manifest.json');
    const computed = extensionIdFromKey(manifest.key);
    const script = readExt('install-native.sh');
    assert.equal(script.includes('chrome-extension://'), true);
    assert.equal(script.includes(computed), true, 'install-native.sh must embed the derived id');
    assert.equal(script.includes('io.github.wilkin.segmenta'), true);
    assert.equal(script.includes('allowed_origins'), true);
    assert.equal(script.includes('segmenta@wilkin.github.io'), true);
    assert.equal(script.includes('allowed_extensions'), true);
    assert.equal(script.includes('nativeMessagingHosts') || script.includes('NativeMessagingHosts'), true);
  });

  test('firefox manifest is a WebExtension with the gecko id', async () => {
    const manifest = readJson('firefox', 'manifest.json');
    assert.equal(manifest.manifest_version, 3);
    assert.equal(manifest.version, '0.2.1');
    assert.equal(manifest.browser_specific_settings.gecko.id, 'segmenta@wilkin.github.io');
    assert.equal(manifest.permissions.includes('nativeMessaging'), true);
    assert.equal(manifest.permissions.includes('storage'), true);
    assert.equal(manifest.permissions.includes('downloads'), true);
    assert.equal(manifest.permissions.includes('contextMenus'), true);
    assert.equal(manifest.permissions.includes('notifications'), true);
    assert.equal(manifest.host_permissions.includes('https://www.youtube.com/*'), true);
    assert.equal(Array.isArray(manifest.background.scripts), true);
    assert.equal(manifest.background.scripts[manifest.background.scripts.length - 1], 'background.js');
    assert.equal(manifest.background.service_worker, undefined);
    for (const file of manifest.background.scripts) {
      assert.equal(existsExt('firefox', file), true, `missing firefox/${file}`);
    }
    for (const entry of manifest.content_scripts) {
      for (const file of entry.js) {
        assert.equal(existsExt('firefox', file), true, `missing firefox/${file}`);
      }
    }
    for (const size of ICONS) {
      assert.equal(existsExt('firefox', `icons/icon-${size}.png`), true, `missing firefox icon ${size}`);
    }
  });

  test('firefox manifest declares the MAIN world sniffer safely', async () => {
    const manifest = readJson('firefox', 'manifest.json');
    const main = manifest.content_scripts.find((entry) => entry.world === 'MAIN');
    assert.ok(main, 'firefox needs a MAIN world document script');
    assert.deepEqual(main.js, ['document.js']);
    const isolated = manifest.content_scripts.find((entry) => entry.world === 'ISOLATED');
    assert.ok(isolated, 'firefox needs an ISOLATED content script');
    assert.equal(isolated.js.includes('content.js'), true);
  });

  test('shared sources and icons are in sync with the chromium target', async () => {
    const files = [
      ['shared', 'browser-api.js'],
      ['shared', 'protocol.js'],
      ['shared', 'panel.js'],
      ['shared', 'validate.js'],
      ['background.js'],
      ['content.js'],
      ['document.js'],
    ];
    for (const parts of files) {
      const canonical = parts[0] === 'shared'
        ? readFileSync(extFile('shared', parts[1]))
        : readFileSync(extFile('chromium', ...parts));
      const copy = readFileSync(extFile('chromium', ...parts));
      assert.equal(copy.equals(canonical), true, `chromium/${parts.join('/')} is stale`);
    }
    const firefoxShared = [['browser-api.js'], ['protocol.js'], ['panel.js'], ['validate.js']];
    for (const [file] of firefoxShared) {
      assert.equal(
        readFileSync(extFile('firefox', 'shared', file)).equals(readFileSync(extFile('shared', file))),
        true,
        `firefox/shared/${file} is stale`,
      );
    }
    for (const file of ['background.js', 'content.js', 'document.js', 'confirm.html', 'confirm.js']) {
      assert.equal(
        readFileSync(extFile('firefox', file)).equals(readFileSync(extFile('chromium', file))),
        true,
        `firefox/${file} is stale`,
      );
    }
    for (const size of ICONS) {
      const icon = `icon-${size}.png`;
      assert.equal(
        readFileSync(extFile('chromium', 'icons', icon)).equals(readFileSync(extFile('icons', icon))),
        true,
        `chromium icons/${icon} is stale`,
      );
      assert.equal(
        readFileSync(extFile('firefox', 'icons', icon)).equals(readFileSync(extFile('icons', icon))),
        true,
        `firefox icons/${icon} is stale`,
      );
    }
  });

  test('icons are valid PNGs with the documented sizes', async () => {
    for (const size of ICONS) {
      const buffer = readExtBuffer('icons', `icon-${size}.png`);
      const dimensions = pngSize(buffer);
      assert.ok(dimensions, `icon-${size}.png is not a PNG`);
      assert.deepEqual(dimensions, { width: size, height: size });
      assert.equal(buffer.length > 100, true, `icon-${size}.png looks empty`);
    }
  });

  test('icon tool exists and documents its dependencies', async () => {
    assert.equal(existsExt('tools', 'make-icons.mjs'), true);
    const source = readExt('tools', 'make-icons.mjs');
    assert.equal(source.includes('rsvg-convert'), true);
    assert.equal(source.includes('ffmpeg'), true);
  });

  test('extension sources avoid remote code and html sinks', async () => {
    const files = [
      ['shared', 'protocol.js'],
      ['shared', 'validate.js'],
      ['shared', 'browser-api.js'],
      ['shared', 'panel.js'],
      ['chromium', 'background.js'],
      ['chromium', 'content.js'],
      ['chromium', 'document.js'],
      ['chromium', 'popup.js'],
      ['chromium', 'confirm.js'],
    ];
    const forbidden = [
      ['innerHTML', /\binnerHTML\b/],
      ['outerHTML', /\bouterHTML\b/],
      ['insertAdjacentHTML', /insertAdjacentHTML/],
      ['document.write', /document\.write\s*\(/],
      ['eval', /[^A-Za-z0-9_$.]eval\s*\(/],
      ['new Function', /new\s+Function\s*\(/],
      ['postMessage wildcard', /postMessage\s*\([^)]*,\s*["']\*["']/],
      ['remote url in code', /https?:\/\/(?!www\.youtube\.com|cdn\.|example\.|localhost)[A-Za-z0-9]/],
    ];
    for (const parts of files) {
      const source = readExt(...parts);
      for (const [label, pattern] of forbidden) {
        assert.equal(pattern.test(source), false, `${parts.join('/')} uses ${label}`);
      }
    }
  });

  test('popup page has no inline handlers or inline scripts from remote hosts', async () => {
    const html = readExt('chromium', 'popup.html');
    assert.equal(/\son[a-z]+\s*=/i.test(html), false, 'popup.html uses inline handlers');
    assert.equal(/src\s*=\s*["']https?:/i.test(html), false, 'popup.html loads remote scripts');
    assert.equal(html.includes('popup.js'), true);
    assert.equal(html.includes('shared/browser-api.js'), true);
  });

  test('the confirmation window only loads extension scripts', async () => {
    for (const target of ['chromium', 'firefox']) {
      const html = readExt(target, 'confirm.html');
      assert.equal(/\son[a-z]+\s*=/i.test(html), false, `${target}/confirm.html uses inline handlers`);
      assert.equal(/src\s*=\s*["']https?:/i.test(html), false, `${target}/confirm.html loads remote scripts`);
      assert.equal(/\.innerHTML\s*=/.test(html), false, `${target}/confirm.html writes html`);
      assert.equal(html.includes('confirm.js'), true, `${target}/confirm.html must load confirm.js`);
      assert.equal(
        html.includes('shared/browser-api.js'),
        true,
        `${target}/confirm.html must load the api adapter`,
      );
      // The whole-path -> folder rule is shared with the background, so the page
      // loads the same protocol helper instead of reimplementing it.
      assert.equal(
        html.includes('shared/protocol.js'),
        true,
        `${target}/confirm.html must load the protocol helpers`,
      );
      // Both actions exist and start disabled: nothing can be decided before the
      // script validated the pending download.
      assert.equal(/id="accept"[^>]*disabled/.test(html), true, `${target}/confirm.html accept button`);
      assert.equal(/id="reject"[^>]*disabled/.test(html), true, `${target}/confirm.html reject button`);
    }
  });
}
