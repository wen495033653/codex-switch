// Shared by the usage summary on the cards and the usage detail drawer.

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
