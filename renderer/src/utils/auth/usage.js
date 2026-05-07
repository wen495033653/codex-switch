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

    const message = typeof raw.message === 'string' ? raw.message : '';
    const code = typeof raw.code === 'string' ? raw.code : '';
    const status = Number(raw.status);
    if (!message && !code && !Number.isFinite(status)) return null;

    return {
        message,
        code,
        status: Number.isFinite(status) ? status : 0
    };
}

export function getUsageNotice(custom, usage, usageWindows) {
    const usageError = normalizeErrorState(custom.usage_error);
    const usageStatus = typeof custom.usage_status === 'string' && custom.usage_status
        ? custom.usage_status
        : (usageError ? 'error' : (usage ? 'ok' : 'missing'));
    const usageStatusMessage = typeof custom.usage_status_message === 'string'
        ? custom.usage_status_message
        : '';

    if (usageStatus === 'error' || usageError) {
        return {
            tone: 'error',
            message: usageStatusMessage || (usageError && usageError.message) || '配额状态异常'
        };
    }

    if (usageStatus === 'syncing') {
        return {
            tone: 'info',
            message: usageStatusMessage || '配额同步中，请稍候...'
        };
    }

    if (usageStatus === 'missing' || !usage || usageWindows.length === 0) {
        return {
            tone: 'info',
            message: usageStatusMessage || '配额数据缺失，请刷新'
        };
    }

    return null;
}
