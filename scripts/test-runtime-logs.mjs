import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import fs from 'node:fs';
import vm from 'node:vm';
import { fileURLToPath } from 'node:url';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';

let server, RuntimeLogView, SettingsPage, I18nProvider;
before(async () => {
  server = await createServer({ configFile: fileURLToPath(new URL('../vite.config.mjs', import.meta.url)),
    server: { middlewareMode: true, hmr: false, ws: false, watch: null }, appType: 'custom', logLevel: 'error' });
  ({ RuntimeLogView } = await server.ssrLoadModule('/src/components/settings/RuntimeLogDialog.jsx'));
  ({ default: SettingsPage } = await server.ssrLoadModule('/src/components/SettingsPage.jsx'));
  ({ I18nProvider } = await server.ssrLoadModule('/src/i18n.jsx'));
});
after(async () => { await server?.close(); });

const entry = (level, id) => ({ id, level, title: 'fixture result', timestamp: '2026-09-20T03:00:00Z',
  event: 'fixture_event', version: '6.0.2', summary: 'fixture summary', action: '', fields: [{ label: '将自动重试', value: false }] });
const render = (Component, props, language = 'zh-CN') => renderToStaticMarkup(createElement(I18nProvider, { preference: language }, createElement(Component, props)));

test('settings exposes runtime logs in release builds at the toolbar right side', () => {
  const html = render(SettingsPage, { settingsTab: 'about', isDevBuild: false, settingsDraft: {} });
  assert.match(html, /settings-page-toolbar/);
  assert.match(html, /settings-log-button/);
  assert.match(html, />日志<\/button>/);
  assert.ok(html.indexOf('settings-log-button') > html.indexOf('settings-tabs'));
});

test('runtime log view has three levels, human-readable evidence, and collapsed details', () => {
  const html = render(RuntimeLogView, { entries: ['success', 'warn', 'error'].map(entry), loading: false });
  for (const level of ['success', 'warn', 'error']) assert.match(html, new RegExp(`runtime-log-level ${level}`));
  assert.match(html, /技术详情/);
  assert.match(html, /<dd>否/);
  assert.doesNotMatch(html, /<details[^>]* open/);
  assert.doesNotMatch(html, /rawBase64/);
});

test('read and write failures are visible instead of a misleading empty state', () => {
  const html = render(RuntimeLogView, { entries: [], loading: false, error: 'fixture read failed', writeError: 'fixture disk full' });
  assert.equal((html.match(/role="alert"/g) || []).length, 2);
  assert.match(html, /fixture read failed/);
  assert.match(html, /fixture disk full/);
  assert.doesNotMatch(html, />暂无日志</);
  assert.match(render(RuntimeLogView, { entries: [], loading: false }, 'en'), /No logs/);
});

async function hookFixture() {
  const values = [];
  const pending = [];
  let cursor = 0, effect, calls = 0;
  const react = {
    useState(initial) { const i = cursor++; if (!(i in values)) values[i] = initial;
      return [values[i], value => { values[i] = value; }]; },
    useRef(initial) { const i = cursor++; return values[i] ||= { current: initial }; },
    useCallback(fn) { return fn; },
    useEffect(fn) { effect = fn; }
  };
  const context = vm.createContext({ window: { api: { getRuntimeLogEntries() {
    calls += 1; return new Promise((resolve, reject) => pending.push({ resolve, reject }));
  } } } });
  const module = new vm.SourceTextModule(fs.readFileSync(new URL('../renderer/src/hooks/useRuntimeLogs.js', import.meta.url), 'utf8'), { context });
  await module.link(() => new vm.SyntheticModule(Object.keys(react), function () {
    for (const [key, value] of Object.entries(react)) this.setExport(key, value);
  }, { context }));
  await module.evaluate();
  return { pending, get calls() { return calls; }, mount() { return effect(); },
    render() { cursor = 0; return module.namespace.useRuntimeLogs(); } };
}

test('log refresh keeps the newest request, reports errors and never auto-retries', async () => {
  const f = await hookFixture();
  const first = f.render().refresh();
  const second = f.render().refresh();
  f.pending[1].resolve({ entries: [entry('warn', 'new')], writeError: 'fixture write failed' });
  await second;
  f.pending[0].resolve({ entries: [entry('success', 'old')] });
  await first;
  assert.equal(f.render().entries[0].id, 'new');
  assert.equal(f.render().writeError, 'fixture write failed');
  const third = f.render().refresh();
  f.pending[2].reject('fixture read error');
  await third;
  assert.equal(f.render().error, 'fixture read error');
  assert.equal(f.render().loading, false);
  assert.equal(f.calls, 3);
});

test('closing the log dialog discards an in-flight response', async () => {
  const f = await hookFixture();
  f.render();
  const cleanup = f.mount();
  cleanup();
  f.pending[0].resolve({ entries: [entry('warn', 'late')] });
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(f.render().entries.length, 0);
  assert.equal(f.calls, 1);
});
