import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';

function deferred() {
  let resolve, reject;
  const promise = new Promise((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}

async function fixture() {
  const values = [];
  let cursor = 0;
  const requests = [], toasts = [];
  const api = {
    refreshAccountToken(id) {
      const request = deferred();
      requests.push({ id, ...request });
      return request.promise;
    },
  };
  const context = vm.createContext({ window: { api } });
  const stubs = {
    react: { useState(initial) {
      const index = cursor++;
      if (!(index in values)) values[index] = initial;
      return [values[index], value => { values[index] = typeof value === 'function' ? value(values[index]) : value; }];
    } },
    '../utils/auth': {
      getAccountId: account => account.profile_id,
      getAccountName: account => account.name,
      maskAccountDisplayName: name => name,
    },
    '../utils/errors': { getErrorMessage: (error, fallback) => error?.message || fallback },
  };
  const source = fs.readFileSync(new URL('../renderer/src/hooks/useRefreshTokenModal.js', import.meta.url), 'utf8');
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
    requests, toasts,
    render() {
      cursor = 0;
      return module.namespace.useRefreshTokenModal({
        maskAccountName: false,
        requireStore: res => res.store,
        setStore() {},
        toast(message) { toasts.push(message); },
        toastError() {},
      });
    },
  };
}

const account = key => ({ profile_id: `fixture-${key}`, name: `${key}@example.invalid`, tokens: { refresh_token: `rt-${key}-old` } });

test('the dialog cannot be closed while its token refresh is running', async () => {
  const f = await fixture();
  f.render().openRefreshTokenModal(account('a'));
  const refreshing = f.render().handleRefreshAccountToken();
  assert.equal(f.render().refreshTokenModal.loading, true);

  f.render().closeRefreshTokenModal();
  assert.equal(f.render().refreshTokenModal.visible, true);
  assert.equal(f.render().refreshTokenModal.accountId, 'fixture-a');

  f.requests[0].resolve({ ok: true, refresh_token: 'rt-a-new', store: {} });
  await refreshing;
  const modal = f.render().refreshTokenModal;
  assert.equal(modal.refreshToken, 'rt-a-new');
  assert.equal(modal.loading, false);

  f.render().closeRefreshTokenModal();
  assert.equal(f.render().refreshTokenModal.visible, false);
});

test("a refresh result never lands in another account's dialog", async () => {
  for (const outcome of ['success', 'failed response', 'rejected']) {
    const f = await fixture();
    f.render().openRefreshTokenModal(account('a'));
    const refreshing = f.render().handleRefreshAccountToken();
    f.render().openRefreshTokenModal(account('b'));

    const request = f.requests[0];
    assert.equal(request.id, 'fixture-a');
    if (outcome === 'success') request.resolve({ ok: true, refresh_token: 'rt-a-new', store: {} });
    if (outcome === 'failed response') request.resolve({ ok: false, message: 'fixture refresh failure' });
    if (outcome === 'rejected') request.reject(new Error('fixture network failure'));
    await refreshing;

    const modal = f.render().refreshTokenModal;
    assert.equal(modal.accountId, 'fixture-b', outcome);
    assert.equal(modal.refreshToken, 'rt-b-old', outcome);
    assert.equal(modal.error, '', outcome);
    assert.equal(modal.loading, false, outcome);
  }
});
