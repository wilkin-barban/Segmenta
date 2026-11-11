import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, readdirSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { extensionIdFromKey, extFile, readExt, readJson } from './helpers.mjs';

function runScript(args, env = {}) {
  return spawnSync('bash', [extFile('install-native.sh'), ...args], {
    encoding: 'utf8',
    env: { ...process.env, ...env },
  });
}

export default function suite({ test }) {
  test('install-native.sh is a safe bash script', async () => {
    const source = readExt('install-native.sh');
    assert.equal(source.startsWith('#!'), true, 'missing shebang');
    assert.equal(source.includes('set -euo pipefail'), true, 'missing strict mode');
    assert.equal(/\brm\s+-rf\b/.test(source), false, 'must not delete fixed paths');
    assert.equal(source.includes('mktemp'), true, 'must use mktemp for scratch');
    assert.equal(source.includes('segmenta-host'), true);
    const syntax = spawnSync('bash', ['-n', extFile('install-native.sh')], { encoding: 'utf8' });
    assert.equal(syntax.status, 0, `bash -n failed: ${syntax.stderr}`);
    const mode = spawnSync('stat', ['-c', '%a', extFile('install-native.sh')], { encoding: 'utf8' });
    assert.equal(/[1357]/.test(mode.stdout.trim().slice(-1)), true, 'script must be executable');
  });

  test('--print lists every supported browser manifest path without writing', async () => {
    const home = mkdtempSync(path.join(tmpdir(), 'segmenta-native-'));
    const result = runScript(['--print'], { HOME: home, XDG_CONFIG_HOME: path.join(home, '.config') });
    assert.equal(result.status, 0, `--print failed: ${result.stderr}`);
    const output = result.stdout;
    for (const fragment of [
      'google-chrome/NativeMessagingHosts',
      'chromium/NativeMessagingHosts',
      'BraveSoftware/Brave-Browser/NativeMessagingHosts',
      'microsoft-edge/NativeMessagingHosts',
      'vivaldi/NativeMessagingHosts',
      'opera/NativeMessagingHosts',
      'mozilla/native-messaging-hosts',
    ]) {
      assert.equal(output.includes(fragment), true, `missing ${fragment}`);
    }
    assert.deepEqual(readdirSync(home), [], '--print must not write anything');
  });

  test('--print honors XDG_CONFIG_HOME', async () => {
    const base = mkdtempSync(path.join(tmpdir(), 'segmenta-native-xdg-'));
    const xdg = path.join(base, 'cfg');
    const result = runScript(['--print'], { HOME: base, XDG_CONFIG_HOME: xdg });
    assert.equal(result.status, 0, result.stderr);
    assert.equal(result.stdout.includes(`${xdg}/google-chrome/NativeMessagingHosts`), true);
  });

  test('rejects unknown arguments', async () => {
    const result = runScript(['--force-everything'], {});
    assert.notEqual(result.status, 0);
  });

  test('the embedded chromium origin matches the manifest key', async () => {
    const manifest = readJson('chromium', 'manifest.json');
    const script = readExt('install-native.sh');
    const id = extensionIdFromKey(manifest.key);
    assert.equal(script.includes(`chrome-extension://${id}/`), true);
    assert.equal(script.includes('io.github.wilkin.segmenta'), true);
    assert.equal(script.includes('segmenta@wilkin.github.io'), true);
  });
}
