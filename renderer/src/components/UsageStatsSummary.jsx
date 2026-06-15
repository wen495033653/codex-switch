function getWindow(stats, key) {
  const value = stats && stats[key];
  return value && typeof value === 'object' ? value : null;
}

function hasUsage(windowStats) {
  return Boolean(windowStats && Number(windowStats.total_tokens) > 0);
}

function formatTokens(value) {
  const tokens = Number(value) || 0;
  if (tokens >= 1_000_000) return `${(tokens / 1_000_000).toFixed(tokens >= 10_000_000 ? 1 : 2)}M`;
  if (tokens >= 10_000) return `${Math.round(tokens / 1_000)}K`;
  return new Intl.NumberFormat('zh-CN').format(tokens);
}

function formatCost(windowStats) {
  if (!windowStats || windowStats.priced === false || windowStats.estimated_cost_usd === null) {
    return '未定价';
  }
  const cost = Number(windowStats.estimated_cost_usd) || 0;
  if (cost > 0 && cost < 0.0001) return '<$0.0001';
  return `$${cost.toFixed(cost < 0.01 ? 4 : 2)}`;
}

function UsageMetric({ label, windowStats }) {
  const cost = formatCost(windowStats);
  const isUnpriced = cost === '未定价';

  return (
    <div className="usage-stats-metric primary">
      <span className="usage-stats-metric-label">{label}</span>
      <span className="usage-stats-metric-value">{formatTokens(windowStats.total_tokens)}</span>
      <span className={`usage-stats-metric-cost ${isUnpriced ? 'unpriced' : ''}`}>{cost}</span>
    </div>
  );
}

export default function UsageStatsSummary({ stats, onOpenDetails }) {
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
        'aria-label': '查看 token 详情'
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
            <span className="usage-stats-metric-label">今日</span>
            <span className="usage-stats-empty-text">暂无会话</span>
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
        <UsageMetric label="今日" windowStats={today} />
      </div>
    </div>
  );
}
