import { useI18n } from '../i18n';

function expireTitle(subscription, t) {
    if (subscription && subscription.willRenew === true) return t('订阅到期日期，到期后自动续费');
    if (subscription && subscription.willRenew === false) return t('订阅到期日期，到期后不再续费');
    return t('订阅到期日期');
}

/**
 * Subscription facts on an account card: renewal date (or the unsynced marker when only a
 * stale id_token claim is available), past-due state, and available usage resets.
 */
export default function SubscriptionBadges({ info, planLabel }) {
    const { language, t } = useI18n();
    const { subscription, resetCredits } = info;
    const expiresAtText = info.expiresAt ? new Date(info.expiresAt).toLocaleDateString(language) : '';

    return (
        <>
            {info.showExpiresAt && info.expiresAt && (
                info.expiresAtStale ? (
                    <span
                        className="expire-date expire-date-stale"
                        title={t('登录信息中的订阅到期时间 {date} 已过，但账号仍是 {plan}；OpenAI 最后核对于 {checked}，尚未更新到期信息', {
                            date: expiresAtText,
                            plan: planLabel,
                            checked: info.subscriptionLastCheckedAt
                                ? new Date(info.subscriptionLastCheckedAt).toLocaleString(language)
                                : t('未知')
                        })}
                    >
                        {t('到期时间未同步')}
                    </span>
                ) : (
                    <span className="expire-date" title={expireTitle(subscription, t)}>
                        {t('到期 {date}', { date: expiresAtText })}
                    </span>
                )
            )}
            {subscription && subscription.isDelinquent === true && (
                <span className="delinquent-badge" title={t('订阅扣款失败，请在 ChatGPT 更新支付方式')}>
                    {t('欠费')}
                </span>
            )}
            {resetCredits && resetCredits.availableCount > 0 && (
                <span className="reset-credits-badge" title={t('可用的 Codex 用量重置次数，请在 Codex 中使用')}>
                    {t('可重置 {count} 次', { count: resetCredits.availableCount })}
                </span>
            )}
        </>
    );
}
