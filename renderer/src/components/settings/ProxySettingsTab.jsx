import { useEffect, useState } from 'react';
import { getAccountId, getChatgptAccountId, isApiModeAccount } from '../../utils/auth/account';
import { getAccountName, maskAccountDisplayName, parseAuthInfo } from '../../utils/auth/info';

function normalizePids(value) {
    if (!Array.isArray(value)) return [];
    return value
        .map(pid => Number(pid))
        .filter(pid => Number.isInteger(pid) && pid > 0);
}

function formatRemoteControlAccountLabel(account, maskAccountName) {
    const accountId = getAccountId(account);
    const name = getAccountName(account);
    const displayName = maskAccountName ? maskAccountDisplayName(name) : name;
    const info = parseAuthInfo(account);
    const plan = info.planType ? info.planType.toUpperCase() : '';
    const chatgptAccountId = getChatgptAccountId(account);
    const accountTag = chatgptAccountId ? chatgptAccountId.split('-')[0] : '';
    const details = [plan, accountTag].filter(Boolean);
    const label = displayName || accountId || '账号数据异常';
    return details.length ? `${label} · ${details.join(' · ')}` : label;
}

function remoteControlRawMessage(...items) {
    for (const item of items) {
        const raw = item && Object.prototype.hasOwnProperty.call(item, 'raw') ? item.raw : item;
        if (raw === null || raw === undefined) continue;
        const text = typeof raw === 'string' ? raw : JSON.stringify(raw);
        if (text && text.trim()) return text.trim();
    }
    return '';
}

export default function ProxySettingsTab({
    accounts = [],
    codexSessionSyncEnabled,
    maskAccountName,
    savingCodexProxyEnv,
    savingCodexRemoteControl,
    savingCodexSessionSync,
    savingProxySettings,
    restartingCodexApp,
    restartCurrentCodexAppNormal,
    codexRemoteControlPendingEnabled,
    setSettingsDraft,
    setCodexProxyEnvEnabled,
    setCodexRemoteControlAccountId,
    setCodexRemoteControlEnabled,
    setCodexSessionSyncEnabled,
    settingsDraft,
    switching,
    updateCodexProxySettings,
    updateSettingsDraftAndSave
}) {
    const proxyEnvEnabled = settingsDraft.codex_proxy_env_enabled === true;
    const codexPluginsEnabled = settingsDraft.codex_plugins_enabled === true;
    const codexRemoteControlEnabled = settingsDraft.codex_remote_control_enabled === true;
    const remoteControlAccountId = String(settingsDraft.codex_remote_control_account_id || '').trim();
    const remoteControlAccounts = Array.isArray(accounts)
        ? accounts.filter(account => !isApiModeAccount(account) && getAccountId(account))
        : [];
    const remoteControlLegacyMatches = remoteControlAccounts
        .filter(account => getChatgptAccountId(account) === remoteControlAccountId);
    const remoteControlAccount = remoteControlAccounts.find(account => getAccountId(account) === remoteControlAccountId)
        || (remoteControlLegacyMatches.length === 1 ? remoteControlLegacyMatches[0] : null);
    const remoteControlSelectedAccountId = remoteControlAccount
        ? getAccountId(remoteControlAccount)
        : remoteControlAccountId;
    const remoteControlAccountLabel = remoteControlAccount
        ? formatRemoteControlAccountLabel(remoteControlAccount, maskAccountName)
        : remoteControlAccountId
            ? '账号不存在，请重新选择'
            : '未选择';
    const codexDeleteButtonEnabled = settingsDraft.codex_delete_button_enabled === true;
    const saving = savingProxySettings || savingCodexProxyEnv;
    const sessionSyncHelp = '切换订阅/API 模式后，重新打开 Codex app 或 VS Code 前同步会话列表。';
    const [codexAppProcessStatus, setCodexAppProcessStatus] = useState({
        loading: true,
        error: '',
        pids: [],
        processCount: 0
    });
    const [remoteControlStatus, setRemoteControlStatus] = useState({
        loading: false,
        error: '',
        backendError: null,
        helperStatus: null,
        backendEnvironment: null,
        connectionStatus: null
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

    useEffect(() => {
        let disposed = false;

        async function refreshRemoteControlStatus() {
            if (!codexRemoteControlEnabled || !window.api || !window.api.getCodexRemoteControlStatus) {
                if (!disposed) {
                    setRemoteControlStatus({
                        loading: false,
                        error: '',
                        backendError: null,
                        helperStatus: null,
                        backendEnvironment: null,
                        connectionStatus: null
                    });
                }
                return;
            }

            setRemoteControlStatus(prev => ({ ...prev, loading: true, error: '' }));
            try {
                const result = await window.api.getCodexRemoteControlStatus();
                if (!disposed) {
                    setRemoteControlStatus({
                        loading: false,
                        error: '',
                        backendError: result && result.backendError ? result.backendError : null,
                        helperStatus: result && result.helperStatus ? result.helperStatus : null,
                        backendEnvironment: result && result.backendEnvironment ? result.backendEnvironment : null,
                        connectionStatus: result && result.connectionStatus ? result.connectionStatus : null
                    });
                }
            } catch (err) {
                if (!disposed) {
                    setRemoteControlStatus({
                        loading: false,
                        error: err && err.message ? err.message : '读取远程控制状态失败',
                        backendError: null,
                        helperStatus: null,
                        backendEnvironment: null,
                        connectionStatus: null
                    });
                }
            }
        }

        refreshRemoteControlStatus();
        if (!codexRemoteControlEnabled) {
            return () => {
                disposed = true;
            };
        }
        const timer = window.setInterval(refreshRemoteControlStatus, 4000);
        return () => {
            disposed = true;
            window.clearInterval(timer);
        };
    }, [codexRemoteControlEnabled, remoteControlAccountId]);

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
    const remoteControlBackendError = remoteControlStatus.backendError;
    const remoteControlHelperStatus = remoteControlStatus.helperStatus;
    const remoteControlConnectionStatus = remoteControlStatus.connectionStatus;
    const remoteControlRawStatusMessage = remoteControlRawMessage(
        remoteControlConnectionStatus,
        remoteControlBackendError,
        remoteControlHelperStatus
    );
    const remoteControlStatusMessage = remoteControlStatus.error
        || (remoteControlConnectionStatus && remoteControlConnectionStatus.message)
        || (remoteControlBackendError && remoteControlBackendError.message)
        || (remoteControlHelperStatus && remoteControlHelperStatus.message)
        || '';
    const remoteControlStatusState = remoteControlConnectionStatus && remoteControlConnectionStatus.state
        ? remoteControlConnectionStatus.state
        : (remoteControlBackendError || remoteControlStatus.error || (remoteControlHelperStatus && remoteControlHelperStatus.status === 'errored'))
        ? 'error'
        : 'muted';
    const remoteControlPendingStatus = codexRemoteControlPendingEnabled === true
        ? '打开中'
        : codexRemoteControlPendingEnabled === false
            ? '关闭中'
            : '';
    const remoteControlWarningStatus = remoteControlRawStatusMessage
        && !(remoteControlConnectionStatus && remoteControlConnectionStatus.message)
        ? remoteControlRawStatusMessage
        : remoteControlConnectionStatus && remoteControlConnectionStatus.status === 'mfa_required'
        ? '需要 MFA'
        : (remoteControlStatusMessage || '需要重新登录').replace(/[。.]$/, '');
    const remoteControlDisplayStatus = remoteControlPendingStatus || (!codexRemoteControlEnabled
        ? '未启用'
        : remoteControlStatus.loading && !remoteControlStatusMessage
            ? '检测中'
            : remoteControlStatusState === 'warning'
                ? remoteControlWarningStatus
                : (remoteControlStatusMessage || '等待连接').replace(/[。.]$/, ''));
    const remoteControlStatusTitle = (remoteControlConnectionStatus && remoteControlConnectionStatus.title)
        || remoteControlRawStatusMessage
        || (remoteControlStatusState === 'warning' ? remoteControlDisplayStatus : '');
    const remoteControlMissingAccount = !codexRemoteControlEnabled && !remoteControlAccount;
    const remoteControlToggleDisabled = savingCodexRemoteControl
        || switching
        || remoteControlMissingAccount;
    const remoteControlSwitchLabel = codexRemoteControlPendingEnabled === true
        ? '打开中'
        : codexRemoteControlPendingEnabled === false
            ? '关闭中'
            : codexRemoteControlEnabled
                ? '已启用'
                : '启用';
    const remoteControlAccountSelectDisabled = codexRemoteControlEnabled
        || savingCodexRemoteControl
        || switching
        || remoteControlAccounts.length === 0;
    const remoteControlAccountSelectTitle = codexRemoteControlEnabled
        ? '关闭 app远程控制后可切换控制账号'
        : remoteControlAccountLabel;
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
                    <div className="settings-section-title">Plugin 增强</div>
                    <div className="settings-section-desc">API 模式支持安装 Plugin</div>
                </div>
                <button
                    type="button"
                    className={`settings-toggle-row ${codexPluginsEnabled ? 'active' : ''}`}
                    aria-pressed={codexPluginsEnabled}
                    aria-label={codexPluginsEnabled ? '关闭 Plugin 增强' : '开启 Plugin 增强'}
                    disabled={switching}
                    onClick={() => updateSettingsDraftAndSave({ codex_plugins_enabled: !codexPluginsEnabled })}
                >
                    <span className="settings-toggle-copy">
                        <span className="settings-toggle-title">启用</span>
                    </span>
                    <span className="settings-switch" aria-hidden="true">
                        <span className="settings-switch-thumb" />
                    </span>
                </button>
            </section>

            <section className="settings-section settings-app-card-section settings-remote-control-section">
                <div className="settings-remote-control-topbar">
                    <div className="settings-remote-control-title-group">
                        <div className="settings-section-title">app远程控制</div>
                        <div
                            className={`settings-remote-control-status-badge ${remoteControlStatusState}`}
                            title={remoteControlStatusTitle || undefined}
                        >
                            <span className="settings-remote-control-status-dot" aria-hidden="true" />
                            <span className="settings-remote-control-status-text">{remoteControlDisplayStatus}</span>
                        </div>
                    </div>
                    <button
                        type="button"
                        className={`settings-remote-control-switch ${codexRemoteControlEnabled ? 'active' : ''}`}
                        aria-pressed={codexRemoteControlEnabled}
                        aria-label={codexRemoteControlEnabled ? '关闭 app远程控制' : '开启 app远程控制'}
                        disabled={remoteControlToggleDisabled}
                        title={remoteControlMissingAccount ? '请先选择 app远程控制账号' : ''}
                        onClick={() => setCodexRemoteControlEnabled(!codexRemoteControlEnabled)}
                    >
                        <span className="settings-remote-control-switch-label">
                            {remoteControlSwitchLabel}
                        </span>
                        <span className="settings-switch" aria-hidden="true">
                            <span className="settings-switch-thumb" />
                        </span>
                    </button>
                </div>
                <div className="settings-remote-control-account-grid">
                    <label className="settings-remote-control-account-field">
                        <span className="settings-inline-field-label">控制账号（app登录账号）</span>
                        <div className="settings-remote-control-account-select-wrap">
                            <select
                                className="settings-input settings-select settings-remote-control-account-select"
                                value={remoteControlSelectedAccountId}
                                disabled={remoteControlAccountSelectDisabled}
                                title={remoteControlAccountSelectTitle}
                                onChange={e => setCodexRemoteControlAccountId(e.target.value)}
                            >
                                <option value="">未选择</option>
                                {remoteControlAccountId && !remoteControlAccount && (
                                    <option value={remoteControlAccountId}>账号不存在</option>
                                )}
                                {remoteControlAccounts.map(account => {
                                    const accountId = getAccountId(account);
                                    return (
                                        <option key={accountId} value={accountId}>
                                            {formatRemoteControlAccountLabel(account, maskAccountName)}
                                        </option>
                                    );
                                })}
                            </select>
                            <span
                                className="settings-remote-control-account-select-arrow"
                                aria-hidden="true"
                            />
                        </div>
                    </label>
                </div>
            </section>

            <section className="settings-section settings-app-card-section settings-plugin-section">
                <div className="settings-section-head">
                    <div className="settings-section-title">会话删除</div>
                    <div className="settings-section-desc">在 Codex 会话列表增加删除入口，删除后可恢复</div>
                </div>
                <button
                    type="button"
                    className={`settings-toggle-row ${codexDeleteButtonEnabled ? 'active' : ''}`}
                    aria-pressed={codexDeleteButtonEnabled}
                    aria-label={codexDeleteButtonEnabled ? '关闭会话删除入口' : '开启会话删除入口'}
                    disabled={switching}
                    onClick={() => updateSettingsDraftAndSave({ codex_delete_button_enabled: !codexDeleteButtonEnabled })}
                >
                    <span className="settings-toggle-copy">
                        <span className="settings-toggle-title">启用</span>
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
