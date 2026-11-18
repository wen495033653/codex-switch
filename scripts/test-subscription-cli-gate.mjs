import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';

let server;
let SubscriptionIntelligenceModal;
let I18nProvider;

before(async () => {
  server = await createServer({
    configFile: fileURLToPath(new URL('../vite.config.mjs', import.meta.url)),
    server: { middlewareMode: true, hmr: false, ws: false, watch: null },
    appType: 'custom',
    logLevel: 'error'
  });
  ({ default: SubscriptionIntelligenceModal } = await server.ssrLoadModule('/src/components/SubscriptionIntelligenceModal.jsx'));
  ({ I18nProvider } = await server.ssrLoadModule('/src/i18n.jsx'));
});

after(async () => { await server?.close(); });

function renderModal(modelList, selectedTestIds = ['candy'], language = 'zh-CN') {
  const html = renderToStaticMarkup(createElement(I18nProvider, { preference: language },
    createElement(SubscriptionIntelligenceModal, {
      accountName: 'Fixture',
      model: 'gpt-test',
      modelList: { models: [], ...modelList },
      onStart() {},
      selectedTestIds
    })));
  return { html, start: html.match(/<button[^>]*iq-start-button[^>]*>/)?.[0] };
}

test('a subscription test cannot start until the bundled CLI is verified', () => {
  const pending = renderModal({ loading: true });
  assert.match(pending.html, /正在获取模型列表/);
  assert.match(pending.start, /disabled/);
  const earlyError = renderModal({ error: 'fixture account error' });
  assert.match(earlyError.html, /title="fixture account error"/);
  assert.match(earlyError.start, /disabled/);

  const missingCli = { cliUnavailable: true, error: 'fixture CLI missing' };
  for (const questionIds of [['candy'], ['candy', 'pelican']]) {
    const { html, start } = renderModal(missingCli, questionIds);
    assert.match(html, /Codex Desktop 自带的 CLI 不可用/);
    assert.match(html, /title="fixture CLI missing"/);
    assert.match(start, /disabled/);
  }
  assert.match(renderModal(missingCli, ['candy'], 'en').html, /Install or update Codex Desktop/);
});

test('a verified CLI enables the test even if the remote model list fails', () => {
  const version = { clientVersion: '0.164.0' };
  const ok = renderModal({ ...version, models: ['gpt-test'] });
  assert.match(ok.html, /1 个可用模型（Codex 0\.164\.0）/);
  assert.doesNotMatch(ok.start, /disabled/);
  const remoteFailure = renderModal({ ...version, error: 'fixture HTTP 503' });
  assert.match(remoteFailure.html, /title="fixture HTTP 503"/);
  assert.doesNotMatch(remoteFailure.start, /disabled/);
});
