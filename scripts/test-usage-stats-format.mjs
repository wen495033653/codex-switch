import assert from 'node:assert/strict';
import test from 'node:test';
import { formatCost, formatTokens, getWindow, hasUsage } from '../renderer/src/utils/usageStatsFormat.js';

const t = text => `t(${text})`;

test('usage windows are read only when they are objects with tokens', () => {
  assert.deepEqual(getWindow({ today: { total_tokens: 1 } }, 'today'), { total_tokens: 1 });
  assert.equal(getWindow({ today: 5 }, 'today'), null);
  assert.equal(getWindow(null, 'today'), null);
  assert.equal(hasUsage({ total_tokens: '12' }), true);
  assert.equal(hasUsage({ total_tokens: 0 }), false);
  assert.equal(hasUsage(null), false);
});

test('token counts are shortened from 10K and 1M', () => {
  assert.equal(formatTokens(9999, 'en'), '9,999');
  assert.equal(formatTokens(12345, 'en'), '12K');
  assert.equal(formatTokens(1_500_000, 'en'), '1.50M');
  assert.equal(formatTokens(15_000_000, 'en'), '15.0M');
  assert.equal(formatTokens(undefined, 'en'), '0');
});

test('costs show four decimals below one cent and mark unpriced windows', () => {
  assert.equal(formatCost(null, t), 't(未定价)');
  assert.equal(formatCost({ priced: false, estimated_cost_usd: 1 }, t), 't(未定价)');
  assert.equal(formatCost({ estimated_cost_usd: null }, t), 't(未定价)');
  assert.equal(formatCost({ estimated_cost_usd: 0.00005 }, t), '<$0.0001');
  assert.equal(formatCost({ estimated_cost_usd: 0.005 }, t), '$0.0050');
  assert.equal(formatCost({ estimated_cost_usd: 0 }, t), '$0.0000');
  assert.equal(formatCost({ estimated_cost_usd: 1.234 }, t), '$1.23');
});
