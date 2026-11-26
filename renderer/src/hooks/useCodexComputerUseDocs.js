import { useCallback, useEffect, useRef, useState } from 'react';

const EMPTY_SUMMARY = {
  totalFiles: 0,
  pendingCount: 0,
  appliedCount: 0,
  restorableCount: 0
};

function normalizeItemSummary(raw) {
  if (!raw || typeof raw !== 'object') return { ...EMPTY_SUMMARY };
  return {
    totalFiles: Number(raw.totalFiles) || 0,
    pendingCount: Number(raw.pendingCount) || 0,
    appliedCount: Number(raw.appliedCount) || 0,
    restorableCount: Number(raw.restorableCount) || 0
  };
}

function normalizeStatusPayload(payload) {
  const source = payload && typeof payload === 'object' ? payload : {};
  const guidance = normalizeItemSummary(source.guidance);
  const confirmations = normalizeItemSummary(source.confirmations);
  const restorableCount = typeof source.restorableCount === 'number'
    ? source.restorableCount
    : guidance.restorableCount + confirmations.restorableCount;
  return {
    guidance,
    confirmations,
    restorableCount
  };
}

export function useCodexComputerUseDocs({ toast, toastError }) {
  const [status, setStatus] = useState(() => normalizeStatusPayload(null));
  const [loading, setLoading] = useState(true);
  const [clearing, setClearing] = useState(false);
  const [restoring, setRestoring] = useState(false);
  const [error, setError] = useState('');
  const mountedRef = useRef(true);
  const requestIdRef = useRef(0);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const refresh = useCallback(async ({ notifyError = false } = {}) => {
    const requestId = ++requestIdRef.current;
    if (!window.api || typeof window.api.inspectCodexComputerUseDocs !== 'function') {
      if (mountedRef.current && requestId === requestIdRef.current) {
        setLoading(false);
        setError('桌面接口不可用');
      }
      return null;
    }

    setLoading(true);
    setError('');
    try {
      const res = await window.api.inspectCodexComputerUseDocs();
      if (!mountedRef.current || requestId !== requestIdRef.current) return null;
      if (!res || res.ok !== true) {
        throw new Error((res && res.message) || '检测 Computer Use 增强状态失败');
      }
      const next = normalizeStatusPayload(res);
      setStatus(next);
      return next;
    } catch (err) {
      if (!mountedRef.current || requestId !== requestIdRef.current) return null;
      const message = err && err.message ? String(err.message) : '检测 Computer Use 增强状态失败';
      setError(message);
      if (notifyError && typeof toastError === 'function') {
        toastError(err, '检测 Computer Use 增强状态失败', 7000);
      }
      return null;
    } finally {
      if (mountedRef.current && requestId === requestIdRef.current) {
        setLoading(false);
      }
    }
  }, [toastError]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const clearRestrictions = useCallback(async () => {
    if (clearing || restoring || loading) return null;
    if (!window.api || typeof window.api.clearCodexComputerUseRestrictions !== 'function') {
      if (typeof toastError === 'function') {
        toastError(new Error('桌面接口不可用'), '清理 Computer Use 限制失败', 7000);
      }
      return null;
    }

    const requestId = ++requestIdRef.current;
    setClearing(true);
    setError('');
    try {
      const res = await window.api.clearCodexComputerUseRestrictions();
      if (!mountedRef.current) return null;
      if (!res || res.ok !== true) {
        throw new Error((res && res.message) || '清理 Computer Use 限制失败');
      }
      if (requestId === requestIdRef.current && res.status) {
        setStatus(normalizeStatusPayload(res.status));
      }
      if (res.message && typeof toast === 'function') {
        toast(res.message);
      }
      return res;
    } catch (err) {
      if (!mountedRef.current) return null;
      const message = err && err.message ? String(err.message) : '清理 Computer Use 限制失败';
      setError(message);
      if (typeof toastError === 'function') {
        toastError(err, '清理 Computer Use 限制失败', 7000);
      }
      return null;
    } finally {
      if (mountedRef.current) {
        setClearing(false);
      }
    }
  }, [clearing, loading, restoring, toast, toastError]);

  const restoreDocs = useCallback(async () => {
    if (clearing || restoring || loading) return null;
    if (!window.api || typeof window.api.restoreCodexComputerUseDocs !== 'function') {
      if (typeof toastError === 'function') {
        toastError(new Error('桌面接口不可用'), '还原 Computer Use 原文件失败', 7000);
      }
      return null;
    }

    const requestId = ++requestIdRef.current;
    setRestoring(true);
    setError('');
    try {
      const res = await window.api.restoreCodexComputerUseDocs();
      if (!mountedRef.current) return null;
      if (!res || res.ok !== true) {
        throw new Error((res && res.message) || '还原 Computer Use 原文件失败');
      }
      if (requestId === requestIdRef.current && res.status) {
        setStatus(normalizeStatusPayload(res.status));
      }
      if (res.message && typeof toast === 'function') {
        toast(res.message);
      }
      return res;
    } catch (err) {
      if (!mountedRef.current) return null;
      const message = err && err.message ? String(err.message) : '还原 Computer Use 原文件失败';
      setError(message);
      if (typeof toastError === 'function') {
        toastError(err, '还原 Computer Use 原文件失败', 7000);
      }
      return null;
    } finally {
      if (mountedRef.current) {
        setRestoring(false);
      }
    }
  }, [clearing, loading, restoring, toast, toastError]);

  return {
    status,
    loading,
    clearing,
    restoring,
    error,
    refresh,
    clearRestrictions,
    restoreDocs
  };
}
