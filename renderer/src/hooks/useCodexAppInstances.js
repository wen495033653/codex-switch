import { useState } from 'react';
import {
  getCodexAppInstanceKey,
  isSameCodexAppInstanceStatus,
  markCodexAppInstanceRunning,
  normalizeCodexAppInstanceStatus
} from '../utils/codexAppInstances';
import { createPollingErrorLog } from '../utils/pollingErrorLog';

const CODEX_APP_INSTANCE_REOPEN_ERRORS = [
  '独立 Codex 窗口未运行',
  '未找到独立 Codex 的可见窗口'
];

function shouldReopenCodexAppInstanceAfterShowError(err) {
  const message = typeof err === 'string'
    ? err
    : String((err && (err.message || err.error)) || '');
  return CODEX_APP_INSTANCE_REOPEN_ERRORS.some(text => message.includes(text));
}

// Which managed Codex instances are running, and opening (or focusing) one of them.
export function useCodexAppInstances({ toast, toastError }) {
  const [codexAppInstanceStatus, setCodexAppInstanceStatus] = useState(() => normalizeCodexAppInstanceStatus(null));
  const [openingCodexAppTarget, setOpeningCodexAppTarget] = useState('');
  const [statusErrorLog] = useState(() => createPollingErrorLog('get_codex_app_instance_status'));

  // Returning the previous object when nothing changed keeps a 3 s poll from re-rendering App.
  const applyInstanceStatus = (res) => {
    const next = normalizeCodexAppInstanceStatus(res);
    setCodexAppInstanceStatus(prev => (isSameCodexAppInstanceStatus(prev, next) ? prev : next));
  };

  const refreshCodexAppInstanceStatus = async ({ silent = true } = {}) => {
    if (!window.api || typeof window.api.getCodexAppInstanceStatus !== 'function') {
      applyInstanceStatus(null);
      return null;
    }
    try {
      const res = await window.api.getCodexAppInstanceStatus();
      statusErrorLog.succeeded();
      applyInstanceStatus(res);
      return res;
    } catch (err) {
      if (silent) statusErrorLog.failed(err);
      else toastError(err, '加载 Codex 多开状态失败', 7000);
      return null;
    }
  };

  const openCodexAppInstance = async (kind, id) => {
    const targetId = String(id || '').trim();
    if (!kind || !targetId || openingCodexAppTarget) return;
    const targetKey = `${kind}:${targetId}`;
    const instanceKey = getCodexAppInstanceKey(kind, targetId);
    const instanceRunning = Boolean(
      instanceKey && codexAppInstanceStatus.runningByKey[instanceKey]
    );
    setOpeningCodexAppTarget(targetKey);
    try {
      let usedOpenCommand = false;
      const openTargetInstance = async () => {
        usedOpenCommand = true;
        return window.api.openCodexAppInstance({ kind, id: targetId });
      };

      let res;
      if (instanceRunning && typeof window.api.showCodexAppInstance === 'function') {
        try {
          res = await window.api.showCodexAppInstance({ kind, id: targetId });
        } catch (err) {
          if (!shouldReopenCodexAppInstanceAfterShowError(err)) throw err;
          res = await openTargetInstance();
        }
      } else {
        res = await openTargetInstance();
      }
      if (usedOpenCommand) {
        setCodexAppInstanceStatus(prev => markCodexAppInstanceRunning(prev, res));
      }
      toast((res && res.message) || (usedOpenCommand ? '已打开 Codex' : '已打开 Codex 窗口'));
      window.setTimeout(() => refreshCodexAppInstanceStatus({ silent: true }), 1200);
    } catch (err) {
      toastError(err, '打开 Codex 失败', 7000);
    } finally {
      setOpeningCodexAppTarget(prev => (prev === targetKey ? '' : prev));
    }
  };

  return {
    codexAppInstanceStatus,
    openCodexAppInstance,
    openingCodexAppTarget,
    refreshCodexAppInstanceStatus
  };
}
