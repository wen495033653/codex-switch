import { useEffect, useMemo, useState } from 'react';
import { useI18n } from '../i18n';

const ALL_MODELS_KEY = '__all__';

const USAGE_WINDOWS = [
  { key: 'today', label: '今日' },
  { key: 'days_7', label: '7 天' },
  { key: 'days_30', label: '30 天' },
  { key: 'all', label: '全部' }
];

const PRICING_CONTEXT_LABELS = {
  standard_short_context: '标准上下文',
  standard_long_context: '长上下文'
};

const UNPRICED_REASON_LABELS = {
  missing_model_price: '缺少模型价格',
  missing_cached_input_price: '缺少缓存输入价格'
};

function getWindow(stats, key) {
  const value = stats && stats[key];
  return value && typeof value === 'object' ? value : null;
}

function getObjectMap(value) {
  return value && typeof value === 'object' && !Array.isArray(value) ? value : {};
}

function getByModel(windowStats) {
  return getObjectMap(windowStats && windowStats.by_model);
}

function hasUsage(windowStats) {
  return Boolean(windowStats && Number(windowStats.total_tokens) > 0);
}

function formatTokens(value, language) {
  const tokens = Number(value) || 0;
  if (tokens >= 1_000_000) return `${(tokens / 1_000_000).toFixed(tokens >= 10_000_000 ? 1 : 2)}M`;
  if (tokens >= 10_000) return `${Math.round(tokens / 1_000)}K`;
  return new Intl.NumberFormat(language).format(tokens);
}

function formatExactTokens(value, language) {
  return new Intl.NumberFormat(language).format(Number(value) || 0);
}

function formatCost(windowStats, t) {
  if (!windowStats || windowStats.priced === false || windowStats.estimated_cost_usd === null) {
    return t('未定价');
  }
  const cost = Number(windowStats.estimated_cost_usd) || 0;
  if (cost > 0 && cost < 0.0001) return '<$0.0001';
  return `$${cost.toFixed(cost < 0.01 ? 4 : 2)}`;
}

function formatLastUsed(value, language, t) {
  if (!value) return t('无');
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return String(value);
  return new Intl.DateTimeFormat(language, {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    hour12: false
  }).format(date);
}

function getFirstCountedLabel(counts, labels) {
  for (const [key, rawCount] of Object.entries(counts)) {
    if (Number(rawCount) > 0) return labels[key] || key;
  }
  return '';
}

function formatPricingHint(windowStats, t) {
  if (!windowStats) return '';
  const unpricedReason = getFirstCountedLabel(
    getObjectMap(windowStats.unpriced_reasons),
    UNPRICED_REASON_LABELS
  );
  if (windowStats.priced === false) {
    return (unpricedReason ? t(unpricedReason) : t('部分未定价'));
  }

  const contexts = getObjectMap(windowStats.pricing_contexts);
  const longCount = Number(contexts.standard_long_context) || 0;
  const shortCount = Number(contexts.standard_short_context) || 0;
  if (longCount > 0 && shortCount > 0) return t('含长上下文');
  if (longCount > 0) return t(PRICING_CONTEXT_LABELS.standard_long_context);
  return '';
}

function formatModelLabel(model, t) {
  return model === 'unknown' ? t('未知模型') : model;
}

function MetricCard({ label, value, tone, hint }) {
  return (
    <div className={`usage-detail-metric ${tone ? `usage-detail-metric-${tone}` : ''}`}>
      <span>{label}</span>
      <strong>{value}</strong>
      {hint ? <small>{hint}</small> : null}
    </div>
  );
}

function DetailRow({ label, value }) {
  return (
    <div className="usage-detail-row">
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

function ModelFilterRow({ active, label, stats, totalTokens, onClick }) {
  const { language, t } = useI18n();
  const tokens = Number(stats && stats.total_tokens) || 0;
  const percent = totalTokens > 0 ? Math.max(1, Math.round((tokens / totalTokens) * 100)) : 0;
  const hint = formatPricingHint(stats, t);
  const cost = formatCost(stats, t);
  const meta = [
    t('{count} 会话', { count: stats?.session_count || 0 }),
    cost,
    hint
  ].filter(Boolean).join(' · ');
  const breakdownItems = [
    [t('输入'), stats?.input_tokens],
    [t('缓存'), stats?.cached_input_tokens],
    [t('输出'), stats?.output_tokens],
    [t('推理'), stats?.reasoning_output_tokens]
  ];

  return (
    <button
      type="button"
      className={`usage-detail-model-row ${active ? 'active' : ''}`}
      onClick={onClick}
      aria-pressed={active}
    >
      <div className="usage-detail-model-copy">
        <span className="usage-detail-model-name">{label}</span>
        <span className="usage-detail-model-meta">{meta}</span>
      </div>
      <div className="usage-detail-model-stat">
        <strong>{formatTokens(tokens, language)}</strong>
        <span>{percent}%</span>
      </div>
      <div className="usage-detail-model-bar" aria-hidden="true">
        <i style={{ width: `${percent}%` }} />
      </div>
      {active && (
        <div className="usage-detail-model-breakdown" aria-label={`${label} ${t('Token 明细')}`}>
          {breakdownItems.map(([itemLabel, value]) => (
            <span key={itemLabel}>
              <em>{itemLabel}</em>
              <strong>{formatExactTokens(value, language)}</strong>
            </span>
          ))}
        </div>
      )}
    </button>
  );
}

export default function UsageStatsDetailDrawer({ detail, onClose }) {
  const { language, t } = useI18n();
  const [activeWindow, setActiveWindow] = useState('today');
  const [activeModel, setActiveModel] = useState(ALL_MODELS_KEY);
  const isOpen = Boolean(detail);
  const stats = detail && detail.stats ? detail.stats : null;
  const activeStats = getWindow(stats, activeWindow);
  const byModel = getByModel(activeStats);
  const hasActiveUsage = hasUsage(activeStats);
  const ownerName = detail?.ownerName || t('未命名');
  const ownerTypeLabel = detail?.ownerTypeLabel || t('Token 统计');
  const modelEntries = useMemo(() => {
    return Object.entries(byModel).sort((left, right) => {
      const rightTokens = Number(right[1]?.total_tokens) || 0;
      const leftTokens = Number(left[1]?.total_tokens) || 0;
      if (rightTokens !== leftTokens) return rightTokens - leftTokens;
      return left[0].localeCompare(right[0]);
    });
  }, [byModel]);
  const selectedStats = activeModel === ALL_MODELS_KEY
    ? activeStats
    : byModel[activeModel] || activeStats;
  const selectedModelLabel = activeModel === ALL_MODELS_KEY
    ? t('全部模型')
    : formatModelLabel(activeModel, t);
  const windowCounts = useMemo(() => {
    const counts = {};
    for (const item of USAGE_WINDOWS) {
      counts[item.key] = getWindow(stats, item.key)?.session_count || 0;
    }
    return counts;
  }, [stats]);

  useEffect(() => {
    if (isOpen) {
      setActiveWindow('today');
      setActiveModel(ALL_MODELS_KEY);
    }
  }, [isOpen, ownerName, ownerTypeLabel]);

  useEffect(() => {
    setActiveModel(ALL_MODELS_KEY);
  }, [activeWindow]);

  useEffect(() => {
    if (activeModel !== ALL_MODELS_KEY && !byModel[activeModel]) {
      setActiveModel(ALL_MODELS_KEY);
    }
  }, [activeModel, byModel]);

  useEffect(() => {
    if (!isOpen) return undefined;
    const handleKeyDown = event => {
      if (event.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [isOpen, onClose]);

  if (!isOpen) return null;

  return (
    <div className="usage-detail-overlay" onClick={event => event.target === event.currentTarget && onClose()}>
      <aside className="usage-detail-drawer" role="dialog" aria-modal="true" aria-labelledby="usage-detail-title">
        <div className="usage-detail-head">
          <div className="usage-detail-title-stack">
            <span className="usage-detail-eyebrow">{ownerTypeLabel}</span>
            <h2 id="usage-detail-title">{ownerName}</h2>
          </div>
          <button type="button" className="usage-detail-close" onClick={onClose} aria-label={t('关闭 token 详情')}>
            ×
          </button>
        </div>

        <div className="usage-detail-tabs" role="tablist" aria-label={t('Token 统计窗口')}>
          {USAGE_WINDOWS.map(item => (
            <button
              key={item.key}
              type="button"
              className={`usage-detail-tab ${activeWindow === item.key ? 'active' : ''}`}
              onClick={() => setActiveWindow(item.key)}
              role="tab"
              aria-selected={activeWindow === item.key}
            >
              <span>{t(item.label)}</span>
              <em>{windowCounts[item.key]}</em>
            </button>
          ))}
        </div>

        {hasActiveUsage ? (
          <div className="usage-detail-content">
            <div className="usage-detail-metrics">
              <MetricCard label={t('总 tokens')} value={formatTokens(activeStats.total_tokens, language)} tone="primary" />
              <MetricCard label={t('费用')} value={formatCost(activeStats, t)} hint={formatPricingHint(activeStats, t)} />
              <MetricCard label={t('会话')} value={activeStats.session_count || 0} />
              <MetricCard label={t('最后使用')} value={formatLastUsed(activeStats.last_used, language, t)} />
            </div>

            {modelEntries.length > 0 && (
              <div className="usage-detail-section usage-detail-model-section">
                <div className="usage-detail-section-title">{t('模型分布')}</div>
                <div className="usage-detail-model-list">
                  <ModelFilterRow
                    active={activeModel === ALL_MODELS_KEY}
                    label={t('全部模型')}
                    stats={activeStats}
                    totalTokens={Number(activeStats.total_tokens) || 0}
                    onClick={() => setActiveModel(ALL_MODELS_KEY)}
                  />
                  {modelEntries.map(([model, modelStats]) => (
                    <ModelFilterRow
                      key={model}
                      active={activeModel === model}
                      label={formatModelLabel(model, t)}
                      stats={modelStats}
                      totalTokens={Number(activeStats.total_tokens) || 0}
                      onClick={() => setActiveModel(model)}
                    />
                  ))}
                </div>
              </div>
            )}

            <div className="usage-detail-section">
              <div className="usage-detail-section-title usage-detail-section-title-split">
                <span>{t('Token 明细')}</span>
                <em>{selectedModelLabel}</em>
              </div>
              <DetailRow label={t('输入')} value={formatExactTokens(selectedStats.input_tokens, language)} />
              <DetailRow label={t('缓存输入')} value={formatExactTokens(selectedStats.cached_input_tokens, language)} />
              <DetailRow label={t('输出')} value={formatExactTokens(selectedStats.output_tokens, language)} />
              <DetailRow label={t('推理输出')} value={formatExactTokens(selectedStats.reasoning_output_tokens, language)} />
              <DetailRow label={t('总计')} value={formatExactTokens(selectedStats.total_tokens, language)} />
            </div>
          </div>
        ) : (
          <div className="usage-detail-empty">
            <strong>{t('{window}暂无会话', { window: t(USAGE_WINDOWS.find(item => item.key === activeWindow)?.label || '当前窗口') })}</strong>
            <span>{t('只统计功能上线后且能匹配到此卡片的 Codex session。')}</span>
          </div>
        )}
      </aside>
    </div>
  );
}
