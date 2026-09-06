import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';

const source = fs.readFileSync(new URL('../src-tauri/src/codex_launcher/plugin_unlock.js', import.meta.url), 'utf8');

async function fixture({ missingAsset = false, duplicate = false } = {}) {
  const calls = [];
  const original = function (...args) { calls.push(args); return 'forwarded'; };
  const dispatcher = { dispatchMessage: original, subscribe() {}, deliverMessage() {} };
  const context = vm.createContext({
    document: { querySelectorAll: () => missingAsset ? [] : [{ href: 'app://-/assets/app-initial-fixture.js' }] },
    performance: { getEntriesByType: () => [] },
    setTimeout: () => 1, clearTimeout() {},
  });
  context.window = context;
  const module = new vm.SyntheticModule(['dispatcher', 'second'], function () {
    this.setExport('dispatcher', dispatcher);
    this.setExport('second', duplicate ? { ...dispatcher } : dispatcher);
  }, { context });
  await module.link(() => {});
  await module.evaluate();
  const script = new vm.Script(source, { importModuleDynamically: async () => module });
  const status = await script.runInContext(context);
  return { calls, original, dispatcher, context, script, status };
}

test('expands only local + vertical catalog and preserves envelope/input', async () => {
  const f = await fixture();
  assert.equal(f.status.patched, true);
  const params = { marketplaceKinds: ['local', 'vertical'], cwds: ['fixture'], forceRefetch: true };
  const payload = { hostId: 'local', priority: 'background', request: { id: 'fixture', method: 'plugin/list', params } };
  assert.equal(f.dispatcher.dispatchMessage('mcp-request', payload, 42), 'forwarded');
  assert.equal(f.calls[0][1].request.params.marketplaceKinds, undefined);
  assert.equal(f.calls[0][1].priority, 'background');
  assert.equal(f.calls[0][1].request.id, 'fixture');
  assert.equal(f.calls[0][1].request.params.cwds, params.cwds);
  assert.equal(f.calls[0][2], 42);
  assert.deepEqual(params.marketplaceKinds, ['local', 'vertical']);
  assert.equal(f.context.__codexSwitchPluginUnlockStatus.matchedRequests, 1);
});

test('other methods, message types and intentional marketplace filters pass unchanged', async () => {
  const f = await fixture();
  for (const [type, method, kinds] of [
    ['mcp-request', 'plugin/install', ['local', 'vertical']],
    ['other', 'plugin/list', ['local', 'vertical']],
    ['mcp-request', 'plugin/list', ['vertical']],
    ['mcp-request', 'plugin/list', ['local']],
    ['mcp-request', 'plugin/list', ['local', 'vertical', 'other']],
    ['mcp-request', 'plugin/list', undefined],
  ]) {
    const payload = { request: { method, params: { marketplaceKinds: kinds } } };
    f.dispatcher.dispatchMessage(type, payload);
    assert.equal(f.calls.at(-1)[1], payload);
  }
  assert.equal(f.context.__codexSwitchPluginUnlockStatus.matchedRequests, 0);
});

test('reinjection is idempotent and stop restores original dispatcher', async () => {
  const f = await fixture();
  const patched = f.dispatcher.dispatchMessage;
  await f.script.runInContext(f.context);
  assert.equal(f.dispatcher.dispatchMessage, patched);
  f.context.__codexSwitchPluginUnlockController.stop();
  assert.equal(f.dispatcher.dispatchMessage, f.original);
  assert.equal(f.context.__codexSwitchPluginUnlockStatus.patched, false);
});

test('missing asset and ambiguous dispatcher report actual failures', async () => {
  for (const options of [{ missingAsset: true }, { duplicate: true }]) {
    const f = await fixture(options);
    assert.equal(f.status.patched, false);
    assert.notEqual(f.status.error, '');
    assert.equal(f.dispatcher.dispatchMessage, f.original);
  }
});
