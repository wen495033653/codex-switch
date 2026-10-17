import { useEffect, useMemo, useRef, useState } from 'react';
import { useI18n } from '../i18n';
import { getAccountId, getAccountName, maskAccountDisplayName } from '../utils/auth';
import { getErrorMessage } from '../utils/errors';
import { createPollingErrorLog } from '../utils/pollingErrorLog';
import {
  ALL_FILTER_KEY,
  BILL_PAGE_SIZE_OPTIONS,
  formatFullTime,
  modelLabel,
  modelQuery,
  ownerKey,
  ownerLabel,
  ownerQuery
} from '../utils/usageBills';
import BillDetailDrawer from './bills/BillDetailDrawer';
import BillsSummary from './bills/BillsSummary';
import BillsTable from './bills/BillsTable';
import BillsToolbar from './bills/BillsToolbar';

const OWNER_GROUPS = [
  { ownerType: 'subscription', label: '订阅账号' },
  { ownerType: 'api_profile', label: 'API 配置' },
  { ownerType: '', label: '未归属' }
];

// The selected option stays in the list when the current range has no records for it.
function withSelected(options, selectedKey, buildSelected) {
  if (selectedKey === ALL_FILTER_KEY || options.some(option => option.key === selectedKey)) {
    return options;
  }
  return [...options, buildSelected(selectedKey)];
}

export default function UsageBillsPage({
  accounts,
  apiProfiles,
  filter,
  maskAccountName,
  onFilterChange,
  refreshKey
}) {
  const { language, t } = useI18n();
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(BILL_PAGE_SIZE_OPTIONS[0]);
  // The response with the query it answers: a failed or pending query shows no other query's data.
  const [answer, setAnswer] = useState(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  const [selectedRecord, setSelectedRecord] = useState(null);
  const latestRequestRef = useRef(0);
  const [errorLog] = useState(() => createPollingErrorLog('usage_bills_list'));

  const namesByKey = useMemo(() => {
    const names = new Map();
    for (const account of accounts || []) {
      const accountId = getAccountId(account);
      if (!accountId) continue;
      const name = getAccountName(account);
      names.set(ownerKey('subscription', accountId), maskAccountName ? maskAccountDisplayName(name) : name);
    }
    (apiProfiles || []).forEach((profile, index) => {
      if (profile && profile.id) {
        names.set(ownerKey('api_profile', profile.id), profile.name || `API ${index + 1}`);
      }
    });
    return names;
  }, [accounts, apiProfiles, maskAccountName]);

  const queryKey = JSON.stringify([filter.range, filter.ownerKey, filter.modelKey, page, pageSize]);
  const result = answer && answer.queryKey === queryKey ? answer.result : null;

  // refreshKey changes after every usage statistics refresh, which is when new records arrive.
  useEffect(() => {
    const requestId = latestRequestRef.current + 1;
    latestRequestRef.current = requestId;
    setLoading(true);
    window.api.listUsageBills({
      range: filter.range,
      ...ownerQuery(filter.ownerKey),
      model: modelQuery(filter.modelKey),
      page,
      pageSize
    })
      .then(res => {
        if (latestRequestRef.current !== requestId) return;
        if (!res || res.ok !== true) {
          throw new Error((res && res.message) || '读取 token 账单失败');
        }
        errorLog.succeeded();
        setAnswer({ queryKey, result: res });
        setError('');
      })
      .catch(err => {
        if (latestRequestRef.current !== requestId) return;
        errorLog.failed(err);
        setError(getErrorMessage(err, '读取 token 账单失败'));
      })
      .finally(() => {
        if (latestRequestRef.current === requestId) setLoading(false);
      });
  }, [errorLog, filter.modelKey, filter.ownerKey, filter.range, page, pageSize, queryKey, refreshKey]);

  const ownerGroups = useMemo(() => {
    const options = (result?.owners || []).map(owner => ({
      key: ownerKey(owner.owner_type, owner.owner_id),
      ownerType: owner.owner_type,
      label: ownerLabel(owner.owner_type, owner.owner_id, namesByKey, t),
      count: owner.request_count
    }));
    const listed = withSelected(options, filter.ownerKey, key => {
      const separator = key.indexOf(':');
      const ownerType = separator > 0 ? key.slice(0, separator) : '';
      const ownerId = separator > 0 ? key.slice(separator + 1) : '';
      return { key, ownerType, label: ownerLabel(ownerType, ownerId, namesByKey, t), count: 0 };
    });
    return OWNER_GROUPS
      .map(group => ({
        label: t(group.label),
        options: listed.filter(option => option.ownerType === group.ownerType)
      }))
      .filter(group => group.options.length > 0);
  }, [filter.ownerKey, namesByKey, result, t]);

  const modelOptions = useMemo(() => {
    const options = (result?.models || []).map(item => ({
      key: item.model,
      label: modelLabel(item.model, t),
      count: item.request_count
    }));
    return withSelected(options, filter.modelKey, key => ({ key, label: modelLabel(key, t), count: 0 }));
  }, [filter.modelKey, result, t]);

  const historyNote = filter.range === 'all' && result?.has_session_statistics
    ? t('逐条账单从 {date} 开始记录。更早的用量来自旧版统计，账单里只有其最后 30 天按事件迁移的记录（标「旧版」），所以这里的合计会少于卡片上的「全部」。', {
      date: formatFullTime(result.records_counted_after, language)
    })
    : '';

  const changeFilter = patch => {
    onFilterChange({ ...filter, ...patch });
    setPage(1);
  };

  const selectedOwnerName = selectedRecord
    ? ownerLabel(selectedRecord.owner_type, selectedRecord.owner_id, namesByKey, t)
    : '';

  return (
    <div className="usage-bills-page">
      <BillsToolbar
        filter={filter}
        modelOptions={modelOptions}
        ownerGroups={ownerGroups}
        onModelChange={modelKey => changeFilter({ modelKey })}
        onOwnerChange={ownerKey => changeFilter({ ownerKey, modelKey: ALL_FILTER_KEY })}
        onRangeChange={range => changeFilter({ range })}
      />
      {error && (
        <div className="usage-bills-error" role="alert">
          {error}
        </div>
      )}
      <BillsSummary summary={result?.summary} note={historyNote} />
      <BillsTable
        emptyText={result ? t('这个范围内没有请求记录') : loading ? t('加载中...') : ''}
        namesByKey={namesByKey}
        page={page}
        pageSize={pageSize}
        records={result?.records || []}
        selectedId={selectedRecord?.response_id || ''}
        total={result ? result.summary.request_count : null}
        onPageChange={setPage}
        onPageSizeChange={value => {
          setPageSize(value);
          setPage(1);
        }}
        onSelect={setSelectedRecord}
      />
      <BillDetailDrawer
        ownerName={selectedOwnerName}
        record={selectedRecord}
        onClose={() => setSelectedRecord(null)}
      />
    </div>
  );
}
