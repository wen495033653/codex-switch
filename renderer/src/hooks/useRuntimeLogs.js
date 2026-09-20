import { useCallback, useEffect, useRef, useState } from 'react';

export function useRuntimeLogs() {
  const [entries, setEntries] = useState([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [writeError, setWriteError] = useState('');
  const request = useRef(0);

  const refresh = useCallback(async () => {
    const id = ++request.current;
    setLoading(true);
    setError('');
    try {
      const result = await window.api.getRuntimeLogEntries();
      if (!result || !Array.isArray(result.entries)) throw new Error('日志返回格式错误');
      if (id !== request.current) return;
      setEntries(result.entries);
      setWriteError(result.writeError || '');
    } catch (failure) {
      if (id === request.current) setError(failure instanceof Error ? failure.message : String(failure));
    } finally {
      if (id === request.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    refresh();
    return () => { request.current += 1; };
  }, [refresh]);

  return { entries, loading, error, writeError, refresh };
}
