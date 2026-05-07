import { useEffect } from 'react';

export function useAppBootstrap({
  applyOauthUpdate,
  applySettings,
  applyUpdateStatus,
  requireStore,
  setAppVersion,
  setCodexState,
  setDataDir,
  setRefreshAllStatus,
  setStore,
  toastError
}) {
  useEffect(() => {
    if (!window.api || window.api.isTauriRuntime === false) {
      toastError(
        new Error('Tauri API 未加载，请使用 npm run dev 或桌面应用运行。'),
        '桌面接口不可用'
      );
      return undefined;
    }

    window.api.getStore()
      .then(res => setStore(requireStore(res)))
      .catch(error => toastError(error, '加载账号失败'));
    if (typeof window.api.getAppVersion === 'function') {
      window.api.getAppVersion().then(res => {
        if (res && res.ok === true && res.version) setAppVersion(res.version);
      }).catch(error => toastError(error, '读取版本失败'));
    }
    if (typeof window.api.getDataDir === 'function') {
      window.api.getDataDir().then(res => {
        if (res && res.ok === true && res.path) setDataDir(res.path);
      }).catch(error => toastError(error, '读取数据目录失败'));
    }
    window.api.getRefreshAllStatus().then(res => {
      if (res && res.ok === true && res.status) setRefreshAllStatus(res.status);
    }).catch(error => toastError(error, '读取刷新状态失败'));
    window.api.getSettings().then(applySettings).catch(error => toastError(error, '加载设置失败'));

    const off = window.api.onOauthUpdate(applyOauthUpdate);
    const offStore = window.api.onStoreUpdated(data => {
      if (data && data.store) setStore(data.store);
      if (data && data.codex_state) setCodexState(data.codex_state);
    });
    const offRefreshAllStatus = window.api.onRefreshAllStatus(data => {
      if (data && data.status) setRefreshAllStatus(data.status);
    });
    const offUpdateStatus = typeof window.api.onUpdateStatus === 'function'
      ? window.api.onUpdateStatus(applyUpdateStatus)
      : (() => {});

    return () => {
      off();
      offStore();
      offRefreshAllStatus();
      offUpdateStatus();
    };
  }, []);
}
