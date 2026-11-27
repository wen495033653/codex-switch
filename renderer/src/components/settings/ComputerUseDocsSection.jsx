import { useState } from 'react';
import ConfirmDialog from '../ConfirmDialog';
import { useI18n } from '../../i18n';

function resolveBadgeState(summary, loading, hasError, labels) {
  if (loading) {
    return { tone: 'muted', text: labels.loading };
  }
  if (hasError) {
    return { tone: 'error', text: labels.error };
  }
  if (!summary || summary.totalFiles === 0) {
    return { tone: 'muted', text: labels.missing };
  }
  if (summary.pendingCount > 0) {
    return { tone: 'warning', text: labels.pending };
  }
  return { tone: 'active', text: labels.done };
}

export default function ComputerUseDocsSection({
  clearing,
  error,
  loading,
  onClear,
  onRefresh,
  onRestore,
  restoring,
  status,
  switching
}) {
  const { t, translateRuntimeText } = useI18n();
  const [confirmAction, setConfirmAction] = useState(null);
  const guidance = status && status.guidance
    ? status.guidance
    : { totalFiles: 0, pendingCount: 0, appliedCount: 0, restorableCount: 0 };
  const confirmations = status && status.confirmations
    ? status.confirmations
    : { totalFiles: 0, pendingCount: 0, appliedCount: 0, restorableCount: 0 };
  const restorableCount = Number(status && status.restorableCount) || 0;
  const hasError = Boolean(error);
  const busy = loading || clearing || restoring;
  const hasAnyFiles = guidance.totalFiles > 0 || confirmations.totalFiles > 0;
  const hasPending = guidance.pendingCount > 0 || confirmations.pendingCount > 0;
  const allClean = !loading && !hasError && hasAnyFiles && !hasPending;

  const safetyBadge = resolveBadgeState(guidance, loading, hasError, {
    loading: t('检测中'),
    error: t('读取失败'),
    missing: t('未检测到'),
    pending: t('待清理'),
    done: t('已清理')
  });
  const confirmationsBadge = resolveBadgeState(confirmations, loading, hasError, {
    loading: t('检测中'),
    error: t('读取失败'),
    missing: t('未检测到'),
    pending: t('待应用'),
    done: t('已生效')
  });

  const clearButtonLabel = clearing
    ? t('清理中...')
    : allClean
      ? t('已清理')
      : t('一键清除');

  const handleConfirmClear = async () => {
    const res = await onClear();
    if (res && res.ok === true) {
      setConfirmAction(null);
    }
  };

  const handleConfirmRestore = async () => {
    const res = await onRestore();
    if (res && res.ok === true) {
      setConfirmAction(null);
    }
  };

  return (
    <section className="settings-section settings-app-card-section codex-computer-use-section">
      {confirmAction === 'clear' && (
        <ConfirmDialog
          title={t('确认清理？')}
          content={(
            <div className="codex-computer-use-confirm-list">
              <p>{t('1. 删除 guidance.md 中的 Non-negotiable Windows Automation Safety 段落。')}</p>
              <p>{t('2. confirmations.md 改为仅金钱/付款确认')}</p>
            </div>
          )}
          confirmText={t('确认清除')}
          loadingText={t('清理中...')}
          isLoading={clearing}
          width="460px"
          onConfirm={handleConfirmClear}
          onCancel={() => !clearing && setConfirmAction(null)}
        />
      )}

      {confirmAction === 'restore' && (
        <ConfirmDialog
          title={t('确认还原？')}
          message={t('将从备份还原 Computer Use 原文件。')}
          confirmText={t('确认还原')}
          loadingText={t('还原中...')}
          isLoading={restoring}
          onConfirm={handleConfirmRestore}
          onCancel={() => !restoring && setConfirmAction(null)}
        />
      )}

      <div className="codex-computer-use-head">
        <div className="settings-section-head">
          <div className="settings-section-title">{t('Computer Use 增强')}</div>
          <div className="settings-section-desc">{t('移除安全禁令，仅保留金钱/付款确认')}</div>
        </div>
        <div className="codex-computer-use-actions">
          <button
            type="button"
            className="codex-computer-use-refresh-button"
            disabled={busy || switching}
            onClick={onRefresh}
          >
            {loading ? t('检测中...') : t('重新检测')}
          </button>
          {restorableCount > 0 && (
            <button
              type="button"
              className="codex-computer-use-refresh-button"
              disabled={busy || switching}
              onClick={() => setConfirmAction('restore')}
            >
              {restoring ? t('还原中...') : t('还原')}
            </button>
          )}
          <button
            type="button"
            className="codex-computer-use-clear-button"
            disabled={busy || switching || !hasPending}
            onClick={() => setConfirmAction('clear')}
          >
            {clearButtonLabel}
          </button>
        </div>
      </div>

      <div className="codex-computer-use-pills">
        <span className={`codex-computer-use-pill ${safetyBadge.tone}`}>
          <span className="codex-computer-use-pill-dot" aria-hidden="true" />
          <span className="codex-computer-use-pill-name">{t('安全禁令')}</span>
          <span className="codex-computer-use-pill-sep" aria-hidden="true">·</span>
          <span className="codex-computer-use-pill-state">{safetyBadge.text}</span>
        </span>
        <span className={`codex-computer-use-pill ${confirmationsBadge.tone}`}>
          <span className="codex-computer-use-pill-dot" aria-hidden="true" />
          <span className="codex-computer-use-pill-name">{t('自动授权（付款除外）')}</span>
          <span className="codex-computer-use-pill-sep" aria-hidden="true">·</span>
          <span className="codex-computer-use-pill-state">{confirmationsBadge.text}</span>
        </span>
      </div>

      {hasError && (
        <div className="codex-computer-use-error" role="alert">
          {translateRuntimeText(error)}
        </div>
      )}
    </section>
  );
}
