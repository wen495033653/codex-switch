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
    '../utils/appState': { REPOSITORY_URL: 'https://example.test' },
    '../utils/errors': { getErrorMessage: (error, defaultMessage) => error?.message || defaultMessage },
  };
  const source = fs.readFileSync(new URL('../renderer/src/hooks/useSettingsActions.js', import.meta.url), 'utf8');
  const module = new vm.SourceTextModule(source, { context });
  await module.link(async specifier => {
    if (stubs[specifier]) {
      const exports = stubs[specifier];
      return new vm.SyntheticModule(Object.keys(exports), function () {
        for (const [name, value] of Object.entries(exports)) this.setExport(name, value);
      }, { context });
    }
    const url = new URL(`../renderer/src/hooks/${specifier}.js`, import.meta.url);
    const dependency = new vm.SourceTextModule(fs.readFileSync(url, 'utf8'), { context });
    await dependency.link(() => { throw Error('Unexpected nested import'); });
    return dependency;
  });
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

const confirmation = () => ({ ok: false, confirmationRequired: true, path: 'fixture/gpt-unrestricted.md' });
const success = enabled => ({ ok: true, restartRequired: true, settings: { codex_model_instructions_enabled: enabled } });

test('existing-file prompt does not enable, and cancel performs no second request', async () => {
  const f = await fixture([confirmation()]);
  await f.render().setCodexModelInstructionsEnabled(true);
  assert.equal(f.draft.codex_model_instructions_enabled, false);
  assert.equal(f.applied.length, 0);
  f.render().modelInstructionsConfirmation.onCancel();
  assert.equal(f.render().modelInstructionsConfirmation, null);
  assert.equal(f.calls.length, 1);
  assert.equal(f.calls[0].overwriteLocal, null);
});

test('keep and overwrite send distinct explicit decisions and close only after success', async () => {
  for (const overwrite of [false, true]) {
    const f = await fixture([confirmation(), success(true)]);
    await f.render().setCodexModelInstructionsEnabled(true);
    const modal = f.render().modelInstructionsConfirmation;
    await (overwrite ? modal.onOverwrite() : modal.onKeep());
    assert.equal(f.calls[1].overwriteLocal, overwrite);
    assert.equal(f.render().modelInstructionsConfirmation, null);
    assert.equal(f.draft.codex_model_instructions_enabled, true);
  }
});

test('failed overwrite leaves local choice dialog open and enabled state unchanged', async () => {
  const f = await fixture([confirmation(), new Error('fixture backup failure')]);
  await f.render().setCodexModelInstructionsEnabled(true);
  await f.render().modelInstructionsConfirmation.onOverwrite();
  assert.notEqual(f.render().modelInstructionsConfirmation, null);
  assert.equal(f.draft.codex_model_instructions_enabled, false);
  assert.equal(f.applied.length, 0);
  assert.ok(f.errors.some(message => message.includes('fixture backup failure')));
});

test('disable and first-time enable complete without a confirmation response', async () => {
  for (const enabled of [false, true]) {
    const f = await fixture([success(enabled)]);
    await f.render().setCodexModelInstructionsEnabled(enabled);
    assert.equal(f.render().modelInstructionsConfirmation, null);
    assert.equal(f.draft.codex_model_instructions_enabled, enabled);
  }
});
