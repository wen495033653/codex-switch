import { normalizeApiBaseUrlInput } from './appState';
import { getErrorMessage } from './errors';

export const DEFAULT_API_TEST_MODEL = 'gpt-5.5';
export const API_TEST_CACHE_TTL_MS = 60 * 60 * 1000;

const API_TEST_SIGNATURE_VERSION = 'responses-v1';

export function normalizeApiTestModelInput(value) {
  const trimmed = String(value || '').trim();
  return trimmed || DEFAULT_API_TEST_MODEL;
}

export function getApiKeyFingerprint(value) {
  const text = String(value || '');
  let hash = 2166136261;
  for (let i = 0; i < text.length; i += 1) {
    hash ^= text.charCodeAt(i);
    hash = Math.imul(hash, 16777619);
  }
  return `${text.length}:${(hash >>> 0).toString(16)}`;
}

export function getApiTestSignature(baseUrl, apiKey, model) {
  return [
    API_TEST_SIGNATURE_VERSION,
    String(baseUrl || ''),
    getApiKeyFingerprint(apiKey),
    normalizeApiTestModelInput(model)
  ].join('\n');
}

export function normalizeApiTestResults(value) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return {};
  return Object.fromEntries(
    Object.entries(value).filter(([, item]) => item && typeof item === 'object' && !Array.isArray(item))
  );
}

export function isFreshApiTest(test, now = Date.now()) {
  return Boolean(
    test
    && !test.loading
    && Number.isFinite(test.checkedAt)
    && now - test.checkedAt < API_TEST_CACHE_TTL_MS
  );
}

export function getApiLastAvailableAt(test) {
  if (!test) return null;
  if (Number.isFinite(test.lastAvailableAt)) return test.lastAvailableAt;
  if (test.ok && Number.isFinite(test.checkedAt)) return test.checkedAt;
  return null;
}

export function hasFreshSuccessfulApiPrecheck(profile, test, model = DEFAULT_API_TEST_MODEL) {
  const signature = getApiTestSignature(
    profile && profile.base_url,
    profile && profile.api_key,
    model
  );
  return Boolean(test && test.signature === signature && test.ok && isFreshApiTest(test));
}

export function mergeApiTestResult(apiTestResults, profileId, test) {
  return {
    ...normalizeApiTestResults(apiTestResults),
    [profileId]: test
  };
}

export function getApiPrecheckFailureMessage(test, fallback = 'API 预检失败') {
  const message = test && test.message ? String(test.message).trim() : '';
  return message ? `API 预检失败：${message}` : fallback;
}

export async function runApiProfilePrecheck({
  profile,
  profileName,
  model,
  previousTest,
  onUpdate
}) {
  const sourceProfile = profile && typeof profile === 'object' ? profile : {};
  const sourceBaseUrl = sourceProfile.base_url || '';
  const apiKey = sourceProfile.api_key || '';
  const testModel = normalizeApiTestModelInput(model);
  const signature = getApiTestSignature(sourceBaseUrl, apiKey, testModel);
  const startedAt = Date.now();
  const previousLastAvailableAt = getApiLastAvailableAt(previousTest);

  const complete = (test) => {
    const nextTest = {
      signature,
      sourceBaseUrl,
      profileName,
      protocol: 'responses',
      ...test
    };
    if (typeof onUpdate === 'function') onUpdate(nextTest);
    return nextTest;
  };

  let normalizedBaseUrl = '';
  try {
    normalizedBaseUrl = normalizeApiBaseUrlInput(sourceBaseUrl);
  } catch (err) {
    return complete({
      baseUrl: sourceBaseUrl,
      apiKeyPresent: Boolean(apiKey),
      testModel,
      stage: 'input',
      startedAt,
      checkedAt: Date.now(),
      lastAvailableAt: previousLastAvailableAt,
      loading: false,
      ok: false,
      message: getErrorMessage(err, 'API Base URL 格式无效')
    });
  }

  if (!String(apiKey).trim()) {
    return complete({
      baseUrl: normalizedBaseUrl,
      apiKeyPresent: false,
      testModel,
      stage: 'input',
      startedAt,
      checkedAt: Date.now(),
      lastAvailableAt: previousLastAvailableAt,
      loading: false,
      ok: false,
      message: 'API Key 不能为空'
    });
  }

  const loadingTest = {
    baseUrl: normalizedBaseUrl,
    apiKeyPresent: true,
    testModel,
    stage: 'models',
    startedAt,
    lastAvailableAt: previousLastAvailableAt,
    loading: true,
    ok: false,
    message: '正在预检 API'
  };
  if (typeof onUpdate === 'function') {
    onUpdate({
      signature,
      sourceBaseUrl,
      profileName,
      protocol: 'responses',
      ...loadingTest
    });
  }

  try {
    const res = await window.api.testApiBaseUrl({
      baseUrl: normalizedBaseUrl,
      apiKey,
      model: testModel
    });
    const result = res && typeof res === 'object' ? res : {};
    const checkedAt = Date.now();
    return complete({
      ...result,
      baseUrl: normalizedBaseUrl,
      apiKeyPresent: true,
      testModel: result.testModel || testModel,
      startedAt,
      checkedAt,
      lastAvailableAt: result.ok ? checkedAt : previousLastAvailableAt,
      loading: false,
      ok: Boolean(result.ok),
      message: result.message || 'API 预检完成'
    });
  } catch (err) {
    return complete({
      baseUrl: normalizedBaseUrl,
      apiKeyPresent: true,
      testModel,
      stage: 'responses',
      startedAt,
      checkedAt: Date.now(),
      lastAvailableAt: previousLastAvailableAt,
      loading: false,
      ok: false,
      message: getErrorMessage(err, 'API 预检失败')
    });
  }
}
