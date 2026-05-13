import BrandMark from './BrandMark';

export default function AppNavbar({
  apiModeActive,
  currentModeDetail,
  currentModeLabel,
  onAccountsClick,
  onApiClick,
  onSessionsClick,
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
      <div className={`current-mode-pill ${apiModeActive ? 'api' : subscriptionModeActive ? 'subscription' : 'unknown'}`}>
        <span className="current-mode-dot" aria-hidden="true" />
        <span className="current-mode-label">当前：{currentModeLabel}</span>
        <span className="current-mode-detail">{currentModeDetail}</span>
      </div>
    </div>
  );
}
