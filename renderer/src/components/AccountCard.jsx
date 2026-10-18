import QuotaItem from './QuotaItem';
import SubscriptionBadges from './SubscriptionBadges';
import UsageStatsSummary from './UsageStatsSummary';
import CardMoreMenu from './CardMoreMenu';
import { IntelligenceRunningTag } from './IntelligencePanel';
import { getCodexAppInstanceKey } from '../utils/codexAppInstances';
import { parseAuthInfo, getAccountName, getAccountId, getChatgptAccountId, isAuthSessionInvalid, maskAccountDisplayName } from '../utils/auth';
import { useI18n } from '../i18n';

const SINGLE_WORKSPACE_PLANS = new Set(['free', 'plus', 'pro', 'personal']);

function authErrorIncludes(message, patterns) {
    const value = typeof message === 'string' ? message.toLowerCase() : '';
    return patterns.some(pattern => value.includes(String(pattern).toLowerCase()));
}

function getAuthErrorLabel(info, t) {
    const message = typeof info.authStatusMessage === 'string' ? info.authStatusMessage : '';
    if (info.isApiMode) {
        return message.includes('API Key') ? t('未配置 API Key') : t('API 配置异常');
    }

    if (message.includes('账号数据异常') || authErrorIncludes(message, ['id_token', 'claims'])) {
        return t('账号数据异常');
    }
    if (message.includes('刷新后账号标识不一致')) {
        return t('账号不匹配');
    }
    if (isAuthSessionInvalid(info)) {
        return t('登录已失效');
    }
    if (authErrorIncludes(message, ['timeout', 'timed out', 'dns', 'proxy', 'connect', 'connection', 'tls', 'network'])) {
        return t('网络刷新失败');
    }
    if (authErrorIncludes(message, ['too many requests', 'http 429'])) {
        return t('刷新太频繁');
    }
    if (authErrorIncludes(message, ['service temporarily unavailable', 'http 500', 'http 502', 'http 503', 'http 504'])) {
        return t('服务暂不可用');
    }
    if (authErrorIncludes(message, ['deactivated_workspace', 'workspace has been deactivated'])) {
        return t('Workspace 已停用');
    }
    return t('刷新失败');
}

function getAuthBadge(info, t, translateRuntimeText) {
    if (info.authStatus === 'error') {
        const label = getAuthErrorLabel(info, t);
        const message = typeof info.authStatusMessage === 'string' ? info.authStatusMessage.trim() : '';
        return {
            label,
            title: message ? `${label}: ${translateRuntimeText(message)}` : label,
            className: 'status-badge auth-error'
        };
    }

    return null;
}

export default function AccountCard({ acc, isCurrent, refreshing, switching, usageStats, maskAccountName, onSwitch, onOpenCodexAppInstance, openingCodexAppTarget, runningCodexAppInstances, onRefresh, onDelete, onViewRefreshToken, onOpenUsageStatsDetail, onTestIntelligence, intelligenceProgress }) {
    const { t, translateRuntimeText } = useI18n();
    const info = parseAuthInfo(acc);
    const authBadge = getAuthBadge(info, t, translateRuntimeText);
    const normalizedPlan = info.planType.toLowerCase();
    const planLabel = info.planType.toUpperCase();
    const isPersonalPlan = normalizedPlan === 'personal';
    const showAccountTag = accountId => {
        if (info.isApiMode) return false;
        if (!accountId) return false;
        return !SINGLE_WORKSPACE_PLANS.has(normalizedPlan);
    };
    const name = getAccountName(acc);
    const displayName = maskAccountName ? maskAccountDisplayName(name) : name;
    const accountId = getAccountId(acc);
    const codexAppTargetKey = accountId ? `account:${accountId}` : '';
    const codexAppInstanceKey = accountId ? getCodexAppInstanceKey('account', accountId) : '';
    const openingThisCodexApp = openingCodexAppTarget === codexAppTargetKey;
    const openingAnyCodexApp = Boolean(openingCodexAppTarget);
    const codexAppInstanceRunning = Boolean(
        codexAppInstanceKey && runningCodexAppInstances && runningCodexAppInstances[codexAppInstanceKey]
    );
    const chatgptAccountId = getChatgptAccountId(acc);
    const accountTag = chatgptAccountId ? chatgptAccountId.split('-')[0] : '';
    const authStatusMessage = typeof info.authStatusMessage === 'string'
        ? info.authStatusMessage.trim()
        : '';
    const accountNotice = info.authStatus === 'error'
        ? {
            tone: 'error',
            message: authStatusMessage || (authBadge && authBadge.label) || t('刷新失败'),
            detail: authStatusMessage || (authBadge && authBadge.label) || t('刷新失败')
        }
        : info.usageNotice;
    const accountNoticeTitle = accountNotice
        ? (accountNotice.detail || accountNotice.message)
        : '';
    const localizedAccountNoticeTitle = translateRuntimeText(accountNoticeTitle);

    return (
        <div className={`account-card ${isCurrent ? 'active' : ''}`}>
            <div className="account-card-head">
                <div className="account-card-name-row">
                    <div className="account-card-name" title={displayName}>{displayName}</div>
                    {isCurrent && <span className="current-badge">{t('当前')}</span>}
                    {codexAppInstanceRunning && (
                        <span className="codex-app-running-badge" title={t('独立 Codex 正在运行')}>
                            {t('窗口运行中')}
                        </span>
                    )}
                    {intelligenceProgress && <IntelligenceRunningTag progress={intelligenceProgress} />}
                </div>
                <div className="account-badges account-card-badges">
                    {planLabel && !isPersonalPlan && (
                        <span className={`plan-badge plan-${normalizedPlan}`}>
                            {planLabel}
                        </span>
                    )}
                    {authBadge && (
                        <span className={authBadge.className} title={authBadge.title}>
                            {authBadge.label}
                        </span>
                    )}
                    {showAccountTag(chatgptAccountId) && accountTag && (
                        <span className="account-id-badge" title={t('账号ID: {id}', { id: chatgptAccountId })}>
                            {accountTag}
                        </span>
                    )}
                    <SubscriptionBadges info={info} planLabel={planLabel} />
                </div>
            </div>

            <div className={`account-card-body ${accountNotice ? 'account-card-body-status' : ''}`}>
                <div className={`account-card-quotas ${accountNotice ? 'account-card-quotas-status' : ''}`}>
                    {accountNotice ? (
                        <div
                            className={accountNotice.tone === 'error' ? 'quota-error' : 'quota-status quota-status-info'}
                            title={localizedAccountNoticeTitle}
                            aria-label={localizedAccountNoticeTitle}
                        >
                            <span className="error-icon">{accountNotice.tone === 'error' ? '⚠️' : 'ℹ️'}</span>
                            <span className="error-msg" title={localizedAccountNoticeTitle}>{translateRuntimeText(accountNotice.message)}</span>
                        </div>
                    ) : (
                        <>
                        {info.usageWindows.map((usageWindow, index) => (
                            <QuotaItem
                                key={`${usageWindow.limit_window_seconds}-${usageWindow.reset_at}-${index}`}
                                window={usageWindow}
                                variant="card"
                            />
                        ))}
                        {info.usageWindows.length < 2 && (
                            <div className="quota-item quota-item-card quota-item-card-placeholder" aria-hidden="true" />
                        )}
                        </>
                    )}
                </div>
                <UsageStatsSummary
                    stats={usageStats}
                    onOpenDetails={() => onOpenUsageStatsDetail?.({
                        ownerType: 'subscription',
                        ownerId: accountId,
                        ownerName: displayName,
                        ownerTypeLabel: t('订阅账号')
                    })}
                />
            </div>

            <div className="account-card-footer">
                <div className="action-btns">
                    {!info.isApiMode && (
                        <button
                            type="button"
                            className="icon-btn"
                            title={t('降智测试')}
                            aria-label={t('降智测试')}
                            onClick={() => onTestIntelligence({ accountId, name: displayName })}
                            disabled={!accountId}
                        >
                            <svg fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 3h6m-5 0v6.5l-5.2 8.7A1.8 1.8 0 0 0 6.35 21h11.3a1.8 1.8 0 0 0 1.55-2.8L14 9.5V3M7.5 15h9" />
                            </svg>
                        </button>
                    )}
                    {!info.isApiMode && (
                        <button
                            type="button"
                            className="icon-btn"
                            title={t('查看 Refresh Token')}
                            aria-label={t('查看 Refresh Token')}
                            onClick={() => onViewRefreshToken(acc)}
                        >
                            <svg fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M1.5 12s3.75-7.5 10.5-7.5S22.5 12 22.5 12s-3.75 7.5-10.5 7.5S1.5 12 1.5 12Z" />
                                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 15.75A3.75 3.75 0 1 0 12 8.25a3.75 3.75 0 0 0 0 7.5Z" />
                            </svg>
                        </button>
                    )}
                    {!info.isApiMode && (
                        <button
                            type="button"
                            className="icon-btn"
                            title={refreshing ? t('刷新配额中') : t('刷新配额')}
                            aria-label={refreshing ? t('刷新配额中') : t('刷新配额')}
                            onClick={() => onRefresh(accountId)}
                            disabled={refreshing}
                        >
                            <svg className={refreshing ? 'icon-spin' : ''} fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
                            </svg>
                        </button>
                    )}
                    {!isCurrent && (
                        <button
                            type="button"
                            className="icon-btn"
                            title={t('切换到此账号')}
                            aria-label={t('切换到此账号')}
                            onClick={() => onSwitch(accountId)}
                            disabled={switching}
                        >
                            ⚡
                        </button>
                    )}
                    {!info.isApiMode && (
                        <CardMoreMenu
                            items={[
                                {
                                    key: 'codex',
                                    label: openingThisCodexApp ? t('正在打开 Codex') : t('打开独立 Codex'),
                                    disabled: !accountId || openingAnyCodexApp,
                                    onSelect: () => onOpenCodexAppInstance(accountId)
                                },
                                { key: 'delete', label: t('删除'), danger: true, onSelect: () => onDelete(acc) }
                            ]}
                        />
                    )}
                </div>
            </div>
        </div>
    );
}
