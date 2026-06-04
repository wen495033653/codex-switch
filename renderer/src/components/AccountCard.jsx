import QuotaItem from './QuotaItem';
import { parseAuthInfo, getAccountName, getAccountId, getChatgptAccountId, maskAccountDisplayName } from '../utils/auth';

const SINGLE_WORKSPACE_PLANS = new Set(['free', 'plus', 'pro', 'personal']);

function authErrorIncludes(message, patterns) {
    const value = typeof message === 'string' ? message.toLowerCase() : '';
    return patterns.some(pattern => value.includes(String(pattern).toLowerCase()));
}

function getAuthErrorLabel(info) {
    const message = typeof info.authStatusMessage === 'string' ? info.authStatusMessage : '';
    if (info.isApiMode) {
        return message.includes('API Key') ? 'API Key 未配置' : 'API 配置异常';
    }

    if (message.includes('账号数据异常') || authErrorIncludes(message, ['id_token', 'claims'])) {
        return '账号数据异常';
    }
    if (message.includes('刷新后账号标识不一致')) {
        return '账号不匹配';
    }
    if (authErrorIncludes(message, ['timeout', 'timed out', 'dns', 'proxy', 'connect', 'connection', 'tls', 'network'])) {
        return '网络刷新失败';
    }
    if (authErrorIncludes(message, ['too many requests', 'http 429'])) {
        return '刷新太频繁';
    }
    if (authErrorIncludes(message, ['service temporarily unavailable', 'http 500', 'http 502', 'http 503', 'http 504'])) {
        return '服务暂不可用';
    }
    if (authErrorIncludes(message, ['deactivated_workspace', 'workspace has been deactivated'])) {
        return 'Workspace 已停用';
    }
    if (authErrorIncludes(message, ['invalid_grant', 'unauthorized', 'authorization expired', '缺少 refreshtoken', '缺少 refresh_token', '刷新结果缺少 refresh_token'])) {
        return '登录已失效';
    }

    return '刷新失败';
}

function getAuthBadge(info) {
    if (info.authStatus === 'error') {
        const label = getAuthErrorLabel(info);
        const message = typeof info.authStatusMessage === 'string' ? info.authStatusMessage.trim() : '';
        return {
            label,
            title: message ? `${label}：${message}` : label,
            className: 'status-badge auth-error'
        };
    }

    return null;
}

export default function AccountCard({ acc, isCurrent, refreshing, switching, maskAccountName, onSwitch, onRefresh, onDelete, onViewRefreshToken }) {
    const info = parseAuthInfo(acc);
    const plan = info.planType;
    const authBadge = getAuthBadge(info);
    const normalizedPlan = typeof plan === 'string' ? plan.toLowerCase() : '';
    const isPersonalPlan = normalizedPlan === 'personal';
    const showAccountTag = accountId => {
        if (info.isApiMode) return false;
        if (!accountId) return false;
        return !SINGLE_WORKSPACE_PLANS.has(normalizedPlan);
    };
    const name = getAccountName(acc);
    const displayName = maskAccountName ? maskAccountDisplayName(name) : name;
    const accountId = getAccountId(acc);
    const chatgptAccountId = getChatgptAccountId(acc);
    const accountTag = chatgptAccountId ? chatgptAccountId.split('-')[0] : '';
    const usageNoticeTitle = info.usageNotice
        ? (info.usageNotice.detail || info.usageNotice.message)
        : '';

    return (
        <div className={`account-card ${isCurrent ? 'active' : ''}`}>
            <div className="account-card-head">
                <div className="account-card-name-row">
                    <div className="account-card-name" title={displayName}>{displayName}</div>
                    {isCurrent && <span className="current-badge">当前</span>}
                </div>
                <div className="account-badges account-card-badges">
                    {plan && !isPersonalPlan && (
                        <span className={`plan-badge plan-${normalizedPlan}`}>
                            {plan.toUpperCase()}
                        </span>
                    )}
                    {authBadge && (
                        <span className={authBadge.className} title={authBadge.title}>
                            {authBadge.label}
                        </span>
                    )}
                    {showAccountTag(chatgptAccountId) && accountTag && (
                        <span className="account-id-badge" title={`账号ID: ${chatgptAccountId}`}>
                            {accountTag}
                        </span>
                    )}
                    {info.showExpiresAt && info.expiresAt && (
                        <span className="expire-date" title="订阅到期日期">
                            到期 {new Date(info.expiresAt).toLocaleDateString()}
                        </span>
                    )}
                </div>
            </div>

            <div className="account-card-body">
                <div className={`account-card-quotas ${info.usageNotice ? 'account-card-quotas-status' : ''}`}>
                    {info.usageNotice ? (
                        <div
                            className={info.usageNotice.tone === 'error' ? 'quota-error' : 'quota-status quota-status-info'}
                            title={usageNoticeTitle}
                            aria-label={usageNoticeTitle}
                        >
                            <span className="error-icon">{info.usageNotice.tone === 'error' ? '⚠️' : 'ℹ️'}</span>
                            <span className="error-msg" title={usageNoticeTitle}>{info.usageNotice.message}</span>
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
            </div>

            <div className="account-card-footer">
                <div className="action-btns">
                    {!info.isApiMode && (
                        <button className="icon-btn" title="查看 Refresh Token" onClick={() => onViewRefreshToken(acc)}>
                            <svg fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M1.5 12s3.75-7.5 10.5-7.5S22.5 12 22.5 12s-3.75 7.5-10.5 7.5S1.5 12 1.5 12Z" />
                                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 15.75A3.75 3.75 0 1 0 12 8.25a3.75 3.75 0 0 0 0 7.5Z" />
                            </svg>
                        </button>
                    )}
                    {!info.isApiMode && (
                        <button
                            className="icon-btn"
                            title={refreshing ? '刷新配额中' : '刷新配额'}
                            aria-label={refreshing ? '刷新配额中' : '刷新配额'}
                            onClick={() => onRefresh(accountId)}
                            disabled={refreshing}
                        >
                            <svg className={refreshing ? 'icon-spin' : ''} fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
                            </svg>
                        </button>
                    )}
                    {!isCurrent && (
                        <button className="icon-btn" title="切换到此账号" onClick={() => onSwitch(accountId)} disabled={switching}>
                            ⚡
                        </button>
                    )}
                    {!info.isApiMode && (
                        <button className="icon-btn danger" title="删除" onClick={() => onDelete(acc)}>
                            <svg fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 7h12m-9 0V5.75A1.75 1.75 0 0 1 10.75 4h2.5A1.75 1.75 0 0 1 15 5.75V7m-7.75 0 .75 12.25A1.75 1.75 0 0 0 9.75 21h4.5A1.75 1.75 0 0 0 16 19.25L16.75 7M10 11v6m4-6v6" />
                            </svg>
                        </button>
                    )}
                </div>
            </div>
        </div>
    );
}
