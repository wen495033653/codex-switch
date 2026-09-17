import { useI18n } from '../../i18n';

export default function SessionConflictDialog({
  actionLoading,
  conflictConfirm,
  resolveConflictDialog,
}) {
  const { t } = useI18n();
  return (
    <div className="modal-overlay">
      <div className="modal-content modal-content-lg session-conflict-dialog" role="dialog" aria-modal="true" aria-labelledby="session-conflict-title" aria-describedby="session-conflict-message">
        <h3 id="session-conflict-title">{conflictConfirm.title}</h3>
        <p id="session-conflict-message">{conflictConfirm.message}</p>
        <div className="session-conflict-list">
          {conflictConfirm.conflicts.slice(0, 8).map((item, index) => (
            <div key={`${item.target || item.relative_path || item.delete_id || index}`} className="session-conflict-item">
              <strong title={item.title || item.target || ''}>{item.title || item.target || t('冲突会话')}</strong>
              <span title={item.target || item.relative_path || item.delete_id || ''}>
                {item.target || item.relative_path || item.delete_id}
              </span>
            </div>
          ))}
          {conflictConfirm.conflicts.length > 8 && (
            <div className="session-conflict-item">
              <strong>{t('还有 {count} 个冲突', { count: conflictConfirm.conflicts.length - 8 })}</strong>
            </div>
          )}
        </div>
        <div className="session-conflict-actions">
          <button type="button" className="btn btn-secondary" onClick={() => resolveConflictDialog('skip')} disabled={actionLoading}>
            {t('跳过')}
          </button>
          <button type="button" className="btn btn-secondary" onClick={() => resolveConflictDialog('modify_id')} disabled={actionLoading}>
            {t('修改 ID')}
          </button>
          <button type="button" className="btn btn-danger" onClick={() => resolveConflictDialog('overwrite')} disabled={actionLoading}>
            {t('覆盖')}
          </button>
        </div>
      </div>
    </div>
  );
}
