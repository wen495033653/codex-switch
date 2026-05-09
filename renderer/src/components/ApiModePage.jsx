import { useState } from 'react';

export default function ApiModePage({
  apiConfigComplete,
  apiDraft,
  codexSessionSyncEnabled,
  apiModeActive,
  onOpenCodexConfigToml,
  onOpenGptPool,
  onToggleCodexSessionSync,
  onSwitchToApiMode,
  onUpdateApiDraft,
  savingApiMode,
  savingCodexSessionSync,
  switching
}) {
  const [showApiKey, setShowApiKey] = useState(false);
  const sessionSyncHelp = 'Codex 订阅和 API 模式默认使用独立 workspace，会话列表不同步；开启后会同步两种模式的会话列表。';

  return (
    <div className="api-mode-page">
      <div className="api-console-grid">
        <button
          type="button"
          className="api-promo-banner"
          onClick={onOpenGptPool}
        >
          <span className="api-promo-ad-label">广告</span>
          <span className="api-promo-brand">GPT Pool</span>
          <span className="api-promo-title">低价、稳定的 API 站点</span>
          <span className="api-promo-action">获取 API Key</span>
        </button>
        <div className="api-config-stack">
          <div className="api-config-cluster">
            <div className="api-page-actions">
              <div className="api-session-sync-control">
                <label
                  className={`api-session-sync-toggle ${codexSessionSyncEnabled ? 'active' : ''} ${savingCodexSessionSync || switching ? 'disabled' : ''}`}
                  title={sessionSyncHelp}
                >
                  <span className="api-session-sync-copy">
                    <span className="api-session-sync-title-row">
                      <span className="api-session-sync-label">会话同步</span>
                      <span
                        className="api-session-sync-help"
                        title={sessionSyncHelp}
                        aria-label={sessionSyncHelp}
                        role="img"
                      >
                        ?
                      </span>
                    </span>
                    <span className="api-session-sync-desc">订阅/API 沿用同一份会话列表</span>
                  </span>
                  <input
                    type="checkbox"
                    role="switch"
                    checked={codexSessionSyncEnabled}
                    disabled={savingCodexSessionSync || switching}
                    aria-label="会话同步"
                    onChange={event => onToggleCodexSessionSync(event.target.checked)}
                  />
                  <span className="settings-switch" aria-hidden="true">
                    <span className="settings-switch-thumb" />
                  </span>
                </label>
              </div>
              <button
                type="button"
                className="btn btn-secondary api-config-open-button"
                onClick={onOpenCodexConfigToml}
              >
                打开 config.toml
              </button>
            </div>
            <div className={`mode-card api-config-card ${apiModeActive ? 'active' : ''}`}>
              <div className="api-mode-current">
                <div className="api-mode-current-text">
                  <div className="api-mode-current-title">
                    {apiModeActive ? '当前正在使用 API 模式' : '当前未启用 API 模式'}
                  </div>
                  {!apiModeActive && (
                    <div className="api-mode-current-desc">
                      填写 Base URL 和 API Key 后，点击“应用 API 模式”会保存并写入 Codex 配置。
                    </div>
                  )}
                </div>
                <span className={`api-mode-current-badge ${apiModeActive ? 'active' : ''}`}>
                  {apiModeActive ? '已启用' : '未启用'}
                </span>
              </div>
              <div className="api-mode-form">
                <label className="api-mode-field">
                  <span className="api-mode-label">Base URL</span>
                  <input
                    className="api-mode-input"
                    value={(apiDraft && apiDraft.base_url) || ''}
                    placeholder="https://api.example.com/v1"
                    onChange={event => onUpdateApiDraft({ base_url: event.target.value })}
                  />
                </label>
                <label className="api-mode-field">
                  <span className="api-mode-label">API Key</span>
                  <span className="api-key-input-wrap">
                    <input
                      className="api-mode-input api-key-input"
                      type={showApiKey ? 'text' : 'password'}
                      value={(apiDraft && apiDraft.api_key) || ''}
                      placeholder="sk-..."
                      onChange={event => onUpdateApiDraft({ api_key: event.target.value })}
                    />
                    <button
                      type="button"
                      className={`api-key-eye-button ${showApiKey ? 'active' : ''}`}
                      aria-label={showApiKey ? '隐藏 API Key' : '显示 API Key'}
                      title={showApiKey ? '隐藏 API Key' : '显示 API Key'}
                      onClick={() => setShowApiKey(value => !value)}
                    >
                      <svg viewBox="0 0 24 24" aria-hidden="true">
                        <path d="M12 5.5c4.22 0 7.56 2.36 9.5 6.5-1.94 4.14-5.28 6.5-9.5 6.5S4.44 16.14 2.5 12C4.44 7.86 7.78 5.5 12 5.5Zm0 2C8.78 7.5 6.17 9.08 4.73 12 6.17 14.92 8.78 16.5 12 16.5s5.83-1.58 7.27-4.5C17.83 9.08 15.22 7.5 12 7.5Zm0 2.25A2.25 2.25 0 1 1 12 14.25 2.25 2.25 0 0 1 12 9.75Z" />
                      </svg>
                    </button>
                  </span>
                </label>
                <div className="api-mode-actions">
                  <button
                    type="button"
                    className="btn btn-primary"
                    onClick={onSwitchToApiMode}
                    disabled={!apiConfigComplete || savingApiMode || switching}
                  >
                    {switching || savingApiMode ? '应用中...' : '应用 API 模式'}
                  </button>
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
