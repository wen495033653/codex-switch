import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { createServer } from 'vite';

let server;
let proxySaveFeedback;

before(async () => {
  server = await createServer({
    configFile: fileURLToPath(new URL('../vite.config.mjs', import.meta.url)),
    server: { middlewareMode: true, hmr: false, ws: false, watch: null },
    appType: 'custom',
    logLevel: 'error'
  });
  ({ proxySaveFeedback } = await server.ssrLoadModule('/src/utils/codexSettingsStatus.js'));
});

after(async () => { await server?.close(); });

test('proxy save preserves restart requirement and surfaces runtime failure', () => {
  const result = proxySaveFeedback({ message: '配置已保存', restartRequired: true, remoteControl: { error: 'fixture connection error' } });
  assert.equal(result.restartRequired, true);
  assert.equal(result.warning, true);
  assert.equal(result.message, '配置已保存 远程控制代理更新失败：fixture connection error');
  assert.equal(proxySaveFeedback({ message: '配置已保存' }).warning, false);
});
