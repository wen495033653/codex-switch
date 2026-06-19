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

export function normalizeCodexAppInstanceStatus(result) {
  const rawInstances = Array.isArray(result && result.instances)
    ? result.instances
    : [];
  const instances = rawInstances
    .map(instance => ({
      ...instance,
      instanceKey: String(instance && instance.instanceKey || ''),
      targetKey: String(instance && instance.targetKey || ''),
      running: Boolean(instance && instance.running)
    }))
    .filter(instance => instance.instanceKey);
  const runningByKey = {};
  const runningByTargetKey = {};
  const instancesByKey = {};

  for (const instance of instances) {
    instancesByKey[instance.instanceKey] = instance;
    if (!instance.running) continue;
    runningByKey[instance.instanceKey] = true;
    if (instance.targetKey) {
      runningByTargetKey[instance.targetKey] = true;
    }
  }

  return {
    instances,
    instancesByKey,
    runningByKey,
    runningByTargetKey
  };
}

export function markCodexAppInstanceRunning(status, result) {
  const instanceKey = String(result && result.instanceKey || '');
  if (!instanceKey) return status;
  const targetKey = result && result.kind && result.targetId
    ? `${result.kind}:${result.targetId}`
    : '';
  const instance = {
    ...(status.instancesByKey && status.instancesByKey[instanceKey]),
    instanceKey,
    targetKey,
    kind: result.kind || '',
    targetId: result.targetId || '',
    channel: result.channel || '',
    instanceRoot: result.instanceRoot || '',
    codexHome: result.codexHome || '',
    userDataDir: result.userDataDir || '',
    running: true
  };

  return {
    ...status,
    instances: [
      ...(status.instances || []).filter(item => item.instanceKey !== instanceKey),
      instance
    ],
    instancesByKey: {
      ...(status.instancesByKey || {}),
      [instanceKey]: instance
    },
    runningByKey: {
      ...(status.runningByKey || {}),
      [instanceKey]: true
    },
    runningByTargetKey: targetKey
      ? {
          ...(status.runningByTargetKey || {}),
          [targetKey]: true
        }
      : (status.runningByTargetKey || {})
  };
}
