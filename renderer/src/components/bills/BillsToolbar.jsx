import { useI18n } from '../../i18n';
import { ALL_FILTER_KEY, BILL_RANGES } from '../../utils/usageBills';

export default function BillsToolbar({
  filter,
  modelOptions,
  ownerGroups,
  onModelChange,
  onOwnerChange,
  onRangeChange
}) {
  const { t } = useI18n();
  return (
    <div className="usage-bills-toolbar">
      <div className="nav-tabs usage-bills-ranges" role="tablist" aria-label={t('账单时间范围')}>
        {BILL_RANGES.map(item => (
          <button
            key={item.key}
            type="button"
            role="tab"
            aria-selected={filter.range === item.key}
            className={`nav-item usage-bills-range ${filter.range === item.key ? 'active' : ''}`}
            onClick={() => onRangeChange(item.key)}
          >
            {t(item.label)}
          </button>
        ))}
      </div>
      <label className="usage-bills-select">
        <span>{t('归属')}</span>
        <select value={filter.ownerKey} onChange={event => onOwnerChange(event.target.value)}>
          <option value={ALL_FILTER_KEY}>{t('全部归属')}</option>
          {ownerGroups.map(group => (
            <optgroup key={group.label} label={group.label}>
              {group.options.map(option => (
                <option key={option.key} value={option.key}>
                  {t('{name}（{count} 条）', { name: option.label, count: option.count })}
                </option>
              ))}
            </optgroup>
          ))}
        </select>
      </label>
      <label className="usage-bills-select">
        <span>{t('模型')}</span>
        <select value={filter.modelKey} onChange={event => onModelChange(event.target.value)}>
          <option value={ALL_FILTER_KEY}>{t('全部模型')}</option>
          {modelOptions.map(option => (
            <option key={option.key} value={option.key}>
              {t('{name}（{count} 条）', { name: option.label, count: option.count })}
            </option>
          ))}
        </select>
      </label>
    </div>
  );
}
