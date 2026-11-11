// Packaging guard: a ZIP without `_locales` makes Chrome refuse the extension.
import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { extensionDir, languages, syncLocales, targets, validateLocales, walk } from '../tools/pack.mjs';

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
  }
}
