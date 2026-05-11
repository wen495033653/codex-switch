export default function ProxySettingsTab({
    codexSessionSyncEnabled,
    savingCodexProxyEnv,
    savingCodexSessionSync,
    savingProxySettings,
    setSettingsDraft,
    setCodexProxyEnvEnabled,
    setCodexSessionSyncEnabled,
    settingsDraft,
    switching,
    updateCodexProxySettings
}) {
    const proxyEnvEnabled = settingsDraft.codex_proxy_env_enabled === true;
    const saving = savingProxySettings || savingCodexProxyEnv;
    const sessionSyncHelp = 'Codex 订阅和 API 模式默认使用独立 workspace，会话列表不同步；开启后会同步两种模式的会话列表。';

    return (
        <>
            <section className="settings-section settings-app-card-section settings-proxy-section">
                <div className="settings-proxy-head">
                    <div className="settings-section-title">Codex app 代理</div>
                    <button
                        type="button"
                        className={`settings-proxy-switch-button ${proxyEnvEnabled ? 'active' : ''}`}
                        aria-pressed={proxyEnvEnabled}
                        disabled={saving}
                        onClick={() => setCodexProxyEnvEnabled(!proxyEnvEnabled)}
                    >
                        <span className="settings-proxy-switch-label">启用代理</span>
                        <span className="settings-switch" aria-hidden="true">
                            <span className="settings-switch-thumb" />
                        </span>
                    </button>
                </div>

                <label className="settings-field settings-proxy-field">
                    <span className="settings-inline-field-label">代理地址</span>
                    <input
                        className="settings-input settings-proxy-input"
                        value={settingsDraft.codex_proxy_url || ''}
                        placeholder="127.0.0.1:10808"
                        onChange={e => setSettingsDraft(prev => ({ ...prev, codex_proxy_url: e.target.value }))}
                        onBlur={e => updateCodexProxySettings({ codex_proxy_url: e.target.value })}
                        onKeyDown={e => {
                            if (e.key === 'Enter') e.currentTarget.blur();
                        }}
                    />
                </label>
            </section>

            <section className="settings-section settings-app-card-section settings-session-sync-section">
                <div className="settings-section-head">
                    <div className="settings-section-title">会话同步</div>
                    <div className="settings-section-desc">订阅/API 模式使用同一份会话列表</div>
                </div>
                <button
                    type="button"
                    className={`settings-toggle-row ${codexSessionSyncEnabled ? 'active' : ''}`}
                    aria-pressed={codexSessionSyncEnabled}
                    aria-label={codexSessionSyncEnabled ? '关闭会话同步' : '开启会话同步'}
                    disabled={savingCodexSessionSync || switching}
                    title={sessionSyncHelp}
                    onClick={() => setCodexSessionSyncEnabled(!codexSessionSyncEnabled)}
                >
                    <span className="settings-toggle-copy">
                        <span className="settings-toggle-title">启用</span>
                    </span>
                    <span className="settings-switch" aria-hidden="true">
                        <span className="settings-switch-thumb" />
                    </span>
                </button>
            </section>
        </>
    );
}
