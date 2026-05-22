import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

const MAX_DEV_LOG_ENTRIES = 160;
const CONSOLE_METHODS = ['debug', 'log', 'info', 'warn', 'error'];

function formatDebugArg(value) {
  if (value instanceof Error) {
    return value.stack || value.message;
  }
  if (typeof value === 'string') return value;
  if (value === undefined) return 'undefined';
  if (value === null) return 'null';
  if (typeof value === 'number' || typeof value === 'boolean' || typeof value === 'bigint') {
    return String(value);
  }

  try {
    const seen = new WeakSet();
    return JSON.stringify(value, (_key, item) => {
      if (typeof item === 'object' && item !== null) {
        if (seen.has(item)) return '[Circular]';
        seen.add(item);
      }
      return item;
    }, 2);
  } catch (_err) {
    return String(value);
  }
}

function normalizeMessage(args) {
  return args.map(formatDebugArg).join(' ');
}

function consoleLevel(method) {
  if (method === 'error') return 'error';
  if (method === 'warn') return 'warn';
  if (method === 'debug') return 'debug';
  return 'info';
}

function normalizeDevLogLevel(level) {
  if (level === 'error' || level === 'warn' || level === 'info' || level === 'debug') {
    return level;
  }
  return 'debug';
}

function formatEntryTime(timestamp) {
  if (typeof timestamp === 'string' && timestamp.trim()) {
    const date = new Date(timestamp);
    if (!Number.isNaN(date.getTime())) {
      return date.toLocaleTimeString('zh-CN', { hour12: false });
    }
  }
  return new Date().toLocaleTimeString('zh-CN', { hour12: false });
}

function hasVisibleDetails(value) {
  if (value === undefined || value === null) return false;
  if (typeof value === 'object' && !Array.isArray(value)) {
    return Object.keys(value).length > 0;
  }
  return true;
}

function normalizeDevLogPayload(payload, fallbackId) {
  if (!payload || typeof payload !== 'object') {
    return {
      id: `backend-${fallbackId}`,
      level: 'debug',
      source: 'backend',
      time: formatEntryTime(),
      message: formatDebugArg(payload).slice(0, 4000)
    };
  }

  const envelope = payload.details && typeof payload.details === 'object'
    ? payload.details
    : {};
  const sequence = payload.sequence ?? fallbackId;
  const eventName = payload.message || envelope.event || 'dev-log';
  const eventDetails = Object.prototype.hasOwnProperty.call(envelope, 'details')
    ? envelope.details
    : undefined;
  const message = hasVisibleDetails(eventDetails)
    ? `${eventName}\n${formatDebugArg(eventDetails)}`
    : eventName;

  return {
    id: `backend-${sequence}`,
    level: normalizeDevLogLevel(payload.level),
    source: payload.source || 'backend',
    time: formatEntryTime(envelope.timestamp),
    message: message.slice(0, 4000)
  };
}

export function useDevDiagnostics({ enabled }) {
  const [entries, setEntries] = useState([]);
  const [isOpen, setIsOpen] = useState(enabled);
  const nextIdRef = useRef(1);

  const addEntry = useCallback((level, source, args) => {
    if (!enabled) return;

    const entry = {
      id: nextIdRef.current,
      level,
      source,
      time: new Date().toLocaleTimeString('zh-CN', { hour12: false }),
      message: normalizeMessage(args).slice(0, 4000)
    };
    nextIdRef.current += 1;
    setEntries(prev => [entry, ...prev].slice(0, MAX_DEV_LOG_ENTRIES));
  }, [enabled]);

  const addDevLogPayloads = useCallback((payloads) => {
    if (!enabled) return;

    const normalized = (Array.isArray(payloads) ? payloads : [payloads])
      .map(payload => {
        const fallbackId = nextIdRef.current;
        nextIdRef.current += 1;
        return normalizeDevLogPayload(payload, fallbackId);
      })
      .reverse();

    if (normalized.length === 0) return;

    setEntries(prev => {
      const existingIds = new Set(prev.map(entry => entry.id));
      const nextEntries = normalized.filter(entry => !existingIds.has(entry.id));
      if (nextEntries.length === 0) return prev;
      return [...nextEntries, ...prev].slice(0, MAX_DEV_LOG_ENTRIES);
    });
  }, [enabled]);

  useEffect(() => {
    if (!enabled || typeof window === 'undefined') return undefined;

    const originalConsole = {};
    CONSOLE_METHODS.forEach(method => {
      originalConsole[method] = console[method];
      console[method] = (...args) => {
        originalConsole[method].apply(console, args);
        addEntry(consoleLevel(method), `console.${method}`, args);
      };
    });

    const onWindowError = event => {
      addEntry('error', 'window.error', [event.message, event.error]);
    };
    const onUnhandledRejection = event => {
      addEntry('error', 'unhandledrejection', [event.reason]);
    };
    window.addEventListener('error', onWindowError);
    window.addEventListener('unhandledrejection', onUnhandledRejection);
    window.__codexSwitchDevLog = (level, message, details) => {
      addEntry(level || 'debug', 'window.__codexSwitchDevLog', [message, details]);
    };

    return () => {
      CONSOLE_METHODS.forEach(method => {
        console[method] = originalConsole[method];
      });
      window.removeEventListener('error', onWindowError);
      window.removeEventListener('unhandledrejection', onUnhandledRejection);
      delete window.__codexSwitchDevLog;
    };
  }, [addEntry, enabled]);

  useEffect(() => {
    if (!enabled || typeof window === 'undefined') return undefined;
    const api = window.api;
    if (!api || !api.isTauriRuntime) return undefined;

    let disposed = false;
    if (typeof api.getDevLogEntries === 'function') {
      api.getDevLogEntries()
        .then(payloads => {
          if (!disposed) addDevLogPayloads(payloads);
        })
        .catch(err => {
          if (!disposed) addEntry('error', 'dev-log.bridge', ['读取 DEV Logs 失败', err]);
        });
    }

    const unsubscribe = typeof api.onDevLog === 'function'
      ? api.onDevLog(payload => addDevLogPayloads(payload))
      : () => {};

    return () => {
      disposed = true;
      unsubscribe();
    };
  }, [addDevLogPayloads, addEntry, enabled]);

  const summary = useMemo(() => ({
    errorCount: entries.filter(entry => entry.level === 'error').length,
    warningCount: entries.filter(entry => entry.level === 'warn').length,
    totalCount: entries.length
  }), [entries]);

  return {
    entries,
    isOpen,
    clear: () => setEntries([]),
    close: () => setIsOpen(false),
    open: () => setIsOpen(true),
    toggle: () => setIsOpen(prev => !prev),
    ...summary
  };
}
