import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import test from 'node:test';
import vm from 'node:vm';

test('listUsageBills forwards the optional camelCase thread filter to Tauri', async () => {
  const window = {};
  const calls = [];
  const context = vm.createContext({ window });
  const core = new vm.SyntheticModule(['isTauri', 'invoke'], function () {
    this.setExport('isTauri', () => true);
    this.setExport('invoke', async (command, args) => calls.push([command, JSON.parse(JSON.stringify(args))]));
  }, { context });
  const event = new vm.SyntheticModule(['listen'], function () {
    this.setExport('listen', () => {});
  }, { context });
  const source = await fs.readFile(new URL('../renderer/src/desktopApi.js', import.meta.url), 'utf8');
  const module = new vm.SourceTextModule(source, { context });
  await module.link(specifier => ({
    '@tauri-apps/api/core': core,
    '@tauri-apps/api/event': event
  })[specifier]);
  await module.evaluate();
  module.namespace.installTauriApiBridge();

  const query = { range: 'all', ownerType: null, ownerId: null, model: '', page: 1, pageSize: 50 };
  await window.api.listUsageBills(query);
  await window.api.listUsageBills({ ...query, threadId: 'requested-thread' });
  assert.deepEqual(calls, [
    ['usage_bills_list', query],
    ['usage_bills_list', { ...query, threadId: 'requested-thread' }]
  ]);
});
