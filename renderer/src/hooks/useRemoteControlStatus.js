import { useEffect, useRef, useState } from 'react';
import { useAsyncPolling } from './useAsyncPolling';

export const EMPTY_REMOTE_CONTROL_STATUS = Object.freeze({
  loading: false,
  error: '',
  backendError: null,
  helperStatus: null,
  backendEnvironment: null,
  connectionStatus: null
});

// Remote control connection status, polled every 4 s while remote control is enabled.
// A result that carries settings goes to `onAutoDisabled`; an automatic disable is passed
// on once per enable. Texts are kept as the backend sent them; the view translates them.
export function useRemoteControlStatus({ enabled, accountId, onAutoDisabled }) {
  const [remoteControlStatus, setRemoteControlStatus] = useState(EMPTY_REMOTE_CONTROL_STATUS);
  const autoDisableNotifiedRef = useRef(false);
  const onAutoDisabledRef = useRef(onAutoDisabled);

  useEffect(() => {
    onAutoDisabledRef.current = onAutoDisabled;
  }, [onAutoDisabled]);
  useEffect(() => {
    if (enabled) {
      autoDisableNotifiedRef.current = false;
    }
  }, [enabled, accountId]);
  useEffect(() => {
    if (!enabled) {
      setRemoteControlStatus(EMPTY_REMOTE_CONTROL_STATUS);
    }
  }, [enabled, accountId]);

  useAsyncPolling(async ({ isCurrent }) => {
    if (!window.api || !window.api.getCodexRemoteControlStatus) return;

    if (isCurrent()) setRemoteControlStatus(prev => ({ ...prev, loading: true, error: '' }));
    try {
      const result = await window.api.getCodexRemoteControlStatus();
      if (!isCurrent()) return;
      if (result && result.settings && typeof onAutoDisabledRef.current === 'function') {
        const autoDisabled = result.autoDisabled === true;
        if (!autoDisabled || !autoDisableNotifiedRef.current) {
          if (autoDisabled) autoDisableNotifiedRef.current = true;
          onAutoDisabledRef.current(result);
        }
      }
      setRemoteControlStatus({
        loading: false,
        error: '',
        backendError: result && result.backendError ? result.backendError : null,
        helperStatus: result && result.helperStatus ? result.helperStatus : null,
        backendEnvironment: result && result.backendEnvironment ? result.backendEnvironment : null,
        connectionStatus: result && result.connectionStatus ? result.connectionStatus : null
      });
    } catch (err) {
      if (isCurrent()) {
        setRemoteControlStatus({
          ...EMPTY_REMOTE_CONTROL_STATUS,
          error: err && err.message ? err.message : '读取远程控制状态失败'
        });
      }
    }
  }, {
    enabled,
    intervalMs: 4000,
    refreshKey: accountId
  });

  return remoteControlStatus;
}
