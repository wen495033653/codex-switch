import { useState } from 'react';
import { REPOSITORY_URL } from '../utils/appState';
import { getErrorMessage } from '../utils/errors';

function hasRunningCodexApp(processStatus) {
  const pids = Array.isArray(processStatus && processStatus.pids) ? processStatus.pids : [];
  return pids.some(pid => {
    const value = Number(pid);
    return Number.isInteger(value) && value > 0;
  });
}

export function useSettingsActions({
  applySettings,
  settings,
  settingsDraft,
  setSettingsDraft,
  setViewMode,
  toast,
  toastError
}) {
  const [savingProxySettings, setSavingProxySettings] = useState(false);
  const [savingCodexProxyEnv, setSavingCodexProxyEnv] = useState(false);
  const [savingCodexRemoteControlHook, setSavingCodexRemoteControlHook] = useState(false);
  const [pluginRestartNoticeVisible, setPluginRestartNoticeVisible] = useState(false);
  const [pluginRestartNoticeMessage, setPluginRestartNoticeMessage] = useState(
    'Plugin 解锁设置已保存，重启 Codex app 后生效。'
  );
  const [pluginRestartNoticeLoading, setPluginRestartNoticeLoading] = useState(false);
  const [restartingCodexApp, setRestartingCodexApp] = useState(false);

  const openSettingsPage = async () => {
    try {
      const res = await window.api.getSettings();
      applySettings(res);
    } catch (_err) {
      setSettingsDraft(settings);
    }
    setViewMode('settings');
  };

  const updateSettingsDraftAndSave = async (patch) => {
    const pluginEnabledBeforeSave = settingsDraft.codex_plugins_enabled === true;
    const shouldCheckPluginRestartNotice = Object.prototype.hasOwnProperty.call(patch, 'codex_plugins_enabled')
      && pluginEnabledBeforeSave === false
      && patch.codex_plugins_enabled === true;
    setSettingsDraft(prev => ({ ...prev, ...patch }));
    try {
      const res = await window.api.updateSettings(patch);
      applySettings(res);
      if (shouldCheckPluginRestartNotice) {
        let processStatus;
        try {
          processStatus = await window.api.getCurrentCodexAppProcesses();
        } catch (err) {
          toastError(err, '检测 Codex app 状态失败', 7000);
          return;
        }
        if (!hasRunningCodexApp(processStatus)) return;
        setPluginRestartNoticeMessage('Plugin 解锁设置已保存，重启 Codex app 后生效。');
        setPluginRestartNoticeVisible(true);
      }
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
      toast((res && res.message) || (enabled ? 'Codex app 代理已启用' : 'Codex app 代理已关闭'));
    } catch (err) {
      setSettingsDraft(settings);
      toast(getErrorMessage(err, enabled ? '启用 Codex app 代理失败' : '关闭 Codex app 代理失败'), 7000);
    } finally {
      setSavingCodexProxyEnv(false);
    }
  };

  const setCodexRemoteControlHookEnabled = async (enabled) => {
    if (savingCodexRemoteControlHook || savingProxySettings) return;
    setSettingsDraft(prev => ({ ...prev, codex_remote_control_hook_enabled: enabled }));

    setSavingCodexRemoteControlHook(true);
    try {
      const res = await window.api.setCodexRemoteControlHookEnabled({ enabled });
      applySettings(res);
      toast((res && res.message) || (enabled ? 'Remote Control Hook 已启用' : 'Remote Control Hook 已关闭'));
    } catch (err) {
      setSettingsDraft(settings);
      toast(getErrorMessage(err, enabled ? '启用 Remote Control Hook 失败' : '关闭 Remote Control Hook 失败'), 7000);
    } finally {
      setSavingCodexRemoteControlHook(false);
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

  const restartCodexAppForPluginSetting = async () => {
    if (pluginRestartNoticeLoading) return;
    setPluginRestartNoticeLoading(true);
    try {
      const res = await window.api.restartCurrentCodexAppForPluginSetting();
      setPluginRestartNoticeVisible(false);
      toast((res && res.message) || 'Codex app 已重启');
    } catch (err) {
      toastError(err, '重启 Codex app 失败', 7000);
    } finally {
      setPluginRestartNoticeLoading(false);
    }
  };

  const restartCurrentCodexAppNormal = async () => {
    if (restartingCodexApp) return;
    setRestartingCodexApp(true);
    try {
      const res = await window.api.restartCurrentCodexAppNormal();
      toast((res && res.message) || 'Codex app 已重启');
    } catch (err) {
      toastError(err, '重启 Codex app 失败', 7000);
    } finally {
      setRestartingCodexApp(false);
    }
  };

  return {
    openCodexConfigToml,
    openDataDir,
    openRepository,
    pluginRestartNotice: {
      visible: pluginRestartNoticeVisible,
      loading: pluginRestartNoticeLoading,
      message: pluginRestartNoticeMessage,
      onRestart: restartCodexAppForPluginSetting,
      onClose: () => !pluginRestartNoticeLoading && setPluginRestartNoticeVisible(false)
    },
    openSettingsPage,
    restartingCodexApp,
    restartCurrentCodexAppNormal,
    savingCodexProxyEnv,
    savingCodexRemoteControlHook,
    savingProxySettings,
    setCodexProxyEnvEnabled,
    setCodexRemoteControlHookEnabled,
    updateCodexProxySettings,
    updateSettingsDraftAndSave
  };
}
