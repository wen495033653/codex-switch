import { DEFAULT_API_TEST_MODEL, getApiLastAvailableAt, normalizeApiTestModelInput } from './apiPrecheck';

export function formatApiCheckTime(value, language) {
  if (!Number.isFinite(value)) return '';
  try {
    return new Intl.DateTimeFormat(language, {
      month: '2-digit',
      day: '2-digit',
      hour: '2-digit',
      minute: '2-digit',
      second: '2-digit',
      hour12: false
    }).format(new Date(value));
  } catch {
    return '';
  }
}

export function getApiCheckTimeText(test, t, language) {
  if (!test) return '';
  const timestamp = Number.isFinite(test.checkedAt) ? test.checkedAt : test.startedAt;
  const formatted = formatApiCheckTime(timestamp, language);
  if (!formatted) return '';
  return test.loading
    ? t('开始时间 {time}', { time: formatted })
    : t('预检时间 {time}', { time: formatted });
}

export function getApiLastAvailableTimeText(test, t, language) {
  const timestamp = getApiLastAvailableAt(test);
  const formatted = formatApiCheckTime(timestamp, language);
  return formatted ? t('上次可用 {time}', { time: formatted }) : '';
}

export function getApiTestModelOptions(test, fallbackModel) {
  const models = Array.isArray(test && test.modelIds)
    ? test.modelIds.map(item => String(item || '').trim()).filter(Boolean)
    : [];
  const uniqueModels = [...new Set(models)];
  const fallback = normalizeApiTestModelInput(fallbackModel);
  if (uniqueModels.length > 0) {
    return uniqueModels.includes(fallback) ? uniqueModels : [fallback, ...uniqueModels];
  }
  return [fallback];
}

export function getApiModelsStatus(test, t) {
  if (!test) {
    return { state: 'idle', text: t('等待 /models') };
  }
  if (test.loading) {
    return { state: 'loading', text: t('正在获取 /models') };
  }
  const response = test.modelsResponse || null;
  if (response && response.error) {
    return { state: 'error', text: t('/models 请求失败') };
  }
  const status = response && Number.isFinite(response.status) ? response.status : null;
  if (status && (status < 200 || status >= 300)) {
    return { state: 'error', text: `/models HTTP ${status}` };
  }
  const models = Array.isArray(test.modelIds)
    ? test.modelIds.map(item => String(item || '').trim()).filter(Boolean)
    : [];
  if (models.length > 0) {
    if (!models.includes(DEFAULT_API_TEST_MODEL) && test.testModel === DEFAULT_API_TEST_MODEL) {
      return { state: 'warning', text: t('/models：{count} 个模型，默认模型不在列表', { count: models.length }) };
    }
    return { state: 'success', text: t('/models：{count} 个模型', { count: models.length }) };
  }
  if (response) {
    return { state: 'warning', text: t('/models 未返回可选模型') };
  }
  return { state: 'idle', text: t('等待 /models') };
}

export function getApiTestState(test) {
  if (!test) return 'idle';
  if (test.loading) return 'loading';
  return test.ok ? 'success' : 'error';
}

export function getApiTestStateLabel(test, t) {
  const state = getApiTestState(test);
  if (state === 'loading') return t('预检中');
  if (state === 'success') return t('可用');
  if (state === 'error') return t('不可用');
  return t('未预检');
}

export function prettyApiTestBody(body) {
  if (typeof body !== 'string' || !body) return '';
  try {
    return JSON.stringify(JSON.parse(body), null, 2);
  } catch {
    return body;
  }
}

export function formatApiTestJson(value) {
  if (value === null || value === undefined) return '';
  return JSON.stringify(value, null, 2);
}

export function getApiTestResponseBody(response, t) {
  if (!response) return '';
  if (typeof response.body === 'string' && response.body) return prettyApiTestBody(response.body);
  if (response.json !== null && response.json !== undefined) return formatApiTestJson(response.json);
  if (response.error) return String(response.error);
  return t('空响应');
}

export function getApiTestResponseStatusLabel(response, t) {
  if (!response) return t('未请求');
  const status = Number.isFinite(response.status) ? response.status : null;
  const statusText = response.statusText ? ` ${response.statusText}` : '';
  return status ? `HTTP ${status}${statusText}` : (response.error ? t('请求失败') : t('无状态'));
}

export function getApiModelsDetailLabel(test, t) {
  const response = test && test.modelsResponse ? test.modelsResponse : null;
  if (!response) return '/models';
  const status = Number.isFinite(response.status) ? response.status : null;
  if (status) return `/models ${status}`;
  if (response.error) return t('/models 失败');
  return t('/models 详情');
}
