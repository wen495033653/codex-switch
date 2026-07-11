import BrandMark from './BrandMark';
import { useI18n } from '../i18n';

function NavIcon({ name }) {
  const paths = {
    accounts: <><path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2" /><circle cx="9" cy="7" r="4" /><path d="M22 21v-2a4 4 0 0 0-3-3.87M16 3.13a4 4 0 0 1 0 7.75" /></>,
    api: <><path d="M8 9h8M8 15h5" /><rect x="3" y="3" width="18" height="18" rx="4" /><path d="m16 14 2 2 3-3" /></>,
    codex: <><path d="m8 9-3 3 3 3M16 9l3 3-3 3M14 5l-4 14" /></>,
    sessions: <><path d="M21 15a4 4 0 0 1-4 4H8l-5 3V7a4 4 0 0 1 4-4h10a4 4 0 0 1 4 4z" /><path d="M8 8h8M8 12h5" /></>,
    settings: <><circle cx="12" cy="12" r="3" /><path d="M19.4 15a1.7 1.7 0 0 0 .34 1.88l.06.06-2.83 2.83-.06-.06a1.7 1.7 0 0 0-1.88-.34 1.7 1.7 0 0 0-1.03 1.56V21h-4v-.09A1.7 1.7 0 0 0 9 19.36a1.7 1.7 0 0 0-1.88.34l-.06.06-2.83-2.83.06-.06A1.7 1.7 0 0 0 4.64 15a1.7 1.7 0 0 0-1.55-1.03H3v-4h.09A1.7 1.7 0 0 0 4.64 9a1.7 1.7 0 0 0-.34-1.88l-.06-.06 2.83-2.83.06.06A1.7 1.7 0 0 0 9 4.64a1.7 1.7 0 0 0 1.03-1.55V3h4v.09A1.7 1.7 0 0 0 15 4.64a1.7 1.7 0 0 0 1.88-.34l.06-.06 2.83 2.83-.06.06A1.7 1.7 0 0 0 19.36 9a1.7 1.7 0 0 0 1.55 1.03H21v4h-.09A1.7 1.7 0 0 0 19.4 15Z" /></>
  };
  return (
    <svg className="top-nav-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      {paths[name]}
    </svg>
  );
}

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
  onCodexClick,
  onSessionsClick,
  onSettingsClick,
  subscriptionModeActive,
  viewMode
}) {
  const { t, translateRuntimeText } = useI18n();
  const devChipToneClass = devErrorCount > 0
    ? 'has-errors'
    : devWarningCount > 0
      ? 'has-warnings'
      : 'has-info';
  const devChipTitle = t('{logs} 条日志，{errors} 个错误，{warnings} 个警告', {
    logs: devLogCount,
    errors: devErrorCount,
    warnings: devWarningCount
  });
  const navItems = [
    { key: 'accounts', label: t('账号'), onClick: onAccountsClick },
    { key: 'api', label: 'API', onClick: onApiClick },
    { key: 'codex', label: 'Codex', onClick: onCodexClick },
    { key: 'sessions', label: t('会话'), onClick: onSessionsClick },
    { key: 'settings', label: t('设置'), onClick: onSettingsClick }
  ];
  const localizedModeLabel = translateRuntimeText(currentModeLabel);
  const localizedModeDetail = translateRuntimeText(currentModeDetail);

  return (
    <div className="navbar">
      <div className="brand">
        <BrandMark />
        <span className="brand-copy">
          <span className="brand-title">Codex Switch</span>
        </span>
      </div>
      <div className="top-nav-tabs">
        {navItems.map(item => (
          <button
            key={item.key}
            type="button"
            className={`top-nav-item ${viewMode === item.key ? 'active' : ''}`}
            onClick={item.onClick}
            aria-current={viewMode === item.key ? 'page' : undefined}
          >
            <NavIcon name={item.key} />
            <span>{item.label}</span>
          </button>
        ))}
      </div>
      <div className="navbar-status">
        {isDevBuild && (
          <button
            type="button"
            className={`dev-build-chip ${devChipToneClass}`}
            onClick={onDevDiagnosticsToggle}
            title={devChipTitle}
          >
            <span className="dev-build-chip-main">{t('开发日志')}</span>
            {devLogCount > 0 && <span className="dev-build-chip-dot" aria-hidden="true" />}
          </button>
        )}
        <div
          className={`current-mode-pill ${apiModeActive ? 'api' : subscriptionModeActive ? 'subscription' : 'unknown'}`}
          title={`${localizedModeLabel}${localizedModeDetail ? ` ${localizedModeDetail}` : ''}`}
        >
          <span className="current-mode-dot" aria-hidden="true" />
          <span className="current-mode-label">{t('当前：{mode}', { mode: localizedModeLabel })}</span>
          <span className="current-mode-detail">{localizedModeDetail}</span>
        </div>
      </div>
    </div>
  );
}
