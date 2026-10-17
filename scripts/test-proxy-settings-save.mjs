import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';

async function fixture({ proxyEnvEnabled }) {
  const values = [], calls = [], toasts = [];
  let cursor = 0;
  let settings = { codex_proxy_url: '127.0.0.1:10808', codex_proxy_env_enabled: proxyEnvEnabled };
  let draft = settings;
  const api = {
    async updateSettings(patch) {
      calls.push(['updateSettings', { ...patch }]);
      return { ok: true, message: '设置已保存', settings: { ...settings, ...patch } };
    },
    async setCodexProxyEnvEnabled(payload) {
      calls.push(['setCodexProxyEnvEnabled', { ...payload }]);
      return {
        ok: true,
        message: 'Codex 代理配置已启用，重启 Codex 后生效。',
        restartRequired: true,
        settings: { ...settings, codex_proxy_url: payload.proxyUrl },
        remoteControl: { changed: false }
      };
    },
  };
  const context = vm.createContext({ window: { api } });
  const stubs = {
    react: { useState(initial) {
      const index = cursor++;
      if (!(index in values)) values[index] = initial;
      return [values[index], value => { values[index] = typeof value === 'function' ? value(values[index]) : value; }];
    } },
    '../utils/appState': { CODEX_DESKTOP_UPDATE_URL: 'https://example.test/update', REPOSITORY_URL: 'https://example.test' },
    '../utils/errors': { getErrorMessage: (error, fallback) => error?.message || fallback },
    '../i18n': {
      translate: (source, params) => String(source).replace(/\{(\w+)\}/g, (_, key) => String(params[key])),
      translateRuntimeText: value => value,
    },
  };
  const link = async specifier => {
    const exports = stubs[specifier];
    if (exports) return new vm.SyntheticModule(Object.keys(exports), function () {
      for (const [name, value] of Object.entries(exports)) this.setExport(name, value);
    }, { context });
    const dependency = new vm.SourceTextModule(fs.readFileSync(new URL(`../renderer/src/hooks/${specifier}.js`, import.meta.url), 'utf8'), { context });
    await dependency.link(link);
    return dependency;
  };
  const module = new vm.SourceTextModule(fs.readFileSync(new URL('../renderer/src/hooks/useSettingsActions.js', import.meta.url), 'utf8'), { context });
  await module.link(link);
  await module.evaluate();
  return {
    calls, toasts,
    render() {
      cursor = 0;
      return module.namespace.useSettingsActions({
        settings, settingsDraft: draft,
        setSettingsDraft: value => { draft = typeof value === 'function' ? value(draft) : value; },
        applySettings: res => { settings = res.settings; draft = res.settings; },
        toast(message) { toasts.push(message); }, toastError(error) { toasts.push(error.message); },
        setViewMode() {}, subscriptionModeActive: false,
      });
    },
  };
}

test('leaving the proxy field unedited saves nothing and asks for no restart', async () => {
  for (const proxyEnvEnabled of [true, false]) {
    const f = await fixture({ proxyEnvEnabled });
    await f.render().updateCodexProxySettings({ codex_proxy_url: '127.0.0.1:10808' });
    await f.render().updateCodexProxySettings({ codex_proxy_url: ' 127.0.0.1:10808 ' });
    assert.deepEqual(f.calls, []);
    assert.deepEqual(f.toasts, []);
    assert.equal(f.render().codexRestartNotice.visible, false);
  }
});

test('an edited proxy address is saved through the active proxy path', async () => {
  const enabled = await fixture({ proxyEnvEnabled: true });
  await enabled.render().updateCodexProxySettings({ codex_proxy_url: ' 127.0.0.1:7890 ' });
  assert.deepEqual(enabled.calls, [['setCodexProxyEnvEnabled', { enabled: true, proxyUrl: '127.0.0.1:7890' }]]);
  assert.equal(enabled.render().codexRestartNotice.visible, true);

  const disabled = await fixture({ proxyEnvEnabled: false });
  await disabled.render().updateCodexProxySettings({ codex_proxy_url: '127.0.0.1:7890' });
  assert.deepEqual(disabled.calls, [['updateSettings', { codex_proxy_url: '127.0.0.1:7890' }]]);
  assert.equal(disabled.render().codexRestartNotice.visible, false);
});
