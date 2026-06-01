import BrandMark from './BrandMark';

export default function AppNavbar({
  apiModeActive,
  currentModeDetail,
  currentModeLabel,
  devErrorCount = 0,
  devLogCount = 0,
  isDevBuild = false,
  onDevDiagnosticsToggle,
  onAccountsClick,
  onApiClick,
  onSettingsClick,
  subscriptionModeActive,
  viewMode
}) {
  return (
    <div className="navbar">
      <div className="brand">
        <BrandMark />
        <span>Codex Switch</span>
      </div>
      <div className="top-nav-tabs">
        <button
          type="button"
          className={`top-nav-item ${viewMode === 'accounts' ? 'active' : ''}`}
          onClick={onAccountsClick}
        >
          账号
        </button>
        <button
          type="button"
          className={`top-nav-item ${viewMode === 'api' ? 'active' : ''}`}
          onClick={onApiClick}
        >
          API
        </button>
        <button
          type="button"
          className={`top-nav-item ${viewMode === 'settings' ? 'active' : ''}`}
          onClick={onSettingsClick}
        >
          设置
        </button>
      </div>
      {isDevBuild && (
        <button
          type="button"
          className={`dev-build-chip ${devErrorCount > 0 ? 'has-errors' : ''}`}
          onClick={onDevDiagnosticsToggle}
          title={devErrorCount > 0 ? `${devErrorCount} 个错误，${devLogCount} 条日志` : `${devLogCount} 条日志`}
        >
          <span className="dev-build-chip-main">开发日志</span>
          {devErrorCount > 0 && <span className="dev-build-chip-dot" aria-hidden="true" />}
        </button>
      )}
      <div className={`current-mode-pill ${apiModeActive ? 'api' : subscriptionModeActive ? 'subscription' : 'unknown'}`}>
        <span className="current-mode-dot" aria-hidden="true" />
        <span className="current-mode-label">当前：{currentModeLabel}</span>
        <span className="current-mode-detail">{currentModeDetail}</span>
      </div>
    </div>
  );
}
