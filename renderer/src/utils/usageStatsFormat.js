// Shared by the usage summary on the cards, the usage detail drawer and the bills page.

export const UNPRICED_REASON_LABELS = {
  missing_model_price: '缺少模型价格',
  missing_tier_price: '缺少该档位价格',
  missing_cached_input_price: '缺少缓存输入价格',
  missing_cache_write_price: '缺少缓存写入价格',
  missing_service_tier: '档位未知'
};

export function getWindow(stats, key) {
  const value = stats && stats[key];
  return value && typeof value === 'object' ? value : null;
}

export function hasUsage(windowStats) {
  return Boolean(windowStats && Number(windowStats.total_tokens) > 0);
}

export function formatTokens(value, language) {
  const tokens = Number(value) || 0;
  if (tokens >= 1_000_000) return `${(tokens / 1_000_000).toFixed(tokens >= 10_000_000 ? 1 : 2)}M`;
  if (tokens >= 10_000) return `${Math.round(tokens / 1_000)}K`;
  return new Intl.NumberFormat(language).format(tokens);
}

export function formatCost(windowStats, t) {
  if (!windowStats || windowStats.priced === false || windowStats.estimated_cost_usd === null) {
    return t('未定价');
  }
  const cost = Number(windowStats.estimated_cost_usd) || 0;
  if (cost > 0 && cost < 0.0001) return '<$0.0001';
  return `$${cost.toFixed(cost < 0.01 ? 4 : 2)}`;
}

export function formatExactTokens(value, language) {
  return new Intl.NumberFormat(language).format(Number(value) || 0);
}
