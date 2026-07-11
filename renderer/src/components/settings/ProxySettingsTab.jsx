import { useEffect, useRef, useState } from 'react';
import { useAsyncPolling } from '../../hooks/useAsyncPolling';
import { getAccountId, getChatgptAccountId, isApiModeAccount } from '../../utils/auth/account';
import { getAccountName, maskAccountDisplayName, parseAuthInfo } from '../../utils/auth/info';
import { useI18n } from '../../i18n';

const CODEX_DESKTOP_UPDATE_URL = 'https://learn.chatgpt.com/docs/whats-new#use-codex-in-the-chatgpt-desktop-app';

function normalizePids(value) {
    if (!Array.isArray(value)) return [];
    return value
        .map(pid => Number(pid))
        .filter(pid => Number.isInteger(pid) && pid > 0);
}

function formatRemoteControlAccountLabel(account, maskAccountName, t) {
    const accountId = getAccountId(account);
    const name = getAccountName(account);
    const displayName = maskAccountName ? maskAccountDisplayName(name) : name;
    const info = parseAuthInfo(account);
    const plan = info.planType ? info.planType.toUpperCase() : '';
    const chatgptAccountId = getChatgptAccountId(account);
    const accountTag = chatgptAccountId ? chatgptAccountId.split('-')[0] : '';
    const details = [plan, accountTag].filter(Boolean);
    const label = displayName || accountId || t('账号数据异常');
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

export function CodexProcessCard({
    restartingCodexApp,
    restartCurrentCodexAppNormal
}) {
    const { t, translateRuntimeText } = useI18n();
    const [codexAppProcessStatus, setCodexAppProcessStatus] = useState({
        loading: true,
        error: '',
        pids: [],
        processCount: 0,
        supported: true,
        requiresUpdate: false,
        compatibilityMessage: ''
    });
    useAsyncPolling(async ({ isCurrent }) => {
        if (!window.api || !window.api.getCurrentCodexAppProcesses) {
            if (isCurrent()) {
                setCodexAppProcessStatus({
                    loading: false,
                    error: '',
                    pids: [],
                    processCount: 0,
                    supported: false,
                    requiresUpdate: true,
                    compatibilityMessage: t('无法检测 ChatGPT Desktop 版本')
                });
            }
            return;
        }

        try {
            const result = await window.api.getCurrentCodexAppProcesses();
            if (isCurrent()) {
                setCodexAppProcessStatus({
                    loading: false,
                    error: result && result.error ? String(result.error) : '',
                    pids: normalizePids(result && result.pids),
                    processCount: Number(result && result.processCount) || 0,
                    supported: result && result.supported !== false,
                    requiresUpdate: result && result.requiresUpdate === true,
                    compatibilityMessage: result && result.compatibilityMessage
                        ? String(result.compatibilityMessage)
                        : ''
                });
            }
        } catch (err) {
            if (isCurrent()) {
                setCodexAppProcessStatus({
                    loading: false,
                    error: err && err.message ? translateRuntimeText(err.message) : t('读取失败'),
                    pids: [],
                    processCount: 0,
                    supported: false,
                    requiresUpdate: false,
                    compatibilityMessage: ''
                });
            }
        }
    }, { intervalMs: 3000 });

    const codexAppPidText = codexAppProcessStatus.loading
        ? t('检测中')
        : codexAppProcessStatus.requiresUpdate
            ? t('需要更新')
            : translateRuntimeText(codexAppProcessStatus.error) || (codexAppProcessStatus.pids.length ? codexAppProcessStatus.pids.join(', ') : t('未检测到'));
    const codexAppPidTitle = codexAppProcessStatus.processCount > codexAppProcessStatus.pids.length
        ? t('共检测到 {count} 个 Codex 进程，这里显示主进程 PID', { count: codexAppProcessStatus.processCount })
        : '';
    const codexAppPidState = codexAppProcessStatus.error || codexAppProcessStatus.requiresUpdate
        ? 'error'
        : codexAppProcessStatus.pids.length
            ? 'active'
            : 'empty';
    const restartCodexAppDisabled = restartingCodexApp
        || codexAppProcessStatus.loading
        || !codexAppProcessStatus.supported
        || Boolean(codexAppProcessStatus.error)
        || codexAppProcessStatus.pids.length === 0;

    return (
        <section className="settings-codex-app-pid-card" aria-label={t('当前 Codex PID')}>
            <span
                className="settings-codex-app-pid-label"
                title={codexAppProcessStatus.compatibilityMessage || undefined}
            >
                {t('当前 Codex PID')}
            </span>
            <span className="settings-codex-app-pid-actions">
                <span className={`settings-codex-app-pid-value ${codexAppPidState}`} title={codexAppPidTitle}>{codexAppPidText}</span>
                {codexAppProcessStatus.requiresUpdate ? (
                    <button
                        type="button"
                        className="settings-codex-app-restart-button"
                        onClick={() => window.api.openExternalUrl(CODEX_DESKTOP_UPDATE_URL)}
                    >
                        {t('更新 ChatGPT')}
                    </button>
                ) : (
                    <button
                        type="button"
                        className="settings-codex-app-restart-button"
                        disabled={restartCodexAppDisabled}
                        onClick={restartCurrentCodexAppNormal}
                    >
                        {restartingCodexApp ? t('重启中...') : t('重启 Codex')}
                    </button>
                )}
            </span>
        </section>
    );
}

export default function ProxySettingsTab({
    accounts = [],
    codexSessionSyncEnabled,
    maskAccountName,
    savingCodexProxyEnv,
    savingCodexRemoteControl,
    savingCodexSessionSync,
    savingProxySettings,
    subscriptionModeActive,
    codexRemoteControlPendingEnabled,
    onCodexRemoteControlAutoDisabled,
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
    const { t, translateRuntimeText } = useI18n();
    const proxyEnvEnabled = settingsDraft.codex_proxy_env_enabled === true;
    const codexPluginsEnabled = settingsDraft.codex_plugins_enabled === true;
    const codexRemoteControlEnabled = settingsDraft.codex_remote_control_enabled === true;
    const remoteControlBlockedBySubscription = subscriptionModeActive === true;
    const remoteControlEnabledInCurrentMode = codexRemoteControlEnabled && !remoteControlBlockedBySubscription;
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
        ? formatRemoteControlAccountLabel(remoteControlAccount, maskAccountName, t)
        : remoteControlAccountId
            ? t('账号不存在，请重新选择')
            : t('未选择');
    const saving = savingProxySettings || savingCodexProxyEnv;
    const sessionSyncHelp = t('切换订阅/API 模式后，重新打开 Codex 或 VS Code 前同步会话列表。');
    const [remoteControlStatus, setRemoteControlStatus] = useState({
        loading: false,
        error: '',
        backendError: null,
        helperStatus: null,
        backendEnvironment: null,
        connectionStatus: null
    });
    const remoteControlAutoDisableNotifiedRef = useRef(false);
    const onRemoteControlAutoDisabledRef = useRef(onCodexRemoteControlAutoDisabled);
    useEffect(() => {
        onRemoteControlAutoDisabledRef.current = onCodexRemoteControlAutoDisabled;
    }, [onCodexRemoteControlAutoDisabled]);
    useEffect(() => {
        if (remoteControlEnabledInCurrentMode) {
            remoteControlAutoDisableNotifiedRef.current = false;
        }
    }, [remoteControlEnabledInCurrentMode, remoteControlAccountId]);
    useEffect(() => {
        if (!remoteControlEnabledInCurrentMode) {
            setRemoteControlStatus({
                loading: false,
                error: '',
                backendError: null,
                helperStatus: null,
                backendEnvironment: null,
                connectionStatus: null
            });
        }
    }, [remoteControlEnabledInCurrentMode, remoteControlAccountId]);
    useAsyncPolling(async ({ isCurrent }) => {
        if (!window.api || !window.api.getCodexRemoteControlStatus) return;

        if (isCurrent()) setRemoteControlStatus(prev => ({ ...prev, loading: true, error: '' }));
        try {
            const result = await window.api.getCodexRemoteControlStatus();
            if (!isCurrent()) return;
            if (result && result.autoDisabled === true && !remoteControlAutoDisableNotifiedRef.current) {
                remoteControlAutoDisableNotifiedRef.current = true;
                if (typeof onRemoteControlAutoDisabledRef.current === 'function') {
                    onRemoteControlAutoDisabledRef.current(result);
                }
            }
            setRemoteControlStatus({
                loading: false,
                error: '',
                backendError: result && result.backendError ? result.backendError : null,
                helperStatus: result && result.helperStatus ? result.helperStatus : null,
                backendEnvironment: result && result.backendEnvironment ? result.backendEnvironment : null,
                connectionStatus: result && result.connectionStatus ? result.connectionStatus : null
            });
        } catch (err) {
            if (isCurrent()) {
                setRemoteControlStatus({
                    loading: false,
                    error: err && err.message ? translateRuntimeText(err.message) : t('读取远程控制状态失败'),
                    backendError: null,
                    helperStatus: null,
                    backendEnvironment: null,
                    connectionStatus: null
                });
            }
        }
    }, {
        enabled: remoteControlEnabledInCurrentMode,
        intervalMs: 4000,
        refreshKey: remoteControlAccountId
    });

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
    const remoteControlStatusState = remoteControlBlockedBySubscription
        ? 'muted'
        : remoteControlConnectionStatus && remoteControlConnectionStatus.state
            ? remoteControlConnectionStatus.state
            : (remoteControlBackendError || remoteControlStatus.error || (remoteControlHelperStatus && remoteControlHelperStatus.status === 'errored'))
                ? 'error'
                : 'muted';
    const remoteControlPendingStatus = codexRemoteControlPendingEnabled === true
        ? t('打开中')
        : codexRemoteControlPendingEnabled === false
            ? t('关闭中')
            : '';
    const remoteControlWarningStatus = remoteControlRawStatusMessage
        && !(remoteControlConnectionStatus && remoteControlConnectionStatus.message)
        ? remoteControlRawStatusMessage
        : remoteControlConnectionStatus && remoteControlConnectionStatus.status === 'mfa_required'
        ? t('需要 MFA')
        : (translateRuntimeText(remoteControlStatusMessage) || t('需要重新登录')).replace(/[。.]$/, '');
    const remoteControlDisplayStatus = remoteControlBlockedBySubscription
        ? t('订阅模式不可用')
        : remoteControlPendingStatus || (!remoteControlEnabledInCurrentMode
            ? t('未启用')
            : remoteControlStatus.loading && !remoteControlStatusMessage
                ? t('检测中')
                : remoteControlStatusState === 'warning'
                    ? remoteControlWarningStatus
                    : (translateRuntimeText(remoteControlStatusMessage) || t('等待连接')).replace(/[。.]$/, ''));
    const remoteControlStatusTitle = (remoteControlConnectionStatus && remoteControlConnectionStatus.title)
        || remoteControlRawStatusMessage
        || (remoteControlStatusState === 'warning' ? remoteControlDisplayStatus : '');
    const remoteControlMissingAccount = !remoteControlBlockedBySubscription && !remoteControlEnabledInCurrentMode && !remoteControlAccount;
    const remoteControlToggleDisabled = savingCodexRemoteControl
        || switching
        || remoteControlBlockedBySubscription
        || remoteControlMissingAccount;
    const remoteControlSwitchLabel = codexRemoteControlPendingEnabled === true
        ? t('打开中')
        : codexRemoteControlPendingEnabled === false
            ? t('关闭中')
            : codexRemoteControlEnabled
                ? t('启动')
                : remoteControlBlockedBySubscription
                    ? t('不可用')
                    : t('启用');
    const remoteControlAccountSelectDisabled = remoteControlBlockedBySubscription
        || remoteControlEnabledInCurrentMode
        || savingCodexRemoteControl
        || switching
        || remoteControlAccounts.length === 0;
    const remoteControlAccountSelectTitle = remoteControlBlockedBySubscription
        ? t('订阅模式下不可开启远程控制')
        : remoteControlEnabledInCurrentMode
        ? t('关闭远程控制后可切换控制账号')
        : remoteControlAccountLabel;
    return (
        <>
            <section className="settings-section settings-app-card-section settings-proxy-section">
                <div className="settings-proxy-copy">
                    <div className="settings-section-title">{t('Codex 代理')}</div>
                    <div className="settings-section-desc">{t('设置 Codex 使用的代理地址')}</div>
                </div>

                <label className="settings-field settings-proxy-field">
                    <span className="settings-inline-field-label">{t('代理地址')}</span>
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

                <button
                    type="button"
                    className={`settings-feature-switch settings-proxy-switch-button ${proxyEnvEnabled ? 'active' : ''}`}
                    aria-pressed={proxyEnvEnabled}
                    disabled={saving}
                    onClick={() => setCodexProxyEnvEnabled(!proxyEnvEnabled)}
                >
                    <span className="settings-feature-switch-label settings-proxy-switch-label">{t('启动')}</span>
                    <span className="settings-switch" aria-hidden="true">
                        <span className="settings-switch-thumb" />
                    </span>
                </button>
            </section>

            <section className="settings-section settings-app-card-section settings-plugin-section">
                <div className="settings-feature-head">
                    <div className="settings-section-head">
                        <div className="settings-section-title">{t('Plugin 增强')}</div>
                        <div className="settings-section-desc">{t('API 模式支持安装 Plugin')}</div>
                    </div>
                    <button
                        type="button"
                        className={`settings-feature-switch ${codexPluginsEnabled ? 'active' : ''}`}
                        aria-pressed={codexPluginsEnabled}
                        aria-label={codexPluginsEnabled ? t('关闭 Plugin 增强') : t('开启 Plugin 增强')}
                        disabled={switching}
                        onClick={() => updateSettingsDraftAndSave({ codex_plugins_enabled: !codexPluginsEnabled })}
                    >
                        <span className="settings-feature-switch-label">{t('启用')}</span>
                        <span className="settings-switch" aria-hidden="true">
                            <span className="settings-switch-thumb" />
                        </span>
                    </button>
                </div>
            </section>

            <section className={`settings-section settings-app-card-section settings-remote-control-section ${remoteControlBlockedBySubscription ? 'disabled' : ''}`}>
                <div className="settings-remote-control-topbar">
                    <div className="settings-remote-control-title-group">
                        <div className="settings-section-title">{t('远程控制')}</div>
                        <span className="settings-remote-control-mode-badge">{t('仅 API 模式')}</span>
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
                        aria-label={codexRemoteControlEnabled ? t('关闭远程控制') : t('开启远程控制')}
                        disabled={remoteControlToggleDisabled}
                        title={remoteControlBlockedBySubscription ? t('订阅模式下不可开启远程控制') : remoteControlMissingAccount ? t('请先选择远程控制账号') : ''}
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
                <div className="settings-section-desc settings-remote-control-note">
                    <span className="settings-remote-control-note-title">{t('仅 API 模式下使用')}</span>
                    <span className="settings-remote-control-note-text">{t('请求流量走 API，控制操作使用选定的 Codex 登录账号。')}</span>
                </div>
                <div className="settings-remote-control-account-grid">
                    <label className="settings-remote-control-account-field">
                        <span className="settings-inline-field-label">{t('控制账号（Codex 登录账号）')}</span>
                        <div className="settings-remote-control-account-select-wrap">
                            <select
                                className="settings-input settings-select settings-remote-control-account-select"
                                value={remoteControlSelectedAccountId}
                                disabled={remoteControlAccountSelectDisabled}
                                title={remoteControlAccountSelectTitle}
                                onChange={e => setCodexRemoteControlAccountId(e.target.value)}
                            >
                                <option value="">{t('未选择')}</option>
                                {remoteControlAccountId && !remoteControlAccount && (
                                    <option value={remoteControlAccountId}>{t('账号不存在')}</option>
                                )}
                                {remoteControlAccounts.map(account => {
                                    const accountId = getAccountId(account);
                                    return (
                                        <option key={accountId} value={accountId}>
                                            {formatRemoteControlAccountLabel(account, maskAccountName, t)}
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

            <section className="settings-section settings-app-card-section settings-session-sync-section">
                <div className="settings-feature-head">
                    <div className="settings-section-head">
                        <div className="settings-section-title">{t('会话同步')}</div>
                        <div className="settings-section-desc">{t('订阅/API 模式下会话列表保持同步')}</div>
                    </div>
                    <button
                        type="button"
                        className={`settings-feature-switch ${codexSessionSyncEnabled ? 'active' : ''}`}
                        aria-pressed={codexSessionSyncEnabled}
                        aria-label={codexSessionSyncEnabled ? t('关闭会话同步') : t('开启会话同步')}
                        disabled={savingCodexSessionSync || switching}
                        title={sessionSyncHelp}
                        onClick={() => setCodexSessionSyncEnabled(!codexSessionSyncEnabled)}
                    >
                        <span className="settings-feature-switch-label">{t('启用')}</span>
                        <span className="settings-switch" aria-hidden="true">
                            <span className="settings-switch-thumb" />
                        </span>
                    </button>
                </div>
            </section>
        </>
    );
}
