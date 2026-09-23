import { useState } from 'react';
import { useAsyncPolling } from './useAsyncPolling';

function normalizePids(value) {
  if (!Array.isArray(value)) return [];
  return value
    .map(pid => Number(pid))
    .filter(pid => Number.isInteger(pid) && pid > 0);
}

// The running Codex desktop processes, polled every 3 s while the Codex page is open. Texts
// are kept as the backend sent them; the view translates them.
export function useCodexProcessStatus() {
  const [codexAppProcessStatus, setCodexAppProcessStatus] = useState({
    loading: true,
    error: '',
    pids: [],
    processCount: 0,
    supported: true,
    requiresUpdate: false,
    compatibilityMessage: ''
  });

  useAsyncPolling(async ({ isCurrent }) => {
    if (!window.api || !window.api.getCurrentCodexAppProcesses) {
      if (isCurrent()) {
        setCodexAppProcessStatus({
          loading: false,
          error: '',
          pids: [],
          processCount: 0,
          supported: false,
          requiresUpdate: true,
          compatibilityMessage: '无法检测 ChatGPT Desktop 版本'
        });
      }
      return;
    }

    try {
      const result = await window.api.getCurrentCodexAppProcesses();
      if (isCurrent()) {
        setCodexAppProcessStatus({
          loading: false,
          error: result && result.error ? String(result.error) : '',
          pids: normalizePids(result && result.pids),
          processCount: Number(result && result.processCount) || 0,
          supported: result && result.supported !== false,
          requiresUpdate: result && result.requiresUpdate === true,
          compatibilityMessage: result && result.compatibilityMessage
            ? String(result.compatibilityMessage)
            : ''
        });
      }
    } catch (err) {
      if (isCurrent()) {
        setCodexAppProcessStatus({
          loading: false,
          error: err && err.message ? err.message : '读取失败',
          pids: [],
          processCount: 0,
          supported: false,
          requiresUpdate: false,
          compatibilityMessage: ''
        });
      }
    }
  }, { intervalMs: 3000 });

  return codexAppProcessStatus;
}
