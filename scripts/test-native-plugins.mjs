import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';

async function fixture(restartError) {
  const values = [], calls = [], errors = [];
  let cursor = 0;
  let draft = { codex_plugins_enabled: true };
  const api = {
    async updateSettings(patch) { calls.push('save'); return { settings: patch }; },
    async restartCurrentCodexAppNormal() {
      calls.push('restart');
      if (restartError) throw restartError;
      return { ok: true, restarted: true, message: 'fixture restarted' };
    },
    async getCurrentCodexAppProcesses() { calls.push('status'); return { pids: [1] }; },
  };
  const context = vm.createContext({ window: { api } });
  const stubs = {
    react: { useState(initial) {
      const index = cursor++;
      if (!(index in values)) values[index] = initial;
      return [values[index], value => { values[index] = typeof value === 'function' ? value(values[index]) : value; }];
    } },
    '../utils/appState': { CODEX_DESKTOP_UPDATE_URL: 'https://example.test/update', REPOSITORY_URL: 'https://example.test' },
    '../utils/errors': { getErrorMessage: error => error.message },
  };
  const module = new vm.SourceTextModule(fs.readFileSync(new URL('../renderer/src/hooks/useSettingsActions.js', import.meta.url), 'utf8'), { context });
  await module.link(async specifier => {
    const exports = stubs[specifier];
    if (exports) return new vm.SyntheticModule(Object.keys(exports), function () {
      for (const [name, value] of Object.entries(exports)) this.setExport(name, value);
    }, { context });
    const dependency = new vm.SourceTextModule(fs.readFileSync(new URL(`../renderer/src/hooks/${specifier}.js`, import.meta.url), 'utf8'), { context });
    await dependency.link(() => { throw Error('Unexpected nested import'); });
    return dependency;
  });
  await module.evaluate();
  return { calls, errors, render() {
    cursor = 0;
    return module.namespace.useSettingsActions({
      settings: draft, settingsDraft: draft,
      setSettingsDraft: value => { draft = typeof value === 'function' ? value(draft) : value; },
      applySettings: value => { draft = value.settings; },
      toast() {}, toastError(error) { errors.push(error.message); }, setViewMode() {}, subscriptionModeActive: false,
    });
  } };
}

test('retired plugin flag does not trigger process probing or a restart notice on settings save', async () => {
  const f = await fixture();
  await f.render().updateSettingsDraftAndSave({ ui_language: 'en' });
  assert.deepEqual(f.calls, ['save']);
  assert.equal(f.render().codexRestartNotice.visible, false);
});

test('settings restart notice uses the common command and closes on success', async () => {
  const f = await fixture();
  f.render().setCodexRestartNoticeVisible(true);
  await f.render().codexRestartNotice.onRestart();
  assert.deepEqual(f.calls, ['restart']);
  assert.equal(f.render().codexRestartNotice.visible, false);
});

test('restart failure remains visible and surfaces the actual error', async () => {
  const f = await fixture(new Error('fixture exitCode=23'));
  f.render().setCodexRestartNoticeVisible(true);
  await f.render().codexRestartNotice.onRestart();
  assert.equal(f.render().codexRestartNotice.visible, true);
  assert.equal(f.render().codexRestartNotice.loading, false);
  assert.deepEqual(f.errors, ['fixture exitCode=23']);
});

test('plugin card and obsolete desktop command are absent', () => {
  const source = fs.readFileSync(new URL('../renderer/src/components/settings/ProxySettingsTab.jsx', import.meta.url), 'utf8');
  assert.ok(!source.includes('codex_plugins_enabled'));
  assert.ok(!source.includes('Plugin 增强'));
  const api = fs.readFileSync(new URL('../renderer/src/desktopApi.js', import.meta.url), 'utf8');
  assert.ok(!api.includes('restart_current_codex_app_for_plugin_setting'));
  assert.ok(api.includes('restart_current_codex_app_normal'));
});
