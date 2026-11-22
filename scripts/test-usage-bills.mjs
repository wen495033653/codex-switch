import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer, transformWithOxc } from 'vite';
import * as billUtils from '../renderer/src/utils/usageBills.js';
import { locateSession } from '../renderer/src/utils/sessionManager.js';

const {
  ALL_FILTER_KEY,
  DEFAULT_BILLS_FILTER,
  billsFilterForOwner,
  billsFilterForSession,
  cacheHitRate,
  createBillTimeFormatter,
  formatBillCost,
  formatFullTime,
  formatPercent,
  formatRate,
  formatSeconds,
  formatUnitPrice,
  modelQuery,
  outputRate,
  ownerKey,
  ownerLabel,
  ownerQuery,
  priceLabelText,
  totalPages
} = billUtils;

const t = (text, params = {}) => text.replace(/\{(\w+)\}/g, (_, key) => String(params[key]));

test('owner filter keys round-trip to the backend query', () => {
  assert.deepEqual(ownerQuery(ALL_FILTER_KEY), { ownerType: null, ownerId: null });
  assert.deepEqual(ownerQuery(ownerKey('', '')), { ownerType: 'unattributed', ownerId: null });
  // Profile ids may contain the separator; only the first one splits.
  assert.deepEqual(
    ownerQuery(ownerKey('subscription', 'abc:def@example.com')),
    { ownerType: 'subscription', ownerId: 'abc:def@example.com' }
  );
  assert.deepEqual(ownerQuery(ownerKey('api_profile', 'p1')), { ownerType: 'api_profile', ownerId: 'p1' });
});

test('the unknown model is an empty model, distinct from all models', () => {
  assert.equal(modelQuery(ALL_FILTER_KEY), null);
  assert.equal(modelQuery(''), '');
  assert.equal(modelQuery('gpt-5.5'), 'gpt-5.5');
});

test('the drawer link opens the bills of its owner, window and model', () => {
  assert.deepEqual(billsFilterForOwner('subscription', 'acc-1', 'days_7', null), {
    range: 'days_7',
    ownerKey: 'subscription:acc-1',
    modelKey: ALL_FILTER_KEY
  });
  assert.equal(billsFilterForOwner('api_profile', 'p1', 'all', 'unknown').modelKey, '');
  assert.equal(billsFilterForOwner('api_profile', 'p1', 'all', 'gpt-5.5').modelKey, 'gpt-5.5');
});

test('a session opens bills across all owners, models and history', () => {
  const sessionFilter = billsFilterForSession('thread-archived');
  assert.deepEqual(sessionFilter, {
    range: 'all',
    ownerKey: ALL_FILTER_KEY,
    modelKey: ALL_FILTER_KEY,
    threadId: 'thread-archived'
  });
  assert.equal(billsFilterForOwner('api_profile', 'p1', 'today', null).threadId, undefined);
});

test('a bill locates an exact session and finds its page even when hidden by filters', () => {
  const threadId = 'target-id';
  const decoy = { id: 'other-id', title: threadId, status: 'active' };
  const filler = { id: 'archived-filler', title: 'Historical session', status: 'archived' };
  const target = { id: threadId, title: 'Historical session', status: 'archived', relative_path: 'archived_sessions/target.jsonl' };
  const conversations = [decoy, filler, target];

  assert.deepEqual(locateSession(conversations, threadId, 'Historical', 'archived', 1),
    { item: target, page: 2, clearFilters: false });
  assert.deepEqual(locateSession(conversations, threadId, 'does not match', 'active', 2),
    { item: target, page: 2, clearFilters: true });
  assert.equal(locateSession([decoy], threadId, '', 'all', 1), null, 'a matching title is not a matching ID');
  assert.equal(locateSession(conversations, 'target', '', 'all', 1), null, 'a partial ID is not a matching ID');
});

test('owners without a current name are shown by type and short id', () => {
  const names = new Map([[ownerKey('subscription', 'acc-1'), 'a***@example.com']]);
  assert.equal(ownerLabel('subscription', 'acc-1', names, t), 'a***@example.com');
  assert.equal(ownerLabel('subscription', '0123456789abcdef', names, t), '已删除的账号 01234567');
  assert.equal(ownerLabel('api_profile', 'p1', names, t), '已删除的 API 配置 p1');
  assert.equal(ownerLabel('', '', names, t), '未归属');
});

test('price labels name the tier and context or the Auto-review waiver', () => {
  assert.equal(priceLabelText('standard_short_context', t), '标准');
  assert.equal(priceLabelText('fast_long_context', t), '快速 · 长上下文');
  assert.equal(priceLabelText('ultrafast_short_context', t), '超快');
  assert.equal(priceLabelText('ultrafast_long_context', t), '超快 · 长上下文');
  assert.equal(priceLabelText('auto_review_free', t), '免费 · Auto-review');
  assert.equal(priceLabelText('missing_model_price', t), '未定价');
});

test('Ultrafast is shown independently in bills and usage statistics', async () => {
  const server = await createServer({
    configFile: fileURLToPath(new URL('../vite.config.mjs', import.meta.url)),
    server: { middlewareMode: true, hmr: false, ws: false, watch: null },
    appType: 'custom',
    logLevel: 'error'
  });
  try {
    const { I18nProvider } = await server.ssrLoadModule('/src/i18n.jsx');
    const { default: BillsTable } = await server.ssrLoadModule('/src/components/bills/BillsTable.jsx');
    const { default: BillDetailDrawer } = await server.ssrLoadModule('/src/components/bills/BillDetailDrawer.jsx');
    const { default: UsageStatsDetailDrawer } = await server.ssrLoadModule('/src/components/UsageStatsDetailDrawer.jsx');
    const render = (Component, props, preference = 'zh-CN') => renderToStaticMarkup(
      createElement(I18nProvider, { preference }, createElement(Component, props))
    );
    const record = {
      response_id: 'resp-ultrafast',
      timestamp: '2026-11-22T18:40:24Z',
      owner_type: 'subscription',
      owner_id: 'fixture-account',
      model: 'gpt-6.2-sol',
      price_label: 'ultrafast_short_context',
      fresh_input_tokens: 255,
      cached_input_tokens: 247168,
      output_tokens: 327,
      estimated_cost_usd: 0.0854904,
      duration_ms: null,
      first_token_ms: null,
      charges: []
    };
    const table = render(BillsTable, { records: [record], namesByKey: new Map(), page: 1, pageSize: 50, total: 1 });
    assert.match(table, /class="usage-bills-text" title="超快">超快</);
    assert.match(table, /\$0\.0855/);
    assert.doesNotMatch(table, /未定价/);

    const subscription = render(BillDetailDrawer, { ownerName: 'Fixture', record }, 'en');
    assert.match(subscription, />Ultrafast</);
    assert.match(subscription, /USD estimate, not a deduction from included usage or credits/);
    const api = render(BillDetailDrawer, {
      ownerName: 'Fixture',
      record: { ...record, owner_type: 'api_profile', price_label: 'ultrafast_long_context' }
    }, 'en');
    assert.match(api, />Ultrafast · Long context</);
    assert.doesNotMatch(api, /deduction from included usage/);

    const renderStats = (pricing_contexts, language) => render(UsageStatsDetailDrawer, {
      detail: { ownerName: 'Fixture' },
      stats: { today: { total_tokens: 1, priced: true, estimated_cost_usd: 0.08, pricing_contexts } }
    }, language);
    assert.match(renderStats({ ultrafast_short_context: 1 }, 'en'), />Ultrafast mode</);
    assert.match(renderStats({ fast_short_context: 1, ultrafast_long_context: 1 }),
      />含快速模式 · 含超快模式 · 含长上下文</);
  } finally {
    await server.close();
  }
});

test('request costs keep four decimals below one dollar', () => {
  assert.equal(formatBillCost(null), '');
  assert.equal(formatBillCost(0), '$0');
  assert.equal(formatBillCost(0.00005), '<$0.0001');
  assert.equal(formatBillCost(0.01234), '$0.0123');
  assert.equal(formatBillCost(1.5), '$1.50');
});

test('unit prices show at least two decimals per million tokens', () => {
  assert.equal(formatUnitPrice(null), '');
  assert.equal(formatUnitPrice(2), '$2.00/M');
  assert.equal(formatUnitPrice(0.1), '$0.10/M');
  assert.equal(formatUnitPrice(0.025), '$0.025/M');
  assert.equal(formatUnitPrice(1.25), '$1.25/M');
});

test('cache hit rate is cached over all input', () => {
  assert.equal(cacheHitRate({ input_tokens: 0, cached_input_tokens: 0 }), null);
  assert.equal(formatPercent(null), '—');
  assert.equal(formatPercent(cacheHitRate({ input_tokens: 400, cached_input_tokens: 300 })), '75.0%');
});

test('pages and times', () => {
  assert.equal(totalPages(0, 50), 1);
  assert.equal(totalPages(101, 50), 3);
  const thisYear = new Date().getFullYear();
  const formatTime = createBillTimeFormatter('en-GB');
  assert.equal(formatTime(`${thisYear}-03-04T05:06:07`), '04/03, 05:06:07');
  assert.equal(formatTime(`${thisYear}-03-04T05:06:07`, true), `04/03/${thisYear}, 05:06:07`);
  assert.equal(formatTime('2001-03-04T05:06:07'), '04/03/2001, 05:06:07');
  assert.equal(formatTime('not a time'), 'not a time');
  assert.equal(formatFullTime('2001-03-04T05:06:07', 'zh-CN'), '2001/03/04 05:06:07');
});

async function billsPageFixture(initialFilter = DEFAULT_BILLS_FILTER) {
  const values = [], requests = [], effects = [];
  let filter = initialFilter;
  let cursor = 0;
  const jsx = (type, props) => ({ type, props });
  const context = vm.createContext({
    window: { api: { listUsageBills(query) {
      const response = Promise.withResolvers();
      requests.push({ ...response, query });
      return response.promise;
    } } }
  });
  const bindings = {
    react: {
      useState(initial) {
        const index = cursor++;
        if (!(index in values)) values[index] = typeof initial === 'function' ? initial() : initial;
        return [values[index], next => { values[index] = typeof next === 'function' ? next(values[index]) : next; }];
      },
      useRef(initial) { return values[cursor++] ??= { current: initial }; },
      useMemo: compute => compute(),
      useEffect(effect, deps) {
        const previous = values[cursor];
        values[cursor++] = deps;
        if (!previous || deps.some((dep, index) => dep !== previous[index])) effects.push(effect);
      }
    },
    'react/jsx-runtime': { jsx, jsxs: jsx },
    '../i18n': { useI18n: () => ({ language: 'zh-CN', t }) },
    '../utils/auth': { getAccountId() {}, getAccountName() {}, maskAccountDisplayName() {} },
    '../utils/errors': { getErrorMessage: err => { throw err; } },
    '../utils/pollingErrorLog': { createPollingErrorLog: () => ({ succeeded() {}, failed: assert.fail }) },
    '../utils/usageBills': billUtils
  };
  const source = new URL('../renderer/src/components/UsageBillsPage.jsx', import.meta.url);
  const { code } = await transformWithOxc(await fs.readFile(source, 'utf8'), source.pathname, {
    jsx: { runtime: 'automatic' }
  });
  const module = new vm.SourceTextModule(code, { context });
  await module.link(specifier => {
    const exports = specifier.startsWith('./bills/') ? { default: specifier } : bindings[specifier];
    assert.ok(exports, `Unexpected controller import: ${specifier}`);
    return new vm.SyntheticModule(Object.keys(exports), function () {
      for (const [name, value] of Object.entries(exports)) this.setExport(name, value);
    }, { context });
  });
  await module.evaluate();
  const render = (refreshKey = 0) => {
    cursor = 0;
    const tree = module.namespace.default({ filter, onFilterChange: next => { filter = next; }, refreshKey });
    effects.splice(0).forEach(effect => effect());
    const props = path => tree.props.children.find(node => node?.type === `./bills/${path}`).props;
    return { table: props('BillsTable'), toolbar: props('BillsToolbar') };
  };
  const reply = async (request, total, records) => {
    request.resolve({ ok: true, summary: { request_count: total }, records });
    await new Promise(setImmediate);
  };
  return { requests, render, reply };
}

test('a refresh that loses the current page loads the last remaining page before showing results', async () => {
  const { requests, render, reply } = await billsPageFixture();
  const renderTable = refreshKey => render(refreshKey).table;
  const row = { response_id: 'resp' };

  renderTable();
  await reply(requests[0], 51, [row]);
  renderTable().onPageChange(2);
  assert.equal(renderTable().total, null);
  await reply(requests[1], 51, [row]);
  assert.equal(renderTable().page, 2);

  // Simulate midnight (Today loses yesterday's rows) without changing the selected filter.
  renderTable(1);
  assert.equal(requests[2].query.page, 2);
  await reply(requests[2], 3, []);
  const corrected = renderTable(1);
  assert.equal(corrected.page, 1);
  assert.equal(corrected.total, null, 'out-of-range result is not presented as empty');
  assert.equal(requests[3].query.page, 1);
  await reply(requests[3], 3, [row]);
  assert.equal(renderTable(1).total, 3);
  assert.deepEqual(renderTable(1).records, [row]);
  renderTable(2);
  assert.equal(requests[4].query.page, 1, 'later refreshes keep the corrected page');
});

test('clearing a session filter on a later page omits the thread ID and returns to page 1', async () => {
  const { requests, render, reply } = await billsPageFixture(billsFilterForSession('thread-archived'));
  render();
  assert.equal(requests[0].query.range, 'all');
  assert.equal(requests[0].query.threadId, 'thread-archived');
  assert.equal(requests[0].query.ownerType, null);
  assert.equal(requests[0].query.model, null);
  await reply(requests[0], 51, []);

  render().table.onPageChange(2);
  const { toolbar } = render();
  assert.equal(requests[1].query.page, 2);
  assert.equal(requests[1].query.threadId, 'thread-archived');
  toolbar.onClearSession();

  render();
  assert.equal(requests[2].query.page, 1);
  assert.equal('threadId' in requests[2].query, false);
});

test('durations, first token and output rate', () => {
  assert.equal(formatSeconds(null), '—');
  assert.equal(formatSeconds(4_900), '4.90s');
  assert.equal(formatSeconds(12_720), '12.7s');
  assert.equal(formatSeconds(135_400), '135s');
  // 400 output tokens from the first token at 1.75 s to completion at 4.9 s.
  const rate = outputRate({ duration_ms: 4_900, first_token_ms: 1_750, output_tokens: 400 });
  assert.equal(formatRate(rate), '127 tok/s');
  assert.equal(outputRate({ duration_ms: 4_900, first_token_ms: null, output_tokens: 400 }), null);
  assert.equal(outputRate({ duration_ms: 1_000, first_token_ms: 1_000, output_tokens: 400 }), null);
  // CC Switch's bounds: fewer than 100 output tokens, or under 100 ms from first token to end.
  assert.equal(outputRate({ duration_ms: 4_900, first_token_ms: 1_750, output_tokens: 99 }), null);
  assert.equal(formatRate(outputRate({ duration_ms: 4_900, first_token_ms: 1_750, output_tokens: 100 })), '31.7 tok/s');
  assert.equal(outputRate({ duration_ms: 1_099, first_token_ms: 1_000, output_tokens: 400 }), null);
  assert.equal(formatRate(outputRate({ duration_ms: 1_100, first_token_ms: 1_000, output_tokens: 400 })), '4000 tok/s');
  assert.equal(formatRate(null), '—');
  assert.equal(formatRate(154.2), '154 tok/s');
});
