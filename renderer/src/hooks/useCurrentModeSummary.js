import { useMemo } from 'react';
import { API_MODE_ACCOUNT_ID, getAccountId, getAccountName, maskAccountDisplayName, parseAuthInfo } from '../utils/auth';
import { getApiProviderDisplayName } from '../utils/appState';

export function useCurrentModeSummary({
  apiDraft,
  codexState,
  settings,
  store
}) {
  return useMemo(() => {
    const maskAccountName = settings.mask_account_name === true;
    const apiModeActive = codexState.mode === 'api';
    const subscriptionModeActive = codexState.mode === 'chatgpt';
    const activeSubscriptionAccount = store.accounts.find(account => getAccountId(account) === store.active_id) || null;
    const activeSubscriptionName = activeSubscriptionAccount ? getAccountName(activeSubscriptionAccount) : '';
    const activeSubscriptionDisplayName = activeSubscriptionName
      ? (maskAccountName ? maskAccountDisplayName(activeSubscriptionName) : activeSubscriptionName)
      : '未选择订阅账号';
    const activeSubscriptionPlan = activeSubscriptionAccount
      ? parseAuthInfo(activeSubscriptionAccount).planType.toUpperCase()
      : '';
    const apiProviderDisplayName = codexState.provider_name
      || getApiProviderDisplayName(apiDraft)
      || 'API Provider';
    const currentModeLabel = apiModeActive ? 'API 模式' : subscriptionModeActive ? '订阅模式' : '未识别';
    const currentModeDetail = apiModeActive
      ? apiProviderDisplayName
      : subscriptionModeActive
        ? `${activeSubscriptionDisplayName}${activeSubscriptionPlan ? ` · ${activeSubscriptionPlan}` : ''}`
        : '未检测到 Codex 登录状态';

    return {
      apiModeActive,
      currentAccountId: apiModeActive ? API_MODE_ACCOUNT_ID : store.active_id,
      currentModeDetail,
      currentModeLabel,
      maskAccountName,
      subscriptionModeActive
    };
  }, [apiDraft, codexState, settings.mask_account_name, store.accounts, store.active_id]);
}
