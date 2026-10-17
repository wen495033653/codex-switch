import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';

async function fixture() {
  const values = [];
  let cursor = 0;
  const context = vm.createContext({ window: { api: {} }, console });
  const stubs = {
    react: {
      useEffect() {},
      useRef: initial => ({ current: initial }),
      useState(initial) {
        const index = cursor++;
        if (!(index in values)) values[index] = typeof initial === 'function' ? initial() : initial;
        return [values[index], value => { values[index] = typeof value === 'function' ? value(values[index]) : value; }];
      },
    },
    '../utils/errors': { getErrorMessage: (error, fallback) => error?.message || fallback },
  };
  const source = fs.readFileSync(new URL('../renderer/src/hooks/useUpdateFlow.js', import.meta.url), 'utf8');
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
    render() {
      cursor = 0;
      return module.namespace.useUpdateFlow({ settings: {}, settingsLoaded: false, toast() {}, toastError() {} });
    },
  };
}

const downloading = percent => ({ status: 'downloading', progress: { percent } });

test('download progress replaces the dialog state only when the shown percent changes', async () => {
  const f = await fixture();
  f.render().applyUpdateStatus(downloading(10.2));
  const first = f.render().updateModal;
  assert.equal(first.status, 'downloading');
  assert.equal(first.visible, true);
  assert.equal(first.progress, 10.2);

  f.render().applyUpdateStatus(downloading(10.4));
  assert.equal(f.render().updateModal, first);

  f.render().applyUpdateStatus(downloading(10.6));
  const next = f.render().updateModal;
  assert.notEqual(next, first);
  assert.equal(next.progress, 10.6);

  f.render().applyUpdateStatus({ status: 'downloaded', update: { version: '9.9.9' } });
  assert.equal(f.render().updateModal.status, 'downloaded');
  assert.equal(f.render().updateModal.progress, 100);
});
