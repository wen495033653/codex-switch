import { useI18n } from '../../i18n';

export default function SessionScanReport({ readableCount, report }) {
  const { t, translateRuntimeText } = useI18n();
  const { errors, warnings, refreshError, hasResult } = report;
  if (refreshError === null && errors.length === 0 && warnings.length === 0) return null;

  const detailGroups = [
    [t('刷新失败'), refreshError ? [refreshError] : []],
    [t('读取失败'), errors],
    [t('扫描提示'), warnings],
  ].filter(([, messages]) => messages.length > 0);

  return (
    <section className="session-scan-report" aria-label={t('会话扫描结果')}>
      <div role="status">
        {refreshError !== null && (
          <div className="session-scan-refresh-error">
            {hasResult ? t('刷新失败，当前显示上一次扫描结果。') : t('扫描会话失败，未获取会话列表。')}
          </div>
        )}
        {errors.length > 0 && (
          <div className="session-scan-incomplete">
            {t('已列出 {readable} 条会话，另有 {failed} 条读取失败。', { readable: readableCount, failed: errors.length })}
          </div>
        )}
        {warnings.length > 0 && <div>{t('有 {count} 条扫描提示。', { count: warnings.length })}</div>}
      </div>
      {detailGroups.length > 0 && (
        <details>
          <summary>{t('查看扫描详情')}</summary>
          <div className="session-scan-details">
            {detailGroups.map(([label, messages]) => (
              <div key={label}>
                <strong>{label}</strong>
                <ul>
                  {messages.map((message, index) => <li key={index}>{translateRuntimeText(message)}</li>)}
                </ul>
              </div>
            ))}
          </div>
        </details>
      )}
    </section>
  );
}
