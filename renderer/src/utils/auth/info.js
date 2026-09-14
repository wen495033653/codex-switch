import { getChatgptAccountId, isApiModeAccount } from './account';
import { safeParseJwt } from './jwt';
import { getSubscription } from './subscription';
import { getResetCredits, getUsageNotice, getUsageWindows, normalizeErrorState } from './usage';

const PLAN_TYPE_ALIASES = {
    prolite: 'pro',
    self_serve_business_usage_based: 'team'
};

function normalizePlanType(value) {
    const raw = typeof value === 'string' ? value.trim() : '';
    if (!raw || raw === 'unknown') return '';
    return PLAN_TYPE_ALIASES[raw.toLowerCase()] || raw;
}

export function isAuthSessionInvalid(info) {
    if (!info || info.isApiMode || info.authStatus !== 'error') return false;
    const message = typeof info.authStatusMessage === 'string'
        ? info.authStatusMessage.toLowerCase()
        : '';
    return [
        'refresh_token_invalidated',
        'refresh token invalidated',
        'session has ended',
        'invalid_grant',
        'unauthorized',
        'authorization expired',
        'authentication token is expired',
        'please log out and sign in again',
        '缺少 refreshtoken',
        '缺少 refresh_token',
        '刷新结果缺少 refresh_token',
        '登录已失效',
        '登录已过期',
        '请重新登录'
    ].some(pattern => message.includes(pattern));
}

// Every field the card and settings pages read; the early returns below only override
// what differs, so a new field is declared once.
const EMPTY_AUTH_INFO = Object.freeze({
    isApiMode: false,
    email: '',
    planType: '',
    usage: null,
    usageWindows: [],
    resetCredits: null,
    subscription: null,
    expiresAt: '',
    expiresAtStale: false,
    subscriptionLastCheckedAt: '',
    showExpiresAt: false,
    authStatus: 'active',
    authStatusMessage: '',
    usageStatus: 'missing',
    usageStatusMessage: '',
    usageNotice: null,
    workspace: ''
});

/**
 * `now` is only a parameter so the stale-claim decision is testable; callers use the clock.
 */
export function parseAuthInfo(account, now = Date.now()) {
    if (isApiModeAccount(account)) {
        const api = account.api && typeof account.api === 'object' ? account.api : {};
        const baseUrl = typeof api.base_url === 'string' ? api.base_url : '';
        const configured = api.configured === true;
        return {
            ...EMPTY_AUTH_INFO,
            isApiMode: true,
            planType: 'API',
            authStatus: configured ? 'active' : 'error',
            authStatusMessage: configured ? '' : '请先在设置中填写 API Key',
            usageStatus: 'ok',
            usageNotice: {
                tone: configured ? 'info' : 'error',
                message: baseUrl
                    ? `Base URL: ${baseUrl}`
                    : '未配置 API Base URL'
            },
            workspace: 'API 模式'
        };
    }

    const tokens = account && account.tokens ? account.tokens : {};
    const parsed = safeParseJwt(tokens.id_token);
    if (!parsed.claims) {
        return {
            ...EMPTY_AUTH_INFO,
            authStatus: 'error',
            authStatusMessage: parsed.error,
            usageNotice: {
                tone: 'error',
                message: '账号数据异常，请重新导入或删除后添加'
            },
            workspace: '账号数据异常'
        };
    }

    const claims = parsed.claims;
    const auth = claims['https://api.openai.com/auth'] || {};

    const accountId = getChatgptAccountId(account);
    const custom = account && account.custom ? account.custom : {};
    const usage = custom.usage_info;
    // /wham/usage plan_type refreshes with every quota sync; the id_token claim only
    // changes when the token is re-issued, so prefer the usage value when present.
    const usagePlanType = normalizePlanType(usage && usage.plan_type);
    const subscription = getSubscription(custom);
    const planType = usagePlanType
        || normalizePlanType(subscription && subscription.planType)
        || normalizePlanType(auth.chatgpt_plan_type);
    const isFreePlan = planType.toLowerCase() === 'free';
    // The id_token claim is a snapshot OpenAI does not always refresh, so the renewal date
    // comes from the subscriptions endpoint whenever it has been read for this account.
    const claimExpiresAt = auth.chatgpt_subscription_active_until || '';
    const expiresAt = subscription ? subscription.activeUntil : claimExpiresAt;
    const subscriptionLastCheckedAt = auth.chatgpt_subscription_last_checked || '';
    // Without endpoint data an expired claim next to a paid plan proves only that the claim
    // is stale, so the card must not present that past date as the renewal date.
    const claimExpiresAtTime = Date.parse(claimExpiresAt);
    const expiresAtStale = !subscription && Boolean(usagePlanType) && !isFreePlan
        && Number.isFinite(claimExpiresAtTime) && claimExpiresAtTime <= now;

    const orgs = Array.isArray(auth.organizations) ? auth.organizations : [];
    let workspace = '工作空间缺失';
    if (orgs.length > 0) {
        const currentOrg = orgs.find(o => o && o.id === accountId);
        if (currentOrg) {
            const rawName = currentOrg.title ? currentOrg.title : currentOrg.name;
            if (typeof rawName === 'string' && rawName) workspace = rawName;
        }
    }

    const email = claims.email || '';
    const usageWindows = getUsageWindows(usage);
    const resetCredits = getResetCredits(usage);
    const authError = normalizeErrorState(custom.auth_error);
    const authStatus = typeof custom.auth_status === 'string' && custom.auth_status
        ? custom.auth_status
        : (authError ? 'error' : 'active');
    const authStatusMessage = typeof custom.auth_status_message === 'string'
        ? custom.auth_status_message
        : (authError && authError.message ? authError.message : '');
    const usageStatus = typeof custom.usage_status === 'string' && custom.usage_status
        ? custom.usage_status
        : (usage ? 'ok' : 'missing');
    const usageStatusMessage = typeof custom.usage_status_message === 'string'
        ? custom.usage_status_message
        : '';
    const usageNotice = getUsageNotice(custom, usage, usageWindows);

    return {
        email,
        planType,
        usage,
        usageWindows,
        resetCredits,
        subscription,
        expiresAt: isFreePlan ? '' : expiresAt,
        expiresAtStale,
        subscriptionLastCheckedAt,
        showExpiresAt: !isFreePlan,
        authStatus,
        authStatusMessage,
        usageStatus,
        usageStatusMessage,
        usageNotice,
        workspace
    };
}

export function getAccountName(account) {
    if (isApiModeAccount(account)) {
        const api = account.api && typeof account.api === 'object' ? account.api : {};
        return typeof api.name === 'string' && api.name ? api.name : 'API 模式';
    }

    const tokens = account && account.tokens ? account.tokens : {};
    const parsed = safeParseJwt(tokens.id_token);
    if (!parsed.claims) return '账号数据异常';
    const claims = parsed.claims;
    return claims.email || '';
}

export function maskAccountDisplayName(name) {
    const value = typeof name === 'string' ? name.trim() : '';
    if (!value) return '';

    const atIndex = value.indexOf('@');
    if (atIndex > 0) {
        const local = value.slice(0, atIndex);
        const domain = value.slice(atIndex);
        if (local.length <= 1) return `*${domain}`;
        if (local.length <= 4) return `${local.slice(0, 1)}***${domain}`;
        return `${local.slice(0, 4)}***${domain}`;
    }

    if (value.length <= 1) return '*';
    if (value.length <= 4) return `${value.slice(0, 1)}***`;
    if (value.length <= 8) return `${value.slice(0, 2)}***`;
    return `${value.slice(0, 3)}***${value.slice(-2)}`;
}
