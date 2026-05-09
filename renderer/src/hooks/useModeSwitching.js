import { useState } from 'react';
import { API_MODE_ACCOUNT_ID } from '../utils/auth';
import { buildApiModePayload } from '../utils/appState';

export function useModeSwitching({
  apiDraft,
  applySettings,
  clearApiAutoSaveTimer,
  handleRes,
  showIdeReopen,
  toastError
}) {
  const [savingApiMode, setSavingApiMode] = useState(false);
  const [switching, setSwitching] = useState(false);

  const switchToApiModeFromPage = async () => {
    if (switching || savingApiMode) return;
    setSavingApiMode(true);
    setSwitching(true);
    clearApiAutoSaveTimer();
    try {
      const saveRes = await window.api.updateSettings({
        api_mode: buildApiModePayload(apiDraft)
      });
      applySettings(saveRes);

      const res = await window.api.switchApiMode();
      handleRes(res);
      showIdeReopen(res && res.ide_reopen ? res.ide_reopen : null);
    } catch (err) {
      toastError(err, '切换 API 模式失败');
    } finally {
      setSavingApiMode(false);
      setSwitching(false);
    }
  };

  const handleSwitchAccount = async (accountId) => {
    if (switching) return;
    setSwitching(true);
    try {
      const res = accountId === API_MODE_ACCOUNT_ID
        ? await window.api.switchApiMode()
        : await window.api.switchAccount(accountId);
      handleRes(res);
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
