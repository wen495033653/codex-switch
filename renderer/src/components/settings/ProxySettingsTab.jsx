export default function ProxySettingsTab({
    codexSessionSyncEnabled,
    savingCodexPlugins,
    savingCodexProxyEnv,
    savingCodexSessionSync,
    savingProxySettings,
    restartCodexAppWithPlugins,
    setSettingsDraft,
    setCodexProxyEnvEnabled,
    setCodexSessionSyncEnabled,
    settingsDraft,
    switching,
    updateCodexProxySettings,
    updateSettingsDraftAndSave
}) {
    const proxyEnvEnabled = settingsDraft.codex_proxy_env_enabled === true;
    const codexPluginsEnabled = settingsDraft.codex_plugins_enabled === true;
    const saving = savingProxySettings || savingCodexProxyEnv;
    const pluginSaving = savingCodexPlugins || switching;
    const sessionSyncHelp = '切换订阅/API 模式后，重新打开 Codex app 或 VS Code 前同步会话列表。';

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

            <section className="settings-section settings-app-card-section settings-plugin-section">
                <div className="settings-section-head">
                    <div className="settings-section-title">解锁 API 模式 Plugin</div>
                    <div className="settings-section-desc">开启后由 Codex Switch 解锁 Codex app Plugin</div>
                </div>
                <button
                    type="button"
                    className={`settings-toggle-row ${codexPluginsEnabled ? 'active' : ''}`}
                    aria-pressed={codexPluginsEnabled}
                    aria-label={codexPluginsEnabled ? '关闭 Plugin 解锁' : '开启 Plugin 解锁'}
                    disabled={pluginSaving}
                    onClick={() => updateSettingsDraftAndSave({ codex_plugins_enabled: !codexPluginsEnabled })}
                >
                    <span className="settings-toggle-copy">
                        <span className="settings-toggle-title">解锁 Plugin</span>
                    </span>
                    <span className="settings-switch" aria-hidden="true">
                        <span className="settings-switch-thumb" />
                    </span>
                </button>
                {codexPluginsEnabled ? (
                    <div className="settings-suboption-panel settings-plugin-action-panel">
                        <button
                            type="button"
                            className="btn btn-secondary"
                            disabled={pluginSaving}
                            onClick={restartCodexAppWithPlugins}
                        >
                            {savingCodexPlugins ? '重启中...' : '重启 Codex app'}
                        </button>
                    </div>
                ) : null}
            </section>

            <section className="settings-section settings-app-card-section settings-session-sync-section">
                <div className="settings-section-head">
                    <div className="settings-section-title">会话同步</div>
                    <div className="settings-section-desc">重新打开 IDE 前同步会话列表</div>
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
