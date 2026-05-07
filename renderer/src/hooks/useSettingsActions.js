import { useState } from 'react';
import { REPOSITORY_URL } from '../utils/appState';
import { getErrorMessage } from '../utils/errors';

export function useSettingsActions({
  applySettings,
  settings,
  settingsDraft,
  setSettingsDraft,
  setSettingsTab,
  setViewMode,
  toast,
  toastError
}) {
  const [savingProxySettings, setSavingProxySettings] = useState(false);
  const [savingCodexProxyEnv, setSavingCodexProxyEnv] = useState(false);

  const openSettingsPage = async () => {
    try {
      const res = await window.api.getSettings();
      applySettings(res);
    } catch (_err) {
      setSettingsDraft(settings);
    }
    setSettingsTab('general');
    setViewMode('settings');
  };

  const updateSettingsDraftAndSave = async (patch) => {
    setSettingsDraft(prev => ({ ...prev, ...patch }));
    try {
      const res = await window.api.updateSettings(patch);
      applySettings(res);
    } catch (err) {
      setSettingsDraft(settings);
      toastError(err, '设置保存失败');
    }
  };

  const updateCodexProxySettings = async (patch) => {
    if (savingProxySettings) return;
    const nextPatch = patch && typeof patch === 'object' ? { ...patch } : {};
    const hasProxyUrl = Object.prototype.hasOwnProperty.call(nextPatch, 'codex_proxy_url');
    if (!hasProxyUrl) return;

    setSavingProxySettings(true);
    try {
      nextPatch.codex_proxy_url = String(nextPatch.codex_proxy_url || '').trim();
      const saveRes = settingsDraft.codex_proxy_env_enabled === true
        ? await window.api.setCodexProxyEnvEnabled({
          enabled: true,
          proxyUrl: nextPatch.codex_proxy_url
        })
        : await window.api.updateSettings(nextPatch);
      applySettings(saveRes);
    } catch (err) {
      setSettingsDraft(settings);
      toast(getErrorMessage(err, '更新 Codex app 代理设置失败'), 7000);
    } finally {
      setSavingProxySettings(false);
    }
  };

  const setCodexProxyEnvEnabled = async (enabled) => {
    if (savingCodexProxyEnv || savingProxySettings) return;
    const proxyUrl = String(settingsDraft.codex_proxy_url || '').trim();
    if (enabled && !proxyUrl) {
      toast('代理地址不能为空', 7000);
      return;
    }

    setSavingCodexProxyEnv(true);
    try {
      const res = await window.api.setCodexProxyEnvEnabled({ enabled, proxyUrl });
      applySettings(res);
      toast((res && res.message) || (enabled ? 'Codex app 代理已写入 .env' : 'Codex app 代理已从 .env 移除'));
    } catch (err) {
      setSettingsDraft(settings);
      toast(getErrorMessage(err, enabled ? '写入 Codex app 代理失败' : '移除 Codex app 代理失败'), 7000);
    } finally {
      setSavingCodexProxyEnv(false);
    }
  };

  const openCodexConfigToml = async () => {
    try {
      await window.api.openCodexConfigToml();
    } catch (err) {
      toastError(err, '打开 config.toml 失败', 7000);
    }
  };

  const openDataDir = async () => {
    try {
      await window.api.openDataDir();
    } catch (err) {
      toastError(err, '打开数据目录失败', 7000);
    }
  };

  const openRepository = async () => {
    try {
      await window.api.openExternalUrl(REPOSITORY_URL);
    } catch (err) {
      toastError(err, '打开开源地址失败');
    }
  };

  return {
    openCodexConfigToml,
    openDataDir,
    openRepository,
    openSettingsPage,
    savingCodexProxyEnv,
    savingProxySettings,
    setCodexProxyEnvEnabled,
    updateCodexProxySettings,
    updateSettingsDraftAndSave
  };
}
