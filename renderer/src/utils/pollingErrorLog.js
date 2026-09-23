function describeFailure(detail) {
  if (typeof detail === 'string') return detail;
  if (detail && typeof detail.message === 'string') return detail.message;
  try {
    return JSON.stringify(detail);
  } catch {
    return String(detail);
  }
}

// Failure log for a background poll that repeats every few seconds. One line per tick would
// push everything else out of the dev log (160 entries), so it logs the first failure, each
// change of error, and the recovery, always with the consecutive failure count.
export function createPollingErrorLog(command, logger = console) {
  let failures = 0;
  let lastFailure = '';

  return {
    failed(detail) {
      failures += 1;
      const text = describeFailure(detail);
      if (failures === 1 || text !== lastFailure) {
        logger.error(`[${command}] background request failed (consecutive failures: ${failures})`, detail);
      }
      lastFailure = text;
    },
    succeeded() {
      if (failures > 0) {
        logger.info(`[${command}] background request recovered after ${failures} consecutive failures`);
      }
      failures = 0;
      lastFailure = '';
    }
  };
}
