import { useI18n } from '../../i18n';
import { formatExactTokens, formatTokens } from '../../utils/usageStatsFormat';
import { cacheHitRate, formatBillCost, formatPercent } from '../../utils/usageBills';

function Metric({ label, value, hint, tone }) {
  return (
    <div className={`usage-bills-metric ${tone ? `usage-bills-metric-${tone}` : ''}`}>
      <span>{label}</span>
      <strong>{value}</strong>
      {hint ? <small>{hint}</small> : null}
    </div>
  );
}

// Without a summary (loading, or the read failed) every value is a dash, not a zero.
export default function BillsSummary({ summary, note }) {
  const { language, t } = useI18n();
  if (!summary) {
    return (
      <div className="usage-bills-summary-block">
        <div className="usage-bills-summary">
          {[t('请求数'), t('估算费用'), t('总 tokens'), t('缓存命中率')].map((label, index) => (
            <Metric key={label} label={label} value="—" tone={index === 1 ? 'primary' : ''} />
          ))}
        </div>
      </div>
    );
  }
  const stats = summary;
  const requestCount = Number(stats.request_count) || 0;
  const unpricedCount = Number(stats.unpriced_count) || 0;
  const allUnpriced = requestCount > 0 && unpricedCount === requestCount;
  const costText = allUnpriced ? t('未定价') : formatBillCost(stats.estimated_cost_usd || 0);
  const costHint = unpricedCount > 0 && !allUnpriced
    ? t('另有 {count} 条未定价', { count: formatExactTokens(unpricedCount, language) })
    : '';

  return (
    <div className="usage-bills-summary-block">
      <div className="usage-bills-summary">
        <Metric label={t('请求数')} value={formatExactTokens(requestCount, language)} />
        <Metric label={t('估算费用')} value={costText} hint={costHint} tone="primary" />
        <Metric
          label={t('总 tokens')}
          value={formatTokens(stats.total_tokens, language)}
          hint={t('输入 {input} · 输出 {output}', {
            input: formatTokens(stats.input_tokens, language),
            output: formatTokens(stats.output_tokens, language)
          })}
        />
        <Metric
          label={t('缓存命中率')}
          value={formatPercent(cacheHitRate(stats))}
          hint={t('缓存命中 {cached} · 新增输入 {fresh}', {
            cached: formatTokens(stats.cached_input_tokens, language),
            fresh: formatTokens(stats.fresh_input_tokens, language)
          })}
        />
      </div>
      {note ? <p className="usage-bills-note">{note}</p> : null}
    </div>
  );
}
