import AddAccountModal from './AddAccountModal';
import ConfirmDialog from './ConfirmDialog';
import RefreshTokenDialog from './RefreshTokenDialog';
import UpdateDialog from './UpdateDialog';

export default function AppDialogs({
  addAccount,
  codexShortcut,
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

      {codexShortcut.modal.visible && (
        <ConfirmDialog
          title="创建桌面图标"
          content={<CodexShortcutConfirmContent proxyUrl={codexShortcut.modal.proxyUrl} />}
          isLoading={codexShortcut.isLoading}
          confirmText="创建"
          loadingText="创建中..."
          width="460px"
          onConfirm={codexShortcut.onConfirm}
          onCancel={() => !codexShortcut.isLoading && codexShortcut.onCancel()}
        />
      )}

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
          title="检测到编辑器正在运行"
          message={`切换已完成。是否关闭并重新打开：${ideReopen.summaryText}？`}
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

function CodexShortcutConfirmContent({ proxyUrl }) {
  const hasProxy = Boolean(String(proxyUrl || '').trim());
  const rows = [
    ['图标名称', hasProxy ? 'Codex 代理启动.lnk' : 'Codex 启动.lnk'],
    ['启动方式', '通过桌面图标启动 Codex app'],
    ['代理注入', hasProxy ? '注入 HTTP/HTTPS/WS 代理环境变量' : '不注入代理环境变量'],
    ['代理地址', hasProxy ? proxyUrl : '未填写']
  ];

  return (
    <div className="shortcut-confirm-details">
      <p className="shortcut-confirm-summary">
        将在桌面创建启动图标；如果已有同名图标，会更新为当前配置。
      </p>
      <div className="shortcut-confirm-card">
        {rows.map(([label, value]) => (
          <div className="shortcut-confirm-row" key={label}>
            <span>{label}</span>
            <strong>{value}</strong>
          </div>
        ))}
      </div>
    </div>
  );
}
