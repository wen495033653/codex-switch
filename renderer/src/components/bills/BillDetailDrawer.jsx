import { useEffect } from 'react';
import { useI18n } from '../../i18n';
import { UNPRICED_REASON_LABELS, formatExactTokens } from '../../utils/usageStatsFormat';
import {
  chargeLabel,
  formatBillCost,
  formatFullTime,
  formatRate,
  formatSeconds,
  formatUnitPrice,
  isPriced,
  modelLabel,
  outputRate,
  ownerTypeLabel,
  priceLabelText
} from '../../utils/usageBills';

function Row({ label, value, mono = false }) {
  return (
    <div className="usage-detail-row">
      <span>{label}</span>
      <strong className={mono ? 'usage-bill-mono' : ''} title={typeof value === 'string' ? value : undefined}>
        {value}
      </strong>
    </div>
  );
}

function Section({ title, children }) {
  return (
    <div className="usage-detail-section">
      <div className="usage-detail-section-title">{title}</div>
      {children}
    </div>
  );
}

export default function BillDetailDrawer({ ownerName, record, onClose }) {
  const { language, t } = useI18n();
  const isOpen = Boolean(record);

  useEffect(() => {
    if (!isOpen) return undefined;
    const handleKeyDown = event => {
      if (event.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [isOpen, onClose]);

  if (!record) return null;

  const priced = isPriced(record);
  const tokens = value => formatExactTokens(value, language);
  const pricing = priced
    ? priceLabelText(record.price_label, t)
    : `${t('未定价')} · ${t(UNPRICED_REASON_LABELS[record.price_label] || '缺少价格')}`;

  return (
    <div className="usage-detail-overlay" onClick={event => event.target === event.currentTarget && onClose()}>
      <aside className="usage-detail-drawer usage-bill-drawer" role="dialog" aria-modal="true" aria-labelledby="usage-bill-title">
        <div className="usage-detail-head">
          <div className="usage-detail-title-stack">
            <span className="usage-detail-eyebrow">{t('请求账单')}</span>
            <h2 id="usage-bill-title">{formatFullTime(record.timestamp, language)}</h2>
          </div>
          <button type="button" className="usage-detail-close" onClick={onClose} aria-label={t('关闭请求账单')}>
            ×
          </button>
        </div>

        <div className="usage-detail-content">
          <Section title={t('基础信息')}>
            <Row label={t('归属')} value={`${ownerName} · ${ownerTypeLabel(record.owner_type, t)}`} />
            <Row label={t('模型')} value={modelLabel(record.model, t)} />
            <Row label={t('计价')} value={pricing} />
            <Row
              label={t('来源')}
              value={record.legacy ? t('旧版统计迁移（按会话事件，标准档短上下文计价）') : t('Codex 会话记录')}
            />
          </Section>

          <Section title={t('Token 明细')}>
            <Row label={t('输入')} value={tokens(record.input_tokens)} />
            <Row label={t('其中新增输入')} value={tokens(record.fresh_input_tokens)} />
            <Row label={t('其中缓存命中')} value={tokens(record.cached_input_tokens)} />
            <Row label={t('其中缓存写入')} value={tokens(record.cache_write_input_tokens)} />
            <Row label={t('输出')} value={tokens(record.output_tokens)} />
            <Row label={t('其中推理')} value={tokens(record.reasoning_output_tokens)} />
            <Row label={t('总计')} value={tokens(record.total_tokens)} />
          </Section>

          <Section title={t('时序')}>
            <Row label={t('总耗时')} value={formatSeconds(record.duration_ms)} />
            <Row label={t('首 Token（近似）')} value={formatSeconds(record.first_token_ms)} />
            <Row label={t('输出速率')} value={formatRate(outputRate(record))} />
            <p className="usage-bill-note">
              {record.duration_ms === null
                ? t('会话记录里没有这次请求的输出项（例如上下文压缩），或会话文件已删除，算不出耗时。')
                : t('总耗时是会话记录里请求发出到响应完成的时间。首 Token 是会话记录里第一个思考或回复开始生成的时间，没有时取 Codex 日志里第一个输出项到达的时间。输出速率 = 输出 token ÷（总耗时 − 首 Token），输出不到 100 token 或首 Token 到完成不到 0.1 秒时不算。')}
            </p>
          </Section>

          <Section title={t('费用明细（美元）')}>
            {Array.isArray(record.charges) ? record.charges.map(charge => (
              <Row
                key={charge.kind}
                label={chargeLabel(charge.kind, t)}
                value={charge.price_per_million === null
                  ? formatBillCost(charge.cost_usd)
                  : t('{tokens} × {price} = {cost}', {
                    tokens: tokens(charge.tokens),
                    price: formatUnitPrice(charge.price_per_million),
                    cost: formatBillCost(charge.cost_usd)
                  })}
              />
            )) : null}
            <Row label={t('合计')} value={priced ? formatBillCost(record.estimated_cost_usd) : t('未定价')} />
            {priced && !Array.isArray(record.charges)
              ? <p className="usage-bill-note">{t('这条记录写入时没有保存单价，只有合计。')}</p>
              : null}
            {record.price_label === 'missing_service_tier'
              ? <p className="usage-bill-note">{t('这条记录写入时模型还没有价格；按新价格表重读会话时没有找到它（会话文件可能已删除），无法确认它运行的档位。')}</p>
              : null}
          </Section>

          <Section title={t('标识')}>
            <Row label="response_id" value={record.response_id} mono />
            <Row label={t('会话 ID')} value={record.thread_id} mono />
          </Section>
        </div>
      </aside>
    </div>
  );
}
