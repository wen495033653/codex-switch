function formatQuotaLabel(limitWindowSeconds) {
    const totalSeconds = Number(limitWindowSeconds);
    if (!Number.isFinite(totalSeconds) || totalSeconds <= 0) return '使用限制';

    const hour = 60 * 60;
    const day = 24 * hour;
    const week = 7 * day;

    if (totalSeconds === 5 * hour) return '5 小时使用限制';
    if (totalSeconds === week) return '每周使用限制';
    if (totalSeconds % week === 0) return `${totalSeconds / week} 周使用限制`;
    if (totalSeconds % day === 0) return `${totalSeconds / day} 天使用限制`;
    if (totalSeconds % hour === 0) return `${totalSeconds / hour} 小时使用限制`;
    return `${Math.round(totalSeconds / 60)} 分钟使用限制`;
}

export default function QuotaItem({ window, variant = 'list' }) {
    const used = Number(window.used_percent) || 0;
    const remaining = Math.max(0, Math.min(100, 100 - used));
    const label = formatQuotaLabel(window.limit_window_seconds);

    let colorClass = 'bg-green';
    let textClass = 'text-green';
    if (remaining < 50) { colorClass = 'bg-yellow'; textClass = 'text-yellow'; }
    if (remaining < 20) { colorClass = 'bg-red'; textClass = 'text-red'; }

    const resetDate = new Date(window.reset_at * 1000);
    const timeStr = resetDate.toLocaleString('zh-CN', {
        month: '2-digit',
        day: '2-digit',
        hour: '2-digit',
        minute: '2-digit',
        hour12: false
    });

    return (
        <div className={`quota-item quota-item-${variant}`}>
            <div className="quota-header">
                <span>{label}</span>
                <div className="quota-values">
                    <span className={textClass}>{remaining.toFixed(0)}%</span>
                    {variant === 'card' && (
                        <span className="quota-reset-inline">{timeStr}</span>
                    )}
                </div>
            </div>
            <div className="quota-track">
                <div className={`quota-fill ${colorClass}`} style={{ width: `${remaining}%` }} />
            </div>
            <div className={`quota-reset quota-reset-${variant}`}>
                重置: {timeStr}
            </div>
        </div>
    );
}
