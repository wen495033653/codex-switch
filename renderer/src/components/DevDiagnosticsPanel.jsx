import { useI18n } from '../i18n';

export default function DevDiagnosticsPanel({
  entries,
  errorCount,
  isOpen,
  onClear,
  onToggle,
  warningCount
}) {
  const { t } = useI18n();
  const summaryText = t('{total} 条 / {errors} 错误 / {warnings} 警告', {
    total: entries.length,
    errors: errorCount,
    warnings: warningCount
  });

  if (!isOpen) {
    return null;
  }

  return (
    <section className="dev-diagnostics-panel" aria-label={t('开发日志')}>
      <div className="dev-diagnostics-header">
        <div className="dev-diagnostics-title">
          <strong>{t('开发日志')}</strong>
          <span>{summaryText}</span>
        </div>
        <div className="dev-diagnostics-actions">
          <button type="button" onClick={onClear}>{t('清空')}</button>
          <button type="button" onClick={onToggle}>{t('关闭')}</button>
        </div>
      </div>

      <div className="dev-diagnostics-list">
        {entries.length === 0 ? (
          <div className="dev-diagnostics-empty">{t('暂无日志')}</div>
        ) : entries.map(entry => (
          <article key={entry.id} className={`dev-diagnostics-entry ${entry.level}`}>
            <div className="dev-diagnostics-entry-meta">
              <span className="dev-diagnostics-entry-level">{entry.level}</span>
              <span>{entry.time}</span>
              <span>{entry.source}</span>
            </div>
            <pre>{entry.message}</pre>
          </article>
        ))}
      </div>
    </section>
  );
}
