import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import test from 'node:test';
import vm from 'node:vm';
import { transformWithOxc } from 'vite';
import * as billUtils from '../renderer/src/utils/usageBills.js';

const {
  ALL_FILTER_KEY,
  DEFAULT_BILLS_FILTER,
  billsFilterForOwner,
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
  assert.equal(priceLabelText('auto_review_free', t), '免费 · Auto-review');
  assert.equal(priceLabelText('missing_model_price', t), '未定价');
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

test('a refresh that loses the current page loads the last remaining page before showing results', async () => {
  const values = [], requests = [], effects = [];
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
  const renderTable = (refreshKey = 0) => {
    cursor = 0;
    const tree = module.namespace.default({ filter: DEFAULT_BILLS_FILTER, refreshKey });
    effects.splice(0).forEach(effect => effect());
    return tree.props.children.find(node => node?.type === './bills/BillsTable').props;
  };
  const reply = async (request, total, records) => {
    request.resolve({ ok: true, summary: { request_count: total }, records });
    await new Promise(setImmediate);
  };
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
