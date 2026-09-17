import { useI18n } from '../../i18n';
import {
  STATUS_FILTERS,
} from '../../utils/sessionManager';

export default function SessionFilterBar({
  actionLoading,
  conversations,
  deletedSessions,
  handleImport,
  isDeletedView,
  loading,
  refreshSessions,
  rootPath,
  search,
  setSearch,
  setStatusFilter,
  statusFilter,
}) {
  const { t } = useI18n();
  return (
    <div className="session-filterbar">
      <div className="search-wrapper session-search">
        <svg className="search-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden="true">
          <circle cx="11" cy="11" r="7" />
          <path d="m20 20-3.5-3.5" />
        </svg>
        <input
          className="search-input"
          placeholder={t('搜索标题、ID、工作目录或路径...')}
          aria-label={t('搜索标题、ID、工作目录或路径...')}
          value={search}
          onChange={event => setSearch(event.target.value)}
        />
      </div>
      <div className="nav-tabs session-status-tabs">
        {STATUS_FILTERS.map(item => {
          const count = item.key === 'all'
            ? conversations.length
            : item.key === 'deleted'
              ? deletedSessions.length
              : conversations.filter(conversation => conversation.status === item.key).length;
          return (
            <button
              key={item.key}
                type="button"
                className={`nav-item session-status-tab ${statusFilter === item.key ? 'active' : ''}`}
                aria-pressed={statusFilter === item.key}
                onClick={() => setStatusFilter(item.key)}
              >
                <span className="session-status-label">{t(item.label)}</span>
                <span className="session-status-count">{count}</span>
              </button>
          );
        })}
      </div>
      <div className="session-page-actions">
        <button type="button" className="btn btn-secondary" onClick={() => refreshSessions(rootPath)} disabled={loading || actionLoading}>
          {loading ? t('刷新中...') : t('刷新')}
        </button>
        {!isDeletedView && (
          <button type="button" className="btn btn-primary" onClick={handleImport} disabled={!rootPath || actionLoading}>
            {t('导入会话')}
          </button>
        )}
      </div>
    </div>
  );
}
