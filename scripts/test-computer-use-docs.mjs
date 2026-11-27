import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';

async function fixture({ inspectResponses = [], clearResponses = [], restoreResponses = [] }) {
  const states = [];
  const refs = [];
  const effects = [];
  let stateCursor = 0;
  let refCursor = 0;
  const calls = [];
  const toasts = [];
  const toastErrors = [];

  const api = {
    async inspectCodexComputerUseDocs() {
      calls.push('inspect');
      const next = inspectResponses.shift();
      if (next instanceof Error) throw next;
      return next;
    },
    async clearCodexComputerUseRestrictions() {
      calls.push('clear');
      const next = clearResponses.shift();
      if (next instanceof Error) throw next;
      return next;
    },
    async restoreCodexComputerUseDocs() {
      calls.push('restore');
      const next = restoreResponses.shift();
      if (next instanceof Error) throw next;
      return next;
    }
  };

  const context = vm.createContext({ window: { api } });
  const reactStub = {
    useState(initial) {
      const index = stateCursor++;
      if (!(index in states)) {
        states[index] = typeof initial === 'function' ? initial() : initial;
      }
      return [
        states[index],
        value => {
          states[index] = typeof value === 'function' ? value(states[index]) : value;
        }
      ];
    },
    useRef(initial) {
      const index = refCursor++;
      if (!(index in refs)) refs[index] = { current: initial };
      return refs[index];
    },
    useCallback(fn) {
      return fn;
    },
    useEffect(fn) {
      effects.push(fn);
    }
  };

  const source = fs.readFileSync(
    new URL('../renderer/src/hooks/useCodexComputerUseDocs.js', import.meta.url),
    'utf8'
  );
  const module = new vm.SourceTextModule(source, { context });
  await module.link(async specifier => {
    assert.equal(specifier, 'react');
    return new vm.SyntheticModule(Object.keys(reactStub), function () {
      for (const [name, value] of Object.entries(reactStub)) this.setExport(name, value);
    }, { context });
  });
  await module.evaluate();

  const render = () => {
    stateCursor = 0;
    refCursor = 0;
    effects.length = 0;
    return module.namespace.useCodexComputerUseDocs({
      toast(message) { toasts.push(message); },
      toastError(err, fallback) { toastErrors.push(err?.message || fallback); }
    });
  };

  return { calls, toasts, toastErrors, render, effects };
}

const pendingStatus = {
  ok: true,
  guidance: { totalFiles: 1, pendingCount: 1, appliedCount: 0, restorableCount: 0 },
  confirmations: { totalFiles: 1, pendingCount: 1, appliedCount: 0, restorableCount: 0 },
  restorableCount: 0
};

const cleanedStatus = {
  ok: true,
  guidance: { totalFiles: 1, pendingCount: 0, appliedCount: 1, restorableCount: 1 },
  confirmations: { totalFiles: 1, pendingCount: 0, appliedCount: 1, restorableCount: 1 },
  restorableCount: 2
};

test('inspects status, clears restrictions with backup, and restores cleanly', async () => {
  const f = await fixture({
    inspectResponses: [pendingStatus],
    clearResponses: [{
      ok: true,
      updatedTotal: 2,
      message: '已清理 2 份 Computer Use 限制文件，新对话或重启 Codex 后生效。',
      status: cleanedStatus
    }],
    restoreResponses: [{
      ok: true,
      restoredTotal: 2,
      message: '已还原 2 份 Computer Use 原文件，新对话或重启 Codex 后生效。',
      status: pendingStatus
    }]
  });

  await f.render().refresh();
  let hook = f.render();
  assert.equal(hook.loading, false);
  assert.equal(hook.status.restorableCount, 0);

  await hook.clearRestrictions();
  hook = f.render();
  assert.equal(hook.clearing, false);
  assert.equal(hook.status.guidance.pendingCount, 0);
  assert.equal(hook.status.confirmations.pendingCount, 0);
  assert.equal(hook.status.restorableCount, 2);

  await hook.restoreDocs();
  hook = f.render();
  assert.equal(hook.restoring, false);
  assert.equal(hook.status.guidance.pendingCount, 1);
  assert.equal(hook.status.confirmations.pendingCount, 1);
  assert.equal(hook.status.restorableCount, 0);
  assert.deepEqual(f.calls, ['inspect', 'clear', 'restore']);
  assert.equal(f.toasts.length, 2);
  assert.equal(f.toastErrors.length, 0);
});

test('surfaces inspect, clear, and restore failures explicitly without hiding errors', async () => {
  const f = await fixture({
    inspectResponses: [new Error('fixture inspect permission denied'), pendingStatus],
    clearResponses: [new Error('fixture write locked')],
    restoreResponses: [new Error('fixture backup missing')]
  });

  await f.render().refresh({ notifyError: true });
  let hook = f.render();
  assert.equal(hook.loading, false);
  assert.equal(hook.error, 'fixture inspect permission denied');

  await hook.refresh();
  hook = f.render();
  assert.equal(hook.error, '');

  await hook.clearRestrictions();
  hook = f.render();
  assert.equal(hook.clearing, false);
  assert.equal(hook.error, 'fixture write locked');

  await hook.restoreDocs();
  hook = f.render();
  assert.equal(hook.restoring, false);
  assert.equal(hook.error, 'fixture backup missing');
  assert.deepEqual(f.toastErrors, [
    'fixture inspect permission denied',
    'fixture write locked',
    'fixture backup missing'
  ]);
});

test('desktopApi registers inspect, clear, and restore commands for Computer Use boost', () => {
  const apiSource = fs.readFileSync(new URL('../renderer/src/desktopApi.js', import.meta.url), 'utf8');
  assert.ok(apiSource.includes('inspect_codex_computer_use_docs'));
  assert.ok(apiSource.includes('clear_codex_computer_use_restrictions'));
  assert.ok(apiSource.includes('restore_codex_computer_use_docs'));
});
