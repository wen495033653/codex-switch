import { useState } from 'react';
import { API_MODE_ACCOUNT_ID } from '../utils/auth';

export function useModeSwitching({
  activeApiProfileId,
  applySettings,
  handleRes,
  onUsageStatsRefresh,
  showIdeReopen,
  toastError
}) {
  const [savingApiMode, setSavingApiMode] = useState(false);
  const [switching, setSwitching] = useState(false);

  const handleModeSwitchResponse = (res) => {
    if (res && res.settings && typeof res.settings === 'object') {
      applySettings(res);
    }
    return handleRes(res);
  };

  const switchToApiModeFromPage = async (profileId = activeApiProfileId) => {
    if (switching || savingApiMode) return;
    setSavingApiMode(true);
    setSwitching(true);
    try {
      const res = await window.api.switchApiMode(profileId);
      handleModeSwitchResponse(res);
      if (typeof onUsageStatsRefresh === 'function') onUsageStatsRefresh();
      showIdeReopen(res && res.ide_reopen ? res.ide_reopen : null);
      if (res?.api_switch_runtime_error) {
        toastError(new Error(res.api_switch_runtime_error), 'API 配置和账单归属已切换，但远程控制同步失败');
      }
    } catch (err) {
      toastError(err, '切换 API 模式失败');
    } finally {
      setSavingApiMode(false);
      setSwitching(false);
    }
  };

  const handleSwitchAccount = async (accountId) => {
    if (switching) return;
    if (accountId === API_MODE_ACCOUNT_ID) {
      await switchToApiModeFromPage(activeApiProfileId);
      return;
    }
    setSwitching(true);
    try {
      const res = await window.api.switchAccount(accountId);
      handleModeSwitchResponse(res);
      if (typeof onUsageStatsRefresh === 'function') onUsageStatsRefresh();
      showIdeReopen(res && res.ide_reopen ? res.ide_reopen : null);
    } catch (err) {
      toastError(err, '切换账号失败');
    } finally {
      setSwitching(false);
    }
  };

  return {
    handleSwitchAccount,
    savingApiMode,
    switching,
    switchToApiModeFromPage
  };
}
