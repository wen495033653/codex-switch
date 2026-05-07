import { getAccountId, isApiModeAccount } from './account';
import { safeParseJwt } from './jwt';
import { getUsageNotice, getUsageWindows, normalizeErrorState } from './usage';

export function parseAuthInfo(account) {
    if (isApiModeAccount(account)) {
        const api = account.api && typeof account.api === 'object' ? account.api : {};
        const baseUrl = typeof api.base_url === 'string' ? api.base_url : '';
        const configured = api.configured === true;
        return {
            isApiMode: true,
            email: '',
            planType: 'API',
            usage: null,
            usageWindows: [],
            expiresAt: '',
            showExpiresAt: false,
            authStatus: configured ? 'active' : 'error',
            authStatusMessage: configured ? '' : '请先在设置中填写 API Key',
            usageStatus: 'ok',
            usageStatusMessage: '',
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
            email: '',
            planType: '',
            usage: null,
            usageWindows: [],
            expiresAt: '',
            showExpiresAt: false,
            authStatus: 'error',
            authStatusMessage: parsed.error,
            usageStatus: 'missing',
            usageStatusMessage: '',
            usageNotice: {
                tone: 'error',
                message: '账号数据异常，请重新导入或删除后添加'
            },
            workspace: '账号数据异常'
        };
    }

    const claims = parsed.claims;
    const auth = claims['https://api.openai.com/auth'] || {};

    const accountId = getAccountId(account);
    const rawPlanType = auth.chatgpt_plan_type || '';
    const planType = rawPlanType === 'unknown' ? '' : rawPlanType;
    const isFreePlan = planType.toLowerCase() === 'free';
    const expiresAt = auth.chatgpt_subscription_active_until || '';

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
    const custom = account && account.custom ? account.custom : {};
    const usage = custom.usage_info;
    const usageWindows = getUsageWindows(usage);
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
        expiresAt: isFreePlan ? '' : expiresAt,
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
