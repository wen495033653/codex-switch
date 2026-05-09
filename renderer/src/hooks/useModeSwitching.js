import { useState } from 'react';
import { API_MODE_ACCOUNT_ID } from '../utils/auth';
import { buildApiProfilePayload, buildApiSettingsPayload } from '../utils/appState';

export function useModeSwitching({
  apiDraft,
  activeApiProfileId,
  apiProfiles,
  applySettings,
  clearApiAutoSaveTimer,
  handleRes,
  showIdeReopen,
  toastError
}) {
  const [savingApiMode, setSavingApiMode] = useState(false);
  const [switching, setSwitching] = useState(false);

  const switchToApiModeFromPage = async (profileId) => {
    if (switching || savingApiMode) return;
    setSavingApiMode(true);
    setSwitching(true);
    clearApiAutoSaveTimer();
    try {
      const requestedProfileId = typeof profileId === 'string' && profileId
        ? profileId
        : activeApiProfileId;
      const profileFromList = Array.isArray(apiProfiles)
        ? apiProfiles.find(profile => profile && profile.id === requestedProfileId)
        : null;
      const activeProfile = buildApiProfilePayload(
        requestedProfileId === activeApiProfileId ? apiDraft : (profileFromList || apiDraft),
        requestedProfileId
      );
      const saveRes = await window.api.updateSettings({
        ...buildApiSettingsPayload({
          activeId: activeProfile.id,
          activeProfile,
          profiles: apiProfiles
        })
      });
      applySettings(saveRes);

      const res = await window.api.switchApiMode(activeProfile.id);
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
        ? await window.api.switchApiMode(activeApiProfileId)
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
