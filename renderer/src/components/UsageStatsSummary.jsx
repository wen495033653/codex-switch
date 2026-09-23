import { useI18n } from '../i18n';
import { formatCost, formatTokens, getWindow, hasUsage } from '../utils/usageStatsFormat';

function UsageMetric({ label, windowStats, language, t }) {
  const cost = formatCost(windowStats, t);
  const isUnpriced = cost === t('未定价');

  return (
    <div className="usage-stats-metric primary">
      <span className="usage-stats-metric-label">{label}</span>
      <span className="usage-stats-metric-value">{formatTokens(windowStats.total_tokens, language)}</span>
      <span className={`usage-stats-metric-cost ${isUnpriced ? 'unpriced' : ''}`}>{cost}</span>
    </div>
  );
}

export default function UsageStatsSummary({ stats, onOpenDetails }) {
  const { language, t } = useI18n();
  const today = getWindow(stats, 'today');
  const hasTodayUsage = hasUsage(today);
  const canOpenDetails = typeof onOpenDetails === 'function';
  const handleOpenDetails = () => {
    if (canOpenDetails) onOpenDetails();
  };
  const handleKeyDown = event => {
    if (!canOpenDetails) return;
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      onOpenDetails();
    }
  };
  const openProps = canOpenDetails
    ? {
        role: 'button',
        tabIndex: 0,
        onClick: handleOpenDetails,
        onKeyDown: handleKeyDown,
        'aria-label': t('查看 token 详情')
      }
    : {};

  if (!hasTodayUsage) {
    return (
      <div
        className={`usage-stats-summary usage-stats-empty ${canOpenDetails ? 'usage-stats-clickable' : ''}`}
        {...openProps}
      >
        <div className="usage-stats-header">
          <span className="usage-stats-title">Token</span>
        </div>
        <div className="usage-stats-metrics">
          <div className="usage-stats-metric usage-stats-metric-empty">
            <span className="usage-stats-metric-label">{t('今日')}</span>
            <span className="usage-stats-empty-text">{t('暂无会话')}</span>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div
      className={`usage-stats-summary ${canOpenDetails ? 'usage-stats-clickable' : ''}`}
      {...openProps}
    >
      <div className="usage-stats-header">
        <span className="usage-stats-title">Token</span>
      </div>
      <div className="usage-stats-metrics">
        <UsageMetric label={t('今日')} windowStats={today} language={language} t={t} />
      </div>
    </div>
  );
}
