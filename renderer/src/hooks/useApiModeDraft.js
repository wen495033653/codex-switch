import { useEffect, useRef, useState } from 'react';
import { buildApiModePayload, DEFAULT_SETTINGS } from '../utils/appState';

export function useApiModeDraft({
  applySettings,
  settings,
  toastError,
  viewMode
}) {
  const [apiDraft, setApiDraft] = useState(DEFAULT_SETTINGS.api_mode);
  const autoSaveTimerRef = useRef(null);
  const autoSaveSeqRef = useRef(0);

  const clearApiAutoSaveTimer = () => {
    autoSaveSeqRef.current += 1;
    if (!autoSaveTimerRef.current) return;
    clearTimeout(autoSaveTimerRef.current);
    autoSaveTimerRef.current = null;
  };

  const updateApiPageDraft = (patch) => {
    autoSaveSeqRef.current += 1;
    setApiDraft(prev => ({
      ...(prev || DEFAULT_SETTINGS.api_mode),
      ...patch
    }));
  };

  useEffect(() => {
    if (viewMode !== 'api') return undefined;

    const nextPayload = buildApiModePayload(apiDraft);
    const currentPayload = buildApiModePayload(settings.api_mode);
    if (JSON.stringify(nextPayload) === JSON.stringify(currentPayload)) return undefined;

    if (autoSaveTimerRef.current) clearTimeout(autoSaveTimerRef.current);
    const saveSeq = autoSaveSeqRef.current + 1;
    autoSaveSeqRef.current = saveSeq;
    const timerId = setTimeout(async () => {
      if (autoSaveTimerRef.current === timerId) {
        autoSaveTimerRef.current = null;
      }
      try {
        const res = await window.api.updateSettings({ api_mode: nextPayload });
        if (autoSaveSeqRef.current !== saveSeq) return;
        if (res && res.ok === true && res.settings) {
          applySettings(res);
        }
      } catch (err) {
        if (autoSaveSeqRef.current === saveSeq) {
          toastError(err, 'API 配置自动保存失败');
        }
      }
    }, 500);
    autoSaveTimerRef.current = timerId;

    return () => {
      if (autoSaveSeqRef.current === saveSeq) {
        autoSaveSeqRef.current += 1;
      }
      if (autoSaveTimerRef.current === timerId) {
        clearTimeout(timerId);
        autoSaveTimerRef.current = null;
      }
    };
  }, [apiDraft, settings.api_mode, viewMode]);

  useEffect(() => () => {
    if (autoSaveTimerRef.current) clearTimeout(autoSaveTimerRef.current);
    autoSaveSeqRef.current += 1;
  }, []);

  return {
    apiDraft,
    clearApiAutoSaveTimer,
    setApiDraft,
    updateApiPageDraft
  };
}
