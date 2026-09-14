function optionalBoolean(value) {
    return typeof value === 'boolean' ? value : null;
}

/**
 * Subscription snapshot written by the backend from `/backend-api/subscriptions`.
 * Absent until that endpoint has been read once for the account, in which case the
 * caller falls back to the id_token claim.
 */
export function getSubscription(custom) {
    const raw = custom && custom.subscription;
    if (!raw || typeof raw !== 'object') return null;
    const activeUntil = typeof raw.active_until === 'string' ? raw.active_until.trim() : '';
    if (!activeUntil) return null;

    return {
        activeUntil,
        planType: typeof raw.plan_type === 'string' ? raw.plan_type.trim() : '',
        willRenew: optionalBoolean(raw.will_renew),
        isDelinquent: optionalBoolean(raw.is_delinquent),
        fetchedAt: typeof raw.fetched_at === 'string' ? raw.fetched_at.trim() : ''
    };
}
