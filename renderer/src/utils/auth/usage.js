function isUsageWindowAvailable(window) {
    if (!window || typeof window !== 'object') return false;
    const limitWindowSeconds = Number(window.limit_window_seconds);
    const resetAt = Number(window.reset_at);
    return Number.isFinite(limitWindowSeconds) && limitWindowSeconds > 0
        && Number.isFinite(resetAt) && resetAt > 0;
}

export function getUsageWindows(usage) {
    if (!usage || !usage.rate_limit) return [];

    return ['primary_window', 'secondary_window']
        .map(key => usage.rate_limit[key])
        .filter(isUsageWindowAvailable)
        .sort((a, b) => Number(a.limit_window_seconds) - Number(b.limit_window_seconds));
}

export function normalizeErrorState(raw) {
    if (!raw || typeof raw !== 'object') return null;

    const message = sanitizeUsageText(typeof raw.message === 'string' ? raw.message : '');
    const code = typeof raw.code === 'string' ? raw.code : '';
    const rawMessage = sanitizeUsageText(typeof raw.raw_message === 'string' ? raw.raw_message : '');
    const path = typeof raw.path === 'string' ? raw.path : '';
    const time = typeof raw.time === 'string' ? raw.time : '';
    const status = Number(raw.status);
    if (!message && !code && !rawMessage && !path && !time && !Number.isFinite(status)) return null;

    return {
        message,
        code,
        rawMessage,
        path,
        time,
        status: Number.isFinite(status) ? status : 0
    };
}

function sanitizeUsageText(value) {
    const text = typeof value === 'string' ? value.trim() : '';
    if (!text) return '';

    return text
        .replace(/^配额请求失败[:：]\s*/, '')
        .replace(/^配额刷新失败[:：]\s*/, '')
        .replace(/^配额同步失败[:：]\s*/, '')
        .replace(/^订阅已刷新，但配额刷新失败\s*/, '')
        .trim();
}

function getErrorDetail(error) {
    if (!error) return '';
    return error.message || error.rawMessage || '';
}

export function getUsageNotice(custom, usage, usageWindows) {
    const usageError = normalizeErrorState(custom.usage_error);
    const usageStatus = typeof custom.usage_status === 'string' && custom.usage_status
        ? custom.usage_status
        : (usageError ? 'error' : (usage ? 'ok' : 'missing'));
    const usageStatusMessage = sanitizeUsageText(typeof custom.usage_status_message === 'string'
        ? custom.usage_status_message
        : '');

    if (usageStatus === 'error' || usageError) {
        const message = usageStatusMessage || (usageError && usageError.message) || 'Usage state is abnormal';
        return {
            tone: 'error',
            message,
            detail: getErrorDetail(usageError) || message
        };
    }

    if (usageStatus === 'syncing') {
        const message = usageStatusMessage || 'Usage syncing, please wait...';
        return {
            tone: 'info',
            message,
            detail: message
        };
    }

    if (usageStatus === 'missing' || !usage || usageWindows.length === 0) {
        const message = usageStatusMessage || 'Usage data missing, please refresh';
        return {
            tone: 'info',
            message,
            detail: message
        };
    }

    return null;
}
