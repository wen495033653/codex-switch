import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { createServer } from 'vite';

let server;
let getErrorMessage;
let getRawErrorMessage;
let translateRuntimeText;

before(async () => {
  server = await createServer({
    configFile: fileURLToPath(new URL('../vite.config.mjs', import.meta.url)),
    server: { middlewareMode: true, hmr: false, ws: false, watch: null },
    appType: 'custom',
    logLevel: 'error'
  });
  ({ getErrorMessage, getRawErrorMessage } = await server.ssrLoadModule('/src/utils/errors.js'));
  ({ translateRuntimeText } = await server.ssrLoadModule('/src/i18n.jsx'));
});

after(async () => { await server?.close(); });

test('cancel detection compares the untranslated backend text', () => {
  // Tauri rejects with the command's error string; the English UI translates it.
  assert.equal(translateRuntimeText('导入已取消', 'en'), 'Import canceled');
  for (const err of ['导入已取消', ' 导入已取消 ', new Error('导入已取消'), { error: '导入已取消' }]) {
    assert.equal(getRawErrorMessage(err), '导入已取消');
  }
  assert.equal(getRawErrorMessage(null), '');
  assert.equal(getRawErrorMessage({ message: '  ' }), '');
  assert.equal(getErrorMessage(null, '导出失败'), '导出失败');
});
