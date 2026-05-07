export default function ProxySettingsTab({
    createCodexProxyDesktopShortcut,
    creatingCodexProxyDesktopShortcut,
    launchCodexWithProxy,
    launchingCodexWithProxy,
    savingProxySettings,
    setSettingsDraft,
    settingsDraft,
    updateCodexProxySettings
}) {
    return (
        <section className="settings-section">
            <div className="settings-section-head">
                <div className="settings-section-title">Codex app 启动代理</div>
                <div className="settings-section-desc">只在通过 Codex Switch 或桌面图标启动 Codex 时注入 HTTP/HTTPS/WS 代理环境变量。</div>
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
                    <div className="settings-proxy-launch">
                        <button
                            type="button"
                            className="btn btn-primary"
                            onClick={launchCodexWithProxy}
                            disabled={savingProxySettings || launchingCodexWithProxy || creatingCodexProxyDesktopShortcut}
                        >
                            {launchingCodexWithProxy ? '启动中...' : '启动 Codex'}
                        </button>
                        <button
                            type="button"
                            className="btn btn-secondary"
                            onClick={createCodexProxyDesktopShortcut}
                            disabled={savingProxySettings || launchingCodexWithProxy || creatingCodexProxyDesktopShortcut}
                        >
                            {creatingCodexProxyDesktopShortcut ? '创建中...' : '创建桌面图标'}
                        </button>
                    </div>
                </div>
            </div>
        </section>
    );
}
