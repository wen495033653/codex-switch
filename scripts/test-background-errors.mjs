import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { createServer } from 'vite';
import { createPollingErrorLog } from '../renderer/src/utils/pollingErrorLog.js';

let server;
let installTauriApiBridge;

before(async () => {
  server = await createServer({
    configFile: fileURLToPath(new URL('../vite.config.mjs', import.meta.url)),
    server: { middlewareMode: true, hmr: false, ws: false, watch: null },
    appType: 'custom',
    logLevel: 'error'
  });
  ({ installTauriApiBridge } = await server.ssrLoadModule('/src/desktopApi.js'));
});

after(async () => { await server?.close(); });

test('event registration and removal failures reach the console', async () => {
  const errors = [];
  const originalError = console.error;
  console.error = (...args) => errors.push(args);
  const settle = () => new Promise(resolve => setTimeout(resolve, 0));
  try {
    globalThis.window = {
      __TAURI_INTERNALS__: {
        transformCallback: () => 1,
        async invoke(command) {
          if (command === 'plugin:event|listen') {
            if (globalThis.window.failListen) throw 'fixture listen failure';
            return 7;
          }
          if (command === 'plugin:event|unlisten') throw 'fixture unlisten failure';
          return null;
        }
      },
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
      failListen: true
    };
    installTauriApiBridge();

    const offFailed = globalThis.window.api.onStoreUpdated(() => {});
    await settle();
    offFailed();
    await settle();
    assert.deepEqual(errors, [[
      '[desktopApi] listen "store-updated" failed; this window will not receive the event',
      'fixture listen failure'
    ]]);

    globalThis.window.failListen = false;
    const off = globalThis.window.api.onUpdateStatus(() => {});
    await settle();
    off();
    await settle();
    assert.deepEqual(errors[1], ['[desktopApi] unlisten "update-status" failed', 'fixture unlisten failure']);
    assert.equal(errors.length, 2);
  } finally {
    console.error = originalError;
    delete globalThis.window;
  }
});

function recordingLogger() {
  const lines = [];
  return {
    lines,
    error: (message, detail) => lines.push(['error', message, detail]),
    info: message => lines.push(['info', message])
  };
}

test('a repeating poll failure is logged once per distinct error, with the failure count', () => {
  const logger = recordingLogger();
  const log = createPollingErrorLog('fixture_command', logger);

  log.failed('fixture offline');
  log.failed('fixture offline');
  log.failed(new Error('fixture timeout'));
  log.failed({ ok: false, message: 'fixture timeout' });
  assert.deepEqual(logger.lines.map(([level, message]) => [level, message]), [
    ['error', '[fixture_command] background request failed (consecutive failures: 1)'],
    ['error', '[fixture_command] background request failed (consecutive failures: 3)']
  ]);
  assert.equal(logger.lines[0][2], 'fixture offline');

  log.succeeded();
  log.succeeded();
  log.failed({ ok: false });
  assert.deepEqual(logger.lines.slice(2).map(([level, message]) => [level, message]), [
    ['info', '[fixture_command] background request recovered after 4 consecutive failures'],
    ['error', '[fixture_command] background request failed (consecutive failures: 1)']
  ]);
});
