const FNV_OFFSET = 0xcbf29ce484222325n;
const FNV_PRIME = 0x100000001b3n;
const U64_MASK = 0xffffffffffffffffn;

function stableHexHash(value) {
  const bytes = new TextEncoder().encode(String(value || ''));
  let hash = FNV_OFFSET;
  for (const byte of bytes) {
    hash ^= BigInt(byte);
    hash = (hash * FNV_PRIME) & U64_MASK;
  }
  return hash.toString(16).padStart(16, '0');
}

export function safeCodexAppPathSegment(value) {
  const source = String(value || '').trim();
  let output = '';
  let lastDash = false;
  for (const char of source) {
    const code = char.charCodeAt(0);
    const isAsciiAlpha =
      (code >= 65 && code <= 90) ||
      (code >= 97 && code <= 122);
    const isAsciiDigit = code >= 48 && code <= 57;
    const next = isAsciiAlpha || isAsciiDigit || char === '_' || char === '-'
      ? char.toLowerCase()
      : '-';
    if (next === '-') {
      if (lastDash) continue;
      lastDash = true;
    } else {
      lastDash = false;
    }
    output += next;
    if (output.length >= 80) break;
  }

  const trimmed = output.replace(/^-+|-+$/g, '');
  return trimmed || `channel-${stableHexHash(source)}`;
}

export function getCodexAppInstanceKey(kind, id) {
  const normalizedKind = String(kind || '').trim();
  const targetId = String(id || '').trim();
  if (!normalizedKind || !targetId) return '';
  return `${normalizedKind}-${safeCodexAppPathSegment(targetId)}`;
}

// The cards only read which instances are running, so that is all the status keeps. Keeping
// it that small lets every poll compare it and skip updates that change nothing.
export function normalizeCodexAppInstanceStatus(result) {
  const rawInstances = Array.isArray(result && result.instances)
    ? result.instances
    : [];
  const runningByKey = {};
  for (const instance of rawInstances) {
    const instanceKey = String(instance && instance.instanceKey || '');
    if (instanceKey && instance.running) runningByKey[instanceKey] = true;
  }
  return { runningByKey };
}

export function markCodexAppInstanceRunning(status, result) {
  const instanceKey = String(result && result.instanceKey || '');
  if (!instanceKey) return status;
  return {
    ...status,
    runningByKey: {
      ...status.runningByKey,
      [instanceKey]: true
    }
  };
}

export function isSameCodexAppInstanceStatus(a, b) {
  const aKeys = Object.keys(a.runningByKey);
  return aKeys.length === Object.keys(b.runningByKey).length
    && aKeys.every(key => b.runningByKey[key] === true);
}
