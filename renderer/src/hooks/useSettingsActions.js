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
  subscriptionModeActive,
  toast,
  toastError
}) {
  const [savingProxySettings, setSavingProxySettings] = useState(false);
  const [savingCodexProxyEnv, setSavingCodexProxyEnv] = useState(false);
  const [savingCodexRemoteControl, setSavingCodexRemoteControl] = useState(false);
  const [codexRemoteControlPendingEnabled, setCodexRemoteControlPendingEnabled] = useState(null);
  const [pluginRestartNoticeVisible, setPluginRestartNoticeVisible] = useState(false);
  const [pluginRestartNoticeMessage, setPluginRestartNoticeMessage] = useState(
    'Codex app 增强设置已保存，重启 Codex app 后生效。'
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
    const deleteButtonEnabledBeforeSave = settingsDraft.codex_delete_button_enabled === true;
    const enablesPlugin = Object.prototype.hasOwnProperty.call(patch, 'codex_plugins_enabled')
      && pluginEnabledBeforeSave === false
      && patch.codex_plugins_enabled === true;
    const enablesDeleteButton = Object.prototype.hasOwnProperty.call(patch, 'codex_delete_button_enabled')
      && deleteButtonEnabledBeforeSave === false
      && patch.codex_delete_button_enabled === true;
    const shouldCheckPluginRestartNotice = enablesPlugin || enablesDeleteButton;
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
        setPluginRestartNoticeMessage('Codex app 增强设置已保存，重启 Codex app 后生效。');
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

  const setCodexRemoteControlEnabled = async (enabled) => {
    if (savingCodexRemoteControl || savingProxySettings) return;
    if (enabled && subscriptionModeActive) {
      toast('订阅模式下不可开启远程控制，请先切换到 API 模式', 7000);
      return;
    }
    if (enabled && !String(settingsDraft.codex_remote_control_account_id || '').trim()) {
      toast('请先选择远程控制账号', 7000);
      return;
    }
    setSettingsDraft(prev => ({ ...prev, codex_remote_control_enabled: enabled }));

    setSavingCodexRemoteControl(true);
    setCodexRemoteControlPendingEnabled(enabled);
    try {
      const res = await window.api.setCodexRemoteControlEnabled({ enabled });
      applySettings(res);
      toast((res && res.message) || (enabled ? '远程控制已启用' : '远程控制已关闭'));
      if (res && res.restartRequired) {
        setPluginRestartNoticeMessage('远程控制配置已保存，重启 Codex app 后生效。');
        setPluginRestartNoticeVisible(true);
      }
    } catch (err) {
      setSettingsDraft(settings);
      toast(getErrorMessage(err, enabled ? '启用远程控制失败' : '关闭远程控制失败'), 7000);
    } finally {
      setCodexRemoteControlPendingEnabled(null);
      setSavingCodexRemoteControl(false);
    }
  };

  const setCodexRemoteControlAccountId = async (accountId) => {
    if (savingCodexRemoteControl || savingProxySettings) return;
    const nextAccountId = String(accountId || '').trim();
    if (!nextAccountId) {
      toast('远程控制账号不能为空', 7000);
      return;
    }

    setSettingsDraft(prev => ({ ...prev, codex_remote_control_account_id: nextAccountId }));
    setSavingCodexRemoteControl(true);
    try {
      const res = await window.api.setCodexRemoteControlAccountId(nextAccountId);
      applySettings(res);
      toast((res && res.message) || '远程控制账号已更新');
      if (res && res.restartRequired) {
        setPluginRestartNoticeMessage('远程控制账号已更新，重启 Codex app 后生效。');
        setPluginRestartNoticeVisible(true);
      }
    } catch (err) {
      setSettingsDraft(settings);
      toast(getErrorMessage(err, '更新远程控制账号失败'), 7000);
    } finally {
      setSavingCodexRemoteControl(false);
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
    codexRemoteControlPendingEnabled,
    savingCodexProxyEnv,
    savingCodexRemoteControl,
    savingProxySettings,
    setCodexProxyEnvEnabled,
    setCodexRemoteControlAccountId,
    setCodexRemoteControlEnabled,
    updateCodexProxySettings,
    updateSettingsDraftAndSave
  };
}
