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
  const [launchingCodexWithProxy, setLaunchingCodexWithProxy] = useState(false);
  const [creatingCodexProxyDesktopShortcut, setCreatingCodexProxyDesktopShortcut] = useState(false);
  const [codexShortcutConfirm, setCodexShortcutConfirm] = useState({
    visible: false,
    proxyUrl: ''
  });
  const [savingProxySettings, setSavingProxySettings] = useState(false);

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
      const saveRes = await window.api.updateSettings(nextPatch);
      applySettings(saveRes);
    } catch (err) {
      setSettingsDraft(settings);
      toast(getErrorMessage(err, '更新 Codex app 代理设置失败'), 7000);
    } finally {
      setSavingProxySettings(false);
    }
  };

  const launchCodexWithProxy = async () => {
    if (launchingCodexWithProxy || savingProxySettings) return;
    setLaunchingCodexWithProxy(true);
    try {
      const proxyUrl = String(settingsDraft.codex_proxy_url || '').trim();
      const res = await window.api.launchCodexWithProxy(proxyUrl ? { proxyUrl } : {});
      toast((res && res.message) || '已启动 Codex');
    } catch (err) {
      toast(getErrorMessage(err, '启动 Codex 失败'), 7000);
    } finally {
      setLaunchingCodexWithProxy(false);
    }
  };

  const createCodexProxyDesktopShortcut = async () => {
    if (creatingCodexProxyDesktopShortcut || launchingCodexWithProxy || savingProxySettings) return;
    setCodexShortcutConfirm({
      visible: true,
      proxyUrl: String(settingsDraft.codex_proxy_url || '').trim()
    });
  };

  const cancelCodexProxyDesktopShortcut = () => {
    if (creatingCodexProxyDesktopShortcut) return;
    setCodexShortcutConfirm({ visible: false, proxyUrl: '' });
  };

  const confirmCodexProxyDesktopShortcut = async () => {
    if (creatingCodexProxyDesktopShortcut || launchingCodexWithProxy || savingProxySettings) return;
    setCreatingCodexProxyDesktopShortcut(true);
    try {
      const proxyUrl = String(codexShortcutConfirm.proxyUrl || '').trim();
      const res = await window.api.createCodexProxyDesktopShortcut({ proxyUrl });
      toast((res && res.message) || '已创建桌面图标');
      setCodexShortcutConfirm({ visible: false, proxyUrl: '' });
    } catch (err) {
      toast(getErrorMessage(err, '创建桌面图标失败'), 7000);
    } finally {
      setCreatingCodexProxyDesktopShortcut(false);
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
    cancelCodexProxyDesktopShortcut,
    codexShortcutConfirm,
    confirmCodexProxyDesktopShortcut,
    createCodexProxyDesktopShortcut,
    creatingCodexProxyDesktopShortcut,
    launchCodexWithProxy,
    launchingCodexWithProxy,
    openCodexConfigToml,
    openDataDir,
    openRepository,
    openSettingsPage,
    savingProxySettings,
    updateCodexProxySettings,
    updateSettingsDraftAndSave
  };
}
