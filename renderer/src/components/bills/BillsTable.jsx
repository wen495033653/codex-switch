import { useI18n } from '../../i18n';
import { formatExactTokens } from '../../utils/usageStatsFormat';
import {
  BILL_PAGE_SIZE_OPTIONS,
  formatBillCost,
  formatBillTime,
  formatFullTime,
  formatRate,
  formatSeconds,
  isPriced,
  modelLabel,
  outputRate,
  ownerLabel,
  priceLabelText,
  totalPages
} from '../../utils/usageBills';

export default function BillsTable({
  emptyText,
  namesByKey,
  page,
  pageSize,
  records,
  selectedId,
  total,
  onPageChange,
  onPageSizeChange,
  onSelect
}) {
  const { language, t } = useI18n();
  const pages = totalPages(total || 0, pageSize);

  return (
    <div className="usage-bills-panel">
      <div className="usage-bills-list">
        <div className="usage-bills-header">
          <span>{t('时间')}</span>
          <span>{t('归属')}</span>
          <span>{t('模型')}</span>
          <span>{t('计价')}</span>
          <span className="usage-bills-numeric">{t('新增输入')}</span>
          <span className="usage-bills-numeric">{t('缓存命中')}</span>
          <span className="usage-bills-numeric">{t('输出')}</span>
          <span className="usage-bills-numeric">{t('费用')}</span>
          <span className="usage-bills-numeric">{t('耗时')}</span>
        </div>
        {records.map(record => {
          const priced = isPriced(record);
          const owner = ownerLabel(record.owner_type, record.owner_id, namesByKey, t);
          const model = modelLabel(record.model, t);
          const rate = outputRate(record);
          const timingTitle = record.duration_ms === null
            ? t('没有耗时数据')
            : record.first_token_ms === null
              ? t('总耗时 {duration}，Codex 日志里已没有这次请求的首 Token', {
                duration: formatSeconds(record.duration_ms)
              })
              : t('总耗时 {duration} · 首 Token {firstToken} · {rate}', {
                duration: formatSeconds(record.duration_ms),
                firstToken: formatSeconds(record.first_token_ms),
                rate: formatRate(rate)
              });
          return (
            <button
              key={record.response_id}
              type="button"
              className={`usage-bills-row ${selectedId === record.response_id ? 'active' : ''}`}
              onClick={() => onSelect(record)}
            >
              <span title={formatFullTime(record.timestamp, language)}>
                {formatBillTime(record.timestamp, language)}
              </span>
              <span className="usage-bills-text" title={owner}>{owner}</span>
              <span className="usage-bills-text" title={model}>{model}</span>
              <span className={`usage-bills-tier ${priced ? '' : 'unpriced'}`}>
                <span className="usage-bills-text" title={priceLabelText(record.price_label, t)}>
                  {priceLabelText(record.price_label, t)}
                </span>
                {record.legacy ? <em>{t('旧版')}</em> : null}
              </span>
              <span className="usage-bills-numeric">{formatExactTokens(record.fresh_input_tokens, language)}</span>
              <span className="usage-bills-numeric">{formatExactTokens(record.cached_input_tokens, language)}</span>
              <span className="usage-bills-numeric">{formatExactTokens(record.output_tokens, language)}</span>
              <span className={`usage-bills-numeric usage-bills-cost ${priced ? '' : 'unpriced'}`}>
                {priced ? formatBillCost(record.estimated_cost_usd) : t('未定价')}
              </span>
              <span className="usage-bills-numeric usage-bills-timing" title={timingTitle}>
                <span>{formatSeconds(record.duration_ms)}</span>
                {record.first_token_ms !== null ? (
                  <small>{t('TTFT {firstToken} · {rate}', {
                    firstToken: formatSeconds(record.first_token_ms),
                    rate: formatRate(rate)
                  })}</small>
                ) : null}
              </span>
            </button>
          );
        })}
        {records.length === 0 && emptyText ? <div className="usage-bills-empty">{emptyText}</div> : null}
      </div>
      <div className="usage-bills-footer">
        <span>{t('共 {count} 条', { count: total === null ? '—' : formatExactTokens(total, language) })}</span>
        <div className="usage-bills-pager">
          <button type="button" className="btn btn-secondary" onClick={() => onPageChange(1)} disabled={page <= 1}>
            {t('首页')}
          </button>
          <button type="button" className="btn btn-secondary" onClick={() => onPageChange(page - 1)} disabled={page <= 1}>
            {t('上页')}
          </button>
          <span className="usage-bills-page-number">{page}/{pages}</span>
          <button type="button" className="btn btn-secondary" onClick={() => onPageChange(page + 1)} disabled={page >= pages}>
            {t('下页')}
          </button>
          <button type="button" className="btn btn-secondary" onClick={() => onPageChange(pages)} disabled={page >= pages}>
            {t('末页')}
          </button>
          <select
            value={pageSize}
            aria-label={t('每页条数')}
            onChange={event => onPageSizeChange(Number(event.target.value))}
          >
            {BILL_PAGE_SIZE_OPTIONS.map(value => (
              <option key={value} value={value}>{t('每页 {count}', { count: value })}</option>
            ))}
          </select>
        </div>
      </div>
    </div>
  );
}
