import assert from 'node:assert/strict';
import test from 'node:test';
import {
  ALL_FILTER_KEY,
  billsFilterForOwner,
  cacheHitRate,
  formatBillCost,
  formatBillTime,
  formatPercent,
  formatUnitPrice,
  modelQuery,
  ownerKey,
  ownerLabel,
  ownerQuery,
  priceLabelText,
  totalPages
} from '../renderer/src/utils/usageBills.js';

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

test('price labels name the tier and context, anything else is unpriced', () => {
  assert.equal(priceLabelText('standard_short_context', t), '标准');
  assert.equal(priceLabelText('fast_long_context', t), '快速 · 长上下文');
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
  assert.equal(formatBillTime(`${thisYear}-03-04T05:06:07`, 'en-GB'), '04/03, 05:06:07');
  assert.equal(formatBillTime('2001-03-04T05:06:07', 'en-GB'), '04/03/2001, 05:06:07');
  assert.equal(formatBillTime('not a time', 'en-GB'), 'not a time');
});
