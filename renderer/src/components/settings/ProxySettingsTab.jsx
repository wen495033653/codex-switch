import { useEffect, useState } from 'react';

function normalizePids(value) {
    if (!Array.isArray(value)) return [];
    return value
        .map(pid => Number(pid))
        .filter(pid => Number.isInteger(pid) && pid > 0);
}

export default function ProxySettingsTab({
    codexSessionSyncEnabled,
    savingCodexProxyEnv,
    savingCodexSessionSync,
    savingProxySettings,
    restartingCodexApp,
    restartCurrentCodexAppNormal,
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
    const sessionSyncHelp = '切换订阅/API 模式后，重新打开 Codex app 或 VS Code 前同步会话列表。';
    const [codexAppProcessStatus, setCodexAppProcessStatus] = useState({
        loading: true,
        error: '',
        pids: [],
        processCount: 0
    });

    useEffect(() => {
        let disposed = false;

        async function refreshCodexAppProcesses() {
            if (!window.api || !window.api.getCurrentCodexAppProcesses) {
                if (!disposed) {
                    setCodexAppProcessStatus({ loading: false, error: '', pids: [], processCount: 0 });
                }
                return;
            }

            try {
                const result = await window.api.getCurrentCodexAppProcesses();
                if (!disposed) {
                    setCodexAppProcessStatus({
                        loading: false,
                        error: result && result.error ? String(result.error) : '',
                        pids: normalizePids(result && result.pids),
                        processCount: Number(result && result.processCount) || 0
                    });
                }
            } catch (err) {
                if (!disposed) {
                    setCodexAppProcessStatus({
                        loading: false,
                        error: err && err.message ? err.message : '读取失败',
                        pids: [],
                        processCount: 0
                    });
                }
            }
        }

        refreshCodexAppProcesses();
        const timer = window.setInterval(refreshCodexAppProcesses, 3000);
        return () => {
            disposed = true;
            window.clearInterval(timer);
        };
    }, []);

    const codexAppPidText = codexAppProcessStatus.loading
        ? '检测中'
        : codexAppProcessStatus.error || (codexAppProcessStatus.pids.length ? codexAppProcessStatus.pids.join(', ') : '未检测到');
    const codexAppPidTitle = codexAppProcessStatus.processCount > codexAppProcessStatus.pids.length
        ? `共检测到 ${codexAppProcessStatus.processCount} 个 Codex app 进程，这里显示主进程 PID`
        : '';
    const codexAppPidState = codexAppProcessStatus.error
        ? 'error'
        : codexAppProcessStatus.pids.length
            ? 'active'
            : 'empty';
    const restartCodexAppDisabled = restartingCodexApp
        || codexAppProcessStatus.loading
        || Boolean(codexAppProcessStatus.error)
        || codexAppProcessStatus.pids.length === 0;

    return (
        <>
            <section className="settings-codex-app-pid-card" aria-label="当前 Codex app PID">
                <span className="settings-codex-app-pid-label">当前 Codex app PID</span>
                <span className="settings-codex-app-pid-actions">
                    <span className={`settings-codex-app-pid-value ${codexAppPidState}`} title={codexAppPidTitle}>{codexAppPidText}</span>
                    <button
                        type="button"
                        className="settings-codex-app-restart-button"
                        disabled={restartCodexAppDisabled}
                        onClick={restartCurrentCodexAppNormal}
                    >
                        {restartingCodexApp ? '重启中...' : '重启 Codex app'}
                    </button>
                </span>
            </section>

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
                    <div className="settings-section-title">解锁 Plugin</div>
                    <div className="settings-section-desc">API 模式下可用 Plugin 功能</div>
                </div>
                <button
                    type="button"
                    className={`settings-toggle-row ${codexPluginsEnabled ? 'active' : ''}`}
                    aria-pressed={codexPluginsEnabled}
                    aria-label={codexPluginsEnabled ? '关闭 Plugin 解锁' : '开启 Plugin 解锁'}
                    disabled={switching}
                    onClick={() => updateSettingsDraftAndSave({ codex_plugins_enabled: !codexPluginsEnabled })}
                >
                    <span className="settings-toggle-copy">
                        <span className="settings-toggle-title">启动</span>
                    </span>
                    <span className="settings-switch" aria-hidden="true">
                        <span className="settings-switch-thumb" />
                    </span>
                </button>
            </section>

            <section className="settings-section settings-app-card-section settings-session-sync-section">
                <div className="settings-section-head">
                    <div className="settings-section-title">会话同步</div>
                    <div className="settings-section-desc">订阅/API 模式下会话列表保持同步</div>
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
