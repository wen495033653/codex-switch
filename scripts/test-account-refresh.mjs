import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';

function deferred() {
  let resolve;
  const promise = new Promise(res => { resolve = res; });
  return { promise, resolve };
}

async function fixture() {
  const values = [];
  let cursor = 0;
  const requests = [];
  const api = {
    refreshAccount(id) {
      const request = deferred();
      requests.push({ id, ...request });
      return request.promise;
    },
  };
  const context = vm.createContext({ window: { api } });
  const stubs = {
    react: {
      useMemo: compute => compute(),
      useState(initial) {
        const index = cursor++;
        if (!(index in values)) values[index] = typeof initial === 'function' ? initial() : initial;
        return [values[index], value => { values[index] = typeof value === 'function' ? value(values[index]) : value; }];
      },
    },
    '../utils/auth': {
      getAccountId: account => account.profile_id,
      getAccountName: account => account.name,
      maskAccountDisplayName: name => name,
    },
    '../utils/errors': { getErrorMessage: (error, fallback) => error?.message || fallback, getRawErrorMessage: error => error?.message || '' },
  };
  const source = fs.readFileSync(new URL('../renderer/src/hooks/useAccountOperations.js', import.meta.url), 'utf8');
  const module = new vm.SourceTextModule(source, { context });
  await module.link(async specifier => {
    const exports = stubs[specifier];
    if (!exports) throw Error(`Unexpected import ${specifier}`);
    return new vm.SyntheticModule(Object.keys(exports), function () {
      for (const [name, value] of Object.entries(exports)) this.setExport(name, value);
    }, { context });
  });
  await module.evaluate();
  return {
    requests,
    render() {
      cursor = 0;
      return module.namespace.useAccountOperations({
        applySettings() {}, handleRes() {}, maskAccountName: false,
        setCodexRestartNoticeMessage() {}, setCodexRestartNoticeVisible() {},
        setStore() {}, toast() {}, toastError() {},
      });
    },
  };
}

const refreshingIds = f => [...f.render().refreshingAccountIds].sort();

test('refreshing a second account keeps the first one marked and blocks a duplicate request', async () => {
  const f = await fixture();
  const refreshingA = f.render().handleRefreshAccount('a');
  const refreshingB = f.render().handleRefreshAccount('b');
  assert.deepEqual(refreshingIds(f), ['a', 'b']);

  await f.render().handleRefreshAccount('a');
  assert.deepEqual(f.requests.map(request => request.id), ['a', 'b']);

  f.requests[0].resolve({ ok: true, message: 'fixture refreshed' });
  await refreshingA;
  assert.deepEqual(refreshingIds(f), ['b']);

  f.requests[1].resolve({ ok: true, message: 'fixture refreshed' });
  await refreshingB;
  assert.deepEqual(refreshingIds(f), []);
});
