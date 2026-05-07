export default function ProxySettingsTab({
    savingCodexProxyEnv,
    savingProxySettings,
    setSettingsDraft,
    setCodexProxyEnvEnabled,
    settingsDraft,
    updateCodexProxySettings
}) {
    const proxyEnvEnabled = settingsDraft.codex_proxy_env_enabled === true;
    const saving = savingProxySettings || savingCodexProxyEnv;

    return (
        <section className="settings-section">
            <div className="settings-section-head">
                <div className="settings-section-title">Codex app 代理</div>
                <div className="settings-section-desc">开启后，Codex 下次启动时会使用这里的代理设置。</div>
            </div>

            <div className="settings-field-list">
                <div className="settings-proxy-row">
                    <label className="settings-field settings-proxy-field">
                        <span className="settings-inline-field-label">代理地址</span>
                        <input
                            className="settings-input"
                            value={settingsDraft.codex_proxy_url || ''}
                            placeholder="127.0.0.1:10808"
                            onChange={e => setSettingsDraft(prev => ({ ...prev, codex_proxy_url: e.target.value }))}
                            onBlur={e => updateCodexProxySettings({ codex_proxy_url: e.target.value })}
                            onKeyDown={e => {
                                if (e.key === 'Enter') e.currentTarget.blur();
                            }}
                        />
                    </label>
                    <button
                        type="button"
                        className={`settings-toggle-row settings-proxy-env-toggle ${proxyEnvEnabled ? 'active' : ''}`}
                        aria-pressed={proxyEnvEnabled}
                        disabled={saving}
                        onClick={() => setCodexProxyEnvEnabled(!proxyEnvEnabled)}
                    >
                        <span className="settings-toggle-copy">
                            <span className="settings-toggle-title">启用代理</span>
                            <span className="settings-toggle-desc">
                                {proxyEnvEnabled ? '已为 Codex app 启用代理' : '关闭后 Codex app 不再使用这里的代理'}
                            </span>
                        </span>
                        <span className="settings-switch" aria-hidden="true">
                            <span className="settings-switch-thumb" />
                        </span>
                    </button>
                </div>
            </div>
        </section>
    );
}
