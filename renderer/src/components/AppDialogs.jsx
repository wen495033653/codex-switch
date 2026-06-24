import AddAccountModal from './AddAccountModal';
import ApiProfileModal from './ApiProfileModal';
import ConfirmDialog from './ConfirmDialog';
import RefreshTokenDialog from './RefreshTokenDialog';
import UpdateDialog from './UpdateDialog';

function buildIdeReopenMessage({ sessionSync, summaryText }) {
  const targetText = summaryText || '当前已打开的软件';
  return [
    sessionSync
      ? '切换已完成。是否关闭后重新打开？'
      : '切换已完成。是否重新打开？',
    targetText
  ].join('\n');
}

export default function AppDialogs({
  addAccount,
  apiProfile = {
    modal: { visible: false },
    deleteModal: { visible: false }
  },
  deleteAccount,
  ideReopen,
  message,
  pluginRestartNotice = { visible: false },
  refreshAll,
  refreshToken,
  remoteControlNotice = { visible: false },
  update
}) {
  return (
    <>
      {message && <div className="toast">{message}</div>}

      {pluginRestartNotice.visible && (
        <ConfirmDialog
          title="重启后生效"
          message={pluginRestartNotice.message || 'Codex app 设置已保存，重启后生效。'}
          isLoading={pluginRestartNotice.loading}
          confirmText="重启"
          loadingText="重启中..."
          cancelText="稍后"
          onConfirm={pluginRestartNotice.onRestart}
          onCancel={pluginRestartNotice.onClose}
        />
      )}

      {remoteControlNotice.visible && (
        <ConfirmDialog
          title="远程控制"
          message={remoteControlNotice.message || '当前控制账号过期，远程控制关闭'}
          confirmText="知道了"
          showCancel={false}
          onConfirm={remoteControlNotice.onClose}
          onCancel={remoteControlNotice.onClose}
        />
      )}

      {addAccount.visible && (
        <AddAccountModal
          oauth={addAccount.oauth}
          oauthCallbackSubmitting={addAccount.oauthCallbackSubmitting}
          oauthCallbackUrl={addAccount.oauthCallbackUrl}
          oauthTimeoutHint={addAccount.oauthTimeoutHint}
          refreshTokenInput={addAccount.refreshTokenInput}
          refreshTokenLoading={addAccount.refreshTokenLoading}
          showRefreshTokenPanel={addAccount.showRefreshTokenPanel}
          onCancelOauth={addAccount.onCancelOauth}
          onCaptureCurrent={addAccount.onCaptureCurrent}
          onClose={addAccount.onClose}
          onCopyOauthUrl={addAccount.onCopyOauthUrl}
          onImportAccountsFromBackup={addAccount.onImportAccountsFromBackup}
          onImportByRefreshToken={addAccount.onImportByRefreshToken}
          onOauthCallbackUrlChange={addAccount.onOauthCallbackUrlChange}
          onRefreshTokenInputChange={addAccount.onRefreshTokenInputChange}
          onStartOauth={addAccount.onStartOauth}
          onSubmitOauthCallbackUrl={addAccount.onSubmitOauthCallbackUrl}
          onToggleRefreshTokenPanel={addAccount.onToggleRefreshTokenPanel}
        />
      )}

      {apiProfile.modal.visible && (
        <ApiProfileModal
          modal={apiProfile.modal}
          saving={apiProfile.saving}
          onClose={apiProfile.onClose}
          onSave={apiProfile.onSave}
          onUpdate={apiProfile.onUpdate}
        />
      )}

      {apiProfile.deleteModal.visible && (
        <ConfirmDialog
          title="删除 API 配置"
          message={`确定删除 API 配置：${apiProfile.deleteModal.profileName || apiProfile.deleteModal.profileId}？\n删除后不可恢复。`}
          isLoading={apiProfile.deleteModal.loading}
          confirmText="删除"
          loadingText="删除中..."
          confirmVariant="danger"
          onConfirm={apiProfile.onConfirmDelete}
          onCancel={apiProfile.onCancelDelete}
        />
      )}

      {refreshToken.modal.visible && (
        <RefreshTokenDialog
          accountName={refreshToken.accountName}
          modal={refreshToken.modal}
          onClose={refreshToken.onClose}
          onCopy={refreshToken.onCopy}
          onRefresh={refreshToken.onRefresh}
        />
      )}

      {deleteAccount.modal.visible && (
        <ConfirmDialog
          title="删除账号"
          message={`确定删除账号：${deleteAccount.displayName || deleteAccount.modal.accountId}？\n删除后不可恢复。`}
          isLoading={deleteAccount.modal.loading}
          confirmText="删除"
          loadingText="删除中..."
          confirmVariant="danger"
          onConfirm={deleteAccount.onConfirm}
          onCancel={deleteAccount.onCancel}
        />
      )}

      {refreshAll.visible && (
        <ConfirmDialog
          title="刷新配额"
          message="开始后台刷新所有账号的配额。刷新期间按钮会持续旋转。"
          isLoading={refreshAll.isLoading}
          confirmText="开始刷新"
          loadingText="启动中..."
          onConfirm={refreshAll.onConfirm}
          onCancel={refreshAll.onCancel}
        />
      )}

      {ideReopen.modal.visible && (
        <ConfirmDialog
          message={buildIdeReopenMessage({
            sessionSync: ideReopen.modal.sessionSync,
            summaryText: ideReopen.summaryText
          })}
          isLoading={ideReopen.modal.loading}
          confirmText="重新打开"
          loadingText={ideReopen.modal.sessionSync ? '同步并重新打开中...' : '重新打开中...'}
          cancelText="稍后"
          onConfirm={ideReopen.onConfirm}
          onCancel={() => !ideReopen.modal.loading && ideReopen.onCancel()}
        />
      )}

      {update.modal.visible && (
        <UpdateDialog
          updateModal={update.modal}
          onConfirm={update.onConfirm}
          onCancel={() => !update.modal.loading && update.onCancel()}
        />
      )}
    </>
  );
}
