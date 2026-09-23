import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { createServer } from 'vite';

let server;
let normalizeApiTestResults;
let withInFlightApiTests;

before(async () => {
  server = await createServer({
    configFile: fileURLToPath(new URL('../vite.config.mjs', import.meta.url)),
    server: { middlewareMode: true, hmr: false, ws: false, watch: null },
    appType: 'custom',
    logLevel: 'error'
  });
  ({ normalizeApiTestResults, withInFlightApiTests } = await server.ssrLoadModule('/src/utils/apiPrecheck.js'));
});

after(async () => { await server?.close(); });

const done = { ok: true, loading: false, checkedAt: 1 };
const running = { ok: false, loading: true, message: '正在预检 API' };

test('saved or restored results never contain a running check', () => {
  // The table a finished check used to save while another profile was still running.
  const saved = { a: done, b: running, broken: 'text', list: [done] };
  assert.deepEqual(normalizeApiTestResults(saved), { a: done });
  assert.deepEqual(normalizeApiTestResults(null), {});
  assert.deepEqual(normalizeApiTestResults([done]), {});
});

test('a running check survives a save of another profile, and only while it runs', () => {
  const saved = { a: done };
  const current = { a: { ...done, checkedAt: 0 }, b: running };

  assert.deepEqual(withInFlightApiTests(saved, current, new Set(['b'])), { a: done, b: running });
  assert.deepEqual(withInFlightApiTests(saved, current, new Set()), { a: done });
  assert.deepEqual(withInFlightApiTests(saved, {}, new Set(['b'])), { a: done });
});
