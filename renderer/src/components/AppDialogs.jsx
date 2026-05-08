import AddAccountModal from './AddAccountModal';
import ConfirmDialog from './ConfirmDialog';
import RefreshTokenDialog from './RefreshTokenDialog';
import UpdateDialog from './UpdateDialog';

export default function AppDialogs({
  addAccount,
  deleteAccount,
  ideReopen,
  message,
  refreshAll,
  refreshToken,
  update
}) {
  return (
    <>
      {message && <div className="toast">{message}</div>}

      {addAccount.visible && (
        <AddAccountModal
          oauth={addAccount.oauth}
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
          onRefreshTokenInputChange={addAccount.onRefreshTokenInputChange}
          onStartOauth={addAccount.onStartOauth}
          onToggleRefreshTokenPanel={addAccount.onToggleRefreshTokenPanel}
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
          title="是否重启 Codex app"
          message={`切换已完成。是否重启 Codex app：${ideReopen.summaryText}？`}
          isLoading={ideReopen.modal.loading}
          confirmText="重新打开"
          loadingText="重启中..."
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
