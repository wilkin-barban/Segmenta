// Packaging guard: a ZIP without `_locales` makes Chrome refuse the extension.
import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import path from 'node:path';
import {
  extensionDir,
  languages,
  missingFiles,
  requiredFiles,
  syncLocales,
  targets,
  validateLocales,
  walk,
} from '../tools/pack.mjs';

export default function suite({ test }) {
  for (const target of targets) {
    test(`${target.name}: locales are staged next to the manifest`, () => {
      syncLocales(target.name);
      const root = path.join(extensionDir, target.name);
      for (const language of languages) {
        assert.ok(
          existsSync(path.join(root, '_locales', language, 'messages.json')),
          `${target.name} is missing _locales/${language}/messages.json`,
        );
      }
    });

    test(`${target.name}: default_locale and placeholders resolve`, () => {
      const root = path.join(extensionDir, target.name);
      const problems = validateLocales(root, walk(root));
      assert.deepEqual(problems, [], problems.join('; '));
    });

    test(`${target.name}: manifest default_locale is one of the staged locales`, () => {
      const root = path.join(extensionDir, target.name);
      const manifest = JSON.parse(readFileSync(path.join(root, 'manifest.json'), 'utf8'));
      assert.ok(languages.includes(manifest.default_locale), 'default_locale must be staged');
      for (const language of languages) {
        assert.ok(existsSync(path.join(root, '_locales', language, 'messages.json')));
      }
    });

    test(`${target.name}: the payload ships the capture confirmation window`, () => {
      const root = path.join(extensionDir, target.name);
      const files = walk(root);
      assert.deepEqual(missingFiles(target.name, files), [], 'required files are missing');
      // The confirmation page and its script must be in the file list the packer
      // zips, not only present on disk.
      const shipped = files.map((file) => file.split(path.sep).join('/'));
      assert.equal(shipped.includes('confirm.html'), true);
      assert.equal(shipped.includes('confirm.js'), true);
      assert.equal(requiredFiles[target.name].includes('confirm.html'), true);
      assert.equal(requiredFiles[target.name].includes('confirm.js'), true);
    });

    test(`${target.name}: the confirmation window stays private to the extension`, () => {
      const root = path.join(extensionDir, target.name);
      const manifest = JSON.parse(readFileSync(path.join(root, 'manifest.json'), 'utf8'));
      const entries = Array.isArray(manifest.web_accessible_resources) ? manifest.web_accessible_resources : [];
      // The popup is opened by the background through windows.create, so no page
      // outside the extension may address it: publishing it would widen the surface
      // for nothing.
      const exposed = entries.some((entry) => Array.isArray(entry.resources) && entry.resources.includes('confirm.html'));
      assert.equal(exposed, false, `${target.name}/manifest.json must not expose confirm.html`);
      // `alarms` is the only timer that survives a suspended MV3 worker, so the
      // capture watchdog cannot be built without it.
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
    });
  }
}
