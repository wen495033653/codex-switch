import { useState } from 'react';
import { API_MODE_ACCOUNT_ID } from '../utils/auth';
import {
  buildApiSettingsPayload,
  getActiveApiProfile,
  normalizeApiProfiles
} from '../utils/appState';
import {
  DEFAULT_API_TEST_MODEL,
  getApiPrecheckFailureMessage,
  hasFreshSuccessfulApiPrecheck,
  mergeApiTestResult,
  normalizeApiTestResults,
  runApiProfilePrecheck
} from '../utils/apiPrecheck';

export function useModeSwitching({
  activeApiProfileId,
  apiDraft,
  apiProfiles,
  apiTestResults,
  applySettings,
  clearApiAutoSaveTimer,
  handleRes,
  onSaveApiTestResults,
  onUsageStatsRefresh,
  showIdeReopen,
  toastError
}) {
  const [savingApiMode, setSavingApiMode] = useState(false);
  const [switching, setSwitching] = useState(false);

  const switchToApiModeFromPage = async (profileId = activeApiProfileId) => {
    if (switching || savingApiMode) return;
    setSavingApiMode(true);
    setSwitching(true);
    clearApiAutoSaveTimer();
    try {
      const profiles = normalizeApiProfiles(apiProfiles, apiDraft);
      const activeProfile = profiles.find(profile => profile.id === profileId)
        || getActiveApiProfile({
          active_api_profile_id: activeApiProfileId,
          api_profiles: profiles,
          api_mode: apiDraft
        });
      const currentApiTestResults = normalizeApiTestResults(apiTestResults);
      const existingTest = currentApiTestResults[activeProfile.id] || null;
      let nextApiTestResults = currentApiTestResults;
      if (!hasFreshSuccessfulApiPrecheck(activeProfile, existingTest, DEFAULT_API_TEST_MODEL)) {
        const precheckResult = await runApiProfilePrecheck({
          profile: activeProfile,
          profileName: activeProfile.name,
          model: DEFAULT_API_TEST_MODEL,
          previousTest: existingTest
        });
        nextApiTestResults = mergeApiTestResult(currentApiTestResults, activeProfile.id, precheckResult);
        if (!precheckResult.ok) {
          if (typeof onSaveApiTestResults === 'function') {
            await onSaveApiTestResults(nextApiTestResults);
          }
          throw new Error(getApiPrecheckFailureMessage(precheckResult));
        }
      }

      const settingsPatch = buildApiSettingsPayload({
        activeId: activeProfile.id,
        activeProfile,
        profiles
      });
      settingsPatch.api_test_results = nextApiTestResults;
      const saveRes = await window.api.updateSettings(settingsPatch);
      applySettings(saveRes);

      const res = await window.api.switchApiMode(activeProfile.id);
      handleRes(res);
      if (typeof onUsageStatsRefresh === 'function') onUsageStatsRefresh();
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
    if (accountId === API_MODE_ACCOUNT_ID) {
      await switchToApiModeFromPage(activeApiProfileId);
      return;
    }
    setSwitching(true);
    try {
      const res = await window.api.switchAccount(accountId);
      handleRes(res);
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
