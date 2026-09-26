import { useI18n } from '../../i18n';
import {
  PAGE_SIZE_OPTIONS,
  deletedActiveKey,
  formatCompactTime,
  formatSize,
  formatTime,
  statusLabel,
} from '../../utils/sessionManager';

export default function SessionListPanel({
  actionLoading,
  activePath,
  allPageSelected,
  clearSelection,
  conversations,
  currentPage,
  deletedSessions,
  handleDeleteSessions,
  handleExport,
  handlePurgeDeleted,
  handleRestoreDeleted,
  handleRowKeyDown,
  handleSetStatus,
  isDeletedView,
  loadPreview,
  openContextMenu,
  pageItems,
  pageSize,
  selected,
  selectedActivePaths,
  selectedArchivedPaths,
  selectedCount,
  selectedDeleted,
  selectedSize,
  setPage,
  setPageSize,
  toggleSelectFiltered,
  toggleSelection,
  totalPages,
  visibleItems,
}) {
  const { language, t } = useI18n();
  return (
    <div className="session-list-panel">
      <div className="session-list-header">
        <label className="session-checkbox">
          <input
            type="checkbox"
            aria-label={t('选择本页会话')}
            checked={allPageSelected}
            onChange={toggleSelectFiltered}
          />
        </label>
        <span>{t('标题')}</span>
        <span>{t('状态')}</span>
        <span>{isDeletedView ? t('删除时间') : t('更新时间')}</span>
        <span>{t('大小')}</span>
      </div>
      <div className="session-list-body">
        <div className={`session-list ${selectedCount > 0 ? 'has-batch-actions' : ''}`}>
          {pageItems.map(item => {
            const rowKey = isDeletedView ? item.delete_id : item.relative_path;
            const activeKey = isDeletedView ? deletedActiveKey(item) : item.relative_path;
            const isSelected = isDeletedView
              ? selectedDeleted.has(item.delete_id)
              : selected.has(item.relative_path);
            return (
              <div
                key={rowKey}
                role="button"
                tabIndex={0}
                aria-label={item.title}
                className={`session-row ${activePath === activeKey ? 'active' : ''}`}
                onClick={() => loadPreview(item)}
                onKeyDown={event => handleRowKeyDown(event, item)}
                onContextMenu={event => openContextMenu(event, item)}
              >
                <span className="session-checkbox" onClick={event => event.stopPropagation()}>
                  <input
                    type="checkbox"
                    aria-label={t('选择会话：{title}', { title: item.title || t('未命名') })}
                    checked={isSelected}
                    onChange={event => toggleSelection(item, event.target.checked)}
                  />
                </span>
                <span className="session-title-cell">
                  <strong title={item.title}>{item.title}</strong>
                </span>
                <span className={`session-status-pill ${item.status}`}>{statusLabel(item.status, t)}</span>
                <span
                  className="session-muted"
                  title={formatTime(isDeletedView ? item.deleted_at : item.updated_at, language, t)}
                >
                  {formatCompactTime(isDeletedView ? item.deleted_at : item.updated_at, language, t)}
                </span>
                <span className="session-muted">{formatSize(item.size_bytes)}</span>
              </div>
            );
          })}
          {visibleItems.length === 0 && (
            <div className="empty-state session-empty">
              {isDeletedView ? t('暂无已删除会话') : t('暂无会话数据')}
            </div>
          )}
        </div>
        {selectedCount > 0 && (
          <div className="session-contextual-toolbar" role="toolbar" aria-label={t('会话批量操作')}>
            <span className="session-batch-count">{t('已选 {count}', { count: selectedCount })}</span>
            {isDeletedView ? (
              <>
                <button type="button" className="btn btn-secondary" onClick={() => handleRestoreDeleted()} disabled={actionLoading}>
                  {t('恢复')}
                </button>
                <button type="button" className="btn btn-danger" onClick={() => handlePurgeDeleted()} disabled={actionLoading}>
                  {t('彻底删除')}
                </button>
              </>
            ) : (
              <>
                <button type="button" className="btn btn-secondary" onClick={() => handleExport()} disabled={actionLoading}>
                  {t('导出')}
                </button>
                {selectedActivePaths.length > 0 && (
                  <button type="button" className="btn btn-secondary" onClick={() => handleSetStatus(selectedActivePaths, 'archived')} disabled={actionLoading}>
                    {t('归档')}
                  </button>
                )}
                {selectedArchivedPaths.length > 0 && (
                  <button type="button" className="btn btn-secondary" onClick={() => handleSetStatus(selectedArchivedPaths, 'active')} disabled={actionLoading}>
                    {t('取消归档')}
                  </button>
                )}
                <button type="button" className="btn btn-danger" onClick={() => handleDeleteSessions()} disabled={actionLoading}>
                  {t('删除')}
                </button>
              </>
            )}
            <button type="button" className="btn btn-secondary" onClick={clearSelection} disabled={actionLoading}>
              {t('取消选择')}
            </button>
          </div>
        )}
      </div>
      <div className="session-footer">
        <span>{t('总计 {count} 个', { count: isDeletedView ? deletedSessions.length : conversations.length })}</span>
        <span>{t('筛选 {count} 个', { count: visibleItems.length })}</span>
        <span>{t('本页 {count} 个', { count: pageItems.length })}</span>
        <span>{t('已选 {count} 个', { count: selectedCount })}</span>
        <span>{formatSize(selectedSize)}</span>
        <div className="session-pagination">
          <button type="button" className="btn btn-secondary" onClick={() => setPage(1)} disabled={currentPage <= 1}>
            {t('首页')}
          </button>
          <button type="button" className="btn btn-secondary" onClick={() => setPage(value => Math.max(1, value - 1))} disabled={currentPage <= 1}>
            {t('上页')}
          </button>
          <span>{currentPage}/{totalPages}</span>
          <button type="button" className="btn btn-secondary" onClick={() => setPage(value => Math.min(totalPages, value + 1))} disabled={currentPage >= totalPages}>
            {t('下页')}
          </button>
          <select value={pageSize} onChange={event => setPageSize(Number(event.target.value))}>
            {PAGE_SIZE_OPTIONS.map(value => (
              <option key={value} value={value}>{t('每页 {count}', { count: value })}</option>
            ))}
          </select>
        </div>
      </div>
    </div>
  );
}
