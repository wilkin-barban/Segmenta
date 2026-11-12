import assert from 'node:assert/strict';
import { existsExt, matchAllKeys, readExt, readJson } from './helpers.mjs';

const LOCALES = ['es', 'en', 'pt_BR'];
const PLACEHOLDER = /\$\d/g;
const KEY_SHAPE = /^[A-Za-z0-9_@]+$/;

function placeholders(message) {
  return (message.match(PLACEHOLDER) || []).sort().join(',');
}

export default function suite({ test }) {
  test('the three locale catalogs exist and parse', async () => {
    for (const locale of LOCALES) {
      assert.equal(existsExt('_locales', locale, 'messages.json'), true, `missing ${locale}`);
      const catalog = readJson('_locales', locale, 'messages.json');
      assert.equal(typeof catalog, 'object');
      assert.equal(Object.keys(catalog).length > 20, true, `${locale} catalog is too small`);
    }
  });

  test('locale catalogs share an identical key set', async () => {
    const base = Object.keys(readJson('_locales', 'es', 'messages.json')).sort();
    for (const locale of LOCALES.slice(1)) {
      const keys = Object.keys(readJson('_locales', locale, 'messages.json')).sort();
      assert.deepEqual(keys, base, `${locale} keys differ from es`);
    }
  });

  test('locale messages are non-empty and use safe key names', async () => {
    for (const locale of LOCALES) {
      const catalog = readJson('_locales', locale, 'messages.json');
      for (const [key, entry] of Object.entries(catalog)) {
        assert.match(key, KEY_SHAPE, `${locale}:${key} has an invalid key name`);
        assert.equal(typeof entry, 'object', `${locale}:${key} is not an object`);
        assert.equal(typeof entry.message, 'string', `${locale}:${key} has no message`);
        assert.equal(entry.message.trim().length > 0, true, `${locale}:${key} is empty`);
        assert.equal(/\u0000/.test(entry.message), false, `${locale}:${key} has NUL`);
        if (entry.description !== undefined) {
          assert.equal(typeof entry.description, 'string');
        }
      }
    }
  });

  test('message placeholders match across locales', async () => {
    const base = readJson('_locales', 'es', 'messages.json');
    for (const locale of LOCALES.slice(1)) {
      const catalog = readJson('_locales', locale, 'messages.json');
      for (const key of Object.keys(base)) {
        assert.equal(
          placeholders(catalog[key].message),
          placeholders(base[key].message),
          `${locale}:${key} placeholder mismatch`,
        );
      }
    }
  });

  test('every user-visible key used by the extension exists in the catalogs', async () => {
    const catalog = readJson('_locales', 'es', 'messages.json');
    const sources = [
      readExt('shared', 'panel.js'),
      readExt('chromium', 'content.js'),
      readExt('chromium', 'popup.js'),
    ];
    const used = matchAllKeys(sources.join('\n'), [
      "\\bt\\(\\s*'([A-Za-z0-9_]+)'",
      "getMessage\\(\\s*'([A-Za-z0-9_]+)'",
      "text\\(\\s*t\\s*,\\s*'([A-Za-z0-9_]+)'",
    ]);
    assert.equal(used.size > 5, true, 'no i18n keys detected in sources');
    for (const key of used) {
      assert.equal(
        Object.prototype.hasOwnProperty.call(catalog, key),
        true,
        `locale key missing for ${key}`,
      );
    }
  });

  test('the task-added toast keys keep the documented Spanish title', async () => {
    const keys = ['taskAddedTitle', 'taskAddedBody', 'taskAddedBodyGeneric'];
    for (const locale of LOCALES) {
      const catalog = readJson('_locales', locale, 'messages.json');
      for (const key of keys) {
        assert.equal(
          typeof (catalog[key] && catalog[key].message),
          'string',
          `${locale} is missing ${key}`,
        );
      }
    }
    assert.equal(
      readJson('_locales', 'es', 'messages.json').taskAddedTitle.message,
      'A\u00f1adida a Segmenta',
    );
  });

  test('per-browser locale copies stay identical to the canonical catalogs', async () => {
    for (const target of ['chromium', 'firefox']) {
      for (const locale of LOCALES) {
        assert.equal(
          readExt(target, '_locales', locale, 'messages.json'),
          readExt('_locales', locale, 'messages.json'),
          `${target}/_locales/${locale}/messages.json is stale`,
        );
      }
    }
  });

  test('manifests only reference locale keys that exist', async () => {
    const catalog = readJson('_locales', 'es', 'messages.json');
    for (const target of ['chromium', 'firefox']) {
      const source = readExt(target, 'manifest.json');
      const referenced = matchAllKeys(source, ['__MSG_([A-Za-z0-9_@]+)__']);
      for (const key of referenced) {
        assert.equal(
          Object.prototype.hasOwnProperty.call(catalog, key),
          true,
          `${target}/manifest.json references missing locale key ${key}`,
        );
      }
    }
  });

  test('locale files are valid utf-8 and code sources stay ascii', async () => {
    for (const locale of LOCALES) {
      const raw = readExt('_locales', locale, 'messages.json');
      assert.equal(raw.includes('\uFFFD'), false, `${locale} has replacement chars`);
    }
    const codeFiles = [
      ['shared', 'protocol.js'],
      ['shared', 'validate.js'],
      ['shared', 'browser-api.js'],
      ['shared', 'panel.js'],
      ['chromium', 'background.js'],
      ['chromium', 'content.js'],
      ['chromium', 'document.js'],
      ['chromium', 'popup.js'],
      ['firefox', 'background.js'],
      ['firefox', 'content.js'],
      ['firefox', 'document.js'],
    ];
    for (const parts of codeFiles) {
      const raw = readExt(...parts);
      assert.equal(/[^\u0000-\u007F]/.test(raw), false, `${parts.join('/')} is not ascii`);
    }
  });
}
