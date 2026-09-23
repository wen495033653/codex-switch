import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';

let server;
let ProxySettingsTab;
let I18nProvider;
let EMPTY_REMOTE_CONTROL_STATUS;

before(async () => {
  server = await createServer({
    configFile: fileURLToPath(new URL('../vite.config.mjs', import.meta.url)),
    server: { middlewareMode: true, hmr: false, ws: false, watch: null },
    appType: 'custom',
    logLevel: 'error'
  });
  ({ default: ProxySettingsTab } = await server.ssrLoadModule('/src/components/settings/ProxySettingsTab.jsx'));
  ({ I18nProvider } = await server.ssrLoadModule('/src/i18n.jsx'));
  ({ EMPTY_REMOTE_CONTROL_STATUS } = await server.ssrLoadModule('/src/hooks/useRemoteControlStatus.js'));
});

after(async () => { await server?.close(); });

function account({ expired = false } = {}) {
  const claims = {
    email: 'fixture@example.invalid',
    'https://api.openai.com/auth': { chatgpt_plan_type: 'pro' }
  };
  return {
    profile_id: 'fixture-profile',
    tokens: {
      account_id: 'fixture-account',
      id_token: ['fixture', Buffer.from(JSON.stringify(claims)).toString('base64url'), 'fixture'].join('.')
    },
    custom: expired ? { auth_status: 'error', auth_status_message: 'invalid_grant' } : {}
  };
}

function renderRemote({ subscription = false, enabled = false, selected = 'fixture-profile', accounts = [account()], pending = null, language = 'zh-CN' } = {}) {
  const html = renderToStaticMarkup(createElement(I18nProvider, { preference: language },
    createElement(ProxySettingsTab, {
      accounts,
      subscriptionModeActive: subscription,
      codexRemoteControlPendingEnabled: pending,
      remoteControlStatus: EMPTY_REMOTE_CONTROL_STATUS,
      settingsDraft: { codex_remote_control_enabled: enabled, codex_remote_control_account_id: selected }
    })));
  const section = html.match(/<section class="[^"]*settings-remote-control-section[^"]*">([\s\S]*?)<\/section>/)?.[1];
  assert.ok(section, 'remote-control section must render');
  assert.doesNotMatch(section, /role="status"|仅 API 模式下使用|请先切换到 API 模式/);
  return section;
}

function toggleIsDisabled(html) {
  const button = html.match(/<button[^>]*class="settings-remote-control-switch[^>]*>/)?.[0];
  assert.ok(button, 'remote-control toggle must render');
  return button.includes('disabled=""');
}

test('subscription mode renders one mode badge and no expanded account or warning sections', () => {
  for (const state of [
    { selected: 'removed' },
    { accounts: [account({ expired: true })] },
    { selected: '', accounts: [] }
  ]) {
    const html = renderRemote({ subscription: true, ...state });
    assert.equal(html.split('仅 API 模式').length - 1, 1);
    assert.doesNotMatch(html, /settings-remote-control-(note|account-grid|status-badge)|<select|登录已失效|已选控制账号不存在/);
    assert.equal(toggleIsDisabled(html), true);
  }
});

test('an already-enabled setting can still be switched off in subscription mode', () => {
  const html = renderRemote({ subscription: true, enabled: true });
  assert.equal(toggleIsDisabled(html), false);
  assert.match(html, /aria-pressed="true"/);
});

test('API mode shows a removed account warning only in its selectable placeholder', () => {
  const html = renderRemote({ selected: 'removed' });
  assert.equal(html.split('已选控制账号不存在，请重新选择').length - 1, 1);
  assert.match(html, /<option[^>]*selected=""[^>]*>已选控制账号不存在，请重新选择<\/option>/);
  assert.doesNotMatch(html, /settings-remote-control-status-badge/);
  assert.doesNotMatch(html.match(/<select[^>]*>/)[0], /disabled=/);
  assert.equal(toggleIsDisabled(html), true);
});

test('API mode without accounts retains the empty selector and disabled toggle', () => {
  const html = renderRemote({ selected: '', accounts: [] });
  assert.match(html, /<option[^>]*>未选择<\/option>/);
  assert.match(html.match(/<select[^>]*>/)[0], /disabled=""/);
  assert.doesNotMatch(html, /settings-remote-control-status-badge/);
  assert.equal(toggleIsDisabled(html), true);
});

test('expired login is identified once in the account option, not repeated as a status badge', () => {
  const html = renderRemote({ accounts: [account({ expired: true })] });
  assert.equal(html.split('登录已失效').length - 1, 1);
  assert.match(html, /<option[^>]*selected=""[^>]*>[^<]*登录已失效[^<]*<\/option>/);
  assert.doesNotMatch(html, /settings-remote-control-status-badge/);
  assert.equal(toggleIsDisabled(html), true);
});

test('valid API account retains runtime status and the available toggle', () => {
  const html = renderRemote();
  assert.match(html, /settings-remote-control-status-badge/);
  assert.match(html, /未启用/);
  assert.equal(toggleIsDisabled(html), false);
  assert.equal(html.split('请求流量走 API，控制操作使用选定的 Codex 登录账号。').length - 1, 1);
});

test('enabled API mode retains pending feedback and locks account selection', () => {
  const html = renderRemote({ enabled: true, pending: true });
  assert.match(html, /settings-remote-control-status-badge/);
  assert.match(html, /打开中/);
  assert.match(html.match(/<select[^>]*>/)[0], /disabled=""/);
  assert.equal(toggleIsDisabled(html), false);
});

test('English subscription mode uses the same compact layout', () => {
  const html = renderRemote({ subscription: true, language: 'en' });
  assert.doesNotMatch(html, /仅 API 模式|<select|settings-remote-control-status-badge/);
  assert.match(html, /API/);
  assert.equal(toggleIsDisabled(html), true);
});
