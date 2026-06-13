import BrandMark from './BrandMark';

export default function AppNavbar({
  apiModeActive,
  currentModeDetail,
  currentModeLabel,
  devErrorCount = 0,
  devLogCount = 0,
  devWarningCount = 0,
  isDevBuild = false,
  onDevDiagnosticsToggle,
  onAccountsClick,
  onApiClick,
  onSessionsClick,
  onSettingsClick,
  subscriptionModeActive,
  viewMode
}) {
  const devChipToneClass = devErrorCount > 0
    ? 'has-errors'
    : devWarningCount > 0
      ? 'has-warnings'
      : 'has-info';
  const devChipTitle = `${devLogCount} 条日志，${devErrorCount} 个错误，${devWarningCount} 个警告`;

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
          className={`top-nav-item ${viewMode === 'sessions' ? 'active' : ''}`}
          onClick={onSessionsClick}
        >
          会话
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
          className={`dev-build-chip ${devChipToneClass}`}
          onClick={onDevDiagnosticsToggle}
          title={devChipTitle}
        >
          <span className="dev-build-chip-main">开发日志</span>
          {devLogCount > 0 && <span className="dev-build-chip-dot" aria-hidden="true" />}
        </button>
      )}
      <div
        className={`current-mode-pill ${apiModeActive ? 'api' : subscriptionModeActive ? 'subscription' : 'unknown'}`}
        title={`${currentModeLabel}${currentModeDetail ? ` ${currentModeDetail}` : ''}`}
      >
        <span className="current-mode-dot" aria-hidden="true" />
        <span className="current-mode-label">当前：{currentModeLabel}</span>
        <span className="current-mode-detail">{currentModeDetail}</span>
      </div>
    </div>
  );
}
