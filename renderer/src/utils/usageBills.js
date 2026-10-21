// Pure helpers of the token bills page: filter keys, owner and pricing labels, and number formats.

export const BILL_RANGES = [
  { key: 'today', label: '今日' },
  { key: 'days_7', label: '7 天' },
  { key: 'days_30', label: '30 天' },
  { key: 'all', label: '全部' }
];

export const BILL_PAGE_SIZE_OPTIONS = [50, 100, 200];

export const ALL_FILTER_KEY = '__all__';

export const DEFAULT_BILLS_FILTER = {
  range: 'today',
  ownerKey: ALL_FILTER_KEY,
  modelKey: ALL_FILTER_KEY
};

const UNATTRIBUTED_OWNER_KEY = 'unattributed';

const PRICE_LABEL_TEXT = {
  standard_short_context: '标准',
  standard_long_context: '标准 · 长上下文',
  fast_short_context: '快速',
  fast_long_context: '快速 · 长上下文'
};

const CHARGE_LABELS = {
  input: '新增输入',
  cached_input: '缓存命中',
  cache_write: '缓存写入',
  output: '输出'
};

// Records no attribution matched are stored with an empty owner type.
export function ownerKey(ownerType, ownerId) {
  if (!ownerType) return UNATTRIBUTED_OWNER_KEY;
  return `${ownerType}:${ownerId || ''}`;
}

// The owner part of the backend query; null selects every owner.
export function ownerQuery(key) {
  if (!key || key === ALL_FILTER_KEY) return { ownerType: null, ownerId: null };
  if (key === UNATTRIBUTED_OWNER_KEY) return { ownerType: 'unattributed', ownerId: null };
  const separator = key.indexOf(':');
  return { ownerType: key.slice(0, separator), ownerId: key.slice(separator + 1) };
}

// An empty model is a record whose model was unknown, so "all" needs its own key.
export function modelQuery(key) {
  return key === ALL_FILTER_KEY ? null : key;
}

// The bills of what the token detail drawer shows. The statistics call a model Codex did not
// report 'unknown'; the bills keep it as the empty model it was stored as.
export function billsFilterForOwner(ownerType, ownerId, range, statsModel) {
  return {
    range,
    ownerKey: ownerKey(ownerType, ownerId),
    modelKey: !statsModel ? ALL_FILTER_KEY : statsModel === 'unknown' ? '' : statsModel
  };
}

export function ownerLabel(ownerType, ownerId, namesByKey, t) {
  if (!ownerType) return t('未归属');
  const name = namesByKey.get(ownerKey(ownerType, ownerId));
  if (name) return name;
  const shortId = String(ownerId || '').slice(0, 8);
  return ownerType === 'api_profile'
    ? t('已删除的 API 配置 {id}', { id: shortId })
    : t('已删除的账号 {id}', { id: shortId });
}

export function ownerTypeLabel(ownerType, t) {
  if (ownerType === 'subscription') return t('订阅账号');
  if (ownerType === 'api_profile') return t('API 配置');
  return t('未归属');
}

export function modelLabel(model, t) {
  return model ? model : t('未知模型');
}

export function priceLabelText(priceLabel, t) {
  const text = PRICE_LABEL_TEXT[priceLabel];
  return text ? t(text) : t('未定价');
}

export function chargeLabel(kind, t) {
  return t(CHARGE_LABELS[kind] || kind);
}

export function isPriced(record) {
  return Boolean(record) && record.estimated_cost_usd !== null && record.estimated_cost_usd !== undefined;
}

// One request costs fractions of a cent to a few dollars, so below $1 four decimals are kept.
export function formatBillCost(value) {
  if (value === null || value === undefined) return '';
  const cost = Number(value) || 0;
  if (cost === 0) return '$0';
  if (cost < 0.0001) return '<$0.0001';
  return `$${cost.toFixed(cost < 1 ? 4 : 2)}`;
}

// USD per 1M tokens, with at least two decimals and no trailing zeros beyond them.
export function formatUnitPrice(value) {
  if (value === null || value === undefined) return '';
  const fixed = Number(value).toFixed(4).replace(/0+$/, '');
  const [whole, fraction = ''] = fixed.split('.');
  return `$${whole}.${fraction.padEnd(2, '0')}/M`;
}

export function cacheHitRate(summary) {
  const input = Number(summary && summary.input_tokens) || 0;
  if (input <= 0) return null;
  return (Number(summary.cached_input_tokens) || 0) / input;
}

export function formatPercent(ratio) {
  if (ratio === null || ratio === undefined) return '—';
  return `${(ratio * 100).toFixed(1)}%`;
}

export function formatBillTime(value, language) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return String(value || '');
  const sameYear = date.getFullYear() === new Date().getFullYear();
  return new Intl.DateTimeFormat(language, {
    ...(sameYear ? {} : { year: 'numeric' }),
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    hour12: false
  }).format(date);
}

export function formatFullTime(value, language) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return String(value || '');
  return new Intl.DateTimeFormat(language, {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    hour12: false
  }).format(date);
}

// Seconds with two decimals below 10 s, one below 100 s.
export function formatSeconds(ms) {
  if (ms === null || ms === undefined) return '—';
  const seconds = Math.max(0, Number(ms) || 0) / 1000;
  return `${seconds.toFixed(seconds < 10 ? 2 : seconds < 100 ? 1 : 0)}s`;
}

// CC Switch's bounds: a tool call's few tokens, or output that arrived in one burst, give a
// transfer rate rather than a generation speed.
export const RATE_MIN_OUTPUT_TOKENS = 100;
export const RATE_MIN_GENERATION_MS = 100;

// Output tokens over the time from the first output to completion, as CC Switch computes it;
// null without a first token or below its bounds.
export function outputRate(record) {
  if (!record || record.duration_ms == null || record.first_token_ms == null) return null;
  const outputTokens = Number(record.output_tokens) || 0;
  const generationMs = record.duration_ms - record.first_token_ms;
  if (outputTokens < RATE_MIN_OUTPUT_TOKENS || generationMs < RATE_MIN_GENERATION_MS) return null;
  return outputTokens / (generationMs / 1000);
}

export function formatRate(rate) {
  if (rate === null || rate === undefined) return '—';
  return `${rate.toFixed(rate < 100 ? 1 : 0)} tok/s`;
}

export function totalPages(total, pageSize) {
  return Math.max(1, Math.ceil((Number(total) || 0) / pageSize));
}
