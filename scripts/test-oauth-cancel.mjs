import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';

async function fixture(cancelOauth) {
  const values = [];
  let cursor = 0;
  const toasts = [], errors = [];
  const context = vm.createContext({ window: { api: { cancelOauth } } });
  const stubs = {
    react: {
      useEffect() {},
      useState(initial) {
        const index = cursor++;
        if (!(index in values)) values[index] = typeof initial === 'function' ? initial() : initial;
        return [values[index], value => { values[index] = typeof value === 'function' ? value(values[index]) : value; }];
      },
    },
    '../utils/appState': { createEmptyOauthState: () => ({ running: false, url: '', success: false, error: '', errorCode: '', message: '' }) },
    '../utils/errors': { getErrorMessage: (error, fallback) => error?.message || fallback, getRawErrorMessage: error => error?.message || '' },
  };
  const source = fs.readFileSync(new URL('../renderer/src/hooks/useAddAccountFlow.js', import.meta.url), 'utf8');
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
    toasts, errors,
    render() {
      cursor = 0;
      return module.namespace.useAddAccountFlow({
        handleRes() {},
        toast(message) { toasts.push(message); },
        toastError(error, fallback) { errors.push([error.message, fallback]); },
      });
    },
  };
}

test('a failed OAuth cancel is reported once and does not reject', async () => {
  const f = await fixture(async () => { throw new Error('fixture cancel failure'); });
  await f.render().cancelOauth();
  assert.deepEqual(f.errors, [['fixture cancel failure', '取消 OAuth 登录失败']]);
  assert.deepEqual(f.toasts, []);
});

test('a cancel resets the OAuth state and reports only an actual cancellation', async () => {
  const canceled = await fixture(async () => ({ ok: true, canceled: true, message: '已取消 OAuth 登录' }));
  await canceled.render().cancelOauth();
  assert.equal(canceled.render().oauth.running, false);
  assert.deepEqual(canceled.toasts, ['已取消 OAuth 登录']);

  const idle = await fixture(async () => ({ ok: true, canceled: false, message: '当前没有进行中的 OAuth 登录' }));
  await idle.render().cancelOauth();
  assert.deepEqual(idle.toasts, []);
  assert.deepEqual(idle.errors, []);
});
