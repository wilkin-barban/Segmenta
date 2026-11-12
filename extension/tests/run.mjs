// Segmenta extension test runner.
//
//   node extension/tests/run.mjs
//
// No npm dependencies: Node standard library only. Prints "ok N tests" and exits
// non-zero when any test fails.
const SUITES = [
  './protocol.test.mjs',
  './quality.test.mjs',
  './panel.test.mjs',
  './locales.test.mjs',
  './validate.test.mjs',
  './manifest.test.mjs',
  './browser-api.test.mjs',
  './install-native.test.mjs',
  './pack.test.mjs',
  './capture.test.mjs',
  './confirm.test.mjs',
];

const failures = [];
let passed = 0;
let total = 0;

function formatError(err) {
  if (!err) {
    return 'unknown failure';
  }
  const name = err.name || 'Error';
  const message = err.message || String(err);
  const detail = err.stack && err.code ? '' : '';
  return `${name}: ${message}${detail}`;
}

for (const suiteFile of SUITES) {
  let suiteFactory;
  try {
    const mod = await import(new URL(suiteFile, import.meta.url).href);
    suiteFactory = mod.default;
  } catch (err) {
    failures.push({ name: `${suiteFile} (import)`, err });
    total += 1;
    continue;
  }

  if (typeof suiteFactory !== 'function') {
    failures.push({ name: `${suiteFile} (suite)`, err: new Error('suite must default-export a function') });
    total += 1;
    continue;
  }

  const cases = [];
  const test = (name, fn) => {
    cases.push({ name, fn });
  };

  try {
    suiteFactory({ test });
  } catch (err) {
    failures.push({ name: `${suiteFile} (collect)`, err });
    total += 1;
    continue;
  }

  for (const testCase of cases) {
    total += 1;
    try {
      await testCase.fn();
      passed += 1;
    } catch (err) {
      failures.push({ name: `${suiteFile.replace('./', '')} > ${testCase.name}`, err });
    }
  }
}

if (failures.length === 0) {
  console.log(`ok ${total} tests`);
} else {
  for (const failure of failures) {
    console.log(`not ok - ${failure.name}`);
    console.log(`  ${formatError(failure.err)}`);
    if (failure.err && failure.err.actual !== undefined) {
      console.log(`  actual:   ${JSON.stringify(failure.err.actual)}`);
      console.log(`  expected: ${JSON.stringify(failure.err.expected)}`);
    }
  }
  console.log(`\nnot ok ${passed}/${total} passed, ${failures.length} failed`);
  process.exitCode = 1;
}
