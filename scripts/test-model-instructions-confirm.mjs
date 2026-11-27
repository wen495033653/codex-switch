import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';

async function fixture(responses) {
  const values = [];
  let cursor = 0;
  const calls = [], applied = [], errors = [];
  let draft = { codex_model_instructions_enabled: false };
  const api = {
    async setCodexModelInstructionsEnabled(payload) {
      calls.push(payload);
      const response = responses.shift();
      if (response instanceof Error) throw response;
      return response;
    },
    async getCurrentCodexAppProcesses() { return { pids: [] }; },
  };
  const context = vm.createContext({ window: { api } });
  const stubs = {
    react: { useState(initial) {
      const index = cursor++;
      if (!(index in values)) values[index] = initial;
      return [values[index], value => { values[index] = typeof value === 'function' ? value(values[index]) : value; }];
    } },
    '../utils/appState': { CODEX_DESKTOP_UPDATE_URL: 'https://example.test/update', REPOSITORY_URL: 'https://example.test' },
    '../utils/errors': { getErrorMessage: (error, defaultMessage) => error?.message || defaultMessage },
    '../i18n': {
      translate: (source, params) => String(source).replace(/\{(\w+)\}/g, (_, key) => String(params[key])),
      translateRuntimeText: value => value,
    },
  };
  const source = fs.readFileSync(new URL('../renderer/src/hooks/useSettingsActions.js', import.meta.url), 'utf8');
  const module = new vm.SourceTextModule(source, { context });
  const link = async specifier => {
    if (stubs[specifier]) {
      const exports = stubs[specifier];
      return new vm.SyntheticModule(Object.keys(exports), function () {
        for (const [name, value] of Object.entries(exports)) this.setExport(name, value);
      }, { context });
    }
    const url = new URL(`../renderer/src/hooks/${specifier}.js`, import.meta.url);
    const dependency = new vm.SourceTextModule(fs.readFileSync(url, 'utf8'), { context });
    await dependency.link(link);
    return dependency;
  };
  await module.link(link);
  await module.evaluate();
  return {
    calls, applied, errors,
    get draft() { return draft; },
    render() {
      cursor = 0;
      return module.namespace.useSettingsActions({
        settings: draft, settingsDraft: draft,
        setSettingsDraft: value => { draft = typeof value === 'function' ? value(draft) : value; },
        applySettings: value => { applied.push(value); draft = value.settings; },
        toast(message) { errors.push(message); }, toastError(error) { errors.push(error.message); }, setViewMode() {},
        subscriptionModeActive: false,
      });
    },
  };
}

const existingConfirmation = () => ({
  ok: false,
  confirmationRequired: true,
  path: 'fixture/gpt-unrestricted.md',
  exists: true,
  backupPath: 'fixture/gpt-unrestricted.md.bak',
});
const freshConfirmation = () => ({
  ok: false,
  confirmationRequired: true,
  path: 'fixture/gpt-unrestricted.md',
  exists: false,
  backupPath: null,
});
const success = enabled => ({ ok: true, restartRequired: true, settings: { codex_model_instructions_enabled: enabled } });

test('replace prompt exposes .bak info without enabling, and cancel performs no second request', async () => {
  const f = await fixture([existingConfirmation()]);
  await f.render().setCodexModelInstructionsEnabled(true);
  assert.equal(f.draft.codex_model_instructions_enabled, false);
  assert.equal(f.applied.length, 0);
  const modal = f.render().modelInstructionsConfirmation;
  assert.equal(modal.exists, true);
  assert.equal(modal.backupPath, 'fixture/gpt-unrestricted.md.bak');
  modal.onCancel();
  assert.equal(f.render().modelInstructionsConfirmation, null);
  assert.equal(f.calls.length, 1);
  assert.equal(f.calls[0].overwriteLocal, null);
});

test('confirming replacement sends overwriteLocal=true and closes only after success', async () => {
  for (const prompt of [existingConfirmation(), freshConfirmation()]) {
    const f = await fixture([prompt, success(true)]);
    await f.render().setCodexModelInstructionsEnabled(true);
    await f.render().modelInstructionsConfirmation.onConfirm();
    assert.equal(f.calls[1].overwriteLocal, true);
    assert.equal(f.render().modelInstructionsConfirmation, null);
    assert.equal(f.draft.codex_model_instructions_enabled, true);
  }
});

test('failed replacement leaves confirmation modal open and enabled state unchanged', async () => {
  const f = await fixture([existingConfirmation(), new Error('fixture backup failure')]);
  await f.render().setCodexModelInstructionsEnabled(true);
  await f.render().modelInstructionsConfirmation.onConfirm();
  assert.notEqual(f.render().modelInstructionsConfirmation, null);
  assert.equal(f.draft.codex_model_instructions_enabled, false);
  assert.equal(f.applied.length, 0);
  assert.ok(f.errors.some(message => message.includes('fixture backup failure')));
});

test('disable completes directly without a confirmation dialog', async () => {
  const f = await fixture([success(false)]);
  await f.render().setCodexModelInstructionsEnabled(false);
  assert.equal(f.render().modelInstructionsConfirmation, null);
  assert.equal(f.draft.codex_model_instructions_enabled, false);
});
