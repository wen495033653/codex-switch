import { useEffect, useRef } from 'react';

export function useAsyncPolling(callback, {
  enabled = true,
  intervalMs,
  pauseWhenHidden = true,
  refreshOnFocus = false,
  refreshKey = null,
  refreshOnVisible = true,
  runImmediately = true
} = {}) {
  const callbackRef = useRef(callback);
  const generationRef = useRef(0);
  const inFlightRef = useRef(false);

  useEffect(() => {
    callbackRef.current = callback;
  }, [callback]);

  useEffect(() => {
    const generation = generationRef.current + 1;
    generationRef.current = generation;
    let disposed = false;

    const isCurrent = () => !disposed && generationRef.current === generation;
    const run = async () => {
      if (!enabled || inFlightRef.current) return;
      if (pauseWhenHidden && document.visibilityState === 'hidden') return;

      inFlightRef.current = true;
      try {
        await callbackRef.current({ isCurrent });
      } finally {
        inFlightRef.current = false;
      }
    };

    if (runImmediately) void run();

    const timer = enabled && Number(intervalMs) > 0
      ? window.setInterval(() => void run(), Number(intervalMs))
      : null;
    const handleFocus = () => void run();
    const handleVisibilityChange = () => {
      if (document.visibilityState === 'visible') void run();
    };

    if (enabled && refreshOnFocus) window.addEventListener('focus', handleFocus);
    if (enabled && refreshOnVisible) {
      document.addEventListener('visibilitychange', handleVisibilityChange);
    }

    return () => {
      disposed = true;
      generationRef.current += 1;
      if (timer !== null) window.clearInterval(timer);
      window.removeEventListener('focus', handleFocus);
      document.removeEventListener('visibilitychange', handleVisibilityChange);
    };
  }, [enabled, intervalMs, pauseWhenHidden, refreshKey, refreshOnFocus, refreshOnVisible, runImmediately]);
}
