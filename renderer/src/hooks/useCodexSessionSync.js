import { useState } from 'react';

export function useCodexSessionSync({
  applySettings,
  settings,
  setSettingsDraft,
  toastError
}) {
  const [savingCodexSessionSync, setSavingCodexSessionSync] = useState(false);
  const codexSessionSyncEnabled = settings.codex_session_sync_enabled !== false;

  const updateCodexSessionSyncEnabled = async (enabled) => {
    if (savingCodexSessionSync) return;
    setSavingCodexSessionSync(true);
    const patch = { codex_session_sync_enabled: enabled };
    setSettingsDraft(prev => ({ ...prev, ...patch }));
    try {
      const res = await window.api.updateSettings(patch);
      applySettings(res);
    } catch (err) {
      setSettingsDraft(settings);
      toastError(err, '会话同步设置保存失败', 7000);
      setSavingCodexSessionSync(false);
      return;
    }

    if (enabled) {
      try {
        const res = await window.api.syncCodexSessions();
        if (res && res.ok !== true) {
          throw new Error(res.message || '订阅/API 会话同步失败');
        }
      } catch (err) {
        toastError(err, '会话同步已开启，但当前同步失败', 7000);
      }
    }

    setSavingCodexSessionSync(false);
  };

  return {
    codexSessionSyncEnabled,
    savingCodexSessionSync,
    updateCodexSessionSyncEnabled
  };
}
