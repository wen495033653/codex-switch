import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { createServer } from 'vite';

let server;
let normalizeApiTestResults;
let runApiProfilePrecheck;
let withInFlightApiTests;

before(async () => {
  server = await createServer({
    configFile: fileURLToPath(new URL('../vite.config.mjs', import.meta.url)),
    server: { middlewareMode: true, hmr: false, ws: false, watch: null },
    appType: 'custom',
    logLevel: 'error'
  });
  ({ normalizeApiTestResults, runApiProfilePrecheck, withInFlightApiTests } = await server.ssrLoadModule('/src/utils/apiPrecheck.js'));
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

test('the precheck calls the injected backend function and reports progress through onUpdate', async () => {
  const calls = [];
  const updates = [];
  const result = await runApiProfilePrecheck({
    profile: { base_url: 'api.example.invalid', api_key: 'fixture-key' },
    profileName: 'Fixture',
    model: ' ',
    previousTest: null,
    onUpdate: test => updates.push(test.loading),
    testApiBaseUrl: async payload => {
      calls.push(payload);
      return { ok: true, message: 'fixture ok', stage: 'responses' };
    }
  });
  assert.deepEqual(calls, [{ baseUrl: 'https://api.example.invalid/v1', apiKey: 'fixture-key', model: 'gpt-6-astra' }]);
  assert.deepEqual(updates, [true, false]);
  assert.equal(result.ok, true);
  assert.equal(result.message, 'fixture ok');
  assert.equal(result.loading, false);
});

test('a rejected backend call and an invalid URL end as failed, finished checks', async () => {
  const failed = await runApiProfilePrecheck({
    profile: { base_url: 'https://api.example.invalid/v1', api_key: 'fixture-key' },
    testApiBaseUrl: async () => { throw 'fixture refused'; }
  });
  assert.equal(failed.ok, false);
  assert.equal(failed.loading, false);
  assert.equal(failed.message, 'fixture refused');

  let called = false;
  const invalid = await runApiProfilePrecheck({
    profile: { base_url: 'ftp://api.example.invalid', api_key: 'fixture-key' },
    testApiBaseUrl: async () => { called = true; }
  });
  assert.equal(called, false);
  assert.equal(invalid.stage, 'input');
  assert.equal(invalid.loading, false);
});